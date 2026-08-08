#!/usr/bin/env bash
set -euo pipefail

matched_volumes=0
writable_volumes=0
wav_files=0
wav_bytes=0

plist_value() {
  local key=$1
  local plist=$2
  /usr/bin/plutil -extract "$key" raw -o - - <<<"$plist" 2>/dev/null || true
}

for volume in /Volumes/*; do
  [[ -d "$volume" ]] || continue
  info=$(/usr/sbin/diskutil info -plist "$volume" 2>/dev/null || true)
  [[ -n "$info" ]] || continue

  internal=$(plist_value Internal "$info")
  removable=$(plist_value RemovableMedia "$info")
  writable=$(plist_value WritableVolume "$info")
  media_name=$(plist_value MediaName "$info")
  capacity=$(plist_value TotalSize "$info")

  [[ "$internal" == "false" ]] || continue
  [[ "$removable" == "true" ]] || continue
  [[ "$media_name" == "Mic Tx" || "$media_name" == "Wireless Mic Tx Media" ]] || continue
  [[ "$capacity" =~ ^[0-9]+$ ]] || continue
  (( capacity >= 12000000000 && capacity <= 20000000000 )) || continue

  matched_volumes=$((matched_volumes + 1))
  [[ "$writable" == "true" ]] && writable_volumes=$((writable_volumes + 1))

  volume_files=$(
    /usr/bin/find "$volume" -type d -name '.*' -prune -o -type f -iname '*.wav' -print 2>/dev/null |
      /usr/bin/wc -l |
      /usr/bin/tr -d ' '
  )
  volume_bytes=$(
    /usr/bin/find "$volume" -type d -name '.*' -prune -o -type f -iname '*.wav' -exec /usr/bin/stat -f '%z' {} \; 2>/dev/null |
      /usr/bin/awk '{ total += $1 } END { print total + 0 }'
  )
  wav_files=$((wav_files + volume_files))
  wav_bytes=$((wav_bytes + volume_bytes))
done

printf 'matched_volumes=%d\n' "$matched_volumes"
printf 'writable_volumes=%d\n' "$writable_volumes"
printf 'wav_files=%d\n' "$wav_files"
printf 'wav_bytes=%d\n' "$wav_bytes"
