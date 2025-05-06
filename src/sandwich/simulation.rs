use anyhow::Result;
use eth_encode_packed::ethabi::ethereum_types::{H160 as eH160, U256 as eU256};
use eth_encode_packed::{SolidityDataType, TakeLastXBytes};
use ethers::prelude::*;
use ethers::providers::{Provider, Ws};
use ethers::types::{transaction::eip2930::AccessList, Bytes, H160, H256, I256, U256, U64};
use log::{debug, info, warn};
use revm::primitives::{Bytecode, U256 as rU256};
use std::{collections::HashMap, default::Default, str::FromStr, sync::Arc};

use crate::common::bytecode::SANDOOO_BYTECODE;
use crate::common::constants::{DAI, LINK, MKR, USDC, USDT, WBTC, WETH};
use crate::common::evm::{EvmSimulator, Tx, VictimTx};
use crate::common::pools::{DexVariant, Pool};
use crate::common::routers::{is_known_router, is_known_swap_selector};
use crate::common::streams::{NewBlock, NewPendingTx};
use crate::common::utils::{
    create_new_wallet, is_weth, return_main_and_target_currency, MainCurrency,
};
use crate::common::classifier::{classify_transaction, TxKind};
use ethers::abi::{decode, ParamType, Token};

#[derive(Debug, Clone, Default)]
pub struct PendingTxInfo {
    pub pending_tx: NewPendingTx,
    pub touched_pairs: Vec<SwapInfo>,
}

#[derive(Debug, Clone)]
pub enum SwapDirection {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub struct SwapInfo {
    pub tx_hash: H256,
    pub target_pair: H160,
    pub main_currency: H160,
    pub target_token: H160,
    pub version: DexVariant,
    pub token0_is_main: bool,
    pub fee: u32,
    pub direction: SwapDirection,
}

#[derive(Debug, Clone)]
pub struct Sandwich {
    pub amount_in: U256,
    pub swap_info: SwapInfo,
    pub victim_tx: VictimTx,
    pub optimized_sandwich: Option<OptimizedSandwich>,
}

#[derive(Debug, Clone)]
pub struct BatchSandwich {
    pub sandwiches: Vec<Sandwich>,
    pub swap_info_vec: Vec<SwapInfo>,
    pub flashloan_asset: H160,
}

impl Default for BatchSandwich {
    fn default() -> Self {
        Self {
            sandwiches: Vec::new(),
            swap_info_vec: Vec::new(),
            flashloan_asset: H160::zero(),
        }
    }
}

impl BatchSandwich {
    pub fn new(flashloan_asset: H160) -> Self {
        Self {
            sandwiches: Vec::new(),
            swap_info_vec: Vec::new(),
            flashloan_asset,
        }
    }

    pub fn bundle_id(&self) -> String {
        let mut tx_hashes = Vec::new();
        for sandwich in &self.sandwiches {
            let tx_hash = sandwich.victim_tx.tx_hash;
            let tx_hash_4_bytes = &format!("{:?}", tx_hash)[0..10];
            tx_hashes.push(String::from_str(tx_hash_4_bytes).unwrap());
        }
        tx_hashes.sort();
        tx_hashes.dedup();
        tx_hashes.join("-")
    }

    pub fn victim_tx_hashes(&self) -> Vec<H256> {
        self.sandwiches
            .iter()
            .map(|s| s.victim_tx.tx_hash)
            .collect()
    }

    pub fn target_tokens(&self) -> Vec<H160> {
        self.sandwiches
            .iter()
            .map(|s| s.swap_info.target_token)
            .collect()
    }

    pub fn target_v2_pairs(&self) -> Vec<H160> {
        self.sandwiches
            .iter()
            .filter(|s| s.swap_info.version == DexVariant::UniswapV2)
            .map(|s| s.swap_info.target_pair)
            .collect()
    }

    pub fn target_pairs(&self) -> Vec<H160> {
        self.sandwiches.iter().map(|s| s.swap_info.target_pair).collect()
    }

    pub fn encode_frontrun_tx(
        &self,
        block_number: U256,
        pair_reserves: &HashMap<H160, (U256, U256)>,
    ) -> Result<(Bytes, Vec<Tx>, HashMap<H160, U256>)> {
        let mut starting_mc_values = HashMap::new();

        let mut added_tx_hash = HashMap::new();
        let mut victim_txs = Vec::new();

        let mut frontrun_swap_params = Vec::new();

        let block_number_u256 = eU256::from_dec_str(&block_number.to_string())?;
        frontrun_swap_params.push(
            SolidityDataType::NumberWithShift(block_number_u256, TakeLastXBytes(64)), // blockNumber (uint64)
        );

        for sandwich in &self.sandwiches {
            let tx_hash = sandwich.victim_tx.tx_hash;
            if !added_tx_hash.contains_key(&tx_hash) {
                added_tx_hash.insert(tx_hash, true);
                victim_txs.push(Tx::from(sandwich.victim_tx.clone()));
            }

            // Token swap 0 -> 1
            // Frontrun tx is a main_currency -> target_token BUY tx
            // thus, if token0_is_main, then it is zero_for_one swap
            let zero_for_one = sandwich.swap_info.token0_is_main;

            let new_amount_in = sandwich
                .amount_in
                .checked_sub(U256::from(1))
                .unwrap_or(U256::zero());
            let amount_in_u256 = eU256::from_dec_str(&new_amount_in.to_string())?;
            let amount_out_u256 = {
                // unified support for Uniswap V2 & V3 using on-chain reserves
                match pair_reserves.get(&sandwich.swap_info.target_pair) {
                    Some(reserves) => {
                        let (reserve_in, reserve_out) = if zero_for_one {
                            (reserves.0, reserves.1)
                        } else {
                            (reserves.1, reserves.0)
                        };
                        let amount_out = if sandwich.swap_info.version == DexVariant::UniswapV2 {
                            get_v2_amount_out(new_amount_in, reserve_in, reserve_out)
                        } else {
                            get_v3_amount_out(new_amount_in, reserve_in, reserve_out, sandwich.swap_info.fee)
                        };
                        eU256::from_dec_str(&amount_out.to_string())?
                    }
                    None => {
                        warn!("Missing reserves for pair {:?}, skipping sandwich", sandwich.swap_info.target_pair);
                        continue;
                    }
                }
            };

            let pair = eH160::from_str(&format!("{:?}", sandwich.swap_info.target_pair)).unwrap();
            let token_in =
                eH160::from_str(&format!("{:?}", sandwich.swap_info.main_currency)).unwrap();

            let main_currency = sandwich.swap_info.main_currency;
            if starting_mc_values.contains_key(&main_currency) {
                let prev_mc_value = *starting_mc_values.get(&main_currency).unwrap();
                starting_mc_values.insert(main_currency, prev_mc_value + new_amount_in);
            } else {
                starting_mc_values.insert(main_currency, new_amount_in);
            }

            frontrun_swap_params.extend(vec![
                SolidityDataType::NumberWithShift(
                    eU256::from(zero_for_one as u8),
                    TakeLastXBytes(8),
                ), // zeroForOne (uint8)
                SolidityDataType::Address(pair),     // pair (address)
                SolidityDataType::Address(token_in), // tokenIn (address)
                SolidityDataType::NumberWithShift(amount_in_u256, TakeLastXBytes(256)), // amountIn (uint256)
                SolidityDataType::NumberWithShift(amount_out_u256, TakeLastXBytes(256)), // amountOut (uint256)
            ]);
        }

        let frontrun_calldata = eth_encode_packed::abi::encode_packed(&frontrun_swap_params);
        let frontrun_calldata_bytes = Bytes::from_str(&frontrun_calldata.1).unwrap_or_default();

        Ok((frontrun_calldata_bytes, victim_txs, starting_mc_values))
    }

    pub fn encode_backrun_tx(
        &self,
        block_number: U256,
        pair_reserves: &HashMap<H160, (U256, U256)>,
        token_balances: &HashMap<H160, U256>,
    ) -> Result<Bytes> {
        let mut backrun_swap_params = Vec::new();

        let block_number_u256 = eU256::from_dec_str(&block_number.to_string())?;
        backrun_swap_params.push(
            SolidityDataType::NumberWithShift(block_number_u256, TakeLastXBytes(64)), // blockNumber (uint64)
        );

        for sandwich in &self.sandwiches {
            let amount_in = *token_balances
                .get(&sandwich.swap_info.target_token)
                .unwrap_or(&U256::zero());
            let new_amount_in = amount_in.checked_sub(U256::from(1)).unwrap_or(U256::zero());
            let amount_in_u256 = eU256::from_dec_str(&new_amount_in.to_string())?;

            // this means that the buy order is token0 -> token1
            let zero_for_one = sandwich.swap_info.token0_is_main;

            // in backrun tx we sell tokens we bought in our frontrun tx
            // so it's important to flip the boolean value of zero_for_one
            let amount_out_u256 = {
                // unified support for Uniswap V2 & V3 using on-chain reserves
                match pair_reserves.get(&sandwich.swap_info.target_pair) {
                    Some(reserves) => {
                        let (reserve_in, reserve_out) = if zero_for_one {
                            // token0 is main_currency
                            (reserves.1, reserves.0)
                        } else {
                            // token1 is main_currency
                            (reserves.0, reserves.1)
                        };
                        let amount_out = if sandwich.swap_info.version == DexVariant::UniswapV2 {
                            get_v2_amount_out(new_amount_in, reserve_in, reserve_out)
                        } else {
                            get_v3_amount_out(new_amount_in, reserve_in, reserve_out, sandwich.swap_info.fee)
                        };
                        eU256::from_dec_str(&amount_out.to_string())?
                    }
                    None => {
                        warn!("Missing reserves for pair {:?}, skipping sandwich", sandwich.swap_info.target_pair);
                        continue;
                    }
                }
            };

            let pair = eH160::from_str(&format!("{:?}", sandwich.swap_info.target_pair)).unwrap();
            let token_in =
                eH160::from_str(&format!("{:?}", sandwich.swap_info.target_token)).unwrap();

            backrun_swap_params.extend(vec![
                SolidityDataType::NumberWithShift(
                    eU256::from(!zero_for_one as u8), // <-- make sure to flip boolean value (it's a sell now, not buy)
                    TakeLastXBytes(8),
                ), // zeroForOne (uint8)
                SolidityDataType::Address(pair),     // pair (address)
                SolidityDataType::Address(token_in), // tokenIn (address)
                SolidityDataType::NumberWithShift(amount_in_u256, TakeLastXBytes(256)), // amountIn (uint256)
                SolidityDataType::NumberWithShift(amount_out_u256, TakeLastXBytes(256)), // amountOut (uint256)
            ]);
        }

        let backrun_calldata = eth_encode_packed::abi::encode_packed(&backrun_swap_params);
        let backrun_calldata_bytes = Bytes::from_str(&backrun_calldata.1).unwrap_or_default();

        Ok(backrun_calldata_bytes)
    }

    pub async fn simulate(
        &self,
        provider: Arc<Provider<Ws>>,
        owner: Option<H160>,
        block_number: U64,
        base_fee: U256,
        max_fee: U256,
        front_access_list: Option<AccessList>,
        back_access_list: Option<AccessList>,
        bot_address: Option<H160>,
    ) -> Result<SimulatedSandwich> {
        let mut simulator = EvmSimulator::new(provider.clone(), owner, block_number);

        // set ETH balance so that it's enough to cover gas fees
        match owner {
            None => {
                let initial_eth_balance = U256::from(100) * U256::from(10).pow(U256::from(18));
                simulator.set_eth_balance(simulator.owner, initial_eth_balance);
            }
            _ => {}
        }

        // get reserves for all pairs and target tokens
        let target_pairs = self.target_pairs();
        let target_tokens = self.target_tokens();

        let mut reserves_before = HashMap::new();

        // Fetch reserves per pool version with fallback: V2 tries V3 on revert
        for sandwich in &self.sandwiches {
            let pair = sandwich.swap_info.target_pair;
            let mut reserves_opt: Option<(U256, U256)> = None;
            match sandwich.swap_info.version {
                DexVariant::UniswapV2 => {
                    match simulator.get_pair_reserves(pair) {
                        Ok(res) => reserves_opt = Some(res),
                        Err(e) => {
                            warn!("get V2 reserves reverted for {:?}, falling back to V3: {:?}", pair, e);
                            match simulator.get_v3_pool_reserves(pair) {
                                Ok(res_v3) => reserves_opt = Some(res_v3),
                                Err(e) => warn!("fallback V3 get_v3_pool_reserves failed for {:?}: {:?}", pair, e),
                            }
                        }
                    }
                }
                DexVariant::UniswapV3 => {
                    match simulator.get_v3_pool_reserves(pair) {
                        Ok(res) => reserves_opt = Some(res),
                        Err(e) => {
                            warn!("get V3 reserves reverted for {:?}, falling back to V2: {:?}", pair, e);
                            match simulator.get_pair_reserves(pair) {
                                Ok(res_v2) => reserves_opt = Some(res_v2),
                                Err(e) => warn!("fallback V2 get_pair_reserves failed for {:?}: {:?}", pair, e),
                            }
                        }
                    }
                }
            }
            if let Some(reserves) = reserves_opt {
                reserves_before.insert(pair, reserves);
            } else {
                warn!("Missing reserves for pair {:?}, skipping sandwich", pair);
            }
        }

        let next_block_number = simulator.get_block_number();

        // create frontrun tx calldata and inject main_currency token balance to bot contract
        let (frontrun_calldata, victim_txs, starting_mc_values) =
            self.encode_frontrun_tx(next_block_number, &reserves_before)?;

        // deploy Sandooo bot
        let bot_address = match bot_address {
            Some(bot_address) => bot_address,
            None => {
                let bot_address = create_new_wallet().1;
                simulator.deploy(bot_address, Bytecode::new_raw((*SANDOOO_BYTECODE.0).into()));

                // override owner slot
                let owner_ru256 = rU256::from_str(&format!("{:?}", simulator.owner)).unwrap();
                simulator.insert_account_storage(bot_address, rU256::from(0), owner_ru256)?;

                for (main_currency, starting_value) in &starting_mc_values {
                    let mc = MainCurrency::new(*main_currency);
                    let balance_slot = mc.balance_slot();
                    simulator.set_token_balance(
                        *main_currency,
                        bot_address,
                        balance_slot,
                        (*starting_value).into(),
                    )?;
                }

                bot_address
            }
        };

        // check ETH, MC balance before any txs are run
        let eth_balance_before = simulator.get_eth_balance_of(simulator.owner);
        let mut mc_balances_before = HashMap::new();
        for (main_currency, _) in &starting_mc_values {
            let balance_before = simulator.get_token_balance(*main_currency, bot_address)?;
            mc_balances_before.insert(main_currency, balance_before);
        }

        // set base fee so that gas fees are taken into account
        simulator.set_base_fee(base_fee);

        // Frontrun
        let front_tx = Tx {
            caller: simulator.owner,
            transact_to: bot_address,
            data: frontrun_calldata.0.clone(),
            value: U256::zero(),
            gas_price: base_fee,
            gas_limit: 5000000,
        };
        let front_access_list = match front_access_list {
            Some(access_list) => access_list,
            None => match simulator.get_access_list(front_tx.clone()) {
                Ok(access_list) => access_list,
                _ => AccessList::default(),
            },
        };
        simulator.set_access_list(front_access_list.clone());
        let front_gas_used = match simulator.call(front_tx) {
            Ok(result) => result.gas_used,
            Err(_) => 0,
        };

        // Victim Txs
        for victim_tx in victim_txs {
            match simulator.call(victim_tx) {
                _ => {}
            }
        }

        simulator.set_base_fee(U256::zero());

        // get reserves after frontrun / victim tx
        let mut reserves_after = HashMap::new();
        let mut token_balances = HashMap::new();

        for pair in &target_pairs {
            let mut reserves_opt: Option<(U256, U256)> = None;
            match self.swap_info_vec.iter().find(|si| si.target_pair == *pair).unwrap().version {
                DexVariant::UniswapV2 => {
                    match simulator.get_pair_reserves(*pair) {
                        Ok(res) => reserves_opt = Some(res),
                        Err(e) => {
                            warn!("get V2 reserves reverted for {:?}, falling back to V3: {:?}", pair, e);
                            match simulator.get_v3_pool_reserves(*pair) {
                                Ok(res_v3) => reserves_opt = Some(res_v3),
                                Err(e) => warn!("fallback V3 get_v3_pool_reserves failed for {:?}", pair),
                            }
                        }
                    }
                }
                DexVariant::UniswapV3 => {
                    match simulator.get_v3_pool_reserves(*pair) {
                        Ok(res) => reserves_opt = Some(res),
                        Err(e) => {
                            warn!("get V3 reserves reverted for {:?}, falling back to V2: {:?}", pair, e);
                            match simulator.get_pair_reserves(*pair) {
                                Ok(res_v2) => reserves_opt = Some(res_v2),
                                Err(e) => warn!("fallback V2 get_pair_reserves failed for {:?}", pair),
                            }
                        }
                    }
                }
            }
            if let Some(reserves) = reserves_opt {
                reserves_after.insert(*pair, reserves);
            } else {
                warn!("Missing reserves for pair {:?}, skipping sandwich", pair);
            }
        }

        for token in &target_tokens {
            let token_balance = simulator
                .get_token_balance(*token, bot_address)
                .unwrap_or_default();
            token_balances.insert(*token, token_balance);
        }

        simulator.set_base_fee(base_fee);

        let backrun_calldata =
            self.encode_backrun_tx(next_block_number, &reserves_after, &token_balances)?;

        // Backrun
        let back_tx = Tx {
            caller: simulator.owner,
            transact_to: bot_address,
            data: backrun_calldata.0.clone(),
            value: U256::zero(),
            gas_price: max_fee,
            gas_limit: 5000000,
        };
        let back_access_list = match back_access_list.clone() {
            Some(access_list) => access_list,
            None => match simulator.get_access_list(back_tx.clone()) {
                Ok(access_list) => access_list,
                _ => AccessList::default(),
            },
        };
        let back_access_list = back_access_list.clone();
        simulator.set_access_list(back_access_list.clone());
        let back_gas_used = match simulator.call(back_tx) {
            Ok(result) => result.gas_used,
            Err(_) => 0,
        };

        simulator.set_base_fee(U256::zero());

        let eth_balance_after = simulator.get_eth_balance_of(simulator.owner);
        let mut mc_balances_after = HashMap::new();
        for (main_currency, _) in &starting_mc_values {
            let balance_after = simulator.get_token_balance(*main_currency, bot_address)?;
            mc_balances_after.insert(main_currency, balance_after);
        }

        let eth_used_as_gas = eth_balance_before
            .checked_sub(eth_balance_after)
            .unwrap_or(eth_balance_before);
        let eth_used_as_gas_i256 = I256::from_dec_str(&eth_used_as_gas.to_string())?;

        let usdt = H160::from_str(USDT).unwrap();
        let usdc = H160::from_str(USDC).unwrap();

        let mut weth_before_i256 = I256::zero();
        let mut weth_after_i256 = I256::zero();

        for (main_currency, _) in &starting_mc_values {
            let mc_balance_before = *mc_balances_before.get(&main_currency).unwrap();
            let mc_balance_after = *mc_balances_after.get(&main_currency).unwrap();

            let (mc_balance_before, mc_balance_after) = if *main_currency == usdt {
                let before =
                    convert_usdt_to_weth(&mut simulator, mc_balance_before).unwrap_or_default();
                let after =
                    convert_usdt_to_weth(&mut simulator, mc_balance_after).unwrap_or_default();
                (before, after)
            } else if *main_currency == usdc {
                let before =
                    convert_usdc_to_weth(&mut simulator, mc_balance_before).unwrap_or_default();
                let after =
                    convert_usdc_to_weth(&mut simulator, mc_balance_after).unwrap_or_default();
                (before, after)
            } else {
                (mc_balance_before, mc_balance_after)
            };

            let mc_balance_before_i256 = I256::from_dec_str(&mc_balance_before.to_string())?;
            let mc_balance_after_i256 = I256::from_dec_str(&mc_balance_after.to_string())?;

            weth_before_i256 += mc_balance_before_i256;
            weth_after_i256 += mc_balance_after_i256;
        }

        let profit = (weth_after_i256 - weth_before_i256).as_i128();
        let gas_cost = eth_used_as_gas_i256.as_i128();
        let revenue = profit - gas_cost;

        let simulated_sandwich = SimulatedSandwich {
            revenue,
            profit,
            gas_cost,
            front_gas_used,
            back_gas_used,
            front_access_list,
            back_access_list,
            front_calldata: frontrun_calldata,
            back_calldata: backrun_calldata,
        };

        Ok(simulated_sandwich)
    }
}

#[derive(Debug, Default, Clone)]
pub struct SimulatedSandwich {
    pub revenue: i128,
    pub profit: i128,
    pub gas_cost: i128,
    pub front_gas_used: u64,
    pub back_gas_used: u64,
    pub front_access_list: AccessList,
    pub back_access_list: AccessList,
    pub front_calldata: Bytes,
    pub back_calldata: Bytes,
}

#[derive(Debug, Default, Clone)]
pub struct OptimizedSandwich {
    pub amount_in: U256,
    pub max_revenue: U256,
    pub front_gas_used: u64,
    pub back_gas_used: u64,
    pub front_access_list: AccessList,
    pub back_access_list: AccessList,
    pub front_calldata: Bytes,
    pub back_calldata: Bytes,
}

pub static V2_SWAP_EVENT_ID: &str = "0xd78ad95f";

// Router addresses for DEXes and aggregators
pub static UNISWAP_V2_ROUTER: &str = "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D";
pub static UNISWAP_V3_ROUTER: &str = "0xE592427A0AEce92De3Edee1F18E0157C05861564";
pub static SUSHISWAP_ROUTER: &str = "0xd9e1cE17f2641f24aE83637ab66a2cca9C378B9F";
pub static UNIVERSAL_ROUTER: &str = "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"; // Uniswap Universal Router
pub static ONEINCH_ROUTER: &str = "0x1111111254fb6c44bAC0beD2854e76F90643097d"; // 1inch Router
pub static ZEROX_ROUTER: &str = "0xDef1C0ded9bec7F1a1670819833240f027b25EfF"; // 0x Router
pub static ONEINCH_ROUTER_V4: &str = "0x11111112542D85B3EF69AE05771c2dCCff4fAa26"; // 1inch Router v4
pub static ONEINCH_ROUTER_V5: &str = "0x1111111254EEB25477B68fb85Ed929f73A960582"; // 1inch Router v5

// Router and DEX method signatures for swap detection
pub static UNISWAP_V2_SWAP_EXACT_ETH_FOR_TOKENS: &str = "0x7ff36ab5";
pub static UNISWAP_V2_SWAP_ETH_FOR_EXACT_TOKENS: &str = "0xfb3bdb41";
pub static UNISWAP_V2_SWAP_EXACT_TOKENS_FOR_ETH: &str = "0x18cbafe5";
pub static UNISWAP_V2_SWAP_EXACT_TOKENS_FOR_TOKENS: &str = "0x38ed1739";
pub static UNISWAP_V2_SWAP_TOKENS_FOR_EXACT_TOKENS: &str = "0x8803dbee";
pub static UNISWAP_V2_SWAP_EXACT_ETH_FOR_TOKENS_SUPPORTING_FEE_ON_TRANSFER_TOKENS: &str =
    "0xb6f9de95";
pub static UNISWAP_V2_SWAP_EXACT_TOKENS_FOR_ETH_SUPPORTING_FEE_ON_TRANSFER_TOKENS: &str =
    "0x791ac947";
pub static UNISWAP_V2_SWAP_EXACT_TOKENS_FOR_TOKENS_SUPPORTING_FEE_ON_TRANSFER_TOKENS: &str =
    "0x5c11d795";
pub static UNISWAP_V2_SWAP_TOKENS_FOR_EXACT_ETH: &str = "0x472b43f3";

// Uniswap V3 specific methods
pub static UNISWAP_V3_SWAP_EXACT_TOKENS_FOR_TOKENS: &str = "0xe8e33700";
pub static UNISWAP_V3_SWAP_EXACT_ETH_FOR_TOKENS: &str = "0xdf2ab5bb";
pub static UNISWAP_V3_EXACT_INPUT_SINGLE: &str = "0x414bf389";
pub static UNISWAP_V3_EXACT_OUTPUT_SINGLE: &str = "0xdb3e2198";
pub static UNISWAP_V3_EXACT_INPUT: &str = "0xb858183f";
pub static UNISWAP_V3_EXACT_OUTPUT: &str = "0x09b81346";

// SushiSwap specific methods
pub static SUSHISWAP_SWAP_EXACT_TOKENS_FOR_ETH_SUPPORTING_FEE: &str = "0xddd8a0f2";
pub static SUSHISWAP_SWAP_TOKENS_FOR_EXACT_ETH: &str = "0xb39bea41";

// 1inch / Aggregator methods
pub static ONEINCH_SWAP: &str = "0x12aa3caf";
pub static ONEINCH_ROUTER_SWAP: &str = "0xac9650d8";
pub static ONEINCH_EXECUTE: &str = "0xe449022e";

// Generic DEX aggregator methods
pub static GENERIC_SWAP1: &str = "0x90411a32"; // swap(bytes32[],address,uint256)
pub static GENERIC_SWAP2: &str = "0x58b7f47f"; // swap(tuple,tuple,tuple,tuple,address,bytes)
pub static GENERIC_UNOSWAP: &str = "0x2e95b6c8"; // unoswap(address,uint256,uint256,bytes32[])
pub static GENERIC_FILL_ORDER: &str = "0x5a099843"; // fillOrderTo(address,tuple)

// Direct pool swap method signatures
pub static UNISWAP_V2_PAIR_SWAP: &str = "0x022c0d9f"; // swap - UniswapV2Pair
pub static SWAP_FOR_0: &str = "0xcdd6cda9"; // swapFor0
pub static SWAP_FOR_1: &str = "0xd50e6fcd"; // swapFor1
pub static UNISWAP_V3_POOL_SWAP: &str = "0x128acb08"; // swap - UniswapV3Pool

pub async fn extract_swap_info(
    provider: &Arc<Provider<Ws>>,
    _new_block: &NewBlock,
    pending_tx: &NewPendingTx,
    pools_map: &HashMap<H160, Pool>,
) -> Result<Vec<SwapInfo>> {
    use log::{debug, info};

    let tx_hash = pending_tx.tx.hash;
    let input = pending_tx.tx.input.clone();
    let mut swap_info_vec = Vec::new();

    println!(
        "DEBUG extract_swap_info START: tx_hash={:?}, input.len={}, to_address={:?}",
        tx_hash,
        input.0.len(),
        pending_tx.tx.to.unwrap_or_default()
    );

    let to_address = pending_tx.tx.to.unwrap_or_default();
    println!("DEBUG extract_swap_info: to_address={:?}", to_address);

    info!("Analyzing transaction: {:?}", tx_hash);
    info!("Total number of pools in pool map: {}", pools_map.len());

    // Unified swap detection via classifier, fallback to full trace-based scan
    let tx_kind = classify_transaction(provider, &pending_tx.tx).await;
    debug!("Initial classify_transaction: {:?}", tx_kind);
    let need_trace = tx_kind != TxKind::Swap
        || pending_tx
            .tx
            .to
            .map(|addr| !is_known_router(&addr))
            .unwrap_or(true);

    if need_trace {
        if let Ok(Some(trace_frame)) = debug_trace_call(provider, _new_block, pending_tx).await {
            let mut logs = Vec::new();
            extract_logs(&trace_frame, &mut logs);
            for log in logs {
                if let Some(addr) = log.address {
                    if let Some(pool) = pools_map.get(&addr) {
                        if let Some(si) = pool_direct_swap(tx_hash, addr, pool) {
                            swap_info_vec.push(si);
                        }
                    }
                }
            }
        }
        return Ok(swap_info_vec);
    }

    // Enhanced method signature detection approach
    if let Some(method_id) = get_method_id_hex(&input.0) {
        debug!(
            "Transaction: {:?}, Method signature: {}",
            tx_hash, method_id
        );

        let is_router = is_known_router(&to_address);
        println!("DEBUG extract_swap_info: is_router={}", is_router);

        // Pool detection
        let is_pool = pools_map.contains_key(&to_address);
        println!("DEBUG extract_swap_info: is_pool={}", is_pool);

        info!("Is router: {}, is known pool: {}", is_router, is_pool);

        // Process direct pool transactions
        if pools_map.contains_key(&to_address) {
            let pool = pools_map.get(&to_address).unwrap();
            info!(
                "Found transaction to known pool: {:?}, token0: {:?}, token1: {:?}",
                to_address, pool.token0, pool.token1
            );

            // Check for common swap function signatures
            info!("Transaction function signature: 0x{}", method_id);

            // Check if it's a direct swap on the pool
            let direct_pool_swaps = [
                "022c0d9f", // swap - UniswapV2Pair
                "cdd6cda9", // swapFor0
                "d50e6fcd", // swapFor1
                "128acb08", // swap - UniswapV3Pool
            ];

            // Identify this as a direct pool swap
            let method_id_no_prefix = method_id.trim_start_matches("0x");
            let is_swap = direct_pool_swaps.contains(&method_id_no_prefix);
            info!(
                "Is direct pool swap: {} (method_id: {}, without prefix: {})",
                is_swap, method_id, method_id_no_prefix
            );

            if is_swap {
                let token0 = pool.token0;
                let token1 = pool.token1;

                // Determine which is the main currency (prioritize WETH, then stablecoins)
                let weth_address = H160::from_str(WETH).unwrap_or_default();
                let usdt_address = H160::from_str(USDT).unwrap_or_default();
                let usdc_address = H160::from_str(USDC).unwrap_or_default();
                let wbtc_address = H160::from_str(WBTC).unwrap_or_default();
                let dai_address = H160::from_str(DAI).unwrap_or_default();
                let link_address = H160::from_str(LINK).unwrap_or_default();
                let mkr_address = H160::from_str(MKR).unwrap_or_default();

                // For debugging: show token addresses
                info!(
                    "Main currency addresses - WETH: {:?}, USDT: {:?}, USDC: {:?}",
                    weth_address, usdt_address, usdc_address
                );
                info!("Pool tokens - token0: {:?}, token1: {:?}", token0, token1);

                let token0_is_main = token0 == weth_address
                    || token0 == usdt_address
                    || token0 == usdc_address
                    || token0 == wbtc_address
                    || token0 == dai_address
                    || token0 == link_address
                    || token0 == mkr_address;

                let token1_is_main = token1 == weth_address
                    || token1 == usdt_address
                    || token1 == usdc_address
                    || token1 == wbtc_address
                    || token1 == dai_address
                    || token1 == link_address
                    || token1 == mkr_address;

                // Only proceed if at least one token is a main currency
                let has_main_currency = token0_is_main || token1_is_main;
                info!(
                    "token0_is_main: {}, token1_is_main: {}, has_main_currency: {}",
                    token0_is_main, token1_is_main, has_main_currency
                );

                if has_main_currency {
                    // Determine main currency and target token
                    let (main_currency, target_token, is_token0_main) = if token0_is_main {
                        (token0, token1, true)
                    } else {
                        (token1, token0, false)
                    };

                    // Default to Buy direction for now
                    let direction = SwapDirection::Buy;

                    info!("SUCCESS: Detected direct pool swap: pool={:?}, direction={:?}, main={:?}, target={:?}",
                          to_address, direction, main_currency, target_token);

                    // Create SwapInfo with appropriate pool version
                    let version = match pool.version {
                        DexVariant::UniswapV2 => DexVariant::UniswapV2,
                        DexVariant::UniswapV3 => DexVariant::UniswapV3,
                    };

                    // Create SwapInfo
                    let swap_info = SwapInfo {
                        tx_hash,
                        target_pair: to_address,
                        main_currency,
                        target_token,
                        version,
                        token0_is_main: is_token0_main,
                        fee: pool.fee,
                        direction,
                    };

                    swap_info_vec.push(swap_info);
                }
            }
        }
        // Process router transactions
        else if is_router {
            info!("Detected transaction to router: {:?}", to_address);

            // Swap selector detection
            let selector_bytes: [u8; 4] = input.0[..4].try_into().unwrap();
            if is_known_swap_selector(&selector_bytes) {
                // Extract and process token paths
                let paths = get_token_paths(&method_id, &input.0);
                for path in paths {
                    // TODO: Support multi-hop paths by iterating through each adjacent token pair in `path`:
                    // 1. Initialize `current_amount = amount_in` from the frontrun or router input.
                    // 2. For each (token_in, token_out) window in `path.windows(2)`,
                    //    a) lookup `(pair_address, pool)` via `get_pool_by_tokens`,
                    //    b) fetch reserves via `simulator.get_pair_reserves(pair_address)`,
                    //    c) compute next_amount = if pool.version==V2 then get_v2_amount_out else get_v3_amount_out using `pool.fee`,
                    //    d) set `current_amount = next_amount` and proceed to next hop.
                    // 3. After all hops, `current_amount` is the final output, `pair_address` and `token_out` of last hop are used for SwapInfo.
                    if path.len() != 2 {
                        warn!("Skipping multi-hop path with {} tokens", path.len());
                        continue;
                    }
                    let token_in = path[0];
                    let token_out = path[1];

                    info!(
                        "Extracted path from router call: in={:?}, out={:?}",
                        token_in, token_out
                    );

                    // Check for the pool that would be used for this token pair
                    if let Some((pair_address, pool)) =
                        get_pool_by_tokens(pools_map, token_in, token_out)
                    {
                        let token0 = pool.token0;
                        let token1 = pool.token1;

                        // Check if either token is a main currency (WETH or stablecoins)
                        let weth_address = H160::from_str(WETH).unwrap_or_default();
                        let usdt_address = H160::from_str(USDT).unwrap_or_default();
                        let usdc_address = H160::from_str(USDC).unwrap_or_default();
                        let wbtc_address = H160::from_str(WBTC).unwrap_or_default();
                        let dai_address = H160::from_str(DAI).unwrap_or_default();
                        let link_address = H160::from_str(LINK).unwrap_or_default();
                        let mkr_address = H160::from_str(MKR).unwrap_or_default();

                        let is_token0_main = token0 == weth_address
                            || token0 == usdt_address
                            || token0 == usdc_address
                            || token0 == wbtc_address
                            || token0 == dai_address
                            || token0 == link_address
                            || token0 == mkr_address;

                        let is_token1_main = token1 == weth_address
                            || token1 == usdt_address
                            || token1 == usdc_address
                            || token1 == wbtc_address
                            || token1 == dai_address
                            || token1 == link_address
                            || token1 == mkr_address;

                        let token0_is_main = (token0 == token_in && is_token0_main)
                            || (token1 == token_in && is_token1_main);

                        // Determine main currency and target token
                        let (main_currency, target_token, is_token0_main) = if token0_is_main {
                            (token0, token1, true)
                        } else {
                            (token1, token0, false)
                        };

                        // Determine swap direction based on token flow
                        let direction = if token_in == main_currency {
                            SwapDirection::Buy
                        } else {
                            SwapDirection::Sell
                        };

                        info!("SUCCESS: Extracted swap from router call: pair={:?}, in={:?}, out={:?}, direction={:?}",
                             pair_address, token_in, token_out, direction);

                        // Create SwapInfo
                        let swap_info = SwapInfo {
                            tx_hash,
                            target_pair: pair_address,
                            main_currency,
                            target_token,
                            version: pool.version,
                            token0_is_main: is_token0_main,
                            fee: pool.fee,
                            direction,
                        };

                        swap_info_vec.push(swap_info);
                    }
                }
            }
        }
        // Fallback: scan transaction logs for Swap events if no path detected
        if swap_info_vec.is_empty() {
            if let Ok(Some(receipt)) = provider.get_transaction_receipt(tx_hash).await {
                for log in receipt.logs {
                    let addr = log.address;
                    if pools_map.contains_key(&addr) {
                        if let Some(topic0) = log.topics.get(0) {
                            if format!("{:?}", topic0) == V2_SWAP_EVENT_ID {
                                let t1 = log.topics.get(1).cloned().unwrap_or_default();
                                let t2 = log.topics.get(2).cloned().unwrap_or_default();
                                let token0 = H160::from_slice(&t1.as_bytes()[12..]);
                                let token1 = H160::from_slice(&t2.as_bytes()[12..]);
                                let pool = pools_map.get(&addr).unwrap();
                                let main = if is_main_token(token0) { token0 } else { token1 };
                                let target = if is_main_token(token0) { token1 } else { token0 };
                                let dir = if is_main_token(token0) { SwapDirection::Buy } else { SwapDirection::Sell };
                                let swap_info = SwapInfo {
                                    tx_hash,
                                    target_pair: addr,
                                    main_currency: main,
                                    target_token: target,
                                    version: pool.version,
                                    token0_is_main: is_main_token(token0),
                                    fee: pool.fee,
                                    direction: dir,
                                };
                                info!("DEBUG: fallback swap event detected: {:?}", swap_info);
                                swap_info_vec.push(swap_info);
                            }
                        }
                    }
                }
            }
        }
    }

    // Deduplicate swap info entries by target_pair
    if swap_info_vec.len() > 1 {
        let mut unique_pairs = std::collections::HashSet::new();
        swap_info_vec.retain(|info| unique_pairs.insert(info.target_pair));
    }

    info!(
        "Extracted {} swap info entries from transaction",
        swap_info_vec.len()
    );

    Ok(swap_info_vec)
}

pub async fn debug_trace_call(
    provider: &Arc<Provider<Ws>>,
    new_block: &NewBlock,
    pending_tx: &NewPendingTx,
) -> Result<Option<CallFrame>> {
    let mut opts = GethDebugTracingCallOptions::default();
    let mut call_config = CallConfig::default();
    call_config.with_log = Some(true);

    opts.tracing_options.tracer = Some(GethDebugTracerType::BuiltInTracer(
        GethDebugBuiltInTracerType::CallTracer,
    ));
    opts.tracing_options.tracer_config = Some(GethDebugTracerConfig::BuiltInTracer(
        GethDebugBuiltInTracerConfig::CallTracer(call_config),
    ));

    let block_number = new_block.block_number;
    let mut tx = pending_tx.tx.clone();
    let nonce = provider
        .get_transaction_count(tx.from, Some(block_number.into()))
        .await
        .unwrap_or_default();
    tx.nonce = nonce;

    let trace = provider
        .debug_trace_call(&tx, Some(block_number.into()), opts)
        .await;

    match trace {
        Ok(trace) => match trace {
            GethTrace::Known(call_tracer) => match call_tracer {
                GethTraceFrame::CallTracer(frame) => Ok(Some(frame)),
                _ => Ok(None),
            },
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

pub fn extract_logs(call_frame: &CallFrame, logs: &mut Vec<CallLogFrame>) {
    if let Some(ref logs_vec) = call_frame.logs {
        logs.extend(logs_vec.iter().cloned());
    }

    if let Some(ref calls_vec) = call_frame.calls {
        for call in calls_vec {
            extract_logs(call, logs);
        }
    }
}

pub fn get_v2_amount_out(amount_in: U256, reserve_in: U256, reserve_out: U256) -> U256 {
    let amount_in_with_fee = amount_in * U256::from(997);
    let numerator = amount_in_with_fee * reserve_out;
    let denominator = (reserve_in * U256::from(1000)) + amount_in_with_fee;
    let amount_out = numerator.checked_div(denominator);
    amount_out.unwrap_or_default()
}

pub fn convert_usdt_to_weth(
    simulator: &mut EvmSimulator<Provider<Ws>>,
    amount: U256,
) -> Result<U256> {
    let conversion_pair = H160::from_str("0x0d4a11d5EEaaC28EC3F61d100daF4d40471f1852").unwrap();
    // token0: WETH / token1: USDT
    let reserves = simulator.get_pair_reserves(conversion_pair)?;
    let (reserve_in, reserve_out) = (reserves.1, reserves.0);
    let weth_out = get_v2_amount_out(amount, reserve_in, reserve_out);
    Ok(weth_out)
}

pub fn convert_usdc_to_weth(
    simulator: &mut EvmSimulator<Provider<Ws>>,
    amount: U256,
) -> Result<U256> {
    let conversion_pair = H160::from_str("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc").unwrap();
    // token0: USDC / token1: WETH
    let reserves = simulator.get_pair_reserves(conversion_pair)?;
    let (reserve_in, reserve_out) = (reserves.0, reserves.1);
    let weth_out = get_v2_amount_out(amount, reserve_in, reserve_out);
    Ok(weth_out)
}

impl Sandwich {
    pub fn is_optimized(&mut self) -> bool {
        self.optimized_sandwich.is_some()
    }

    pub fn pretty_print(&self) {
        println!("\n");
        info!("🥪 SANDWICH: [{:?}]", self.victim_tx.tx_hash);
        info!("- Target token: {:?}", self.swap_info.target_token);
        info!(
            "- Target V{:?} pair: {:?}",
            self.swap_info.version, self.swap_info.target_pair
        );

        match &self.optimized_sandwich {
            Some(optimized_sandwich) => {
                info!("----- Optimized -----");
                info!("- Amount in: {:?}", optimized_sandwich.amount_in);
                info!("- Profit: {:?}", optimized_sandwich.max_revenue);
                info!(
                    "- Front gas: {:?} / Back gas: {:?}",
                    optimized_sandwich.front_gas_used, optimized_sandwich.back_gas_used
                );
            }
            _ => {}
        }
    }

    pub async fn optimize(
        &mut self,
        provider: Arc<Provider<Ws>>,
        block_number: U64,
        amount_in_ceiling: U256,
        base_fee: U256,
        max_fee: U256,
        front_access_list: AccessList,
        back_access_list: AccessList,
    ) -> Result<OptimizedSandwich> {
        let main_currency = self.swap_info.main_currency;

        let mut min_amount_in = U256::zero();
        let mut max_amount_in = amount_in_ceiling;
        let tolerance = if is_weth(main_currency) {
            U256::from(1) * U256::from(10).pow(U256::from(14))
        } else {
            U256::from(1) * U256::from(10).pow(U256::from(3))
        };

        if max_amount_in < min_amount_in {
            return Ok(OptimizedSandwich {
                amount_in: U256::zero(),
                max_revenue: U256::zero(),
                front_gas_used: 0,
                back_gas_used: 0,
                front_access_list: AccessList::default(),
                back_access_list: AccessList::default(),
                front_calldata: Bytes::default(),
                back_calldata: Bytes::default(),
            });
        }

        let mut optimized_in = U256::zero();
        let mut max_revenue = U256::zero();
        let mut max_front_gas_used = 0;
        let mut max_back_gas_used = 0;
        let mut max_front_calldata = Bytes::default();
        let mut max_back_calldata = Bytes::default();

        let intervals = U256::from(5);

        loop {
            let diff = max_amount_in - min_amount_in;
            let step = diff.checked_div(intervals).unwrap();

            if step <= tolerance {
                break;
            }

            let mut inputs = Vec::new();
            for i in 0..intervals.as_u64() + 1 {
                let _i = U256::from(i);
                let input = min_amount_in + (_i * step);
                inputs.push(input);
            }

            let mut simulations = Vec::new();

            for (idx, input) in inputs.iter().enumerate() {
                let sim = tokio::task::spawn(simulate_sandwich(
                    idx,
                    provider.clone(),
                    block_number,
                    self.clone(),
                    *input,
                    base_fee,
                    max_fee,
                    front_access_list.clone(),
                    back_access_list.clone(),
                ));
                simulations.push(sim);
            }

            let results = futures::future::join_all(simulations).await;
            let revenue: Vec<(usize, U256, i128, u64, u64, Bytes, Bytes)> =
                results.into_iter().map(|res| res.unwrap()).collect();

            let mut max_idx = 0;

            for (
                idx,
                amount_in,
                profit,
                front_gas_used,
                back_gas_used,
                front_calldata,
                back_calldata,
            ) in &revenue
            {
                if *profit > max_revenue.as_u128() as i128 {
                    optimized_in = *amount_in;
                    max_revenue = U256::from(*profit);
                    max_front_gas_used = *front_gas_used;
                    max_back_gas_used = *back_gas_used;
                    max_front_calldata = front_calldata.clone();
                    max_back_calldata = back_calldata.clone();

                    max_idx = *idx;
                }
            }

            min_amount_in = if max_idx == 0 {
                U256::zero()
            } else {
                revenue[max_idx - 1].1
            };
            max_amount_in = if max_idx == revenue.len() - 1 {
                revenue[max_idx].1
            } else {
                revenue[max_idx + 1].1
            };
        }

        // Gate under minimum profit threshold (0.02 ETH)
        let min_profit_threshold = U256::from(20_000_000_000_000_000u128);  // 0.02 ETH in wei
        if max_revenue < min_profit_threshold {
            return Ok(OptimizedSandwich {
                amount_in: U256::zero(),
                max_revenue: U256::zero(),
                front_gas_used: 0,
                back_gas_used: 0,
                front_access_list: AccessList::default(),
                back_access_list: AccessList::default(),
                front_calldata: Bytes::default(),
                back_calldata: Bytes::default(),
            });
        }

        let optimized_sandwich = OptimizedSandwich {
            amount_in: optimized_in,
            max_revenue,
            front_gas_used: max_front_gas_used,
            back_gas_used: max_back_gas_used,
            front_access_list,
            back_access_list,
            front_calldata: max_front_calldata,
            back_calldata: max_back_calldata,
        };

        self.optimized_sandwich = Some(optimized_sandwich.clone());
        Ok(optimized_sandwich)
    }
}

pub async fn simulate_sandwich(
    idx: usize,
    provider: Arc<Provider<Ws>>,
    block_number: U64,
    sandwich: Sandwich,
    amount_in: U256,
    base_fee: U256,
    max_fee: U256,
    front_access_list: AccessList,
    back_access_list: AccessList,
) -> (usize, U256, i128, u64, u64, Bytes, Bytes) {
    let mut sandwich = sandwich;
    sandwich.amount_in = amount_in;

    let batch_sandwich = BatchSandwich {
        sandwiches: vec![Sandwich {
            victim_tx: sandwich.victim_tx,
            amount_in: sandwich.amount_in,
            swap_info: sandwich.swap_info.clone(),
            optimized_sandwich: None,
        }],
        swap_info_vec: vec![sandwich.swap_info.clone()],
        flashloan_asset: H160::zero(), // Default to zero address as flashloan asset
    };

    let maybe_simulated_sandwich = match futures::executor::block_on(batch_sandwich.simulate(
        provider,
        None,
        block_number,
        base_fee,
        max_fee,
        Some(front_access_list),
        Some(back_access_list),
        None,
    )) {
        Ok(sim) => Some(sim),
        Err(e) => {
            eprintln!("BatchSandwich.simulate error: {:?}", e);
            None
        }
    };

    maybe_simulated_sandwich
        .map(|sim| {
            (
                idx,
                amount_in,
                sim.revenue,
                sim.front_gas_used,
                sim.back_gas_used,
                sim.front_calldata,
                sim.back_calldata,
            )
        })
        .unwrap_or((idx, amount_in, 0, 0, 0, Bytes::default(), Bytes::default()))
}

// Helper function to find a pool by token pair in the pools map
pub fn get_pool_by_tokens(
    pools_map: &HashMap<H160, Pool>,
    token0: H160,
    token1: H160,
) -> Option<(H160, &Pool)> {
    for (pool_address, pool) in pools_map {
        // Check if pool contains both tokens (in any order)
        if (pool.token0 == token0 && pool.token1 == token1)
            || (pool.token0 == token1 && pool.token1 == token0)
        {
            return Some((*pool_address, pool));
        }
    }
    None
}

// Helper function to decode token paths from input data based on method signature
pub fn get_token_paths(method_id: &str, input: &[u8]) -> Vec<Vec<H160>> {
    let mut result = Vec::new();
    if input.len() <= 4 {
        return result;
    }
    let data = &input[4..];
    // Decode V2 style path: address[]
    let v2_sigs = ["7ff36ab5","fb3bdb41","b6f9de95","38ed1739","4a25d94a","18cbafe5","8803dbee","472b43f3","5c11d795","791ac947"];
    if v2_sigs.contains(&method_id) {
        if let Ok(tokens) = decode(&[ParamType::Array(Box::new(ParamType::Address))], data) {
            if let Token::Array(arr) = &tokens[0] {
                let path = arr.iter().filter_map(|t| if let Token::Address(a) = t { Some(*a) } else { None }).collect::<Vec<H160>>();
                if path.len() >= 2 {
                    result.push(path);
                }
            }
        }
    }
    // Decode V3 multi-hop path: bytes
    let v3_sigs = ["04e45aaf","c04b8d59","5023b4df","09b81346"];
    if v3_sigs.contains(&method_id) {
        if let Ok(tokens) = decode(&[ParamType::Bytes], data) {
            if let Token::Bytes(bytes) = &tokens[0] {
                let mut path = Vec::new();
                let mut i = 0;
                while i + 20 <= bytes.len() {
                    let mut addr = [0u8;20];
                    addr.copy_from_slice(&bytes[i..i+20]);
                    path.push(H160::from(addr));
                    i += 23; // 20 bytes address + 3 bytes fee
                }
                if path.len() >= 2 {
                    result.push(path);
                }
            }
        }
    }
    result
}

// Process direct pool swap - helper for simulating direct pool transactions
pub fn pool_direct_swap(tx_hash: H256, pool_address: H160, pool: &Pool) -> Option<SwapInfo> {
    if let Some((main_currency, target_token)) =
        return_main_and_target_currency(pool.token0, pool.token1)
    {
        let token0_is_main = main_currency == pool.token0;

        let swap_info = SwapInfo {
            tx_hash,
            target_pair: pool_address,
            main_currency,
            target_token,
            version: pool.version,
            token0_is_main,
            fee: pool.fee,
            direction: SwapDirection::Buy, // Default to Buy
        };

        return Some(swap_info);
    }

    None
}

// Helper function to check if a token is a main token (WETH or stablecoin)
pub fn is_main_token(token: H160) -> bool {
    let main_tokens = [
        H160::from_str(WETH).unwrap_or_default(),
        H160::from_str(USDT).unwrap_or_default(),
        H160::from_str(USDC).unwrap_or_default(),
        H160::from_str(DAI).unwrap_or_default(),
    ];

    main_tokens.contains(&token)
}

// Helper function to get token name from address
pub fn token_address_to_name(token: H160) -> String {
    match token {
        t if t == H160::from_str(WETH).unwrap_or_default() => "WETH".to_string(),
        t if t == H160::from_str(USDT).unwrap_or_default() => "USDT".to_string(),
        t if t == H160::from_str(USDC).unwrap_or_default() => "USDC".to_string(),
        t if t == H160::from_str(DAI).unwrap_or_default() => "DAI".to_string(),
        t if t == H160::from_str(WBTC).unwrap_or_default() => "WBTC".to_string(),
        t if t == H160::from_str(LINK).unwrap_or_default() => "LINK".to_string(),
        t if t == H160::from_str(MKR).unwrap_or_default() => "MKR".to_string(),
        _ => "Unknown".to_string(),
    }
}

// Helper function to get the method ID from input data
pub fn get_method_id_hex(input: &[u8]) -> Option<String> {
    let len = input.len();
    println!("DEBUG get_method_id_hex: input.len={}", len);

    if len < 4 {
        return None;
    }

    let first4 = hex::encode(&input[..4]);
    println!("DEBUG get_method_id_hex: first4=0x{}", first4);

    Some(first4)
}

/// Calculates output amount for Uniswap V3 pools with a given fee tier (fee in hundredths of a bip).
pub fn get_v3_amount_out(amount_in: U256, reserve_in: U256, reserve_out: U256, fee: u32) -> U256 {
    let fee_denominator = U256::from(1_000_000u64);
    let fee_amount = U256::from(fee);
    let amount_in_with_fee = amount_in * (fee_denominator - fee_amount) / fee_denominator;
    let numerator = amount_in_with_fee * reserve_out;
    let denominator = reserve_in * fee_denominator + amount_in_with_fee;
    numerator.checked_div(denominator).unwrap_or_default()
}
