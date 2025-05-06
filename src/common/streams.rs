/// Streaming utilities for real-time Ethereum block and transaction events.
///
/// Provides types and async functions to stream new blocks and pending transactions using ethers-rs.
use anyhow::Result;
use ethers::{
    providers::{Middleware, Provider, Ws},
    types::{Transaction, U256, U64},
};
use futures::StreamExt;
use log::info;
use std::sync::Arc;
use tokio::sync::broadcast::Sender;

/// Represents a new block event.
#[derive(Debug, Clone)]
pub struct NewBlock {
    /// The block number.
    pub block_number: U64,
    /// The base fee per gas.
    pub base_fee: U256,
    /// The estimated next block base fee.
    pub next_base_fee: U256,
}

/// Represents a new pending transaction event.
#[derive(Debug, Clone, Default)]
pub struct NewPendingTx {
    /// The transaction.
    pub tx: Transaction,
    /// The block when this transaction was added.
    pub added_block: Option<U64>,
}

/// Events that can be streamed.
#[derive(Debug, Clone)]
pub enum Event {
    /// New block event.
    Block(NewBlock),
    /// New pending transaction event.
    PendingTransaction(NewPendingTx),
}

/// Streams new blocks and sends them to the event channel.
///
/// # Arguments
/// * `provider` - An Ethereum provider.
/// * `tx` - A channel to send events.
///
/// # Returns
/// * `Result<()>` - Ok if successful, Error otherwise.
pub async fn stream_new_blocks(
    provider: Arc<Provider<Ws>>,
    tx: Sender<Event>,
) -> Result<(), anyhow::Error> {
    eprintln!("DEBUG: Starting block subscription...");

    // Use the same approach as the original implementation but with proper error handling
    let stream = match provider.subscribe_blocks().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("DEBUG: Failed to subscribe to blocks: {:?}", e);
            return Err(anyhow::anyhow!("Failed to subscribe to blocks: {:?}", e));
        }
    };

    eprintln!("DEBUG: Successfully subscribed to blocks");
    let mut stream = stream;

    while let Some(block) = stream.next().await {
        eprintln!(
            "DEBUG: Received new block #{}",
            block.number.unwrap_or_default()
        );

        // Only process blocks that have a number
        if let Some(number) = block.number {
            let next_base_fee = crate::common::utils::calculate_next_block_base_fee(
                block.gas_used,
                block.gas_limit,
                block.base_fee_per_gas.unwrap_or_default(),
            );

            let new_block = NewBlock {
                block_number: number,
                base_fee: block.base_fee_per_gas.unwrap_or_default(),
                next_base_fee,
            };

            if tx.send(Event::Block(new_block)).is_err() {
                eprintln!("DEBUG: Failed to send block event for #{}", number);
                info!("Failed to send block event");
            } else {
                eprintln!("DEBUG: Successfully sent block event for #{}", number);
            }
        }
    }

    eprintln!("DEBUG: Block stream ended unexpectedly");
    Ok(())
}

/// Stream pending transactions from the Ethereum network.
///
/// Listens for new pending transactions from the provider and sends them to the provided channel.
///
/// # Arguments
/// * `provider` - An Ethereum provider.
/// * `tx` - A channel to send events.
///
/// # Returns
/// * `Result<()>` - Ok if successful, Error otherwise.
pub async fn stream_pending_txs(
    provider: Arc<Provider<Ws>>,
    tx: Sender<Event>,
) -> Result<(), anyhow::Error> {
    eprintln!("DEBUG: Starting pending transaction subscription...");

    // Subscribe to pending transactions
    let mut pending_txs = match provider.subscribe_pending_txs().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "DEBUG: Failed to subscribe to pending transactions: {:?}",
                e
            );
            return Err(anyhow::anyhow!(
                "Failed to subscribe to pending transactions: {:?}",
                e
            ));
        }
    };

    eprintln!("DEBUG: Successfully subscribed to pending transactions");

    // Process each hash and get the full transaction
    while let Some(hash) = pending_txs.next().await {
        eprintln!("DEBUG: Received pending transaction hash: {:?}", hash);

        // Get the full transaction details - this might fail if the transaction is no longer in the mempool
        match provider.get_transaction(hash).await {
            Ok(Some(transaction)) => {
                eprintln!(
                    "DEBUG: Successfully retrieved transaction details for hash {:?}",
                    hash
                );

                // Skip pure ETH transfers and ERC20 approve/transfer
                let kind = crate::common::classifier::classify_transaction(&provider, &transaction).await;
                if kind == crate::common::classifier::TxKind::EthTransfer
                    || kind == crate::common::classifier::TxKind::Erc20Approve
                    || kind == crate::common::classifier::TxKind::Erc20Transfer
                {
                    eprintln!("DEBUG: Skipping tx {:?} as {:?}", hash, kind);
                    continue;
                }

                let pending_tx = NewPendingTx {
                    tx: transaction,
                    added_block: None,
                };

                if tx.send(Event::PendingTransaction(pending_tx)).is_err() {
                    eprintln!("DEBUG: Failed to send pending tx event");
                    info!("Failed to send pending tx event");
                } else {
                    eprintln!(
                        "DEBUG: Successfully sent pending tx event for hash {:?}",
                        hash
                    );
                }
            }
            Ok(None) => {
                eprintln!("DEBUG: Transaction not found for hash {:?}", hash);
            }
            Err(e) => {
                eprintln!("DEBUG: Error retrieving transaction: {:?}, continuing", e);
                // Just log and continue, don't fail the entire stream
            }
        }
    }

    eprintln!("DEBUG: Pending transaction stream ended unexpectedly");
    Ok(())
}
