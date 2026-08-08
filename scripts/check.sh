#!/bin/bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

if rg -n \
  '@tauri-apps/plugin|/Volumes/|/Users/|mount_root|volume_uuid|source_path' \
  src \
  --glob '!preview.tsx' \
  --glob '!**/__tests__/**'; then
  echo "Frontend privacy boundary failed." >&2
  exit 1
fi

cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --workspace --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
pnpm test
pnpm typecheck
pnpm build

echo "All checks passed."
