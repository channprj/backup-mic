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

if rg -n \
  'remove_file|remove_dir|\.Trashes|Command::|std::process|/bin/(ba)?sh|osascript|AppleScript|Finder' \
  src-tauri/crates/backup-core/src/deletion.rs \
  src-tauri/src/platform/macos/trash.rs; then
  echo "Production source retirement boundary failed." >&2
  exit 1
fi

if rg -n \
  '/bin/(ba)?sh|osascript|AppleScript|Finder|\.arg\("-c"\)|Command::new\([^"/]' \
  src-tauri/src/platform/macos/audio.rs; then
  echo "Apple audio tools must be invoked directly without a shell." >&2
  exit 1
fi

if rg -n \
  'set_setting[[:space:]]*\([[:space:]]*key|setting_key|source_uuid|source_hash|trash_destination|process_command' \
  src-tauri/src/commands.rs; then
  echo "Arbitrary settings, path, or process IPC boundary failed." >&2
  exit 1
fi

if rg -n 'TO''DO|TB''D|todo''!|unimplemented''!' src src-tauri scripts README.md --glob '!target/**'; then
  echo "Unresolved implementation marker found." >&2
  exit 1
fi

cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --workspace --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
pnpm test
pnpm typecheck
pnpm build

echo "All checks passed."
