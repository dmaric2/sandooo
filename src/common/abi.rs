/// Module for managing and providing contract ABI (Application Binary Interface) definitions.
///
/// Contains the `Abi` struct, which holds pre-parsed contract ABIs for use throughout the application.
use ethers::abi::parse_abi;
use ethers::prelude::BaseContract;

/// Holds parsed contract ABIs for core DeFi components.
///
/// - `factory`: ABI for the factory contract (provides `getPair`).
/// - `pair`: ABI for the pair contract (provides `token0`, `token1`, `getReserves`).
/// - `token`: ABI for ERC-20 token contract (provides common token functions).
/// - `sando_bot`: ABI for the Sandooo bot contract (provides `recoverToken`).
/// - `sando_v3`: ABI for the SandoooV3 contract with Aave V3 flashloan integration.
#[derive(Clone, Debug)]
pub struct Abi {
    /// Factory contract ABI (UniswapV2-like), with `getPair(address,address)`
    pub factory: BaseContract,
    /// Pair contract ABI, with `token0()`, `token1()`, and `getReserves()`
    pub pair: BaseContract,
    /// ERC-20 token contract ABI, with standard token functions
    pub token: BaseContract,
    /// Sandooo bot contract ABI, with `recoverToken(address,uint256)`
    pub sando_bot: BaseContract,
    /// SandoooV3 contract ABI with Aave V3 flashloan integration
    pub sando_v3: BaseContract,
}

impl Abi {
    /// Creates a new `Abi` instance with pre-parsed contract ABIs for factory, pair, token, and sando_bot.
    ///
    /// # Returns
    /// * `Abi` - Struct containing initialized contract ABIs for use in encoding/decoding calls.
    ///
    /// # Panics
    /// Panics if any ABI parsing fails (should not happen with hardcoded ABIs).
    pub fn new() -> Self {
        let factory = BaseContract::from(
            parse_abi(&["function getPair(address,address) external view returns (address)"])
                .unwrap(),
        );

        let pair = BaseContract::from(
            parse_abi(&[
                "function token0() external view returns (address)",
                "function token1() external view returns (address)",
                "function getReserves() external view returns (uint112,uint112,uint32)",
            ])
            .unwrap(),
        );

        let token = BaseContract::from(
            parse_abi(&[
                "function owner() external view returns (address)",
                "function name() external view returns (string)",
                "function symbol() external view returns (string)",
                "function decimals() external view returns (uint8)",
                "function totalSupply() external view returns (uint256)",
                "function balanceOf(address) external view returns (uint256)",
                "function approve(address,uint256) external view returns (bool)",
                "function transfer(address,uint256) external returns (bool)",
                "function allowance(address,address) external view returns (uint256)",
            ])
            .unwrap(),
        );

        let sando_bot = BaseContract::from(
            parse_abi(&["function recoverToken(address,uint256) public"]).unwrap(),
        );

        let sando_v3 = BaseContract::from(
            parse_abi(&[
                "function executeSandwichWithFlashloan(address,uint256,bytes) external",
                "function executeOperation(address,uint256,uint256,address,bytes) external returns (bool)",
                "function recoverToken(address,uint256) external",
                "function recoverETH() external",
            ])
            .unwrap(),
        );

        Self {
            factory,
            pair,
            token,
            sando_bot,
            sando_v3,
        }
    }
}
