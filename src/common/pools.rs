/// Pool discovery and management utilities for DeFi protocols.
///
/// Provides types and functions for loading, caching, and querying liquidity pools from on-chain and CSV data.
use anyhow::Result;
use csv::StringRecord;
use ethers::abi::{parse_abi, ParamType};
use ethers::prelude::*;
use ethers::{
    providers::{Provider, Ws},
    types::{H160, H256},
};
use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use log::info;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{create_dir_all, OpenOptions},
    path::Path,
    str::FromStr,
    sync::Arc,
};

/// Supported DEX variants.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum DexVariant {
    /// Uniswap V2-like DEX.
    UniswapV2, // 2
    /// Uniswap V3 DEX.
    UniswapV3, // 3
}

impl DexVariant {
    /// Returns the numeric identifier for the DEX variant.
    pub fn num(&self) -> u8 {
        match self {
            DexVariant::UniswapV2 => 2,
            DexVariant::UniswapV3 => 3,
        }
    }
}

/// Represents a liquidity pool in a DEX.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Pool {
    /// Unique pool ID.
    pub id: i64,
    /// Pool contract address.
    pub address: H160,
    /// DEX variant (e.g., UniswapV2).
    pub version: DexVariant,
    /// First token address.
    pub token0: H160,
    /// Second token address.
    pub token1: H160,
    /// Fee tier (Uniswap V3 specific).
    pub fee: u32,
    /// Block number of creation.
    pub block_number: u64,
    /// Timestamp of creation.
    pub timestamp: u64,
}

impl From<StringRecord> for Pool {
    /// Converts a CSV record into a Pool struct.
    fn from(record: StringRecord) -> Self {
        let version = match record.get(2).unwrap().parse().unwrap() {
            2 => DexVariant::UniswapV2,
            3 => DexVariant::UniswapV3,
            _ => DexVariant::UniswapV2,
        };
        Self {
            id: record.get(0).unwrap().parse().unwrap(),
            address: H160::from_str(record.get(1).unwrap()).unwrap(),
            version,
            token0: H160::from_str(record.get(3).unwrap()).unwrap(),
            token1: H160::from_str(record.get(4).unwrap()).unwrap(),
            fee: record.get(5).unwrap().parse().unwrap(),
            block_number: record.get(6).unwrap().parse().unwrap(),
            timestamp: record.get(7).unwrap().parse().unwrap(),
        }
    }
}

impl Pool {
    /// Returns a tuple representing the pool for CSV caching.
    pub fn cache_row(&self) -> (i64, String, i32, String, String, u32, u64, u64) {
        (
            self.id,
            format!("{:?}", self.address),
            self.version.num() as i32,
            format!("{:?}", self.token0),
            format!("{:?}", self.token1),
            self.fee,
            self.block_number,
            self.timestamp,
        )
    }

    /// Checks if the pool trades the given token pair.
    pub fn trades(&self, token_a: H160, token_b: H160) -> bool {
        let is_zero_for_one = self.token0 == token_a && self.token1 == token_b;
        let is_one_for_zero = self.token1 == token_a && self.token0 == token_b;
        is_zero_for_one || is_one_for_zero
    }

    /// Returns a formatted string describing the pool.
    pub fn pretty_msg(&self) -> String {
        format!(
            "[{:?}] {:?}: {:?} --> {:?}",
            self.version, self.address, self.token0, self.token1
        )
    }

    /// Logs a pretty-printed message about the pool.
    pub fn pretty_print(&self) {
        info!("{}", self.pretty_msg());
    }
}

/// Returns the set of pools touched by swaps in a given block.
///
/// # Parameters
/// * `provider`: Arc<Provider<Ws>> - The Ethereum provider.
/// * `block_number`: U64 - The block to scan.
///
/// # Returns
/// * `Result<Vec<H160>>` - Addresses of touched pools.
pub async fn get_touched_pools(
    provider: &Arc<Provider<Ws>>,
    block_number: U64,
) -> Result<Vec<H160>> {
    let v2_swap_event = "Swap(address,uint256,uint256,uint256,uint256,address)";
    let event_filter = Filter::new()
        .from_block(block_number)
        .to_block(block_number)
        .events(vec![v2_swap_event]);
    let logs = provider.get_logs(&event_filter).await?;
    let touched_pools: Vec<H160> = logs.iter().map(|log| log.address).unique().collect();
    Ok(touched_pools)
}

/// Loads all pools from the blockchain and caches them locally.
///
/// This function loads all pools from the blockchain, starting from the given block number, and caches them locally in a CSV file.
/// It also loads any existing cached pools and merges them with the newly loaded ones.
///
/// # Parameters
/// * `wss_url`: String - WebSocket endpoint.
/// * `from_block`: u64 - Start block.
/// * `chunk`: u64 - Block chunk size.
///
/// # Returns
/// * `Result<(Vec<Pool>, i64)>` - All loaded pools and last cached ID.
pub async fn load_all_pools(
    wss_url: String,
    from_block: u64,
    chunk: u64,
) -> Result<(Vec<Pool>, i64)> {
    match create_dir_all("cache") {
        _ => {}
    }
    let cache_file = "cache/.cached-pools.csv";
    let file_path = Path::new(cache_file);
    let file_exists = file_path.exists();
    let file = OpenOptions::new()
        .write(true)
        .append(true)
        .create(true)
        .open(file_path)
        .unwrap();
    let mut writer = csv::Writer::from_writer(file);

    let mut pools = Vec::new();

    let mut v2_pool_cnt = 0;

    if file_exists {
        let mut reader = csv::Reader::from_path(file_path)?;

        for row in reader.records() {
            let row = row.unwrap();
            let pool = Pool::from(row);
            match pool.version {
                DexVariant::UniswapV2 => v2_pool_cnt += 1,
                DexVariant::UniswapV3 => {}
            }
            pools.push(pool);
        }
    } else {
        writer.write_record(&[
            "id",
            "address",
            "version",
            "token0",
            "token1",
            "fee",
            "block_number",
            "timestamp",
        ])?;
    }
    info!("Pools loaded: {:?}", pools.len());
    info!("V2 pools: {:?}", v2_pool_cnt);

    let ws = Ws::connect(wss_url).await?;
    let provider = Arc::new(Provider::new(ws));

    // Uniswap V2
    let pair_created_event = "PairCreated(address,address,address,uint256)";

    let abi = parse_abi(&[&format!("event {}", pair_created_event)]).unwrap();

    let pair_created_signature = abi.event("PairCreated").unwrap().signature();

    let mut id = if pools.len() > 0 {
        pools.last().as_ref().unwrap().id as i64
    } else {
        -1
    };
    let last_id = id as i64;

    let from_block = if id != -1 {
        pools.last().as_ref().unwrap().block_number + 1
    } else {
        from_block
    };
    let to_block = provider.get_block_number().await.unwrap().as_u64();
    let mut blocks_processed = 0;

    let mut block_range = Vec::new();

    loop {
        let start_idx = from_block + blocks_processed;
        let mut end_idx = start_idx + chunk - 1;
        if end_idx > to_block {
            end_idx = to_block;
            block_range.push((start_idx, end_idx));
            break;
        }
        block_range.push((start_idx, end_idx));
        blocks_processed += chunk;
    }
    info!("Block range: {:?}", block_range);

    let pb = ProgressBar::new(block_range.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
        )
        .unwrap()
        .progress_chars("##-"),
    );

    for range in block_range {
        let mut requests = Vec::new();
        requests.push(tokio::task::spawn(load_uniswap_v2_pools(
            provider.clone(),
            range.0,
            range.1,
            pair_created_event,
            pair_created_signature,
        )));
        let results = futures::future::join_all(requests).await;
        for result in results {
            match result {
                Ok(response) => match response {
                    Ok(pools_response) => {
                        pools.extend(pools_response);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        pb.inc(1);
    }

    let mut added = 0;
    pools.sort_by_key(|p| p.block_number);
    for pool in pools.iter_mut() {
        if pool.id == -1 {
            id += 1;
            pool.id = id;
        }
        if (pool.id as i64) > last_id {
            writer.serialize(pool.cache_row())?;
            added += 1;
        }
    }
    writer.flush()?;
    info!("Added {:?} new pools", added);

    Ok((pools, last_id))
}

/// Loads Uniswap V2 pools from logs in the given block range.
///
/// This function loads Uniswap V2 pools from logs in the given block range and returns them as a vector of Pool structs.
///
/// # Parameters
/// * `provider`: Arc<Provider<Ws>> - The Ethereum provider.
/// * `from_block`: u64 - Start block.
/// * `to_block`: u64 - End block.
/// * `event`: &str - Event signature.
/// * `signature`: H256 - Event topic signature.
///
/// # Returns
/// * `Result<Vec<Pool>>` - All loaded pools.
pub async fn load_uniswap_v2_pools(
    provider: Arc<Provider<Ws>>,
    from_block: u64,
    to_block: u64,
    event: &str,
    signature: H256,
) -> Result<Vec<Pool>> {
    let mut pools = Vec::new();
    let mut timestamp_map = HashMap::new();

    let event_filter = Filter::new()
        .from_block(U64::from(from_block))
        .to_block(U64::from(to_block))
        .event(event);
    let logs = provider.get_logs(&event_filter).await?;

    for log in logs {
        let topic = log.topics[0];
        let block_number = log.block_number.unwrap_or_default();

        if topic != signature {
            continue;
        }

        let timestamp = if !timestamp_map.contains_key(&block_number) {
            let block = provider.get_block(block_number).await.unwrap().unwrap();
            let timestamp = block.timestamp.as_u64();
            timestamp_map.insert(block_number, timestamp);
            timestamp
        } else {
            let timestamp = *timestamp_map.get(&block_number).unwrap();
            timestamp
        };

        let token0 = H160::from(log.topics[1]);
        let token1 = H160::from(log.topics[2]);
        if let Ok(input) =
            ethers::abi::decode(&[ParamType::Address, ParamType::Uint(256)], &log.data)
        {
            let pair = input[0].to_owned().into_address().unwrap();
            let pool_data = Pool {
                id: -1,
                address: pair,
                version: DexVariant::UniswapV2,
                token0,
                token1,
                fee: 300,
                block_number: block_number.as_u64(),
                timestamp,
            };
            pools.push(pool_data);
        };
    }

    Ok(pools)
}
