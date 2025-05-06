# Sandooo

Sandooo is a high-performance, research-grade sandwich bot for Ethereum-based decentralized exchanges (DEXs). It is designed for advanced DeFi users, researchers, and developers interested in MEV (Miner Extractable Value), sandwich attacks, and blockchain automation.

## Features

- **Real-time Opportunity Detection:**
  - Streams new Ethereum blocks and pending transactions.
  - Detects profitable sandwich attack opportunities in real time.
- **Automated Simulation & Optimization:**
  - Simulates potential sandwich attacks and optimizes trade parameters for maximum profit.
- **Bundle Construction & Execution:**
  - Constructs and submits transaction bundles to Ethereum builders/relays (e.g., Flashbots).
- **Modular & Extensible Architecture:**
  - Well-documented Rust codebase with clear separation between opportunity detection, simulation, and execution.
  - Solidity smart contracts for on-chain bundle execution and token management.
- **Comprehensive Documentation:**
  - All Rust modules and Solidity contracts are documented with Rustdoc and NatSpec for easy onboarding and auditing.

## Project Structure

- `src/common/` — Core Rust modules for EVM interaction, pool/token discovery, logging, and utilities.
- `src/sandwich/` — Sandwich attack logic: opportunity detection, simulation, scoring, and execution.
- `contracts/src/` — Solidity smart contracts (`Sandooo.sol`, `Request.sol`) for bundle execution and ERC20 metadata queries.
- `contracts/test/` — Foundry/Forge tests for smart contracts.
- `README.md` — Project overview and documentation.

## Getting Started

### Prerequisites
- Rust (latest stable)
- Node.js (for some scripts/tools)
- Foundry (for Solidity testing)
- Ethereum node (e.g., Anvil, Geth, or Infura/WebSocket endpoint)

### Setup
1. Clone the repository:
   ```bash
   git clone https://github.com/yourusername/sandooo.git
   cd sandooo
   ```
2. Install Rust dependencies:
   ```bash
   cargo build
   ```
3. Set up environment variables:
   - Copy `.env.example` to `.env` and fill in RPC endpoints, private keys, and relevant settings.

### Running the Bot
```bash
cargo run --release
```

### Smart Contract Testing
- Install [Foundry](https://book.getfoundry.sh/):
  ```bash
  curl -L https://foundry.paradigm.xyz | bash
  foundryup
  ```
- Run tests:
  ```bash
  cd contracts
  forge test
  ```

## Documentation
- Rust code: Run `cargo doc --open` for full API documentation.
- Solidity contracts: Use `forge doc` or view NatSpec comments in the source files.

## Articles & Resources
- [100 Hours of Building a Sandwich Bot](https://medium.com/@solidquant/100-hours-of-building-a-sandwich-bot-a89235281da3)
- [Let's See If our Sandwich Bot Really Works](https://medium.com/@solidquant/lets-see-if-our-sandwich-bot-really-works-9546c49059bd)
- [Adding Stablecoin Sandwiches and Group Bundling](https://medium.com/@solidquant/adding-stablecoin-sandwiches-and-group-bundling-to-improve-our-sandwich-bot-2037cf741f77)

## Community
- Twitter: [@solidquant](https://twitter.com/solidquant)
- Discord: [Solid Quant Discord Server](https://discord.com/invite/e6KpjTQP98)

## License
MIT