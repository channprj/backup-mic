#!/bin/bash

set -euo pipefail

APP_LEDGER="/Users/channprj/Library/Application Support/com.channprj.DJIMicBackup/ledger.sqlite3"
diagnostic=0
ledger="$APP_LEDGER"

usage() {
  cat >&2 <<'EOF'
Usage: verify-backup.sh [--diagnostic] [--ledger LEDGER] DESTINATION TX01=SOURCE_VOLUME [TX02=SOURCE_VOLUME]

The default output contains counts only. --diagnostic additionally prints
relative paths and SHA-256 evidence. The command is read-only.
EOF
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --diagnostic)
      diagnostic=1
      shift
      ;;
    --ledger)
      [[ $# -ge 2 ]] || usage
      ledger="$2"
      shift 2
      ;;
    --)
      shift
      break
      ;;
    -*)
      usage
      ;;
    *)
      break
      ;;
  esac
done

[[ $# -ge 2 ]] || usage
destination="$1"
shift

if [[ ! -d "$destination" || -L "$destination" ]]; then
  echo "Backup destination is not a regular directory." >&2
  exit 2
fi
if [[ ! -f "$ledger" || -L "$ledger" ]]; then
  echo "Backup ledger is unavailable." >&2
  exit 2
fi

destination="$(cd "$destination" && pwd -P)"
ledger="$(cd "$(dirname "$ledger")" && pwd -P)/$(basename "$ledger")"

temporary="$(mktemp -d -t dji-mic-verify)"
cleanup() {
  case "$(basename "$temporary")" in
    dji-mic-verify.*)
      rm -rf -- "$temporary"
      ;;
    *)
      echo "Refusing to clean an unexpected verification directory." >&2
      ;;
  esac
}
trap cleanup EXIT

if [[ "$(sqlite3 -readonly "$ledger" 'PRAGMA quick_check;')" != "ok" ]]; then
  echo "Backup ledger integrity check failed." >&2
  exit 1
fi
schema_version="$(sqlite3 -readonly "$ledger" 'SELECT COALESCE(MAX(version), 0) FROM schema_migrations;')"
if [[ ! "$schema_version" =~ ^[0-9]+$ || "$schema_version" -lt 3 ]]; then
  echo "Backup ledger is from an older app version; launch the updated app once first." >&2
  exit 1
fi
if ! sqlite3 -readonly "$ledger" \
  "SELECT batch_phase, frozen_m4a_conversion, m4a_profile_id FROM backup_runs LIMIT 0;
   SELECT classification, artifact_relative_path FROM additional_files LIMIT 0;" >/dev/null 2>&1; then
  echo "Backup ledger does not contain complete batch evidence." >&2
  exit 1
fi

separator=$'\034'
evidence="$temporary/ledger-evidence"
sqlite3 -readonly -separator "$separator" "$ledger" \
  "SELECT transmitter, source_relative_path, source_size, source_sha256,
          destination_relative_path, destination_size, destination_sha256,
          artifact_format, COALESCE(artifact_codec, ''),
          COALESCE(artifact_sample_rate_hz, ''),
          COALESCE(artifact_channel_count, ''),
          COALESCE(artifact_valid_frames, ''),
          COALESCE(artifact_duration_micros, '')
     FROM recordings
    ORDER BY verified_at DESC, id DESC;" > "$evidence"

is_safe_relative() {
  local value="$1"
  local component
  [[ -n "$value" && "$value" != /* && "$value" != *$'\n'* && "$value" != *$'\r'* && "$value" != *$'\t'* ]] || return 1
  case "$value" in
    *//*) return 1 ;;
  esac
  local IFS='/'
  for component in $value; do
    [[ -n "$component" && "$component" != "." && "$component" != ".." && "$component" != .* ]] || return 1
  done
  return 0
}

is_safe_additional_relative() {
  local value="$1"
  local parent base
  if is_safe_relative "$value"; then
    return 0
  fi
  parent="$(dirname "$value")"
  base="$(basename "$value")"
  [[ "$base" == ._* && "$base" != "." && "$base" != ".." ]] || return 1
  is_safe_relative "$parent"
}

xml_value() {
  /usr/bin/xmllint --xpath "string($1)" "$2" 2>/dev/null
}

live_source_count=0
verified_artifact_count=0
m4a_count=0
wav_count=0

for source_spec in "$@"; do
  case "$source_spec" in
    TX01=*) transmitter="TX01"; source_root="${source_spec#TX01=}" ;;
    TX02=*) transmitter="TX02"; source_root="${source_spec#TX02=}" ;;
    *) echo "Each source must be labeled TX01=PATH or TX02=PATH." >&2; exit 2 ;;
  esac
  if [[ ! -d "$source_root" || -L "$source_root" ]]; then
    echo "A labeled source volume is not mounted as a regular directory." >&2
    exit 2
  fi
  source_root="$(cd "$source_root" && pwd -P)"
  printf '%s' "$source_root" > "$temporary/source-$transmitter"

  while IFS= read -r -d '' source_file; do
    live_source_count=$((live_source_count + 1))
    source_relative="${source_file#"$source_root"/}"
    if ! is_safe_relative "$source_relative"; then
      echo "A live source path failed the relative-path safety policy." >&2
      exit 1
    fi
    source_size="$(stat -f '%z' "$source_file")"
    source_hash="$(shasum -a 256 "$source_file" | awk '{print $1}')"
    matches="$temporary/matches-$live_source_count"
    awk -F "$separator" \
      -v tx="$transmitter" -v path="$source_relative" \
      -v bytes="$source_size" -v digest="$source_hash" \
      '$1 == tx && $2 == path && $3 == bytes && $4 == digest { print }' \
      "$evidence" > "$matches"
    match_count="$(wc -l < "$matches" | tr -d ' ')"
    if [[ "$match_count" -ne 1 ]]; then
      echo "A live source WAV does not have exactly one matching ledger record." >&2
      [[ "$diagnostic" -eq 0 ]] || echo "source=$transmitter:$source_relative matches=$match_count" >&2
      exit 1
    fi

    IFS="$separator" read -r recorded_tx recorded_source recorded_source_size recorded_source_hash \
      artifact_relative artifact_size artifact_hash artifact_format artifact_codec \
      artifact_sample_rate artifact_channels artifact_valid_frames artifact_duration < "$matches"
    if [[ "$recorded_tx" != "$transmitter" || "$recorded_source" != "$source_relative" \
      || "$recorded_source_size" != "$source_size" || "$recorded_source_hash" != "$source_hash" ]]; then
      echo "Ledger source evidence changed during verification." >&2
      exit 1
    fi
    if ! is_safe_relative "$artifact_relative"; then
      echo "A ledger artifact path failed the relative-path safety policy." >&2
      exit 1
    fi
    artifact_path="$destination/$artifact_relative"
    if [[ ! -f "$artifact_path" || -L "$artifact_path" ]]; then
      echo "A verified artifact is unavailable or is not a regular file." >&2
      exit 1
    fi
    artifact_parent="$(cd "$(dirname "$artifact_path")" && pwd -P)"
    case "$artifact_parent/" in
      "$destination"/*) ;;
      *) echo "A verified artifact resolved outside the destination." >&2; exit 1 ;;
    esac
    actual_artifact_size="$(stat -f '%z' "$artifact_path")"
    actual_artifact_hash="$(shasum -a 256 "$artifact_path" | awk '{print $1}')"
    if [[ "$actual_artifact_size" != "$artifact_size" || "$actual_artifact_hash" != "$artifact_hash" ]]; then
      echo "A final artifact does not match its durable ledger evidence." >&2
      exit 1
    fi

    case "$artifact_format" in
      wav)
        if [[ "$artifact_size" != "$source_size" || "$artifact_hash" != "$source_hash" ]]; then
          echo "A WAV artifact does not match its live source WAV." >&2
          exit 1
        fi
        wav_count=$((wav_count + 1))
        ;;
      m4a)
        case "$artifact_relative" in
          *.m4a|*.M4A) ;;
          *) echo "An M4A ledger artifact has an unexpected extension." >&2; exit 1 ;;
        esac
        afinfo_xml="$temporary/afinfo-$live_source_count.xml"
        if ! /usr/bin/afinfo -x "$artifact_path" > "$afinfo_xml" 2>/dev/null || [[ ! -s "$afinfo_xml" ]]; then
          echo "Apple afinfo rejected an M4A artifact." >&2
          exit 1
        fi
        audio_file_xpath="/*[local-name()='audio_info']/*[local-name()='audio_file']"
        track_xpath="$audio_file_xpath/*[local-name()='tracks']/*[local-name()='track'][1]"
        container="$(xml_value "$audio_file_xpath/*[local-name()='file_type']" "$afinfo_xml")"
        codec="$(xml_value "$track_xpath/*[local-name()='format_type']" "$afinfo_xml")"
        sample_rate="$(xml_value "$track_xpath/*[local-name()='sample_rate']" "$afinfo_xml")"
        channels="$(xml_value "$track_xpath/*[local-name()='num_channels']" "$afinfo_xml")"
        audio_bytes="$(xml_value "$track_xpath/*[local-name()='audio_bytes']" "$afinfo_xml")"
        audio_packets="$(xml_value "$track_xpath/*[local-name()='audio_packets']" "$afinfo_xml")"
        valid_frames="$(xml_value "$track_xpath/*[local-name()='packet_table_info']/*[local-name()='valid_frames']" "$afinfo_xml")"
        container="${container//\'/}"
        container="${container//[[:space:]]/}"
        codec="${codec//[[:space:]]/}"
        codec_lower="$(printf '%s' "$codec" | tr '[:upper:]' '[:lower:]')"
        if [[ "$container" != "m4af" || "$codec_lower" != *aac* \
          || ! "$audio_bytes" =~ ^[1-9][0-9]*$ || ! "$audio_packets" =~ ^[1-9][0-9]*$ \
          || "$sample_rate" != "$artifact_sample_rate" || "$channels" != "$artifact_channels" \
          || "$valid_frames" != "$artifact_valid_frames" \
          || ! "$artifact_duration" =~ ^[1-9][0-9]*$ || "$artifact_codec" != "$codec" ]]; then
          echo "An M4A artifact does not match its ledger audio shape." >&2
          exit 1
        fi
        m4a_count=$((m4a_count + 1))
        ;;
      *)
        echo "A ledger artifact has an unsupported format." >&2
        exit 1
        ;;
    esac
    verified_artifact_count=$((verified_artifact_count + 1))
    if [[ "$diagnostic" -eq 1 ]]; then
      echo "verified source=$transmitter:$source_relative source_sha256=$source_hash artifact=$artifact_relative artifact_sha256=$artifact_hash format=$artifact_format"
    fi
  done < <(find "$source_root" -type d -name '.*' -prune -o -type f -iname '*.wav' -print0)
done

additional_evidence="$temporary/additional-evidence"
sqlite3 -readonly -separator "$separator" "$ledger" \
  "SELECT transmitter, source_relative_path, source_size, source_sha256,
          artifact_relative_path, artifact_size, artifact_sha256, classification
     FROM additional_files
    ORDER BY transmitter, source_relative_path, id;" > "$additional_evidence"

verified_additional_count=0
while IFS="$separator" read -r additional_tx additional_source additional_source_size \
  additional_source_hash additional_artifact additional_artifact_size \
  additional_artifact_hash additional_classification; do
  [[ -n "$additional_tx" ]] || continue
  case "$additional_tx" in
    TX01|TX02) ;;
    *) echo "Additional-file evidence has an invalid transmitter." >&2; exit 1 ;;
  esac
  case "$additional_classification" in
    m4a|apple_double|other) ;;
    *) echo "Additional-file evidence has an invalid classification." >&2; exit 1 ;;
  esac
  if ! is_safe_additional_relative "$additional_source" \
    || ! is_safe_additional_relative "$additional_artifact"; then
    echo "An additional-file path failed the relative-path safety policy." >&2
    exit 1
  fi
  if [[ "$additional_source_size" != "$additional_artifact_size" \
    || "$additional_source_hash" != "$additional_artifact_hash" ]]; then
    echo "Additional-file ledger evidence is not source-equal." >&2
    exit 1
  fi
  additional_artifact_path="$destination/$additional_artifact"
  if [[ ! -f "$additional_artifact_path" || -L "$additional_artifact_path" ]]; then
    echo "A verified additional-file artifact is unavailable." >&2
    exit 1
  fi
  additional_artifact_parent="$(cd "$(dirname "$additional_artifact_path")" && pwd -P)"
  case "$additional_artifact_parent/" in
    "$destination"/*) ;;
    *) echo "An additional-file artifact resolved outside the destination." >&2; exit 1 ;;
  esac
  actual_additional_size="$(stat -f '%z' "$additional_artifact_path")"
  actual_additional_hash="$(shasum -a 256 "$additional_artifact_path" | awk '{print $1}')"
  if [[ "$actual_additional_size" != "$additional_artifact_size" \
    || "$actual_additional_hash" != "$additional_artifact_hash" ]]; then
    echo "An additional-file artifact does not match durable ledger evidence." >&2
    exit 1
  fi
  if [[ -f "$temporary/source-$additional_tx" ]]; then
    additional_source_root="$(<"$temporary/source-$additional_tx")"
    live_additional_path="$additional_source_root/$additional_source"
    if [[ -e "$live_additional_path" || -L "$live_additional_path" ]]; then
      if [[ ! -f "$live_additional_path" || -L "$live_additional_path" ]]; then
        echo "A live additional source is not a regular file." >&2
        exit 1
      fi
      live_additional_parent="$(cd "$(dirname "$live_additional_path")" && pwd -P)"
      case "$live_additional_parent/" in
        "$additional_source_root"/*) ;;
        *) echo "A live additional source resolved outside its volume." >&2; exit 1 ;;
      esac
      live_additional_size="$(stat -f '%z' "$live_additional_path")"
      live_additional_hash="$(shasum -a 256 "$live_additional_path" | awk '{print $1}')"
      if [[ "$live_additional_size" != "$additional_source_size" \
        || "$live_additional_hash" != "$additional_source_hash" ]]; then
        echo "A live additional source changed after backup." >&2
        exit 1
      fi
    fi
  fi
  verified_additional_count=$((verified_additional_count + 1))
  if [[ "$diagnostic" -eq 1 ]]; then
    echo "verified additional=$additional_tx:$additional_source source_sha256=$additional_source_hash artifact=$additional_artifact artifact_sha256=$additional_artifact_hash classification=$additional_classification"
  fi
done < "$additional_evidence"

echo "Live source WAV files: $live_source_count"
echo "Ledger-matched final artifacts: $verified_artifact_count"
echo "Verified WAV artifacts: $wav_count"
echo "Verified M4A artifacts: $m4a_count"
echo "Verified additional-file artifacts: $verified_additional_count"
if [[ "$live_source_count" -eq 0 ]]; then
  echo "No live source WAV files required verification."
else
  echo "Every live source WAV and its final artifact match durable ledger evidence."
fi
