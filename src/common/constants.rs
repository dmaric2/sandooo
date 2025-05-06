/// Defines global constants and environment configuration for the Sandooo project.
///
/// Contains static addresses, environment variable parsing, and token metadata.
pub static PROJECT_NAME: &str = "sandooo";

/// Retrieves the value of an environment variable by key, or returns an empty string if not found.
///
/// # Parameters
/// * `key`: &str - The environment variable name.
///
/// # Returns
/// * `String` - The value of the environment variable, or an empty string if not set.
pub fn get_env(key: &str) -> String {
    std::env::var(key).unwrap_or(String::from(""))
}

/// Holds all environment configuration loaded from environment variables.
///
/// Fields:
/// - `https_url`, `wss_url`: RPC endpoints for HTTPS and WebSocket connections.
/// - `bot_address`, `private_key`, `identity_key`: Credentials for bot operation.
/// - `telegram_token`, `telegram_chat_id`, `use_alert`, `debug`: Telegram and debug settings.
#[derive(Debug, Clone)]
pub struct Env {
    /// HTTPS endpoint for RPC connections.
    pub https_url: String,
    /// WebSocket endpoint for RPC connections.
    pub wss_url: String,
    /// Address of the bot.
    pub bot_address: String,
    /// Private key for bot operation.
    pub private_key: String,
    /// Identity key for bot operation.
    pub identity_key: String,
    /// Telegram token for bot operation.
    pub telegram_token: String,
    /// Telegram chat ID for bot operation.
    pub telegram_chat_id: String,
    /// Whether to use alerts.
    pub use_alert: bool,
    /// Whether to enable debug mode.
    pub debug: bool,
}

impl Env {
    /// Loads environment variables and constructs an `Env` instance.
    ///
    /// # Returns
    /// * `Env` - Populated configuration struct.
    ///
    /// # Panics
    /// Panics if parsing booleans fails for `use_alert` or `debug`.
    pub fn new() -> Self {
        Env {
            https_url: get_env("HTTPS_URL"),
            wss_url: get_env("WSS_URL"),
            bot_address: get_env("BOT_ADDRESS"),
            private_key: get_env("PRIVATE_KEY"),
            identity_key: get_env("IDENTITY_KEY"),
            telegram_token: get_env("TELEGRAM_TOKEN"),
            telegram_chat_id: get_env("TELEGRAM_CHAT_ID"),
            use_alert: get_env("USE_ALERT").parse::<bool>().unwrap(),
            debug: get_env("DEBUG").parse::<bool>().unwrap(),
        }
    }
}

/// Static address for the Flashbots Builder coinbase.
pub static COINBASE: &str = "0xDAFEA492D9c6733ae3d56b7Ed1ADB60692c98Bc5"; // Flashbots Builder

/// Static addresses for contracts and major tokens on Ethereum mainnet.
pub static WETH: &str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
pub static USDT: &str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
pub static USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
pub static WBTC: &str = "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599";
pub static DAI: &str = "0x6B175474E89094C44Da98b954EedeAC495271d0F";
pub static LINK: &str = "0x514910771AF9Ca656af840dff83E8264EcF986CA";
pub static MKR: &str = "0x9f8F72aA9304c8B593d555F12eF6589cC3A579A2";

/// Aave V3 Pool contract address on Ethereum mainnet
pub static AAVE_V3_POOL: &str = "0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2";

/// SandoooV3 contract address for flashloan-based sandwich attacks
/// Note: This is a placeholder and should be replaced with the actual deployed address
pub static SANDOOO_V3_ADDRESS: &str = "0x5770764057f164492BfcE78b4E9c9e49Ee735504";

/// Flashloan fee percentage in basis points (0.09% for Aave V3)
pub static FLASHLOAN_FEE_BASIS_POINTS: u64 = 9;
pub static BASIS_POINTS_DIVISOR: u64 = 10000;

/*
Can figure out the balance slot of ERC-20 tokens using the:
EvmSimulator::get_balance_slot method

However, note that this does not work for all tokens.
Especially tokens that are using proxy patterns.
*/
/// Storage slot for WETH balance (for direct storage access).
pub static WETH_BALANCE_SLOT: i32 = 3;
/// Storage slot for USDT balance (for direct storage access).
pub static USDT_BALANCE_SLOT: i32 = 2;
/// Storage slot for USDC balance (for direct storage access).
pub static USDC_BALANCE_SLOT: i32 = 9;
/// Storage slot for WBTC balance (for direct storage access).
pub static WBTC_BALANCE_SLOT: i32 = 0;
/// Storage slot for DAI balance (for direct storage access).
pub static DAI_BALANCE_SLOT: i32 = 2;
/// Storage slot for LINK balance (for direct storage access).
pub static LINK_BALANCE_SLOT: i32 = 1;
/// Storage slot for MKR balance (for direct storage access).
pub static MKR_BALANCE_SLOT: i32 = 1;

/// Number of decimals for WETH.
pub static WETH_DECIMALS: u8 = 18;
/// Number of decimals for USDT.
pub static USDT_DECIMALS: u8 = 6;
/// Number of decimals for USDC.
pub static USDC_DECIMALS: u8 = 6;
/// Number of decimals for WBTC.
pub static WBTC_DECIMALS: u8 = 8;
/// Number of decimals for DAI.
pub static DAI_DECIMALS: u8 = 18;
/// Number of decimals for LINK.
pub static LINK_DECIMALS: u8 = 18;
/// Number of decimals for MKR.
pub static MKR_DECIMALS: u8 = 18;

/// Chainlink price feed addresses for major tokens
pub static CHAINLINK_ETH_USD: &str = "0x5f4eC3Df9cbd43714FE2740f5E3616155c5b8419";
pub static CHAINLINK_BTC_USD: &str = "0xF4030086522a5bEEa4988F8cA5B36dbC97BeE88c";
pub static CHAINLINK_USDT_USD: &str = "0x3E7d1eAB13ad0104d2750B8863b489D65364e32D";
pub static CHAINLINK_USDC_USD: &str = "0x8fFfFfd4AfB6115b954Bd326cbe7B4BA576818f6";
pub static CHAINLINK_DAI_USD: &str = "0xAed0c38402a5d19df6E4c03F4E2DceD6e29c1ee9";
pub static CHAINLINK_LINK_USD: &str = "0x2c1d072e956AFFC0D435Cb7AC38EF18d24d9127c";
pub static CHAINLINK_MKR_USD: &str = "0xec1D1B3b0443256cc3860e24a46F108e699484Aa";
