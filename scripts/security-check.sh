#!/bin/bash

set -euo pipefail
umask 077

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$PROJECT_ROOT"
for tool in git gitleaks python3 pnpm cargo cargo-audit; do
  command -v "$tool" >/dev/null || {
    echo "Required security-check tool is missing: $tool" >&2
    exit 1
  }
done
if [[ "$(git rev-parse --is-shallow-repository)" != "false" ]]; then
  echo "Security checks require complete Git history; fetch it before continuing." >&2
  exit 1
fi

snapshot="$(mktemp -d "${TMPDIR:-/tmp}/backup-mic-security.XXXXXX")"
trap 'rm -rf -- "$snapshot"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

gitleaks git . --config .gitleaks.toml --log-opts='--all --full-history' \
  --redact=100 --no-banner
python3 "$PROJECT_ROOT/scripts/security-snapshot.py" "$snapshot"
gitleaks dir "$snapshot" --config "$PROJECT_ROOT/.gitleaks.toml" --redact=100 --no-banner
pnpm audit
cargo audit --file src-tauri/Cargo.lock

echo "Repository security checks passed. Review any informational RustSec warnings above."
