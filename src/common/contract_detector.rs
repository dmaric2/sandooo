use ethers::types::H160;
use ethers::providers::Middleware;

const PAIR_TOKEN0_SELECTOR: [u8;4] = [0x0d, 0xfe, 0x16, 0x81]; // token0()
const PAIR_TOKEN1_SELECTOR: [u8;4] = [0xd2, 0x12, 0x20, 0xa7]; // token1()

/// On-chain heuristics the classifier can use with a provider to detect pool/router contracts.
pub async fn is_pool_contract<M: Middleware>(provider: &M, addr: &H160) -> bool {
    // dynamic detection via bytecode introspection
    match provider.get_code(*addr, None).await {
        Ok(code_bytes) => {
            let code = code_bytes.as_ref();
            if code.len() < 100 { return false; }
            // must contain both token0() and token1() selectors
            code.windows(4).any(|w| w == PAIR_TOKEN0_SELECTOR)
                && code.windows(4).any(|w| w == PAIR_TOKEN1_SELECTOR)
        }
        Err(_) => false,
    }
}

pub async fn is_router_like<M: Middleware>(provider: &M, addr: &H160) -> bool {
    // heuristic: routers typically have larger bytecode payloads
    match provider.get_code(*addr, None).await {
        Ok(code_bytes) => code_bytes.as_ref().len() > 1000,
        Err(_) => false,
    }
}
