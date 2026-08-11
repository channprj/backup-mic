#!/bin/bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE_BASE="${TMPDIR:-/tmp}"
FIXTURE_BASE="${FIXTURE_BASE%/}"
FIXTURE_TEMP="$(mktemp -d "$FIXTURE_BASE/backup-mic-rule-fixtures.XXXXXX")"
ZOOM_MOUNT="/Volumes/ZOOM_RULETEST"
SONY_MOUNT="/Volumes/SONY_RULETEST"
SENTINEL=".backup-mic-rule-fixture"
EXPECTED_BYTES=$((64 * 1024 * 1024))
MINIMUM_BYTES=$((60 * 1024 * 1024))
MAXIMUM_BYTES=$((68 * 1024 * 1024))
ATTACHED_DEVICES=()
cleanup_ready=1

cleanup() {
  local status=$?
  local index device
  trap - EXIT INT TERM
  set +e
  for ((index=${#ATTACHED_DEVICES[@]} - 1; index >= 0; index--)); do
    device="${ATTACHED_DEVICES[$index]}"
    if ! hdiutil detach "$device" >/dev/null; then
      cleanup_ready=0
      echo "Fixture device could not be detached safely: $device" >&2
    fi
  done
  if [[ "$cleanup_ready" -eq 1 ]]; then
    case "$FIXTURE_TEMP" in
      "$FIXTURE_BASE"/backup-mic-rule-fixtures.*)
        rm -rf -- "$FIXTURE_TEMP"
        ;;
      *)
        echo "Refusing to clean an unexpected rule fixture directory." >&2
        ;;
    esac
  else
    echo "Fixture images remain for manual recovery at: $FIXTURE_TEMP" >&2
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

for mount_root in "$ZOOM_MOUNT" "$SONY_MOUNT"; do
  if [[ -e "$mount_root" ]] || mount | rg -F -q " on $mount_root "; then
    echo "Refusing to reuse an existing rule fixture mount." >&2
    exit 1
  fi
done
for dji_mount in /Volumes/DJI-MIC-1 /Volumes/DJI-MIC-2; do
  if [[ -e "$dji_mount" ]] || mount | rg -F -q " on $dji_mount "; then
    echo "Disconnect DJI recorder volumes before running disposable rule fixtures." >&2
    exit 1
  fi
done

create_fixture() {
  local label="$1"
  local mount_root="$2"
  local image="$FIXTURE_TEMP/$label.dmg"
  local attach="$FIXTURE_TEMP/$label-attach.plist"
  local info="$FIXTURE_TEMP/$label-info.plist"
  local attached_device identifier partition_device total_size

  hdiutil create -quiet -size 64m -fs 'MS-DOS FAT32' -volname "$label" "$image"
  hdiutil attach -plist -nobrowse "$image" > "$attach"
  attached_device="$(plutil -extract 'system-entities.0.dev-entry' raw -o - "$attach")"
  [[ "$attached_device" =~ ^/dev/disk[0-9]+$ ]] || {
    echo "Rule fixture attachment did not return an isolated disk device." >&2
    exit 1
  }
  ATTACHED_DEVICES+=("$attached_device")
  if [[ ! -d "$mount_root" || -L "$mount_root" ]]; then
    echo "Rule fixture did not mount at its exact isolated path." >&2
    exit 1
  fi
  diskutil info -plist "$mount_root" > "$info"
  identifier="$(plutil -extract DeviceIdentifier raw -o - "$info")"
  [[ "$identifier" =~ ^disk[0-9]+s[0-9]+$ ]] || {
    echo "Rule fixture has an unexpected device identity." >&2
    exit 1
  }
  partition_device="/dev/$identifier"
  if [[ "$partition_device" != "$attached_device"s* ]]; then
    echo "Rule fixture mount does not belong to its recorded disk image device." >&2
    exit 1
  fi
  total_size="$(plutil -extract TotalSize raw -o - "$info")"
  if [[ ! "$total_size" =~ ^[0-9]+$ \
    || "$total_size" -lt "$MINIMUM_BYTES" || "$total_size" -gt "$MAXIMUM_BYTES" ]]; then
    echo "Rule fixture has an unexpected capacity (expected a $EXPECTED_BYTES-byte image)." >&2
    exit 1
  fi
  if [[ "$(plutil -extract MountPoint raw -o - "$info")" != "$mount_root" \
    || "$(plutil -extract VolumeName raw -o - "$info")" != "$label" \
    || "$(plutil -extract FilesystemName raw -o - "$info")" != "MS-DOS FAT32" \
    || "$(plutil -extract BusProtocol raw -o - "$info")" != "Disk Image" \
    || "$(plutil -extract Internal raw -o - "$info")" != "false" \
    || "$(plutil -extract Removable raw -o - "$info")" != "true" \
    || "$(plutil -extract Ejectable raw -o - "$info")" != "true" \
    || "$(plutil -extract WritableVolume raw -o - "$info")" != "true" ]]; then
    echo "Rule fixture is not the expected writable removable FAT32 disk image." >&2
    exit 1
  fi

  printf 'backup-mic-rule-fixture:%s\n' "$label" > "$mount_root/$SENTINEL"
  if [[ "$(cat "$mount_root/$SENTINEL")" != "backup-mic-rule-fixture:$label" ]]; then
    echo "Rule fixture sentinel could not be verified." >&2
    exit 1
  fi
}

write_pcm_wav() {
  local output="$1"
  local first_sample_low_byte="$2"
  mkdir -p "$(dirname "$output")"
  /usr/bin/printf 'RIFF\044\167\001\000WAVEfmt \020\000\000\000\001\000\001\000\200\273\000\000\000\167\001\000\002\000\020\000data\000\167\001\000' > "$output"
  /bin/dd if=/dev/zero bs=96000 count=1 >> "$output" 2>/dev/null
  if [[ "$first_sample_low_byte" -eq 1 ]]; then
    /usr/bin/printf '\001' | /bin/dd of="$output" bs=1 seek=44 conv=notrunc 2>/dev/null
  fi
  /usr/bin/afinfo "$output" >/dev/null
}

create_fixture "ZOOM_RULETEST" "$ZOOM_MOUNT"
create_fixture "SONY_RULETEST" "$SONY_MOUNT"
write_pcm_wav "$ZOOM_MOUNT/RECORD/FOLDER01/ZOOM0001.WAV" 0
write_pcm_wav "$SONY_MOUNT/REC_FILE/FOLDER01/SONY0001.WAV" 1
sync

for fixture in \
  "$ZOOM_MOUNT/$SENTINEL" \
  "$ZOOM_MOUNT/RECORD/FOLDER01/ZOOM0001.WAV" \
  "$SONY_MOUNT/$SENTINEL" \
  "$SONY_MOUNT/REC_FILE/FOLDER01/SONY0001.WAV"; do
  if [[ ! -f "$fixture" || -L "$fixture" ]]; then
    echo "Rule fixture content is missing or unsafe." >&2
    exit 1
  fi
done

cd "$PROJECT_ROOT"
BACKUP_MIC_ZOOM_RULE_FIXTURE="$(cd "$ZOOM_MOUNT" && pwd -P)" \
BACKUP_MIC_SONY_RULE_FIXTURE="$(cd "$SONY_MOUNT" && pwd -P)" \
  cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic \
  --test rule_volume_acceptance -- --ignored --exact --nocapture \
  backs_up_two_isolated_fat32_rule_volumes_through_the_production_pipeline

echo "Disposable FAT32 rule-volume acceptance passed: volumes=2 recordings=2 m4a=2."
