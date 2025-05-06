/// Token registry for managing token metadata and interactions.
///
/// This module provides a central registry for token information, making token handling more data-driven
/// and easier to extend with new tokens.
use anyhow::Result;
use ethers::abi::{ParamType, Token};
use ethers::prelude::*;
use ethers::providers::{Provider, Ws};
use ethers::types::transaction::eip2718::TypedTransaction;
use lazy_static::lazy_static;
use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use crate::common::constants::*;
use crate::common::utils::MainCurrency;

/// Represents detailed token metadata for use in the bot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadata {
    /// Token address (lowercase hex string without 0x prefix)
    pub address: String,
    /// Token name
    pub name: String,
    /// Token symbol
    pub symbol: String,
    /// Number of decimal places
    pub decimals: u8,
    /// Storage slot for balance mapping (for EVM simulation)
    pub balance_slot: i32,
    /// Chainlink price feed address (if available)
    pub price_feed: Option<String>,
    /// Whether this is a main currency the bot supports for sandwiching
    pub is_main_currency: bool,
    /// Priority/weight in pair selection (higher = more preferred)
    pub weight: u8,
    /// Cached last known price in USD (updated periodically)
    pub last_price_usd: Option<f64>,
}

/// The token registry providing access to token metadata.
#[derive(Debug)]
pub struct TokenRegistry {
    /// Map of token address to metadata
    tokens: RwLock<HashMap<H160, TokenMetadata>>,
    /// Path to JSON cache file
    cache_path: String,
}

impl TokenRegistry {
    /// Creates a new token registry and loads tokens from the config file.
    ///
    /// # Returns
    /// * `Self` - New TokenRegistry instance
    pub fn new() -> Self {
        let cache_path = "cache/token_registry.json".to_string();
        let tokens = RwLock::new(HashMap::new());
        let registry = Self { tokens, cache_path };

        // Initialize with built-in tokens
        registry.initialize_default_tokens();
        registry.load_from_cache();
        registry
    }

    /// Initializes the registry with the default supported tokens.
    fn initialize_default_tokens(&self) {
        let default_tokens = [
            self.create_token_metadata(
                WETH,
                "Wrapped Ether",
                "WETH",
                18,
                WETH_BALANCE_SLOT,
                Some(CHAINLINK_ETH_USD),
                true,
                7,
            ),
            self.create_token_metadata(
                USDT,
                "Tether USD",
                "USDT",
                6,
                USDT_BALANCE_SLOT,
                Some(CHAINLINK_USDT_USD),
                true,
                5,
            ),
            self.create_token_metadata(
                USDC,
                "USD Coin",
                "USDC",
                6,
                USDC_BALANCE_SLOT,
                Some(CHAINLINK_USDC_USD),
                true,
                4,
            ),
            self.create_token_metadata(
                WBTC,
                "Wrapped Bitcoin",
                "WBTC",
                8,
                WBTC_BALANCE_SLOT,
                Some(CHAINLINK_BTC_USD),
                true,
                6,
            ),
            self.create_token_metadata(
                DAI,
                "Dai Stablecoin",
                "DAI",
                18,
                DAI_BALANCE_SLOT,
                Some(CHAINLINK_DAI_USD),
                true,
                3,
            ),
            self.create_token_metadata(
                LINK,
                "ChainLink Token",
                "LINK",
                18,
                LINK_BALANCE_SLOT,
                Some(CHAINLINK_LINK_USD),
                true,
                2,
            ),
            self.create_token_metadata(
                MKR,
                "Maker",
                "MKR",
                18,
                MKR_BALANCE_SLOT,
                Some(CHAINLINK_MKR_USD),
                true,
                1,
            ),
        ];

        let mut tokens = self.tokens.write().unwrap();
        for token in default_tokens.iter() {
            let address = H160::from_str(&format!("0x{}", token.address)).unwrap();
            tokens.insert(address, token.clone());
        }
    }

    /// Creates TokenMetadata for a token.
    fn create_token_metadata(
        &self,
        address: &str,
        name: &str,
        symbol: &str,
        decimals: u8,
        balance_slot: i32,
        price_feed: Option<&str>,
        is_main_currency: bool,
        weight: u8,
    ) -> TokenMetadata {
        // Remove 0x prefix if present
        let clean_address = address.trim_start_matches("0x").to_lowercase();

        TokenMetadata {
            address: clean_address,
            name: name.to_string(),
            symbol: symbol.to_string(),
            decimals,
            balance_slot,
            price_feed: price_feed.map(|s| s.to_string()),
            is_main_currency,
            weight,
            last_price_usd: None,
        }
    }

    /// Loads tokens from the JSON cache file.
    fn load_from_cache(&self) {
        let path = Path::new(&self.cache_path);
        if !path.exists() {
            // Save default tokens to create the cache file
            self.save_to_cache();
            return;
        }

        match File::open(path) {
            Ok(mut file) => {
                let mut contents = String::new();
                if file.read_to_string(&mut contents).is_ok() {
                    if let Ok(cache_data) = serde_json::from_str::<Vec<TokenMetadata>>(&contents) {
                        let mut tokens = self.tokens.write().unwrap();
                        for token in cache_data {
                            let address =
                                H160::from_str(&format!("0x{}", token.address)).unwrap_or_default();
                            if !address.is_zero() {
                                tokens.insert(address, token);
                            }
                        }
                        debug!("Loaded {} tokens from cache", tokens.len());
                    }
                }
            }
            Err(_) => {
                info!("Failed to load token cache, using defaults");
            }
        }
    }

    /// Saves the current token registry to the cache file.
    pub fn save_to_cache(&self) {
        let tokens = self.tokens.read().unwrap();
        let token_vec: Vec<TokenMetadata> = tokens.values().cloned().collect();

        // Ensure directory exists
        if let Some(parent) = Path::new(&self.cache_path).parent() {
            std::fs::create_dir_all(parent).ok();
        }

        if let Ok(file) = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.cache_path)
        {
            if let Ok(json) = serde_json::to_string_pretty(&token_vec) {
                let mut writer = std::io::BufWriter::new(file);
                if writer.write_all(json.as_bytes()).is_ok() {
                    writer.flush().ok();
                    debug!("Saved {} tokens to cache", tokens.len());
                }
            }
        }
    }

    /// Gets metadata for a token by its address.
    ///
    /// # Parameters
    /// * `address`: H160 - Token address
    ///
    /// # Returns
    /// * `Option<TokenMetadata>` - Token metadata if found
    pub fn get_token(&self, address: H160) -> Option<TokenMetadata> {
        let tokens = self.tokens.read().unwrap();
        tokens.get(&address).cloned()
    }

    /// Gets metadata for a main currency token by its enum value.
    ///
    /// # Parameters
    /// * `currency`: MainCurrency - Main currency enum
    ///
    /// # Returns
    /// * `Option<TokenMetadata>` - Token metadata if found
    pub fn get_main_currency(&self, currency: &MainCurrency) -> Option<TokenMetadata> {
        let address = currency.address();
        self.get_token(address)
    }

    /// Gets all supported main currencies.
    ///
    /// # Returns
    /// * `Vec<TokenMetadata>` - All main currencies in the registry
    pub fn get_main_currencies(&self) -> Vec<TokenMetadata> {
        let tokens = self.tokens.read().unwrap();
        tokens
            .values()
            .filter(|t| t.is_main_currency)
            .cloned()
            .collect()
    }

    /// Adds or updates a token in the registry.
    ///
    /// # Parameters
    /// * `metadata`: TokenMetadata - Token metadata to add or update
    ///
    /// # Returns
    /// * `Result<()>` - Success or error
    pub fn register_token(&self, metadata: TokenMetadata) -> Result<()> {
        let address = H160::from_str(&format!("0x{}", metadata.address))?;
        let mut tokens = self.tokens.write().unwrap();
        tokens.insert(address, metadata);
        Ok(())
    }

    /// Updates price information for a token.
    ///
    /// # Parameters
    /// * `address`: H160 - Token address
    /// * `price_usd`: f64 - Price in USD
    ///
    /// # Returns
    /// * `Result<()>` - Success or error
    pub fn update_price(&self, address: H160, price_usd: f64) -> Result<()> {
        let mut tokens = self.tokens.write().unwrap();
        if let Some(token) = tokens.get_mut(&address) {
            token.last_price_usd = Some(price_usd);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Token not found"))
        }
    }

    /// Fetches on-chain token information and updates the registry.
    ///
    /// # Parameters
    /// * `provider`: &Arc<Provider<Ws>> - Provider for making calls
    /// * `address`: H160 - Token address
    ///
    /// # Returns
    /// * `Result<TokenMetadata>` - Updated token metadata
    pub async fn fetch_token_info(
        &self,
        provider: &Arc<Provider<Ws>>,
        address: H160,
    ) -> Result<TokenMetadata> {
        // Check if we already have this token
        if let Some(existing) = self.get_token(address) {
            return Ok(existing);
        }

        // ABI for token functions
        let functions = [
            ("name()", ParamType::String),
            ("symbol()", ParamType::String),
            ("decimals()", ParamType::Uint(8)),
        ];

        let mut name = "Unknown".to_string();
        let mut symbol = "UNK".to_string();
        let mut decimals = 18u8;

        for (func_sig, return_type) in functions.iter() {
            let data = ethers::utils::keccak256(func_sig.as_bytes())[..4].to_vec();

            let tx_request = TransactionRequest::new().to(address).data(data.clone());

            // Convert to TypedTransaction
            let tx: TypedTransaction = tx_request.into();

            if let Ok(result) = provider.call(&tx, None).await {
                if !result.is_empty() {
                    let mut decoded = ethers::abi::decode(&[return_type.clone()], &result);
                    if let Ok(ref mut tokens) = decoded {
                        if !tokens.is_empty() {
                            match *func_sig {
                                "name()" => {
                                    if let Some(Token::String(value)) = tokens.pop() {
                                        name = value;
                                    }
                                }
                                "symbol()" => {
                                    if let Some(Token::String(value)) = tokens.pop() {
                                        symbol = value;
                                    }
                                }
                                "decimals()" => {
                                    if let Some(Token::Uint(value)) = tokens.pop() {
                                        decimals = value.as_u32() as u8;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Create new token metadata
        let token = self.create_token_metadata(
            &format!("{:x}", address),
            &name,
            &symbol,
            decimals,
            0,     // Unknown balance slot
            None,  // No price feed
            false, // Not a main currency
            0,     // No weight
        );

        // Register the token
        self.register_token(token.clone())?;

        Ok(token)
    }

    /// Updates token prices from Chainlink price feeds.
    ///
    /// # Parameters
    /// * `provider`: &Arc<Provider<Ws>> - Provider for making calls
    ///
    /// # Returns
    /// * `Result<()>` - Success or error
    pub async fn update_prices_from_chainlink(&self, provider: &Arc<Provider<Ws>>) -> Result<()> {
        let tokens = self.tokens.read().unwrap();

        // ABI for Chainlink price feed function
        let function = "latestRoundData()";
        let function_signature = ethers::utils::keccak256(function.as_bytes())[..4].to_vec();

        for (address, token) in tokens.iter() {
            if let Some(price_feed_str) = &token.price_feed {
                if let Ok(price_feed) =
                    H160::from_str(&format!("0x{}", price_feed_str.trim_start_matches("0x")))
                {
                    let tx_request = TransactionRequest::new()
                        .to(price_feed)
                        .data(function_signature.clone());

                    // Convert to TypedTransaction
                    let tx: TypedTransaction = tx_request.into();

                    if let Ok(result) = provider.call(&tx, None).await {
                        if result.len() >= 96 {
                            // Decode the answer (second 32-byte value in the response)
                            let price_bytes = &result[32..64];
                            if let Ok(price) = U256::from_big_endian(price_bytes).try_into() {
                                let price_u128: u128 = price;
                                // Chainlink prices typically have 8 decimals for USD pairs
                                let price_usd = price_u128 as f64 / 1e8;
                                self.update_price(*address, price_usd)?;
                            }
                        }
                    }
                }
            }
        }

        // Save updated prices to cache
        self.save_to_cache();

        Ok(())
    }
}

/// Provides a global instance of the token registry.
pub struct TokenRegistryProvider {
    registry: Arc<TokenRegistry>,
}

impl TokenRegistryProvider {
    /// Creates a new registry provider.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(TokenRegistry::new()),
        }
    }

    /// Returns the token registry.
    pub fn registry(&self) -> Arc<TokenRegistry> {
        self.registry.clone()
    }
}

lazy_static! {
    static ref TOKEN_REGISTRY: Arc<TokenRegistry> = Arc::new(TokenRegistry::new());
}

/// Gets the token registry instance.
pub fn get_token_registry() -> Arc<TokenRegistry> {
    TOKEN_REGISTRY.clone()
}
