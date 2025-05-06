/// Sandwich attack execution with Aave V3 flashloans integration.
///
/// Extends the original main_dish module with flashloan-based sandwich attack capabilities.
use anyhow::Result;
use bounded_vec_deque::BoundedVecDeque;
use ethers::{
    providers::{Provider, Ws},
    types::{H160, H256, U256},
};
use log::{info, warn};
use std::str::FromStr;
use std::{collections::HashMap, sync::Arc};

use crate::common::alert::Alert;
use crate::common::constants::*;
use crate::common::execution::Executor;
use crate::common::execution_v3::ExecutorV3Extension;
use crate::common::streams::NewBlock;
use crate::sandwich::simulation::{BatchSandwich, PendingTxInfo, Sandwich};

/// Executes the main sandwich attack logic using Aave V3 flashloans.
///
/// # Parameters
/// * `provider`: Ethereum provider.
/// * `alert`: Alert system for notifications.
/// * `executor`: Transaction executor.
/// * `new_block`: Current block info.
/// * `owner`: Owner address.
/// * `bot_address`: Bot's address.
/// * `bribe_pct`: Percentage of profit to use as bribe.
/// * `promising_sandwiches`: Map of promising sandwiches.
/// * `simulated_bundle_ids`: Mutable deque of simulated bundle IDs.
/// * `pending_txs`: Map of all pending transactions.
///
/// # Returns
/// * `Result<()>` - Ok if successful.
pub async fn main_dish_v3(
    provider: &Arc<Provider<Ws>>,
    alert: &Alert,
    executor: &Executor,
    new_block: &NewBlock,
    owner: H160,
    bot_address: H160,
    bribe_pct: U256,
    promising_sandwiches: &HashMap<H256, Vec<Sandwich>>,
    simulated_bundle_ids: &mut BoundedVecDeque<String>,
    pending_txs: &HashMap<H256, PendingTxInfo>,
) -> Result<()> {
    // Select WETH as the flashloan asset
    let flashloan_asset = H160::from_str(WETH).unwrap();

    if promising_sandwiches.is_empty() {
        return Ok(());
    }

    let base_fee = new_block.base_fee;
    let max_fee = base_fee * 2;

    let mut sandwich_ingredients = Vec::new();
    for (tx_hash, sandwiches) in promising_sandwiches.iter() {
        for sandwich in sandwiches {
            // Skip if we've already processed this sandwich
            let bundle_id = format!("{:?}", tx_hash);
            if simulated_bundle_ids.contains(&bundle_id) {
                continue;
            }

            // Skip sandwiches that aren't optimized yet
            if sandwich.optimized_sandwich.is_none() {
                continue;
            }

            let optimized = sandwich.optimized_sandwich.as_ref().unwrap();

            // Calculate profit with flashloan fee included
            // The flashloan fee is 0.09% of the borrowed amount (amount_in)
            let flashloan_fee = executor.calculate_flashloan_fee(sandwich.amount_in);
            let adjusted_max_revenue = if optimized.max_revenue > flashloan_fee {
                optimized.max_revenue - flashloan_fee
            } else {
                continue; // Skip if the fee exceeds the revenue
            };

            // Create ingredients for this sandwich opportunity
            let swap_info = &sandwich.swap_info;
            sandwich_ingredients.push(crate::sandwich::main_dish::Ingredients {
                tx_hash: *tx_hash,
                pair: swap_info.target_pair,
                main_currency: swap_info.main_currency,
                amount_in: sandwich.amount_in,
                max_revenue: adjusted_max_revenue, // Use fee-adjusted revenue
                score: adjusted_max_revenue.as_u64() as f64, // Simple scoring for now
                sandwich: sandwich.clone(),
            });
        }
    }

    // Sort by revenue-based score
    sandwich_ingredients.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Process each sandwich opportunity
    for ingredients in sandwich_ingredients {
        let tx_hash = ingredients.tx_hash;
        let pair = ingredients.pair;
        let amount_in = ingredients.amount_in;
        let sandwich = ingredients.sandwich;
        let bundle_id = format!("{:?}", tx_hash);

        simulated_bundle_ids.push_back(bundle_id.clone());

        // Skip if the victim transaction is not available
        if !pending_txs.contains_key(&tx_hash) {
            continue;
        }

        info!(
            " Executing flashloan sandwich for {:?} with amount {:?}",
            pair, amount_in
        );

        let optimized = sandwich.optimized_sandwich.as_ref().unwrap();

        // Get the front and back run access lists and calldata
        let front_access_list = Some(optimized.front_access_list.clone());
        let back_access_list = Some(optimized.back_access_list.clone());
        // We'll use the simulated calldata later, so we don't need these variables for now
        // let _front_calldata = optimized.front_calldata.clone();
        // let _back_calldata = optimized.back_calldata.clone();

        // Create a batch sandwich for this opportunity
        let mut final_batch_sandwich = BatchSandwich::new(flashloan_asset);
        final_batch_sandwich.sandwiches.push(sandwich.clone());

        // Simulate the sandwich to verify profitability
        let simulated_sandwich = final_batch_sandwich
            .simulate(
                provider.clone(),
                Some(owner),
                new_block.block_number,
                base_fee,
                max_fee,
                front_access_list.clone(),
                back_access_list.clone(),
                Some(bot_address),
            )
            .await;

        if let Err(e) = simulated_sandwich {
            warn!("BatchSandwich.simulate error: {e:?}");
            continue;
        }

        let simulated_sandwich = simulated_sandwich.unwrap();
        if simulated_sandwich.revenue <= 0 {
            continue;
        }

        // Set limit as 30% above what we simulated
        let gas_limit =
            (simulated_sandwich.front_gas_used + simulated_sandwich.back_gas_used) * 13 / 10;

        // Calculate bribe amount based on priority fee
        let realistic_gas_used =
            (simulated_sandwich.front_gas_used + simulated_sandwich.back_gas_used) * 105 / 100;
        let bribe_amount = U256::from(simulated_sandwich.revenue) * bribe_pct / U256::from(10000);
        let max_priority_fee_per_gas = bribe_amount / U256::from(realistic_gas_used);
        let max_fee_per_gas = base_fee + max_priority_fee_per_gas;

        // Calculate the flashloan fee for the final revenue calculation
        let flashloan_fee = executor.calculate_flashloan_fee(amount_in);
        let adjusted_revenue = if U256::from(simulated_sandwich.revenue) > flashloan_fee {
            simulated_sandwich.revenue - flashloan_fee.as_u128() as i128
        } else {
            warn!("Revenue doesn't cover flashloan fee, skipping");
            continue;
        };

        info!(
            " Flashloan Sandwich: {:?} ({})",
            final_batch_sandwich.sandwiches.len(),
            bundle_id
        );
        info!(
            "> Base fee: {:?} / Priority fee: {:?} / Max fee: {:?} / Bribe: {:?}",
            base_fee, max_priority_fee_per_gas, max_fee_per_gas, bribe_amount
        );
        info!(
            "> Revenue: {:?} / Flashloan Fee: {:?} / Adjusted Revenue: {:?}",
            simulated_sandwich.revenue, flashloan_fee, adjusted_revenue
        );
        info!(
            "> Gas used: {:?} / Gas limit: {:?}",
            simulated_sandwich.front_gas_used + simulated_sandwich.back_gas_used,
            gas_limit
        );

        let message = format!(
            "[{:?}] Flashloan Sandwich / Gas: {:?} / Bribe: {:?} / Revenue: {:?}",
            bundle_id,
            simulated_sandwich.front_gas_used + simulated_sandwich.back_gas_used,
            bribe_amount,
            adjusted_revenue,
        );
        match alert.send(&message).await {
            Err(e) => warn!("Telegram error: {e:?}"),
            _ => {}
        }

        // Get victim transactions
        let victim_tx_hashes = final_batch_sandwich.victim_tx_hashes();
        let mut victim_txs = Vec::new();
        for tx_hash in victim_tx_hashes {
            if let Some(tx_info) = pending_txs.get(&tx_hash) {
                let tx = tx_info.pending_tx.tx.clone();
                victim_txs.push(tx);
            }
        }

        // Create flashloan-based sandwich bundle
        let sando_bundle = executor
            .create_flashloan_sando_bundle(
                flashloan_asset,
                amount_in,
                victim_txs,
                simulated_sandwich.front_calldata,
                simulated_sandwich.back_calldata,
                front_access_list.unwrap_or_default(),
                back_access_list.unwrap_or_default(),
                gas_limit as u64,
                base_fee,
                max_priority_fee_per_gas,
                max_fee_per_gas,
            )
            .await;

        if let Err(e) = sando_bundle {
            warn!("Executor.create_flashloan_sando_bundle error: {e:?}");
            continue;
        }

        let sando_bundle = sando_bundle.unwrap();

        // Send the bundle
        match crate::sandwich::main_dish::send_sando_bundle_request(
            &executor,
            sando_bundle,
            new_block.block_number,
            &alert,
        )
        .await
        {
            Err(e) => warn!("send_sando_bundle_request error: {e:?}"),
            _ => {}
        }
    }

    Ok(())
}
