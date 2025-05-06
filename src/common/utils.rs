/// Utility functions for logging, address conversion, token queries, and currency classification in the Sandooo project.
///
/// Provides helpers for logging, base fee calculations, access list conversions, wallet creation, token queries, and currency classification.
use anyhow::Result;
use ethers::core::rand::thread_rng;
use ethers::prelude::*;
use ethers::{
    self,
    types::{
        transaction::eip2930::{AccessList, AccessListItem},
        U256,
    },
};
use fern::colors::{Color, ColoredLevelConfig};
use foundry_evm_mini::evm::utils::{b160_to_h160, h160_to_b160, ru256_to_u256, u256_to_ru256};
use log::LevelFilter;
use rand::Rng;
use revm::primitives::{B160, U256 as rU256};
use std::str::FromStr;
use std::sync::Arc;

use crate::common::constants::*;

/// Sets up a colored logger for the project.
///
/// # Returns
/// * `Result<()>` - Ok if successful.
pub fn setup_logger() -> Result<()> {
    let colors = ColoredLevelConfig {
        trace: Color::Cyan,
        debug: Color::Magenta,
        info: Color::Green,
        warn: Color::Red,
        error: Color::BrightRed,
        ..ColoredLevelConfig::new()
    };

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{}[{}] {}",
                chrono::Local::now().format("[%H:%M:%S]"),
                colors.color(record.level()),
                message
            ))
        })
        .chain(std::io::stdout())
        .level(log::LevelFilter::Error)
        .level_for(PROJECT_NAME, LevelFilter::Info)
        .apply()?;

    Ok(())
}

/// Calculates the next block's base fee according to EIP-1559 rules.
///
/// # Parameters
/// * `gas_used`: U256 - Gas used in the block.
/// * `gas_limit`: U256 - Gas limit of the block.
/// * `base_fee_per_gas`: U256 - Current base fee.
///
/// # Returns
/// * `U256` - Next block's base fee.
pub fn calculate_next_block_base_fee(
    gas_used: U256,
    gas_limit: U256,
    base_fee_per_gas: U256,
) -> U256 {
    let gas_used = gas_used;

    let mut target_gas_used = gas_limit / 2;
    target_gas_used = if target_gas_used == U256::zero() {
        U256::one()
    } else {
        target_gas_used
    };

    let new_base_fee = {
        if gas_used > target_gas_used {
            base_fee_per_gas
                + ((base_fee_per_gas * (gas_used - target_gas_used)) / target_gas_used)
                    / U256::from(8u64)
        } else {
            base_fee_per_gas
                - ((base_fee_per_gas * (target_gas_used - gas_used)) / target_gas_used)
                    / U256::from(8u64)
        }
    };

    let seed = rand::thread_rng().gen_range(0..9);
    new_base_fee + seed
}

/// Converts a revm access list to ethers-rs format.
///
/// # Parameters
/// * `access_list`: Vec<(B160, Vec<rU256>)> - revm access list.
///
/// # Returns
/// * `AccessList` - ethers-rs access list.
pub fn access_list_to_ethers(access_list: Vec<(B160, Vec<rU256>)>) -> AccessList {
    AccessList::from(
        access_list
            .into_iter()
            .map(|(address, slots)| AccessListItem {
                address: b160_to_h160(address),
                storage_keys: slots
                    .into_iter()
                    .map(|y| H256::from_uint(&ru256_to_u256(y)))
                    .collect(),
            })
            .collect::<Vec<AccessListItem>>(),
    )
}

/// Converts an ethers-rs access list to revm format.
///
/// # Parameters
/// * `access_list`: AccessList - ethers-rs access list.
///
/// # Returns
/// * `Vec<(B160, Vec<rU256>)>` - revm access list.
pub fn access_list_to_revm(access_list: AccessList) -> Vec<(B160, Vec<rU256>)> {
    access_list
        .0
        .into_iter()
        .map(|x| {
            (
                h160_to_b160(x.address),
                x.storage_keys
                    .into_iter()
                    .map(|y| u256_to_ru256(y.0.into()))
                    .collect(),
            )
        })
        .collect()
}

abigen!(
    IERC20,
    r#"[
        function balanceOf(address) external view returns (uint256)
    ]"#,
);

/// Gets the ERC-20 token balance for an address.
///
/// # Parameters
/// * `provider`: Arc<Provider<Ws>> - The Ethereum provider.
/// * `owner`: H160 - The address to query.
/// * `token`: H160 - The token contract address.
///
/// # Returns
/// * `Result<U256>` - The token balance.
pub async fn get_token_balance(
    provider: Arc<Provider<Ws>>,
    owner: H160,
    token: H160,
) -> Result<U256> {
    let contract = IERC20::new(token, provider);
    let token_balance = contract.balance_of(owner).call().await?;
    Ok(token_balance)
}

/// Creates a new random wallet and returns its address.
///
/// # Returns
/// * `(LocalWallet, H160)` - The wallet and its address.
pub fn create_new_wallet() -> (LocalWallet, H160) {
    let wallet = LocalWallet::new(&mut thread_rng());
    let address = wallet.address();
    (wallet, address)
}

/// Converts a static string Ethereum address to H160.
///
/// # Parameters
/// * `str_address`: &'static str - Address as string.
///
/// # Returns
/// * `H160` - The address as H160.
pub fn to_h160(str_address: &'static str) -> H160 {
    H160::from_str(str_address).unwrap()
}

/// Checks if a token address is WETH.
///
/// # Parameters
/// * `token_address`: H160 - Token address.
///
/// # Returns
/// * `bool` - True if WETH.
pub fn is_weth(token_address: H160) -> bool {
    token_address == to_h160(WETH)
}

/// Checks if a token address is a main currency (WETH, USDT, USDC, WBTC, DAI, LINK, MKR).
///
/// # Parameters
/// * `token_address`: H160 - Token address.
///
/// # Returns
/// * `bool` - True if main currency.
pub fn is_main_currency(token_address: H160) -> bool {
    let main_currencies = vec![
        to_h160(WETH),
        to_h160(USDT),
        to_h160(USDC),
        to_h160(WBTC),
        to_h160(DAI),
        to_h160(LINK),
        to_h160(MKR),
    ];
    main_currencies.contains(&token_address)
}

/// Main currency enum for classification.
#[derive(Debug, Clone)]
pub enum MainCurrency {
    /// Wrapped Ether (WETH).
    WETH,
    /// Tether (USDT).
    USDT,
    /// USD Coin (USDC).
    USDC,
    /// Wrapped Bitcoin (WBTC).
    WBTC,
    /// Dai Stablecoin (DAI).
    DAI,
    /// Chainlink Token (LINK).
    LINK,
    /// Maker (MKR).
    MKR,
    /// Default (fallback to WETH).
    Default, // Pairs that aren't WETH/Stable pairs. Default to WETH for now
}

impl MainCurrency {
    /// Creates a MainCurrency from an address.
    pub fn new(address: H160) -> Self {
        if address == to_h160(WETH) {
            MainCurrency::WETH
        } else if address == to_h160(USDT) {
            MainCurrency::USDT
        } else if address == to_h160(USDC) {
            MainCurrency::USDC
        } else if address == to_h160(WBTC) {
            MainCurrency::WBTC
        } else if address == to_h160(DAI) {
            MainCurrency::DAI
        } else if address == to_h160(LINK) {
            MainCurrency::LINK
        } else if address == to_h160(MKR) {
            MainCurrency::MKR
        } else {
            MainCurrency::Default
        }
    }

    /// Returns the decimals for the main currency.
    pub fn decimals(&self) -> u8 {
        match self {
            MainCurrency::WETH => WETH_DECIMALS,
            MainCurrency::USDT => USDT_DECIMALS,
            MainCurrency::USDC => USDC_DECIMALS,
            MainCurrency::WBTC => WBTC_DECIMALS,
            MainCurrency::DAI => DAI_DECIMALS,
            MainCurrency::LINK => LINK_DECIMALS,
            MainCurrency::MKR => MKR_DECIMALS,
            MainCurrency::Default => WETH_DECIMALS,
        }
    }

    /// Returns the storage slot for the main currency.
    pub fn balance_slot(&self) -> i32 {
        match self {
            MainCurrency::WETH => WETH_BALANCE_SLOT,
            MainCurrency::USDT => USDT_BALANCE_SLOT,
            MainCurrency::USDC => USDC_BALANCE_SLOT,
            MainCurrency::WBTC => WBTC_BALANCE_SLOT,
            MainCurrency::DAI => DAI_BALANCE_SLOT,
            MainCurrency::LINK => LINK_BALANCE_SLOT,
            MainCurrency::MKR => MKR_BALANCE_SLOT,
            MainCurrency::Default => WETH_BALANCE_SLOT,
        }
    }

    /// Returns the weight (importance) of the main currency.
    pub fn weight(&self) -> u8 {
        match self {
            MainCurrency::WETH => 7,    // Highest priority
            MainCurrency::WBTC => 6,    // High priority for BTC
            MainCurrency::USDT => 5,    // Stablecoin priority
            MainCurrency::USDC => 4,    // Stablecoin priority
            MainCurrency::DAI => 3,     // Stablecoin priority
            MainCurrency::LINK => 2,    // Lower priority for LINK
            MainCurrency::MKR => 1,     // Lowest priority for MKR
            MainCurrency::Default => 7, // default is WETH
        }
    }

    /// Returns the Chainlink price feed address for the token
    pub fn chainlink_feed(&self) -> Option<&'static str> {
        match self {
            MainCurrency::WETH => Some(CHAINLINK_ETH_USD),
            MainCurrency::USDT => Some(CHAINLINK_USDT_USD),
            MainCurrency::USDC => Some(CHAINLINK_USDC_USD),
            MainCurrency::WBTC => Some(CHAINLINK_BTC_USD),
            MainCurrency::DAI => Some(CHAINLINK_DAI_USD),
            MainCurrency::LINK => Some(CHAINLINK_LINK_USD),
            MainCurrency::MKR => Some(CHAINLINK_MKR_USD),
            MainCurrency::Default => None,
        }
    }

    /// Returns the token address as a string
    pub fn address_str(&self) -> &'static str {
        match self {
            MainCurrency::WETH => WETH,
            MainCurrency::USDT => USDT,
            MainCurrency::USDC => USDC,
            MainCurrency::WBTC => WBTC,
            MainCurrency::DAI => DAI,
            MainCurrency::LINK => LINK,
            MainCurrency::MKR => MKR,
            MainCurrency::Default => WETH,
        }
    }

    /// Returns the token address as H160
    pub fn address(&self) -> H160 {
        H160::from_str(self.address_str()).unwrap()
    }
}

/// Determines which token is the main currency and which is the target in a pair.
///
/// # Parameters
/// * `token0`: H160 - First token address.
/// * `token1`: H160 - Second token address.
///
/// # Returns
/// * `Option<(H160, H160)>` - (main, target) or None if neither is main.
pub fn return_main_and_target_currency(token0: H160, token1: H160) -> Option<(H160, H160)> {
    let token0_supported = is_main_currency(token0);
    let token1_supported = is_main_currency(token1);

    if !token0_supported && !token1_supported {
        return None;
    }

    if token0_supported && token1_supported {
        let mc0 = MainCurrency::new(token0);
        let mc1 = MainCurrency::new(token1);

        let token0_weight = mc0.weight();
        let token1_weight = mc1.weight();

        if token0_weight > token1_weight {
            return Some((token0, token1));
        } else {
            return Some((token1, token0));
        }
    }

    if token0_supported {
        return Some((token0, token1));
    } else {
        return Some((token1, token0));
    }
}
