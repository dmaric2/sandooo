/// Sandwich attack strategy orchestration.
///
/// Coordinates the full sandwich attack pipeline: pool/token discovery, event handling, opportunity detection, and bundle execution.
use bounded_vec_deque::BoundedVecDeque;
use ethers::signers::{LocalWallet, Signer};
use ethers::{
    providers::{Middleware, Provider, Ws},
    types::{BlockNumber, H160, H256, U256, U64},
};
use log::{error, info};
use std::{collections::HashMap, str::FromStr, sync::Arc};
use tokio::sync::broadcast::Sender;
use tokio::time::Duration;
use ethers::types::Filter;
use ethers::abi::{decode as abi_decode, ParamType};
use futures::StreamExt;

use crate::common::alert::Alert;
use crate::common::constants::Env;
use crate::common::execution::Executor;
use crate::common::pools::{load_all_pools, Pool};
use crate::common::streams::{Event, NewBlock};
use crate::common::tokens::load_all_tokens;
use crate::common::utils::calculate_next_block_base_fee;
use crate::sandwich::appetizer::appetizer;
use crate::sandwich::main_dish::main_dish;
use crate::sandwich::main_dish_v3::main_dish_v3;
use crate::sandwich::simulation::{extract_swap_info, PendingTxInfo, Sandwich};

/// Sandwich execution mode selection
pub enum SandwichMode {
    /// Traditional sandwich with separate front-run and back-run transactions
    Traditional,
    /// Aave V3 flashloan-based sandwich with atomic execution
    FlashloanV3,
}

/// Runs the full sandwich attack strategy event loop.
///
/// Handles:
/// - Loading pools and tokens
/// - Filtering and updating pending transactions
/// - Detecting sandwich opportunities
/// - Executing and submitting bundles
///
/// # Parameters
/// * `provider`: Ethereum provider.
/// * `event_sender`: Channel to receive block and transaction events.
pub async fn run_sandwich_strategy(provider: Arc<Provider<Ws>>, event_sender: Sender<Event>) -> ! {
    // Add debugging logs
    eprintln!("DEBUG: Initializing sandwich strategy");

    let env = Env::new();
    eprintln!("DEBUG: Environment loaded. WSS URL: {}", env.wss_url);

    // Determine sandwich mode - could be loaded from environment or config
    // For now, we'll use Flashloan V3 as the default
    let sandwich_mode = SandwichMode::FlashloanV3;

    info!(
        "Starting sandwich strategy with mode: {}",
        match sandwich_mode {
            SandwichMode::Traditional => "Traditional",
            SandwichMode::FlashloanV3 => "Aave V3 Flashloan",
        }
    );

    eprintln!("DEBUG: About to load pools. Start block: 22413000, Chunk: 50000");

    let (pools, prev_pool_id) = load_all_pools(env.wss_url.clone(), 22413000, 50000)
        .await
        .unwrap();

    eprintln!(
        "DEBUG: Pools loaded successfully. Count: {}. prev_pool_id: {}",
        pools.len(),
        prev_pool_id
    );

    eprintln!("DEBUG: Getting current block number");
    let block_number = provider.get_block_number().await.unwrap();
    eprintln!("DEBUG: Current block number: {}", block_number);

    eprintln!("DEBUG: Loading tokens for pools");
    let tokens_map = load_all_tokens(&provider, block_number, &pools, prev_pool_id)
        .await
        .unwrap();
    eprintln!(
        "DEBUG: Tokens loaded successfully. Count: {}",
        tokens_map.len()
    );
    info!("Tokens map count: {:?}", tokens_map.len());

    // filter pools that don't have both token0 / token1 info
    eprintln!("DEBUG: Filtering pools based on token existence");
    let pools_vec: Vec<Pool> = pools
        .into_iter()
        .filter(|p| {
            let token0_exists = tokens_map.contains_key(&p.token0);
            let token1_exists = tokens_map.contains_key(&p.token1);
            token0_exists && token1_exists
        })
        .collect();
    eprintln!("DEBUG: Filtered pools count: {}", pools_vec.len());
    info!("Filtered pools by tokens count: {:?}", pools_vec.len());

    eprintln!("DEBUG: Creating pools map");
    let pools_map: HashMap<H160, Pool> = pools_vec
        .clone()
        .into_iter()
        .map(|p| (p.address, p))
        .collect();
    eprintln!("DEBUG: Pools map created with {} entries", pools_map.len());

    // Subscribe to Sync events to update pool reserves
    eprintln!("DEBUG: Subscribing to Sync events for pool reserves");
    let sync_addresses = pools_map.keys().cloned().collect::<Vec<H160>>();
    let sync_filter = Filter::new().event("Sync(uint112,uint112)").address(sync_addresses);

    // Spawn a task to subscribe to Sync events without borrowing `provider`
    let provider_clone = provider.clone();
    let filter_clone = sync_filter.clone();
    tokio::spawn(async move {
        let mut sync_stream = provider_clone.subscribe_logs(&filter_clone).await.unwrap();
        let mut pool_reserves: HashMap<H160, (U256, U256)> = HashMap::new();
        while let Some(log) = sync_stream.next().await {
            if let Ok(tokens) = abi_decode(&[ParamType::Uint(112), ParamType::Uint(112)], &log.data) {
                let r0 = tokens[0].clone().into_uint().unwrap();
                let r1 = tokens[1].clone().into_uint().unwrap();
                pool_reserves.insert(log.address, (r0, r1));
            }
        }
    });

    eprintln!("DEBUG: Getting latest block");
    let block = provider
        .get_block(BlockNumber::Latest)
        .await
        .unwrap()
        .unwrap();
    eprintln!("DEBUG: Latest block retrieved: #{}", block.number.unwrap());

    eprintln!("DEBUG: Creating new block structure");
    let mut new_block = NewBlock {
        block_number: block.number.unwrap(),
        base_fee: block.base_fee_per_gas.unwrap(),
        next_base_fee: calculate_next_block_base_fee(
            block.gas_used,
            block.gas_limit,
            block.base_fee_per_gas.unwrap(),
        ),
    };
    eprintln!(
        "DEBUG: New block created with next base fee: {}",
        new_block.next_base_fee
    );

    eprintln!("DEBUG: Initializing alert system");
    let alert = Alert::new();
    eprintln!("DEBUG: Initializing executor");
    let executor = Executor::new(provider.clone());

    eprintln!("DEBUG: Setting up wallet and addresses");
    let bot_address = H160::from_str(&env.bot_address).unwrap();
    let wallet = env
        .private_key
        .parse::<LocalWallet>()
        .unwrap()
        .with_chain_id(1 as u64);
    let owner = wallet.address();
    eprintln!("DEBUG: Bot address: {:?}, Owner: {:?}", bot_address, owner);

    eprintln!("DEBUG: Creating event subscription");
    let mut event_receiver = event_sender.subscribe();

    eprintln!("DEBUG: Initializing data structures");
    let mut pending_txs: HashMap<H256, PendingTxInfo> = HashMap::new();
    let mut promising_sandwiches: HashMap<H256, Vec<Sandwich>> = HashMap::new();
    let mut simulated_bundle_ids = BoundedVecDeque::new(30);

    eprintln!("DEBUG: Initialization complete, entering main loop");

    let _last_block_number = U64::zero();

    loop {
        eprintln!("DEBUG: Waiting for next event...");
        match event_receiver.recv().await {
            Ok(event) => match event {
                Event::Block(block) => {
                    eprintln!(
                        "DEBUG: Received Block event for block #{}",
                        block.block_number
                    );
                    new_block = block;
                    info!("[Block #{:?}]", new_block.block_number);

                    // remove confirmed transactions
                    eprintln!("DEBUG: Fetching block with transactions");
                    let block_with_txs = provider
                        .get_block_with_txs(new_block.block_number)
                        .await
                        .unwrap()
                        .unwrap();

                    let txs: Vec<H256> = block_with_txs
                        .transactions
                        .into_iter()
                        .map(|tx| tx.hash)
                        .collect();
                    eprintln!("DEBUG: Block contained {} transactions", txs.len());

                    let mut removed_count = 0;
                    for tx_hash in &txs {
                        if pending_txs.contains_key(tx_hash) {
                            let _ = pending_txs.remove(tx_hash); // Ignoring the removed value
                            removed_count += 1;
                        }
                    }
                    eprintln!(
                        "DEBUG: Removed {} confirmed transactions, {} pending remaining",
                        removed_count,
                        pending_txs.len()
                    );

                    // CRITICAL: Remove pending txs older than 3 blocks
                    // This was missing and causing the bot to get stuck
                    eprintln!("DEBUG: Cleaning up old pending transactions");
                    let old_count = pending_txs.len();
                    pending_txs.retain(|_, v| {
                        v.pending_tx
                            .added_block
                            .map_or(true, |block| {
                                (new_block.block_number - block) < U64::from(3)
                            })
                    });
                    promising_sandwiches.retain(|h, _| pending_txs.contains_key(h));

                    eprintln!(
                        "DEBUG: Removed {} old pending transactions, {} pending remaining",
                        old_count - pending_txs.len(),
                        pending_txs.len()
                    );
                }
                Event::PendingTransaction(mut pending_tx) => {
                    eprintln!(
                        "DEBUG: Received PendingTransaction event: {:?}",
                        pending_tx.tx.hash
                    );
                    let tx_hash = pending_tx.tx.hash;
                    let mut should_add = false;

                    // Check if tx is already mined
                    eprintln!("DEBUG: Checking if transaction is already mined");
                    match provider.get_transaction_receipt(tx_hash).await {
                        Ok(receipt) => {
                            match receipt {
                                Some(_) => {
                                    eprintln!("DEBUG: Transaction already mined, removing");
                                    // returning a receipt means that the tx is confirmed
                                    // should not be in pending_txs
                                    pending_txs.remove(&tx_hash);
                                }
                                None => {
                                    eprintln!("DEBUG: Transaction not yet mined");
                                    should_add = true;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("DEBUG: Error checking transaction receipt: {:?}", e);
                        }
                    }

                    let mut victim_gas_price = U256::zero();

                    match pending_tx.tx.transaction_type {
                        Some(tx_type) => {
                            if tx_type == U64::zero() {
                                victim_gas_price = pending_tx.tx.gas_price.unwrap_or_default();
                                should_add = victim_gas_price >= new_block.base_fee;
                                eprintln!("DEBUG: Legacy transaction type (0), gas price: {}, should_add: {}", 
                                          victim_gas_price, should_add);
                            } else if tx_type == U64::from(2) {
                                victim_gas_price =
                                    pending_tx.tx.max_fee_per_gas.unwrap_or_default();
                                should_add = victim_gas_price >= new_block.base_fee;
                                eprintln!("DEBUG: EIP-1559 transaction type (2), max fee: {}, should_add: {}", 
                                          victim_gas_price, should_add);
                            }
                        }
                        _ => {}
                    }

                    eprintln!(
                        "DEBUG: Checking for swap in transaction (should_add={})",
                        should_add
                    );

                    // TEMPORARY DEBUG MODE: Force check for swaps even when should_add is false
                    // This helps us test the swap detection logic without being limited by gas filters
                    let force_check = true; // Set to true to force checking all transactions

                    let swap_info = if should_add || force_check {
                        match extract_swap_info(&provider, &new_block, &pending_tx, &pools_map)
                            .await
                        {
                            Ok(swap_info) => {
                                eprintln!(
                                    "DEBUG: extract_swap_info returned {} items",
                                    swap_info.len()
                                );
                                swap_info
                            }
                            Err(e) => {
                                eprintln!("DEBUG: extract_swap_info error: {:?}", e);
                                error!("extract_swap_info error: {e:?}");
                                Vec::new()
                            }
                        }
                    } else {
                        eprintln!("DEBUG: Skipping swap check as should_add=false");
                        Vec::new()
                    };

                    if swap_info.len() > 0 {
                        eprintln!("DEBUG: Found {} swap pairs in transaction", swap_info.len());
                        pending_tx.added_block = Some(new_block.block_number);
                        let pending_tx_info = PendingTxInfo {
                            pending_tx: pending_tx.clone(),
                            touched_pairs: swap_info.clone(),
                        };
                        pending_txs.insert(tx_hash, pending_tx_info.clone());
                        eprintln!(
                            "DEBUG: Added transaction to pending_txs, count now: {}",
                            pending_txs.len()
                        );
                        // info!(
                        //     "🔴 V{:?} TX ADDED: {:?} / Pending txs: {:?}",
                        //     pending_tx_info.touched_pairs.get(0).unwrap().version,
                        //     tx_hash,
                        //     pending_txs.len()
                        // );

                        eprintln!("DEBUG: Calling appetizer for potential sandwich opportunity");
                        match appetizer(
                            &provider,
                            &new_block,
                            tx_hash,
                            victim_gas_price,
                            &pending_txs,
                            &mut promising_sandwiches,
                        )
                        .await
                        {
                            Ok(_) => {
                                eprintln!("DEBUG: appetizer returned successfully, promising sandwiches: {}", 
                                          promising_sandwiches.len());
                            }
                            Err(e) => {
                                eprintln!("DEBUG: appetizer error: {:?}", e);
                                error!("appetizer error: {e:?}");
                            }
                        }

                        if promising_sandwiches.len() > 0 {
                            eprintln!(
                                "DEBUG: Found {} promising sandwiches, executing...",
                                promising_sandwiches.len()
                            );
                            // Choose execution strategy based on sandwich mode
                            match sandwich_mode {
                                SandwichMode::Traditional => {
                                    match main_dish(
                                        &provider,
                                        &alert,
                                        &executor,
                                        &new_block,
                                        owner,
                                        bot_address,
                                        U256::from(9900), // 99%
                                        &promising_sandwiches,
                                        &mut simulated_bundle_ids,
                                        &pending_txs,
                                    )
                                    .await
                                    {
                                        Err(e) => {
                                            eprintln!("DEBUG: main_dish error: {:?}", e);
                                            error!("main_dish error: {e:?}");
                                        }
                                        _ => {}
                                    }
                                }
                                SandwichMode::FlashloanV3 => {
                                    match main_dish_v3(
                                        &provider,
                                        &alert,
                                        &executor,
                                        &new_block,
                                        owner,
                                        bot_address,
                                        U256::from(9900), // 99%
                                        &promising_sandwiches,
                                        &mut simulated_bundle_ids,
                                        &pending_txs,
                                    )
                                    .await
                                    {
                                        Err(e) => {
                                            eprintln!("DEBUG: main_dish_v3 error: {:?}", e);
                                            error!("main_dish_v3 error: {e:?}");
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            },
            _ => {}
        }

        // Check for transactions that need to be flushed (too old)
        let current_block = new_block.block_number;
        let cutoff_block = if current_block > U64::from(5) {
            current_block - U64::from(5)
        } else {
            U64::zero()
        };

        // Filter out old transactions
        let old_txs: Vec<H256> = pending_txs
            .iter()
            .filter(|(_, v)| {
                v.pending_tx
                    .added_block
                    .map_or(false, |block| block < cutoff_block)
            })
            .map(|(k, _)| *k)
            .collect();

        for tx_hash in old_txs {
            pending_txs.remove(&tx_hash);
        }

        info!("Number of pending transactions: {}", pending_txs.len());

        // Small delay to avoid high CPU usage
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
