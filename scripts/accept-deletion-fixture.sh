#!/bin/bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE_MOUNT="/Volumes/DJI-DELTEST"
FIXTURE_TEMP="$(mktemp -d "${TMPDIR:-/tmp}/dji-mic-delete.XXXXXX")"
DISK_IMAGE="$FIXTURE_TEMP/deletion-fixture.dmg"

cleanup() {
  if mount | rg -q " on $FIXTURE_MOUNT "; then
    hdiutil detach "$FIXTURE_MOUNT" >/dev/null
  fi
  case "$FIXTURE_TEMP" in
    "${TMPDIR:-/tmp}"/dji-mic-delete.*) rm -rf -- "$FIXTURE_TEMP" ;;
  esac
}
trap cleanup EXIT

if [[ -e "$FIXTURE_MOUNT" ]]; then
  echo "Refusing to reuse an existing deletion fixture mount." >&2
  exit 1
fi

hdiutil create -quiet -size 64m -fs 'MS-DOS FAT32' -volname DJI-DELTEST "$DISK_IMAGE"
hdiutil attach -quiet -nobrowse "$DISK_IMAGE"

if [[ ! -d "$FIXTURE_MOUNT" ]]; then
  echo "FAT32 fixture did not mount at the expected isolated path." >&2
  exit 1
fi
if ! diskutil info "$FIXTURE_MOUNT" | rg -q 'File System Personality:.*MS-DOS FAT32'; then
  echo "Deletion fixture is not FAT32." >&2
  exit 1
fi

printf 'isolated-fat32-deletion-test\n' > "$FIXTURE_MOUNT/.dji-mic-backup-delete-fixture"
cd "$PROJECT_ROOT"
DJI_MIC_DELETION_FIXTURE="$FIXTURE_MOUNT" \
  cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup \
  --test fat32_trash_acceptance -- --ignored --exact \
  foundation_moves_a_whole_session_on_the_isolated_fat32_fixture

echo "Isolated FAT32 macOS Trash acceptance passed."
