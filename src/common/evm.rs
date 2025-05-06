/// EVM simulation and transaction modeling utilities for the Sandooo project.
///
/// Provides abstractions for simulating Ethereum transactions, managing state, and interacting with contracts.
use anyhow::{anyhow, Result};
use ethers::prelude::abi;
use ethers::providers::Middleware;
use ethers::types::{transaction::eip2930::AccessList, H160, H256, U256, U64};
use foundry_evm_mini::evm::executor::fork::{BlockchainDb, BlockchainDbMeta, SharedBackend};
use foundry_evm_mini::evm::executor::inspector::{get_precompiles_for, AccessListTracer};
use revm::primitives::bytes::Bytes as rBytes;
use revm::primitives::{Bytes, Log, B160};
use revm::{
    db::{CacheDB, Database},
    primitives::{
        keccak256, AccountInfo, Bytecode, ExecutionResult, Output, TransactTo, B256, U256 as rU256,
    },
    EVM,
};
use std::{collections::BTreeSet, default::Default, str::FromStr, sync::Arc};

use crate::common::abi::Abi;
use crate::common::constants::COINBASE;
use crate::common::utils::{access_list_to_revm, create_new_wallet};
use ethers::abi::parse_abi;
use ethers::prelude::BaseContract;

/// Represents a victim transaction to be simulated or analyzed.
#[derive(Debug, Clone, Default)]
pub struct VictimTx {
    /// The transaction hash.
    pub tx_hash: H256,
    /// The sender address.
    pub from: H160,
    /// The recipient address.
    pub to: H160,
    /// The transaction data.
    pub data: Bytes,
    /// The transaction value.
    pub value: U256,
    /// The transaction gas price.
    pub gas_price: U256,
    /// The transaction gas limit.
    pub gas_limit: Option<u64>,
}

/// Internal transaction representation for EVM simulation.
#[derive(Debug, Clone)]
pub struct Tx {
    /// The caller address.
    pub caller: H160,
    /// The recipient address.
    pub transact_to: H160,
    /// The transaction data.
    pub data: rBytes,
    /// The transaction value.
    pub value: U256,
    /// The transaction gas price.
    pub gas_price: U256,
    /// The transaction gas limit.
    pub gas_limit: u64,
}

impl Tx {
    /// Converts a `VictimTx` into a `Tx` for simulation, defaulting gas_limit if not set.
    ///
    /// # Parameters
    /// * `tx`: VictimTx - The victim transaction to convert.
    ///
    /// # Returns
    /// * `Tx` - Internal transaction representation.
    pub fn from(tx: VictimTx) -> Self {
        let gas_limit = match tx.gas_limit {
            Some(gas_limit) => gas_limit,
            None => 5000000,
        };
        Self {
            caller: tx.from,
            transact_to: tx.to,
            data: tx.data,
            value: tx.value,
            gas_price: tx.gas_price,
            gas_limit,
        }
    }
}

/// Result of a simulated transaction.
#[derive(Debug, Clone)]
pub struct TxResult {
    /// The transaction output.
    pub output: rBytes,
    /// The transaction logs.
    pub logs: Option<Vec<Log>>,
    /// The gas used by the transaction.
    pub gas_used: u64,
    /// The gas refunded by the transaction.
    pub gas_refunded: u64,
}

/// EVM simulator for forking, state manipulation, and contract interaction.
#[derive(Clone)]
pub struct EvmSimulator<M> {
    /// The Ethereum provider.
    pub provider: Arc<M>,
    /// The owner address.
    pub owner: H160,
    /// The EVM instance.
    pub evm: EVM<CacheDB<SharedBackend>>,
    /// The current block number.
    pub block_number: U64,
    /// The ABI instance.
    pub abi: Abi,
}

impl<M: Middleware + 'static> EvmSimulator<M> {
    /// Creates a new `EvmSimulator` with a fresh database forked at a specific block.
    ///
    /// # Parameters
    /// * `provider`: Arc<M> - The Ethereum provider.
    /// * `owner`: Option<H160> - The owner address (random if None).
    /// * `block_number`: U64 - The block number to fork from.
    ///
    /// # Returns
    /// * `EvmSimulator<M>` - Initialized simulator.
    pub fn new(provider: Arc<M>, owner: Option<H160>, block_number: U64) -> Self {
        let shared_backend = SharedBackend::spawn_backend_thread(
            provider.clone(),
            BlockchainDb::new(
                BlockchainDbMeta {
                    cfg_env: Default::default(),
                    block_env: Default::default(),
                    hosts: BTreeSet::from(["".to_string()]),
                },
                None,
            ),
            Some(block_number.into()),
        );
        let db = CacheDB::new(shared_backend);
        EvmSimulator::new_with_db(provider, owner, block_number, db)
    }

    /// Creates a new `EvmSimulator` using an existing state database.
    ///
    /// # Parameters
    /// * `provider`: Arc<M> - The Ethereum provider.
    /// * `owner`: Option<H160> - The owner address (random if None).
    /// * `block_number`: U64 - The block number to fork from.
    /// * `db`: CacheDB<SharedBackend> - The EVM state database.
    ///
    /// # Returns
    /// * `EvmSimulator<M>` - Initialized simulator.
    pub fn new_with_db(
        provider: Arc<M>,
        owner: Option<H160>,
        block_number: U64,
        db: CacheDB<SharedBackend>,
    ) -> Self {
        let owner = match owner {
            Some(owner) => owner,
            None => create_new_wallet().1,
        };

        let mut evm = EVM::new();
        evm.database(db);

        evm.env.block.number = rU256::from(block_number.as_u64() + 1);
        evm.env.block.coinbase = H160::from_str(COINBASE).unwrap().into();

        Self {
            provider,
            owner,
            evm,
            block_number,
            abi: Abi::new(),
        }
    }

    /// Clones the current EVM state database.
    ///
    /// # Returns
    /// * `CacheDB<SharedBackend>` - A clone of the current EVM state.
    pub fn clone_db(&mut self) -> CacheDB<SharedBackend> {
        self.evm.db.as_mut().unwrap().clone()
    }

    /// Replaces the current EVM state with a provided database.
    ///
    /// # Parameters
    /// * `db`: CacheDB<SharedBackend> - The new EVM state database.
    pub fn insert_db(&mut self, db: CacheDB<SharedBackend>) {
        let mut evm = EVM::new();
        evm.database(db);

        self.evm = evm;
    }

    /// Returns the current simulated block number.
    pub fn get_block_number(&mut self) -> U256 {
        self.evm.env.block.number.into()
    }

    /// Returns the coinbase address for the current block.
    pub fn get_coinbase(&mut self) -> H160 {
        self.evm.env.block.coinbase.into()
    }

    /// Returns the current base fee for the simulated block.
    pub fn get_base_fee(&mut self) -> U256 {
        self.evm.env.block.basefee.into()
    }

    /// Sets the base fee for the simulated block.
    pub fn set_base_fee(&mut self, base_fee: U256) {
        self.evm.env.block.basefee = base_fee.into();
    }

    /// Computes the access list for a transaction.
    ///
    /// # Parameters
    /// * `tx`: Tx - The transaction for which to compute the access list.
    ///
    /// # Returns
    /// * `Result<AccessList>` - The computed access list or error.
    pub fn get_access_list(&mut self, tx: Tx) -> Result<AccessList> {
        self.evm.env.tx.caller = tx.caller.into();
        self.evm.env.tx.transact_to = TransactTo::Call(tx.transact_to.into());
        self.evm.env.tx.data = tx.data;
        self.evm.env.tx.value = tx.value.into();
        self.evm.env.tx.gas_price = tx.gas_price.into();
        self.evm.env.tx.gas_limit = tx.gas_limit;
        let mut access_list_tracer = AccessListTracer::new(
            Default::default(),
            tx.caller.into(),
            tx.transact_to.into(),
            get_precompiles_for(self.evm.env.cfg.spec_id),
        );
        let access_list = match self.evm.inspect_ref(&mut access_list_tracer) {
            Ok(_) => access_list_tracer.access_list(),
            Err(_) => AccessList::default(),
        };
        Ok(access_list)
    }

    pub fn set_access_list(&mut self, access_list: AccessList) {
        self.evm.env.tx.access_list = access_list_to_revm(access_list);
    }

    /// Executes a staticcall simulation for the given transaction.
    ///
    /// # Parameters
    /// * `tx`: Tx - The transaction to simulate.
    ///
    /// # Returns
    /// * `Result<TxResult>` - The result of the simulation or error.
    pub fn staticcall(&mut self, tx: Tx) -> Result<TxResult> {
        self._call(tx, false)
    }

    /// Executes a call simulation for the given transaction, committing state changes.
    ///
    /// # Parameters
    /// * `tx`: Tx - The transaction to simulate.
    ///
    /// # Returns
    /// * `Result<TxResult>` - The result of the simulation or error.
    pub fn call(&mut self, tx: Tx) -> Result<TxResult> {
        self._call(tx, true)
    }

    pub fn _call(&mut self, tx: Tx, commit: bool) -> Result<TxResult> {
        self.evm.env.tx.caller = tx.caller.into();
        self.evm.env.tx.transact_to = TransactTo::Call(tx.transact_to.into());
        self.evm.env.tx.data = tx.data;
        self.evm.env.tx.value = tx.value.into();
        self.evm.env.tx.gas_price = tx.gas_price.into();
        self.evm.env.tx.gas_limit = tx.gas_limit;

        let result;

        if commit {
            result = match self.evm.transact_commit() {
                Ok(result) => result,
                Err(e) => return Err(anyhow!("EVM call failed: {:?}", e)),
            };
        } else {
            let ref_tx = self
                .evm
                .transact_ref()
                .map_err(|e| anyhow!("EVM staticcall failed: {:?}", e))?;
            result = ref_tx.result;
        }

        let output = match result {
            ExecutionResult::Success {
                gas_used,
                gas_refunded,
                output,
                logs,
                ..
            } => match output {
                Output::Call(o) => TxResult {
                    output: o,
                    logs: Some(logs),
                    gas_used,
                    gas_refunded,
                },
                Output::Create(o, _) => TxResult {
                    output: o,
                    logs: Some(logs),
                    gas_used,
                    gas_refunded,
                },
            },
            ExecutionResult::Revert { gas_used, output } => {
                return Err(anyhow!(
                    "EVM REVERT: {:?} / Gas used: {:?}",
                    output,
                    gas_used
                ))
            }
            ExecutionResult::Halt { reason, .. } => return Err(anyhow!("EVM HALT: {:?}", reason)),
        };

        Ok(output)
    }

    pub fn basic(&mut self, target: H160) -> Result<Option<AccountInfo>> {
        self.evm
            .db
            .as_mut()
            .unwrap()
            .basic(target.into())
            .map_err(|e| anyhow!("Basic error: {e:?}"))
    }

    /// Inserts account info into the EVM state for a given address.
    ///
    /// # Parameters
    /// * `target`: H160 - The account address.
    /// * `account_info`: AccountInfo - The account info to insert.
    pub fn insert_account_info(&mut self, target: H160, account_info: AccountInfo) {
        self.evm
            .db
            .as_mut()
            .unwrap()
            .insert_account_info(target.into(), account_info);
    }

    /// Inserts a storage slot value for an account in the EVM state.
    ///
    /// # Parameters
    /// * `target`: H160 - The account address.
    /// * `slot`: rU256 - The storage slot.
    /// * `value`: rU256 - The value to set.
    ///
    /// # Returns
    /// * `Result<()>` - Ok if successful, error otherwise.
    pub fn insert_account_storage(
        &mut self,
        target: H160,
        slot: rU256,
        value: rU256,
    ) -> Result<()> {
        self.evm
            .db
            .as_mut()
            .unwrap()
            .insert_account_storage(target.into(), slot, value)?;
        Ok(())
    }

    /// Deploys a contract by inserting its bytecode into the EVM state for a given address.
    ///
    /// # Parameters
    /// * `target`: H160 - The contract address.
    /// * `bytecode`: Bytecode - The contract bytecode.
    pub fn deploy(&mut self, target: H160, bytecode: Bytecode) {
        let contract_info = AccountInfo::new(rU256::ZERO, 0, B256::zero(), bytecode);
        self.insert_account_info(target, contract_info);
    }

    /// Returns the ETH balance of an address in the EVM state.
    ///
    /// # Parameters
    /// * `target`: H160 - The address to query.
    ///
    /// # Returns
    /// * `U256` - ETH balance.
    pub fn get_eth_balance_of(&mut self, target: H160) -> U256 {
        let acc = self.basic(target).unwrap().unwrap();
        acc.balance.into()
    }

    /// Sets the ETH balance of an address in the EVM state.
    ///
    /// # Parameters
    /// * `target`: H160 - The address to set.
    /// * `amount`: U256 - The new ETH balance.
    pub fn set_eth_balance(&mut self, target: H160, amount: U256) {
        let user_balance = amount.into();
        let user_info = AccountInfo::new(user_balance, 0, B256::zero(), Bytecode::default());
        self.insert_account_info(target.into(), user_info);
    }

    /// Returns the token balance of an address for a given ERC-20 token contract.
    ///
    /// # Parameters
    /// * `token_address`: H160 - The token contract address.
    /// * `owner`: H160 - The owner address.
    ///
    /// # Returns
    /// * `Result<U256>` - The token balance or error.
    pub fn get_token_balance(&mut self, token_address: H160, owner: H160) -> Result<U256> {
        let calldata = self.abi.token.encode("balanceOf", owner)?;
        let value = self.staticcall(Tx {
            caller: self.owner,
            transact_to: token_address,
            data: calldata.0,
            value: U256::zero(),
            gas_price: U256::zero(),
            gas_limit: 5000000,
        })?;
        let out = self.abi.token.decode_output("balanceOf", value.output)?;
        Ok(out)
    }

    /// Sets the token balance of an address for a given ERC-20 token contract.
    ///
    /// # Parameters
    /// * `token_address`: H160 - The token contract address.
    /// * `to`: H160 - The owner address.
    /// * `slot`: i32 - The storage slot for the balance.
    /// * `amount`: rU256 - The new token balance.
    ///
    /// # Returns
    /// * `Result<()>` - Ok if successful, error otherwise.
    pub fn set_token_balance(
        &mut self,
        token_address: H160,
        to: H160,
        slot: i32,
        amount: rU256,
    ) -> Result<()> {
        let balance_slot = keccak256(&abi::encode(&[
            abi::Token::Address(to.into()),
            abi::Token::Uint(U256::from(slot)),
        ]));
        self.insert_account_storage(token_address, balance_slot.into(), amount)?;
        Ok(())
    }

    /// Returns the reserves of a Uniswap V2-like pair contract.
    ///
    /// # Parameters
    /// * `pair_address`: H160 - The pair contract address.
    ///
    /// # Returns
    /// * `Result<(U256, U256)>` - (reserve0, reserve1) or error.
    pub fn get_pair_reserves(&mut self, pair_address: H160) -> Result<(U256, U256)> {
        let calldata = self.abi.pair.encode("getReserves", ())?;
        let value = self.staticcall(Tx {
            caller: self.owner,
            transact_to: pair_address,
            data: calldata.0,
            value: U256::zero(),
            gas_price: U256::zero(),
            gas_limit: 5000000,
        })?;
        let out: (U256, U256, U256) = self.abi.pair.decode_output("getReserves", value.output)?;
        Ok((out.0, out.1))
    }

    /// Returns effective reserves of a Uniswap V3 pool by deriving from slot0 and liquidity.
    ///
    /// # Parameters
    /// * `pool_address`: H160 - The pool contract address.
    ///
    /// # Returns
    /// * `Result<(U256, U256)>` - (reserve0, reserve1) or error.
    pub fn get_v3_pool_reserves(&mut self, pool_address: H160) -> Result<(U256, U256)> {
        // Minimal V3 pool ABI: slot0 and liquidity
        let v3_abi = BaseContract::from(
            parse_abi(&[
                "function slot0() external view returns (uint160 sqrtPriceX96,int24 tick,uint16 observationIndex,uint16 observationCardinality,uint16 observationCardinalityNext,uint8 feeProtocol,bool unlocked)",
                "function liquidity() external view returns (uint128)"
            ])?
        );
        // Fetch slot0
        let slot0_calldata = v3_abi.encode("slot0", ())?.0;
        let slot0_res = self.staticcall(Tx {
            caller: self.owner,
            transact_to: pool_address,
            data: slot0_calldata.clone(),
            value: U256::zero(),
            gas_price: U256::zero(),
            gas_limit: 5000000,
        })?;
        let (sqrt_price_x96, _tick, _obs_i, _obs_c, _obs_c_next, _fee_p, _unlocked): (U256, i128, U256, U256, U256, U256, bool) =
            v3_abi.decode_output("slot0", slot0_res.output)?;
        // Fetch liquidity
        let liq_calldata = v3_abi.encode("liquidity", ())?.0;
        let liq_res = self.staticcall(Tx {
            caller: self.owner,
            transact_to: pool_address,
            data: liq_calldata,
            value: U256::zero(),
            gas_price: U256::zero(),
            gas_limit: 5000000,
        })?;
        let (liquidity,): (U256,) = v3_abi.decode_output("liquidity", liq_res.output)?;
        // Compute reserves from price and liquidity
        let q96 = U256::one() << 96;
        let reserve0 = liquidity.checked_mul(q96).unwrap_or_default()
            .checked_div(sqrt_price_x96).unwrap_or_default();
        let reserve1 = liquidity.checked_mul(sqrt_price_x96).unwrap_or_default()
            .checked_div(q96).unwrap_or_default();
        Ok((reserve0, reserve1))
    }

    /// Attempts to find the storage slot for the balance of a given ERC-20 token contract.
    ///
    /// # Parameters
    /// * `token_address`: H160 - The token contract address.
    ///
    /// # Returns
    /// * `Result<i32>` - The balance slot if found, or -1 if not found.
    pub fn get_balance_slot(&mut self, token_address: H160) -> Result<i32> {
        let calldata = self.abi.token.encode("balanceOf", token_address)?;
        self.evm.env.tx.caller = self.owner.into();
        self.evm.env.tx.transact_to = TransactTo::Call(token_address.into());
        self.evm.env.tx.data = calldata.0;
        let result = match self.evm.transact_ref() {
            Ok(result) => result,
            Err(e) => return Err(anyhow!("EVM ref call failed: {e:?}")),
        };
        let token_b160: B160 = token_address.into();
        let token_acc = result.state.get(&token_b160).unwrap();
        let token_touched_storage = token_acc.storage.clone();
        for i in 0..30 {
            let slot = keccak256(&abi::encode(&[
                abi::Token::Address(token_address.into()),
                abi::Token::Uint(U256::from(i)),
            ]));
            let slot: rU256 = U256::from(slot).into();
            match token_touched_storage.get(&slot) {
                Some(_) => {
                    return Ok(i);
                }
                None => {}
            }
        }

        Ok(-1)
    }
}
