use ethers::types::Transaction;
use crate::common::routers::{is_known_router, is_known_swap_selector};
use crate::common::contract_detector::{is_pool_contract, is_router_like};
use ethers::providers::Middleware;

#[derive(Debug, PartialEq)]
pub enum TxKind {
    EthTransfer,
    Erc20Approve,
    Erc20Transfer,
    Swap,
    Other,
}

/// Classifies transactions from the mempool to filter non-swaps.
pub async fn classify_transaction<M: Middleware>(provider: &M, tx: &Transaction) -> TxKind {
    // 1. Address-based detection for known routers
    if let Some(to) = tx.to {
        if is_known_router(&to) {
            return TxKind::Swap;
        }
    }

    // 2. Empty calldata => ETH transfer
    let data = tx.input.as_ref();
    if data.is_empty() {
        return TxKind::EthTransfer;
    }

    // 3. Method ID-based checks
    if data.len() >= 4 {
        let mut method_id = [0u8; 4];
        method_id.copy_from_slice(&data[..4]);
        // 3a. Swap selectors (routers & pools)
        if is_known_swap_selector(&method_id) {
            return TxKind::Swap;
        }
        // 3b. ERC20 approve/transfer
        match method_id {
            [0x09,0x5e,0xa7,0xb3] => return TxKind::Erc20Approve,
            [0xa9,0x05,0x9c,0xbb] => return TxKind::Erc20Transfer,
            _ => {},
        }
    }

    // 4. Heuristic fallback: inspect unknown contracts we’ve never seen before.
    if let Some(to_addr) = tx.to {
        // a) looks like a router we don’t have in the static list (async detection)
        if is_router_like(provider, &to_addr).await {
            return TxKind::Swap;
        }
        // b) direct interaction with a pair contract (async detection)
        if is_pool_contract(provider, &to_addr).await {
            return TxKind::Swap;
        }
    }
    TxKind::Other
}
