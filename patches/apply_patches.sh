#!/usr/bin/env bash
set -e

# Determine directories
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Apply patches under src/common
for patch in routers contract_detector_new mod classifier; do
    patch -p1 -d "$PROJECT_ROOT/src/common" < "$SCRIPT_DIR/$patch.patch"
done

# Apply patches under src/sandwich
for patch in simulation strategy; do
    patch -p1 -d "$PROJECT_ROOT/src/sandwich" < "$SCRIPT_DIR/$patch.patch"
done

# Apply patch to Cargo.toml at project root
patch -p1 -d "$PROJECT_ROOT" < "$SCRIPT_DIR/Cargo.toml.patch"

echo "✅  Patches applied.  Checking build…"
cd "$PROJECT_ROOT"
cargo check
cargo clippy -- -D warnings
