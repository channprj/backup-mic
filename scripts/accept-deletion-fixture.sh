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
VOLUME_INFO="$FIXTURE_TEMP/volume-info.plist"
diskutil info -plist "$FIXTURE_MOUNT" > "$VOLUME_INFO"
if [[ "$(plutil -extract MountPoint raw -o - "$VOLUME_INFO")" != "$FIXTURE_MOUNT" \
  || "$(plutil -extract VolumeName raw -o - "$VOLUME_INFO")" != "DJI-DELTEST" \
  || "$(plutil -extract FilesystemName raw -o - "$VOLUME_INFO")" != "MS-DOS FAT32" \
  || "$(plutil -extract BusProtocol raw -o - "$VOLUME_INFO")" != "Disk Image" \
  || "$(plutil -extract Internal raw -o - "$VOLUME_INFO")" != "false" \
  || "$(plutil -extract Removable raw -o - "$VOLUME_INFO")" != "true" \
  || "$(plutil -extract Ejectable raw -o - "$VOLUME_INFO")" != "true" \
  || "$(plutil -extract WritableVolume raw -o - "$VOLUME_INFO")" != "true" ]]; then
  echo "Deletion fixture identity is not the expected writable removable disk image." >&2
  exit 1
fi

printf 'isolated-fat32-trash-test\n' > "$FIXTURE_MOUNT/.dji-mic-backup-delete-fixture"
cd "$PROJECT_ROOT"
DJI_MIC_DELETION_FIXTURE="$FIXTURE_MOUNT" \
  cargo test --manifest-path src-tauri/Cargo.toml -p backup-core \
  --test fat32_deletion_acceptance -- --ignored --exact \
  moves_a_whole_session_to_recoverable_trash_on_an_isolated_fat32_volume
DJI_MIC_DELETION_FIXTURE="$FIXTURE_MOUNT" \
  cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup \
  --test fat32_trash_acceptance -- --ignored --exact \
  foundation_moves_only_the_requested_generic_session_on_the_isolated_fat32_fixture

if rg -n 'remove_file|remove_dir|\.Trashes|Command::|AppleScript|Finder' \
  src-tauri/crates/backup-core/src/deletion.rs \
  src-tauri/src/platform/macos/trash.rs; then
  echo "Production source retirement contains a permanent-delete or Trash-bypass token." >&2
  exit 1
fi

echo "Isolated FAT32 recoverable Trash acceptance passed."
