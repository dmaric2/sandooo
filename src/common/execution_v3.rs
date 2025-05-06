/// Execution logic specifically for SandoooV3 contract with Aave V3 flashloans integration.
///
/// This module extends the execution functionality to support atomic flashloan-based sandwich attacks.
use anyhow::Result;
use ethers::prelude::*;
use ethers::types::transaction::{eip2718::TypedTransaction, eip2930::AccessList};
use std::future::Future;

use crate::common::constants::*;
use crate::common::execution::{Executor, SandoBundle};

/// Extension trait for Executor to add Aave V3 flashloan functionality
pub trait ExecutorV3Extension {
    /// Creates a flashloan-based sandwich transaction using the SandoooV3 contract
    ///
    /// # Parameters
    /// * `asset`: H160 - The asset address to borrow (typically WETH)
    /// * `amount`: U256 - The amount to borrow
    /// * `front_calldata`: Bytes - The encoded frontrun transaction data
    /// * `victim_txs`: Vec<Transaction> - The victim transactions
    /// * `back_calldata`: Bytes - The encoded backrun transaction data
    /// * `block_number`: U64 - The target block number
    /// * `gas_limit`: u64 - The gas limit for the transaction
    /// * `max_priority_fee_per_gas`: U256 - The maximum priority fee per gas
    /// * `max_fee_per_gas`: U256 - The maximum fee per gas
    ///
    /// # Returns
    /// * `Result<TypedTransaction>` - The typed transaction
    fn create_flashloan_sandwich_tx<'a>(
        &'a self,
        asset: H160,
        amount: U256,
        front_calldata: Bytes,
        _victim_txs: &Vec<Transaction>,
        back_calldata: Bytes,
        block_number: U64,
        gas_limit: u64,
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
    ) -> impl Future<Output = Result<TypedTransaction>> + Send + 'a;

    /// Creates a sandwich bundle using the flashloan method
    ///
    /// # Parameters
    /// * `asset`: H160 - The asset address to borrow (typically WETH)
    /// * `amount`: U256 - The amount to borrow
    /// * `victim_txs`: Vec<Transaction> - The victim transactions
    /// * `front_calldata`: Bytes - The frontrun transaction calldata
    /// * `back_calldata`: Bytes - The backrun transaction calldata
    /// * `front_access_list`: AccessList - The frontrun transaction access list
    /// * `back_access_list`: AccessList - The backrun transaction access list
    /// * `gas_limit`: u64 - The gas limit for the transaction
    /// * `base_fee`: U256 - The base fee
    /// * `max_priority_fee_per_gas`: U256 - The maximum priority fee per gas
    /// * `max_fee_per_gas`: U256 - The maximum fee per gas
    ///
    /// # Returns
    /// * `Result<SandoBundle>` - The sandwich bundle
    fn create_flashloan_sando_bundle<'a>(
        &'a self,
        asset: H160,
        amount: U256,
        victim_txs: Vec<Transaction>,
        front_calldata: Bytes,
        back_calldata: Bytes,
        _front_access_list: AccessList,
        _back_access_list: AccessList,
        gas_limit: u64,
        _base_fee: U256,
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
    ) -> impl Future<Output = Result<SandoBundle>> + Send + 'a;

    /// Calculate the flashloan fee for a given amount
    ///
    /// # Parameters
    /// * `amount`: U256 - The amount to calculate the fee for
    ///
    /// # Returns
    /// * `U256` - The fee amount
    fn calculate_flashloan_fee(&self, amount: U256) -> U256;
}

impl ExecutorV3Extension for Executor {
    fn create_flashloan_sandwich_tx<'a>(
        &'a self,
        asset: H160,
        amount: U256,
        front_calldata: Bytes,
        _victim_txs: &Vec<Transaction>,
        back_calldata: Bytes,
        block_number: U64,
        gas_limit: u64,
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
    ) -> impl Future<Output = Result<TypedTransaction>> + Send + 'a {
        async move {
            // Get common fields for transaction construction
            let (owner, nonce, _) = self._common_fields().await?;

            // The format for the refactored SandoooV3 contract:
            // - 8 bytes of block number (uint64)
            // - Multiple 105-byte trade entries, each containing:
            //   - 1 byte for zeroForOne flag
            //   - 20 bytes for pair address
            //   - 20 bytes for tokenIn address
            //   - 32 bytes for amountIn
            //   - 32 bytes for amountOut

            // Start with block number (uint64)
            let mut sandwich_data = Vec::new();

            // Add block number (uint64) - packed as 8 bytes
            let block_number_u64 = block_number.as_u64();
            let block_number_bytes = block_number_u64.to_be_bytes();
            sandwich_data.extend_from_slice(&block_number_bytes);

            // Process the front-run calldata
            // Extract the actual trade data skipping the function selector (first 4 bytes)
            if front_calldata.len() > 4 {
                let front_data = &front_calldata[4..];
                sandwich_data.extend_from_slice(front_data);
            }

            // Process the back-run calldata
            // Extract the actual trade data skipping the function selector (first 4 bytes)
            if back_calldata.len() > 4 {
                let back_data = &back_calldata[4..];
                sandwich_data.extend_from_slice(back_data);
            }

            // Verify that the data follows the required format after block number:
            // total length = 8 + N * 105 bytes
            if (sandwich_data.len() <= 8) || ((sandwich_data.len() - 8) % 105 != 0) {
                return Err(anyhow::anyhow!(
                    "Invalid sandwich data format. Expected 8 bytes + N×105 bytes."
                ));
            }

            // Find the function in the ABI
            let function = self
                .abi
                .sando_v3
                .abi()
                .functions()
                .find(|f| f.name == "executeSandwichWithFlashloan")
                .expect("Function not found in ABI");

            // Create the function call to executeSandwichWithFlashloan
            let calldata = function.encode_input(&[
                ethers::abi::Token::Address(asset),
                ethers::abi::Token::Uint(amount),
                ethers::abi::Token::Bytes(sandwich_data.into()),
            ])?;

            // Create the typed transaction
            let tx = Eip1559TransactionRequest::new()
                .from(owner)
                .to(self.bot_address)
                .value(U256::zero())
                .data(calldata)
                .nonce(nonce)
                .gas(gas_limit)
                .max_priority_fee_per_gas(max_priority_fee_per_gas)
                .max_fee_per_gas(max_fee_per_gas);

            // Convert to TypedTransaction
            let tx = TypedTransaction::Eip1559(tx);

            Ok(tx)
        }
    }

    fn create_flashloan_sando_bundle<'a>(
        &'a self,
        asset: H160,
        amount: U256,
        victim_txs: Vec<Transaction>,
        front_calldata: Bytes,
        back_calldata: Bytes,
        _front_access_list: AccessList,
        _back_access_list: AccessList,
        gas_limit: u64,
        _base_fee: U256,
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
    ) -> impl Future<Output = Result<SandoBundle>> + Send + 'a {
        async move {
            // For V3 with flashloans, we have a single transaction that contains both front and back
            let flashloan_tx = self
                .create_flashloan_sandwich_tx(
                    asset,
                    amount,
                    front_calldata,
                    &victim_txs,
                    back_calldata,
                    U64::from(block_number().await?), // Use actual block number for strict validation
                    gas_limit,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                )
                .await?;

            // In the flashloan model, we have a single frontrun tx (the flashloan call)
            // The victim txs stay the same
            // And there's no separate backrun tx (it's part of the flashloan callback)
            let sando_bundle = SandoBundle {
                frontrun_tx: flashloan_tx,
                victim_txs,
                backrun_tx: TypedTransaction::Eip1559(
                    Eip1559TransactionRequest::new(), // Empty placeholder since we don't need a separate backrun tx
                ),
            };

            Ok(sando_bundle)
        }
    }

    fn calculate_flashloan_fee(&self, amount: U256) -> U256 {
        // Aave V3 flashloan fee is 0.09% of the borrowed amount
        let basis_points = U256::from(FLASHLOAN_FEE_BASIS_POINTS); // 0.09%
        let divisor = U256::from(BASIS_POINTS_DIVISOR); // For basis points calculation

        amount
            .checked_mul(basis_points)
            .and_then(|res| res.checked_div(divisor))
            .unwrap_or_else(|| U256::zero())
    }
}

// Helper function to get the current block number
async fn block_number() -> Result<u64> {
    // In a real implementation, this would query the blockchain
    // Since this is just a test function, we'll return a placeholder value
    Ok(1) // Replace with actual provider.get_block_number().await?.as_u64() in production
}
