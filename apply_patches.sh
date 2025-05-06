#!/usr/bin/env bash
set -e

# Apply every patch in the exact order below.
patch -p1 < patches/routers.patch
patch -p1 < patches/contract_detector_new.patch
patch -p1 < patches/mod.patch
patch -p1 < patches/classifier.patch
patch -p1 < patches/simulation.patch
patch -p1 < patches/strategy.patch
# Only run the next line if Cargo.toml exists in your repo:
patch -p1 < patches/Cargo.toml.patch

echo "✅  Patches applied.  Checking build…"
cargo check
cargo clippy -- -D warnings
