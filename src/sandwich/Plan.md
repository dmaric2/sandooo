Based on a close read of your current codebase, here are the concrete gaps you’ll need to fill—and some targeted steps—to move from prototype to a production‐grade sandwich bot:

1. Unify your swap‐detection
• Right now extract_swap_info only checks is_known_router and a simple pools‐map lookup. You should call your existing classify_transaction(tx) (which already covers routers + method‐ID selectors) at the top of extract_swap_info.
• If it still isn’t marked as a swap, invoke your debug_trace_call → extract_logs path to scan for V2/V3 Swap events (0xd78ad95f…, 0x128acb08…) in the trace. That catches any internal pool calls or unusual router wrappers.
2. Robust path‐decoding & pools support
• Your get_token_paths uses manual byte slicing keyed off a handful of selectors. Replace it with a real ABI‐decoder (e.g. ethers::abi::AbiDecode) so you can handle arbitrary multi‐hop paths and V3 “encoded bytes” routes reliably.
• Ensure your pool registry (the pools_map) is kept up to date on every new block—either by reindexing logs in your NewBlock handler or subscribing to “Sync” events so reserves never go stale.
3. Fast on-chain simulation & sizing
• In BatchSandwich.simulate and Sandwich.optimize, you already spin up an EvmSimulator. Extend that to do a binary‐search or hill-climb on front/back amounts (e.g. 5 steps per candidate) to find the maximum profitable trade, factoring in the block’s base_fee and max_fee.
• Gate any bundle under a minimum net‐profit threshold (e.g. ≥0.02 ETH) so you don’t waste gas on tiny opportunities.
4. Flashbots bundling
• Add a module (e.g. sandwich/bundler.rs) that wraps FlashbotsMiddleware from ethers-flashbots. It should:
– Pre-sign your front-run, victim (as calldata‐only wrapper), and back-run txs.
– Bundle them and send via eth_sendBundle.
– Handle replacement/nonce management (track nonces locally per block).
5. Mem-pool subscription & latency
• Move from polling to a direct WebSocket subscription to pending transactions (you’ve already wired NewPendingTx). Make sure you’re running on an archival or “fast” node, colocated if possible, to shave 20–50 ms.
• Pre-cache ABI IDs and router addresses in a static set (using lazy_static) so your path checks never allocate at runtime.
6. Resilience & observability
• Swap all println! debug lines to structured log::debug! / info!.
• Instrument key metrics:
– swaps seen vs. swaps simulated vs. bundles sent vs. bundles included
– average simulation time, gas used, and net profit
• Add Prometheus or StatsD integration so you can alert on regressions or rate‐limit errors.
7. Testing & back-testing
• Write unit tests for every router selector, pool‐swap event, and the full extract_swap_info path (including logs).
• Back-test against historical blocks (e.g. using Foundry or Hardhat forking) to verify your bot would have captured real sandwich opportunities.
8. Safety & kill-switch
• Build in a global “circuit breaker” so if > X% of bundles revert or if gas usage spikes, the bot pauses.
• Cap max slippage per token pair to avoid ridiculous price impact.
If you tackle each of these in turn—starting with 1) end‐to‐end detection (classify → trace‐logs), then 2) robust path decoding, then 3) profit‐maximizing simulation, and finally 4) flashbots bundling—you’ll have all the pieces to run live, profitable sandwich attacks. Let me know which part you want to drill into first!