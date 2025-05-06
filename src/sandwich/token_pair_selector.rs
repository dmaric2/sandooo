/// Token pair selection and evaluation logic for sandwich opportunities.
///
/// This module provides functions for identifying and evaluating token pairs for sandwich opportunities,
/// leveraging the data-driven token registry system.
use anyhow::Result;
use ethers::prelude::*;
use ethers::providers::{Provider, Ws};
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::common::pools::Pool;
use crate::common::token_registry::{get_token_registry, TokenMetadata};

/// Represents a token pair's suitability for sandwiching.
#[derive(Debug, Clone)]
pub struct PairEvaluation {
    /// The pool address
    pub pool_address: H160,
    /// First token in the pair
    pub token0: TokenMetadata,
    /// Second token in the pair
    pub token1: TokenMetadata,
    /// The token that will be used as the input for the sandwich (usually WETH)
    pub main_token: H160,
    /// The token that will be the target of the sandwich
    pub target_token: H160,
    /// Overall suitability score (higher is better)
    pub suitability_score: f64,
    /// Estimated liquidity in USD
    pub liquidity_usd: f64,
    /// Factors contributing to the suitability score
    pub factors: HashMap<String, f64>,
}

/// Evaluates token pairs from available pools for sandwich opportunities.
///
/// # Parameters
/// * `provider`: &Arc<Provider<Ws>> - Ethereum provider
/// * `pools`: &[Pool] - Available pools to evaluate
///
/// # Returns
/// * `Result<Vec<PairEvaluation>>` - Ranked list of pair evaluations
pub async fn evaluate_token_pairs(
    provider: &Arc<Provider<Ws>>,
    pools: &[Pool],
) -> Result<Vec<PairEvaluation>> {
    let registry = get_token_registry();

    // Ensure token prices are updated
    if let Err(e) = registry.update_prices_from_chainlink(provider).await {
        warn!("Failed to update token prices: {}", e);
    }

    let mut evaluations = Vec::new();

    // Set of already processed pool addresses to avoid duplicates
    let mut processed_pools = HashSet::new();

    for pool in pools {
        if processed_pools.contains(&pool.address) {
            continue;
        }
        processed_pools.insert(pool.address);

        // Fetch token metadata
        let token0_meta = match registry.get_token(pool.token0) {
            Some(meta) => meta,
            None => {
                // Try to fetch on-chain if not in registry
                match registry.fetch_token_info(provider, pool.token0).await {
                    Ok(meta) => meta,
                    Err(_) => continue, // Skip if we can't get token info
                }
            }
        };

        let token1_meta = match registry.get_token(pool.token1) {
            Some(meta) => meta,
            None => {
                // Try to fetch on-chain if not in registry
                match registry.fetch_token_info(provider, pool.token1).await {
                    Ok(meta) => meta,
                    Err(_) => continue, // Skip if we can't get token info
                }
            }
        };

        // Determine if pair has at least one main currency
        let (has_main_currency, main_token, target_token) = if token0_meta.is_main_currency {
            (true, pool.token0, pool.token1)
        } else if token1_meta.is_main_currency {
            (true, pool.token1, pool.token0)
        } else {
            (false, H160::zero(), H160::zero())
        };

        // Skip pairs without a main currency unless configured to include them
        if !has_main_currency {
            continue;
        }

        // Calculate suitability score based on various factors
        let mut factors = HashMap::new();

        // Factor 1: Token weight from registry
        let main_weight = token0_meta.weight.max(token1_meta.weight) as f64;
        factors.insert("main_weight".to_string(), main_weight);

        // Factor 2: Liquidity (would normally use pool reserves, but those aren't in our Pool struct)
        // Instead, we'll retrieve liquidity data from the registry or another source if needed
        let mut liquidity_usd = 0.0;
        if let (Some(price0), Some(price1)) =
            (token0_meta.last_price_usd, token1_meta.last_price_usd)
        {
            // For now, assign a default value - in a real implementation we'd query actual reserves
            let liquidity_factor = 5.0; // default middle value on our 0-10 scale
            factors.insert("liquidity".to_string(), liquidity_factor);

            // For demonstration, set a synthetic liquidity value based on token prices
            liquidity_usd = (price0 + price1) * 1000.0; // assuming some standard pool size
        }

        // Factor 3: DEX variant preference (V2 pools are generally better for sandwiching)
        let version_factor = match pool.version {
            crate::common::pools::DexVariant::UniswapV2 => 5.0,
            crate::common::pools::DexVariant::UniswapV3 => 5.0, // Use the same factor for V3 pools
        };
        factors.insert("version".to_string(), version_factor);

        // Factor 4: Token decimals difference (high differences can lead to precision issues)
        let decimals_diff =
            (token0_meta.decimals as i32 - token1_meta.decimals as i32).abs() as f64;
        let decimals_factor = if decimals_diff > 10.0 {
            0.5
        } else {
            5.0 - decimals_diff * 0.5
        };
        factors.insert("decimals_compatibility".to_string(), decimals_factor);

        // Calculate overall score
        let mut score = 0.0;
        for factor in factors.values() {
            score += factor;
        }

        // Create evaluation
        let evaluation = PairEvaluation {
            pool_address: pool.address,
            token0: token0_meta,
            token1: token1_meta,
            main_token,
            target_token,
            suitability_score: score,
            liquidity_usd,
            factors,
        };

        evaluations.push(evaluation);
    }

    // Sort by score (descending)
    evaluations.sort_by(|a, b| {
        b.suitability_score
            .partial_cmp(&a.suitability_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Log top pairs
    if !evaluations.is_empty() {
        let top_count = evaluations.len().min(5);
        info!("Top {} pairs for sandwich opportunities:", top_count);
        for (i, eval) in evaluations.iter().take(top_count).enumerate() {
            info!(
                "#{}: Pool {:?} ({}_{}) - Score: {:.2}, Liquidity: ${:.2}",
                i + 1,
                eval.pool_address,
                eval.token0.symbol,
                eval.token1.symbol,
                eval.suitability_score,
                eval.liquidity_usd
            );
        }
    }

    Ok(evaluations)
}

/// Gets a set of recommended token pairs to monitor for sandwich opportunities.
///
/// # Parameters
/// * `provider`: &Arc<Provider<Ws>> - Ethereum provider
/// * `pools`: &[Pool] - Available pools
/// * `min_score`: f64 - Minimum suitability score
/// * `max_pairs`: usize - Maximum number of pairs to return
///
/// # Returns
/// * `Result<Vec<H160>>` - List of recommended pool addresses
pub async fn get_recommended_pairs(
    provider: &Arc<Provider<Ws>>,
    pools: &[Pool],
    min_score: f64,
    max_pairs: usize,
) -> Result<Vec<H160>> {
    let evaluations = evaluate_token_pairs(provider, pools).await?;

    let recommended = evaluations
        .into_iter()
        .filter(|eval| eval.suitability_score >= min_score)
        .take(max_pairs)
        .map(|eval| eval.pool_address)
        .collect();

    Ok(recommended)
}

/// Updates token information in the registry from on-chain data.
///
/// # Parameters
/// * `provider`: &Arc<Provider<Ws>> - Ethereum provider
/// * `pools`: &[Pool] - Pools to scan for tokens
///
/// # Returns
/// * `Result<()>` - Success or error
pub async fn update_token_registry(provider: &Arc<Provider<Ws>>, pools: &[Pool]) -> Result<()> {
    let registry = get_token_registry();

    // Get unique token addresses from pools
    let mut token_addresses = HashSet::new();
    for pool in pools {
        token_addresses.insert(pool.token0);
        token_addresses.insert(pool.token1);
    }

    let token_count = token_addresses.len();
    info!("Updating information for {} tokens", token_count);

    // Fetch missing token information
    let mut updated = 0;
    for address in token_addresses {
        if registry.get_token(address).is_none() {
            if let Ok(_) = registry.fetch_token_info(provider, address).await {
                updated += 1;
                if updated % 100 == 0 {
                    debug!("Updated {} of {} tokens", updated, token_count);
                }
            }
        }
    }

    // Update prices
    registry.update_prices_from_chainlink(provider).await?;

    // Save registry to cache
    registry.save_to_cache();

    info!("Token registry updated with {} new tokens", updated);
    Ok(())
}
