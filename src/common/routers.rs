use ethers::types::H160;
use lazy_static::lazy_static;
use std::str::FromStr;
use std::collections::HashSet;

// Centralized router addresses and swap selectors for multi-DEX support.
lazy_static! {
    // Addresses of known DEX Routers and aggregators.
    pub static ref ROUTER_SET: HashSet<H160> = {
        let mut set = HashSet::new();
        set.insert(H160::from_str("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D").unwrap()); // Uniswap V2
        set.insert(H160::from_str("0xE592427A0AEce92De3Edee1F18E0157C05861564").unwrap()); // Uniswap V3
        set.insert(H160::from_str("0xd9e1cE17f2641f24aE83637ab66a2cca9C378B9F").unwrap()); // SushiSwap
        set.insert(H160::from_str("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45").unwrap()); // Universal Router
        set.insert(H160::from_str("0x1111111254fb6c44bAC0beD2854e76F90643097d").unwrap()); // 1inch
        set.insert(H160::from_str("0xDef1C0ded9bec7F1a1670819833240f027b25EfF").unwrap()); // 0x
        set.insert(H160::from_str("0x11111112542D85B3EF69AE05771c2dCCff4fAa26").unwrap()); // 1inch v4
        set.insert(H160::from_str("0x1111111254EEB25477B68fb85Ed929f73A960582").unwrap()); // 1inch v5
        set.insert(H160::from_str("0xEf1c6E67703c7BD7107eed8303FBe6EC2554BF6B").unwrap()); // Universal Router V2
        set.insert(H160::from_str("0xDEF171Fe48CF0115B1d80b88dc8eAB59176FEe57").unwrap()); // ParaSwap Augustus Swapper V5
        // NEW: aggregators / routers observed since the original release
        set.insert(H160::from_str("0x25d887CE7A35172c62febFd67A1856F20faEbB00").unwrap()); // Pepe / meme aggregator
        // 0x v5
        set.insert(H160::from_str("0xDEF1ABE32c034e558Cdd535791643C58a13aCC10").unwrap());
        // MetaMask swap router
        set.insert(H160::from_str("0x881D40237659C251811CEC9c364ef91dC08D300C").unwrap());
        set.insert(H160::from_str("0x3E66B66Fd1d4e2F8Da5c70762fB54367D115bB62").unwrap()); // Curve
        set.insert(H160::from_str("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D").unwrap()); // Uniswap V2
        set.insert(H160::from_str("0xDef1C0ded9bec7F1a1670819833240f027b25EfF").unwrap()); // 0x
        set
    };
    // 4-byte selectors of swap and router functions across protocols.
    pub static ref SWAP_SELECTOR_SET: HashSet<[u8;4]> = {
        let mut set = HashSet::new();
        // Uniswap V2
        set.insert([0x7f,0xf3,0x6a,0xb5]); // swapExactETHForTokens
        set.insert([0x18,0xcb,0xaf,0xe5]); // swapExactTokensForETH
        set.insert([0x38,0xed,0x17,0x39]); // swapExactTokensForTokens
        set.insert([0x88,0x03,0xdb,0xee]); // swapTokensForExactTokens
        set.insert([0x4a,0x25,0xd9,0x4a]); // swapTokensForExactETH
        set.insert([0xb6,0xf9,0xde,0x95]); // swapExactETHForTokensSupportingFeeOnTransferTokens
        set.insert([0x79,0x1a,0xc9,0x47]); // swapExactTokensForETHSupportingFeeOnTransferTokens
        set.insert([0x5c,0x11,0xd7,0x95]); // swapExactTokensForTokensSupportingFeeOnTransferTokens
        set.insert([0x02,0x2c,0x0d,0x9f]); // UniswapV2Pair.swap
        // Uniswap V3
        set.insert([0x04,0xe4,0x5a,0xaf]); // exactInputSingle
        set.insert([0xc0,0x4b,0x8d,0x59]); // exactInput
        set.insert([0x50,0x23,0xb4,0xdf]); // exactOutputSingle
        set.insert([0x09,0xb8,0x13,0x46]); // exactOutput
        set.insert([0x12,0x8a,0xcb,0x08]); // UniswapV3Pool.swap
        // Universal Router
        set.insert([0x35,0x93,0x56,0x4c]); // execute
        // 0x ExchangeProxy
        set.insert([0xd9,0x62,0x7a,0xa4]); // sellToUniswap
        set.insert([0x6a,0xf4,0x79,0xb2]); // sellTokenForTokenToUniswapV3
        // 1inch Aggregation Router V5
        set.insert([0x12,0xaa,0x3c,0xaf]); // swap
        set.insert([0x2e,0x95,0xb6,0xc8]); // unoswap
        set.insert([0x7c,0x02,0x52,0x00]); // swap (legacy v4)
        // NEW function selectors
        set.insert([0xb8,0x97,0xda,0xeb]); // pepeSwap(address,address)
        set.insert([0x67,0x74,0xb8,0x49]); // unoswapV5 (1inch)
        set.insert([0x12,0xaa,0x3c,0xaf]); // swap (1inch V4/V5)
        set.insert([0x2e,0xb2,0xc2,0xd6]); // simpleSwap (MetaMask)
        set.insert([0x3a,0x45,0x7b,0x3a]); // remove_liquidity_imbalance
        set.insert([0xe8,0xe5,0x25,0xa6]); // remove_liquidity
        set
    };
}

/// Returns true if the given address is a known DEX router.
pub fn is_known_router(addr: &H160) -> bool {
    ROUTER_SET.contains(addr)
}

/// Returns true if the 4-byte selector matches any known swap function.
pub fn is_known_swap_selector(selector: &[u8;4]) -> bool {
    SWAP_SELECTOR_SET.contains(selector)
}
