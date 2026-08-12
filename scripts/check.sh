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
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic \
  --test apple_audio_tools -- --exact \
  afconvert_profile_is_exact_aac_lc_128k_without_a_shell_or_old_bitrate
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core \
  --test batch_barrier -- --exact \
  conversion_requires_every_expected_copy_and_verification
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core \
  --test run_retirement -- --exact \
  any_live_backup_or_barrier_mutation_refuses_before_trash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic \
  --test failure_reporting -- --exact \
  failure_reporter_uses_a_privacy_safe_fallback_when_primary_is_unavailable
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic \
  --test concurrent_settings -- --exact \
  m4a_setting_saves_while_an_operation_keeps_its_frozen_preferences
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic \
  --test verify_backup_script -- --exact \
  independent_verifier_checks_complete_cohort_raw_extras_and_privacy_safe_failures
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --features recovery-tool \
  --test recovery_tool
rule_fixture_guard="$(mktemp -t backup-mic-rule-fixture-guard)"
if env -u BACKUP_MIC_ZOOM_RULE_FIXTURE -u BACKUP_MIC_SONY_RULE_FIXTURE \
  cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic \
  --test rule_volume_acceptance -- --ignored --exact \
  backs_up_two_isolated_fat32_rule_volumes_through_the_production_pipeline \
  >"$rule_fixture_guard" 2>&1; then
  rm -f -- "$rule_fixture_guard"
  echo "Rule fixture acceptance unexpectedly ran without disposable mounts." >&2
  exit 1
fi
if ! rg -q \
  'BACKUP_MIC_RULE_FIXTURES_REQUIRED: run scripts/accept-rule-fixtures.sh' \
  "$rule_fixture_guard"; then
  rm -f -- "$rule_fixture_guard"
  echo "Rule fixture missing-environment guard failed." >&2
  exit 1
fi
rm -f -- "$rule_fixture_guard"
cargo test --manifest-path src-tauri/Cargo.toml --workspace --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
pnpm test
pnpm typecheck
pnpm build

echo "All checks passed."
