#!/bin/bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE_MOUNT="/Volumes/DJI-DELTEST"
FIXTURE_BASE="${TMPDIR:-/tmp}"
FIXTURE_BASE="${FIXTURE_BASE%/}"
FIXTURE_TEMP="$(mktemp -d "$FIXTURE_BASE/dji-mic-delete.XXXXXX")"
DISK_IMAGE="$FIXTURE_TEMP/deletion-fixture.dmg"
ATTACHED_DEVICE=""
cleanup_ready=1

cleanup() {
  local exit_status=$?
  trap - EXIT INT TERM
  set +e
  if [[ -n "$ATTACHED_DEVICE" ]]; then
    if ! hdiutil detach "$ATTACHED_DEVICE" >/dev/null; then
      cleanup_ready=0
      echo "Fixture device could not be detached safely: $ATTACHED_DEVICE" >&2
      if [[ "$exit_status" -eq 0 ]]; then exit_status=1; fi
    fi
  fi
  if [[ "$cleanup_ready" -eq 1 ]]; then
    case "$FIXTURE_TEMP" in
      "$FIXTURE_BASE"/dji-mic-delete.*) rm -rf -- "$FIXTURE_TEMP" ;;
    esac
  else
    echo "Fixture image remains for manual recovery at: $FIXTURE_TEMP" >&2
  fi
  exit "$exit_status"
}
trap cleanup EXIT INT TERM

if [[ -e "$FIXTURE_MOUNT" ]] || mount | rg -F -q " on $FIXTURE_MOUNT "; then
  echo "Refusing to reuse an existing deletion fixture mount." >&2
  exit 1
fi
for dji_mount in /Volumes/DJI-MIC-1 /Volumes/DJI-MIC-2; do
  if [[ -e "$dji_mount" ]] || mount | rg -F -q " on $dji_mount "; then
    echo "Disconnect DJI recorder volumes before running the deletion fixture." >&2
    exit 1
  fi
done

hdiutil create -quiet -size 64m -fs 'MS-DOS FAT32' -volname DJI-DELTEST "$DISK_IMAGE"
cleanup_ready=0
hdiutil attach -plist -nobrowse "$DISK_IMAGE" > "$FIXTURE_TEMP/attach.plist"
ATTACHED_DEVICE="$(python3 "$PROJECT_ROOT/scripts/fixture-device.py" "$FIXTURE_TEMP/attach.plist")"
if [[ ! "$ATTACHED_DEVICE" =~ ^/dev/disk[0-9]+$ ]]; then
  ATTACHED_DEVICE=""
  echo "Fixture attachment did not return an isolated disk device." >&2
  exit 1
fi
cleanup_ready=1

if [[ ! -d "$FIXTURE_MOUNT" || -L "$FIXTURE_MOUNT" ]]; then
  echo "FAT32 fixture did not mount at the expected isolated path." >&2
  exit 1
fi
if ! diskutil info "$FIXTURE_MOUNT" | rg -q 'File System Personality:.*MS-DOS FAT32'; then
  echo "Deletion fixture is not FAT32." >&2
  exit 1
fi
VOLUME_INFO="$FIXTURE_TEMP/volume-info.plist"
diskutil info -plist "$FIXTURE_MOUNT" > "$VOLUME_INFO"
partition_device="/dev/$(plutil -extract DeviceIdentifier raw -o - "$VOLUME_INFO")"
if [[ "$partition_device" != "$ATTACHED_DEVICE"s* \
  || "$(plutil -extract MountPoint raw -o - "$VOLUME_INFO")" != "$FIXTURE_MOUNT" \
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
  cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic \
  --test fat32_trash_acceptance -- --ignored --exact \
  foundation_moves_only_the_requested_generic_session_on_the_isolated_fat32_fixture

scan_status=0
rg -n 'remove_file|remove_dir|\.Trashes|Command::|AppleScript|Finder' \
  src-tauri/crates/backup-core/src/deletion \
  src-tauri/src/platform/macos/trash.rs || scan_status=$?
if [[ "$scan_status" -ne 1 ]]; then
  echo "Production source retirement boundary failed or could not be scanned." >&2
  exit 1
fi

echo "Isolated FAT32 recoverable Trash acceptance passed."
