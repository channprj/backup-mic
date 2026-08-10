#!/bin/bash

set -euo pipefail

APP_LEDGER="/Users/channprj/Library/Application Support/com.channprj.BackupMic/ledger.sqlite3"
PROFILE_ID="aac_lc_128k_v1"
diagnostic=0
ledger="$APP_LEDGER"

usage() {
  cat >&2 <<'EOF'
Usage: verify-backup.sh [--diagnostic] [--ledger LEDGER] DESTINATION RULE_NAME=SOURCE_VOLUME [...]

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
    -*) usage ;;
    *) break ;;
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
temporary="$(mktemp -d -t backup-mic-verify)"

cleanup() {
  case "$(basename "$temporary")" in
    backup-mic-verify.*) rm -rf -- "$temporary" ;;
    *) echo "Refusing to clean an unexpected verification directory." >&2 ;;
  esac
}
trap cleanup EXIT

separator=$'\034'
source_map="$temporary/source-map"
: > "$source_map"
for source_spec in "$@"; do
  [[ "$source_spec" == *=* ]] || {
    echo "Each source must be labeled RULE_NAME=PATH." >&2
    exit 2
  }
  rule_name="${source_spec%%=*}"
  source_root="${source_spec#*=}"
  if [[ -z "$rule_name" || -z "$source_root" || "$rule_name" == *$separator* \
    || "$rule_name" == *$'\n'* || "$rule_name" == *$'\r'* || "$rule_name" == */* ]]; then
    echo "Each source must use a safe non-empty rule name and path." >&2
    exit 2
  fi
  if [[ ! -d "$source_root" || -L "$source_root" ]]; then
    echo "A labeled source volume is not mounted as a regular directory." >&2
    exit 2
  fi
  source_root="$(cd "$source_root" && pwd -P)"
  if awk -F "$separator" -v rule="$rule_name" -v root="$source_root" \
    '$1 == rule && $2 == root { found = 1 } END { exit !found }' "$source_map"; then
    echo "The same rule source may be supplied only once." >&2
    exit 2
  fi
  printf '%s%s%s\n' "$rule_name" "$separator" "$source_root" >> "$source_map"
done

query_ledger="$ledger"
sqlite_readonly=1
if ! quick_check="$(sqlite3 -readonly "$ledger" 'PRAGMA quick_check;' 2>/dev/null)"; then
  if [[ -e "$ledger-wal" || -e "$ledger-shm" ]]; then
    echo "Backup ledger could not be opened safely while its WAL is active." >&2
    exit 1
  fi
  query_ledger="$temporary/ledger-snapshot.sqlite3"
  /bin/cp -p "$ledger" "$query_ledger"
  sqlite_readonly=0
  quick_check="$(sqlite3 "$query_ledger" 'PRAGMA query_only = ON; PRAGMA quick_check;')"
fi
if [[ "$quick_check" != "ok" ]]; then
  echo "Backup ledger integrity check failed." >&2
  exit 1
fi
run_sqlite() {
  if [[ "$sqlite_readonly" -eq 1 ]]; then
    sqlite3 -readonly "$@"
  else
    sqlite3 "$@"
  fi
}

schema_version="$(run_sqlite "$query_ledger" 'SELECT COALESCE(MAX(version), 0) FROM schema_migrations;')"
if [[ ! "$schema_version" =~ ^[0-9]+$ || "$schema_version" -lt 6 ]]; then
  echo "Backup ledger is from an older app version; launch the updated app once first." >&2
  exit 1
fi
if ! run_sqlite "$query_ledger" \
  "SELECT id, rule_id, legacy_slot FROM sources LIMIT 0;
   SELECT source_id, batch_phase, frozen_m4a_conversion, m4a_profile_id FROM backup_runs LIMIT 0;
   SELECT source_id, classification, artifact_relative_path FROM additional_files LIMIT 0;
   SELECT source_id, profile_id, status FROM conversion_cohort_items LIMIT 0;
   SELECT source_id, superseded_wav_retirement_status FROM recordings LIMIT 0;" >/dev/null 2>&1; then
  echo "Backup ledger does not contain complete batch evidence." >&2
  exit 1
fi

active_rules="$temporary/active-rules"
run_sqlite -separator "$separator" "$query_ledger" \
  "SELECT name, archive_directory_name
     FROM backup_rules
    WHERE enabled = 1 AND archived_at IS NULL
    ORDER BY name;" > "$active_rules"
while IFS="$separator" read -r selected_rule _; do
  match_count="$(awk -F "$separator" -v rule="$selected_rule" '$1 == rule { count++ } END { print count + 0 }' "$active_rules")"
  if [[ "$match_count" -ne 1 ]]; then
    echo "A selected source does not name exactly one active backup rule." >&2
    exit 2
  fi
done < "$source_map"

is_safe_relative() {
  local value="$1"
  local component
  [[ -n "$value" && "$value" != /* && "$value" != *$'\n'* && "$value" != *$'\r'* && "$value" != *$'\t'* ]] || return 1
  case "$value" in *//*) return 1 ;; esac
  local IFS='/'
  for component in $value; do
    [[ -n "$component" && "$component" != "." && "$component" != ".." && "$component" != .* ]] || return 1
  done
}

is_safe_additional_relative() {
  local value="$1"
  local parent base
  if is_safe_relative "$value"; then
    return 0
  fi
  parent="$(dirname "$value")"
  base="$(basename "$value")"
  [[ "$base" == .* && "$base" != "." && "$base" != ".." ]] || return 1
  is_safe_relative "$parent"
}

has_wav_extension() {
  local lower
  lower="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
  [[ "$lower" == *.wav ]]
}

xml_value() {
  /usr/bin/xmllint --xpath "string($1)" "$2" 2>/dev/null
}

resolve_regular_artifact() {
  local relative="$1"
  local path parent
  is_safe_additional_relative "$relative" || return 1
  path="$destination/$relative"
  [[ -f "$path" && ! -L "$path" ]] || return 1
  parent="$(cd "$(dirname "$path")" && pwd -P)"
  case "$parent/" in "$destination"/*) ;; *) return 1 ;; esac
}

recording_evidence="$temporary/recording-evidence"
run_sqlite -separator "$separator" "$query_ledger" \
  "SELECT r.id, br.name, r.source_relative_path, r.source_size, r.source_sha256,
          r.destination_relative_path, r.destination_size, r.destination_sha256,
          r.artifact_format, COALESCE(r.artifact_codec, ''),
          COALESCE(r.artifact_sample_rate_hz, ''),
          COALESCE(r.artifact_channel_count, ''),
          COALESCE(r.artifact_valid_frames, ''),
          COALESCE(r.artifact_duration_micros, ''), r.conversion_status,
          COALESCE(r.superseded_wav_relative_path, ''),
          COALESCE(r.superseded_wav_size, ''),
          COALESCE(r.superseded_wav_sha256, ''),
          r.superseded_wav_retirement_status,
          r.backup_run_id, b.outcome, b.batch_phase, b.frozen_m4a_conversion,
          COALESCE(b.m4a_profile_id, ''),
          COALESCE((SELECT MIN(c.profile_id) FROM conversion_cohort_items c
                    WHERE c.backup_run_id = r.backup_run_id AND c.recording_id = r.id), ''),
          COALESCE((SELECT MIN(c.status) FROM conversion_cohort_items c
                    WHERE c.backup_run_id = r.backup_run_id AND c.recording_id = r.id), ''),
          (SELECT COUNT(*) FROM conversion_cohort_items c
           WHERE c.backup_run_id = r.backup_run_id AND c.recording_id = r.id),
          (SELECT COUNT(*) FROM conversion_cohort_items c
           WHERE c.backup_run_id = r.backup_run_id
             AND (c.status != 'verified_m4a' OR c.profile_id != b.m4a_profile_id
                  OR c.source_id != r.source_id)),
          r.source_id,
          CASE
            WHEN b.source_id = r.source_id THEN 1
            WHEN b.source_id IS NULL AND (
              EXISTS(SELECT 1 FROM recordings other
                     WHERE other.backup_run_id = b.id AND other.source_id != r.source_id)
              OR EXISTS(SELECT 1 FROM additional_files other
                        WHERE other.backup_run_id = b.id AND other.source_id != r.source_id)
            ) THEN 1
            ELSE 0
          END,
          br.archive_directory_name, br.archive_directory_locked
     FROM recordings r
     JOIN backup_runs b ON b.id = r.backup_run_id
     JOIN sources s ON s.id = r.source_id
     JOIN backup_rules br ON br.id = s.rule_id
    ORDER BY br.name, r.source_relative_path, r.id;" > "$recording_evidence"

verified_artifact_count=0
m4a_count=0
wav_count=0
row_index=0
while IFS="$separator" read -r recording_id recorded_rule recorded_source recorded_source_size \
  recorded_source_hash artifact_relative artifact_size artifact_hash artifact_format \
  artifact_codec artifact_sample_rate artifact_channels artifact_valid_frames artifact_duration \
  conversion_status superseded_wav_relative superseded_wav_size superseded_wav_hash \
  superseded_wav_status \
  backup_run_id run_outcome batch_phase frozen_m4a run_profile cohort_profile cohort_status \
  cohort_count cohort_invalid_count recorded_source_id source_run_consistent \
  recorded_archive archive_locked; do
  [[ -n "$recording_id" ]] || continue
  row_index=$((row_index + 1))
  if [[ ! "$recorded_source_id" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$ \
    || "$source_run_consistent" != "1" ]]; then
    echo "Recording evidence contains an invalid source identity." >&2
    exit 1
  fi
  if ! is_safe_relative "$recorded_source" || ! resolve_regular_artifact "$artifact_relative"; then
    echo "Recording evidence contains an unsafe or unavailable artifact (1 invalid record)." >&2
    exit 1
  fi
  if [[ "$archive_locked" != "1" || -z "$recorded_archive" \
    || "$artifact_relative" != "$recorded_archive/"* ]]; then
    echo "Recording artifact is outside its locked rule archive (1 invalid record)." >&2
    exit 1
  fi
  artifact_path="$destination/$artifact_relative"
  actual_artifact_size="$(stat -f '%z' "$artifact_path")"
  actual_artifact_hash="$(shasum -a 256 "$artifact_path" | awk '{print $1}')"
  if [[ "$actual_artifact_size" != "$artifact_size" || "$actual_artifact_hash" != "$artifact_hash" ]]; then
    echo "Final artifact verification failed (1 mismatched artifact)." >&2
    exit 1
  fi

  case "$artifact_format" in
    wav)
      if [[ "$artifact_size" != "$recorded_source_size" || "$artifact_hash" != "$recorded_source_hash" \
        || "$conversion_status" != "not_required" || "$frozen_m4a" != "0" \
        || "$batch_phase" != "copies_verified" || "$run_outcome" != "wav_backup_complete_source_retained" \
        || -n "$superseded_wav_relative" || -n "$superseded_wav_size" \
        || -n "$superseded_wav_hash" ]]; then
        echo "WAV-mode batch evidence verification failed (1 invalid record)." >&2
        exit 1
      fi
      wav_count=$((wav_count + 1))
      ;;
    m4a)
      case "$artifact_relative" in *.m4a|*.M4A) ;; *) echo "M4A extension verification failed (1 invalid artifact)." >&2; exit 1 ;; esac
      case "$batch_phase" in m4a_cohort_verified|sources_revalidated|completed) ;; *) echo "M4A cohort barrier verification failed (1 invalid run)." >&2; exit 1 ;; esac
      case "$superseded_wav_status" in
        moved_to_trash|absent_after_conversion) ;;
        *) echo "M4A cohort barrier verification failed (1 invalid record)." >&2; exit 1 ;;
      esac
      if ! is_safe_relative "$superseded_wav_relative" \
        || ! has_wav_extension "$superseded_wav_relative" \
        || [[ "$superseded_wav_size" != "$recorded_source_size" \
          || "$superseded_wav_hash" != "$recorded_source_hash" \
          || "$conversion_status" != "complete" || "$frozen_m4a" != "1" \
          || "$run_outcome" != "completed" || "$run_profile" != "$PROFILE_ID" \
          || "$cohort_profile" != "$PROFILE_ID" || "$cohort_status" != "verified_m4a" \
          || "$cohort_count" != "1" || "$cohort_invalid_count" != "0" ]]; then
        echo "M4A cohort barrier verification failed (1 invalid record)." >&2
        exit 1
      fi
      superseded_wav_path="$destination/$superseded_wav_relative"
      if [[ -e "$superseded_wav_path" || -L "$superseded_wav_path" ]]; then
        echo "Superseded WAV retirement verification failed (1 remaining artifact)." >&2
        exit 1
      fi
      afinfo_xml="$temporary/afinfo-$row_index.xml"
      if ! /usr/bin/afinfo -x "$artifact_path" > "$afinfo_xml" 2>/dev/null || [[ ! -s "$afinfo_xml" ]]; then
        echo "Apple M4A inspection failed (1 invalid artifact)." >&2
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
        echo "M4A audio-shape verification failed (1 invalid artifact)." >&2
        exit 1
      fi
      m4a_count=$((m4a_count + 1))
      ;;
    *) echo "Final artifact format verification failed (1 invalid record)." >&2; exit 1 ;;
  esac
  verified_artifact_count=$((verified_artifact_count + 1))
  if [[ "$diagnostic" -eq 1 ]]; then
    echo "verified source_id=$recorded_source_id source=$recorded_rule:$recorded_source source_sha256=$recorded_source_hash artifact=$artifact_relative artifact_sha256=$artifact_hash format=$artifact_format run=$backup_run_id"
  fi
done < "$recording_evidence"

additional_evidence="$temporary/additional-evidence"
run_sqlite -separator "$separator" "$query_ledger" \
  "SELECT a.id, br.name, a.source_relative_path, a.source_size, a.source_sha256,
          a.artifact_relative_path, a.artifact_size, a.artifact_sha256, a.classification,
          a.backup_run_id, b.outcome, b.batch_phase, b.frozen_m4a_conversion,
          COALESCE(b.m4a_profile_id, ''), a.source_id,
          CASE
            WHEN b.source_id = a.source_id THEN 1
            WHEN b.source_id IS NULL AND (
              EXISTS(SELECT 1 FROM recordings other
                     WHERE other.backup_run_id = b.id AND other.source_id != a.source_id)
              OR EXISTS(SELECT 1 FROM additional_files other
                        WHERE other.backup_run_id = b.id AND other.source_id != a.source_id)
            ) THEN 1
            ELSE 0
          END,
          br.archive_directory_name, br.archive_directory_locked
     FROM additional_files a
     JOIN backup_runs b ON b.id = a.backup_run_id
     JOIN sources s ON s.id = a.source_id
     JOIN backup_rules br ON br.id = s.rule_id
    ORDER BY br.name, a.source_relative_path, a.id;" > "$additional_evidence"

verified_additional_count=0
while IFS="$separator" read -r additional_id additional_rule additional_source additional_source_size \
  additional_source_hash additional_artifact additional_artifact_size additional_artifact_hash \
  additional_classification additional_run_id additional_run_outcome additional_batch_phase \
  additional_frozen_m4a additional_run_profile additional_source_id source_run_consistent \
  additional_archive archive_locked; do
  [[ -n "$additional_id" ]] || continue
  if [[ ! "$additional_source_id" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$ \
    || "$source_run_consistent" != "1" ]]; then
    echo "Additional-file evidence contains an invalid source identity." >&2
    exit 1
  fi
  case "$additional_classification" in m4a|apple_double|other) ;; *) echo "Additional-file classification verification failed (1 invalid record)." >&2; exit 1 ;; esac
  if ! is_safe_additional_relative "$additional_source" \
    || ! resolve_regular_artifact "$additional_artifact" \
    || [[ "$additional_source_size" != "$additional_artifact_size" \
      || "$additional_source_hash" != "$additional_artifact_hash" ]]; then
    echo "Raw additional-file evidence verification failed (1 invalid record)." >&2
    exit 1
  fi
  if [[ "$archive_locked" != "1" || -z "$additional_archive" \
    || "$additional_artifact" != "$additional_archive/"* ]]; then
    echo "Additional-file artifact is outside its locked rule archive (1 invalid record)." >&2
    exit 1
  fi
  if [[ "$additional_frozen_m4a" == "1" ]]; then
    if [[ -z "$additional_run_profile" ]]; then
      if [[ "$additional_batch_phase" != "copies_verified" || "$additional_run_outcome" != "completed" ]]; then
        echo "Additional-file batch verification failed (1 invalid record)." >&2
        exit 1
      fi
    else
      case "$additional_batch_phase" in m4a_cohort_verified|sources_revalidated|completed) ;; *) echo "Additional-file batch barrier verification failed (1 invalid run)." >&2; exit 1 ;; esac
      if [[ "$additional_run_outcome" != "completed" || "$additional_run_profile" != "$PROFILE_ID" ]]; then
        echo "Additional-file batch barrier verification failed (1 invalid record)." >&2
        exit 1
      fi
    fi
  elif [[ "$additional_frozen_m4a" != "0" || "$additional_batch_phase" != "copies_verified" \
    || "$additional_run_outcome" != "wav_backup_complete_source_retained" ]]; then
    echo "Additional-file batch barrier verification failed (1 invalid record)." >&2
    exit 1
  fi
  additional_artifact_path="$destination/$additional_artifact"
  actual_additional_size="$(stat -f '%z' "$additional_artifact_path")"
  actual_additional_hash="$(shasum -a 256 "$additional_artifact_path" | awk '{print $1}')"
  if [[ "$actual_additional_size" != "$additional_artifact_size" \
    || "$actual_additional_hash" != "$additional_artifact_hash" ]]; then
    echo "Raw additional-file artifact verification failed (1 mismatched artifact)." >&2
    exit 1
  fi
  verified_additional_count=$((verified_additional_count + 1))
  if [[ "$diagnostic" -eq 1 ]]; then
    echo "verified source_id=$additional_source_id additional=$additional_rule:$additional_source source_sha256=$additional_source_hash artifact=$additional_artifact artifact_sha256=$additional_artifact_hash classification=$additional_classification run=$additional_run_id"
  fi
done < "$additional_evidence"

live_source_count=0
live_additional_count=0
source_ordinal=0
while IFS="$separator" read -r rule_name source_root; do
  source_ordinal=$((source_ordinal + 1))
  while IFS= read -r -d '' unsafe_entry; do
    echo "Live source safety verification failed (1 unsafe entry)." >&2
    exit 1
  done < <(find "$source_root" -mindepth 1 \
    \( -type d -path "$source_root/.*" -prune \) -o \
    \( -type l -o -type p -o -type s -o -type b -o -type c \) -print0)
  file_ordinal=0
  while IFS= read -r -d '' source_file; do
    file_ordinal=$((file_ordinal + 1))
    source_relative="${source_file#"$source_root"/}"
    base="$(basename "$source_relative")"
    is_safe_additional_relative "$source_relative" || {
      echo "Live source contains an unsafe relative path (1 invalid file)." >&2
      exit 1
    }
    source_size="$(stat -f '%z' "$source_file")"
    source_hash="$(shasum -a 256 "$source_file" | awk '{print $1}')"
    recording_matches="$temporary/live-recording-$source_ordinal-$file_ordinal"
    additional_matches="$temporary/live-additional-$source_ordinal-$file_ordinal"
    awk -F "$separator" -v rule="$rule_name" -v path="$source_relative" \
      -v bytes="$source_size" -v digest="$source_hash" \
      '$2 == rule && $3 == path && $4 == bytes && $5 == digest { print }' \
      "$recording_evidence" > "$recording_matches"
    awk -F "$separator" -v rule="$rule_name" -v path="$source_relative" \
      -v bytes="$source_size" -v digest="$source_hash" \
      '$2 == rule && $3 == path && $4 == bytes && $5 == digest { print }' \
      "$additional_evidence" > "$additional_matches"
    recording_match_count="$(wc -l < "$recording_matches" | tr -d ' ')"
    additional_match_count="$(wc -l < "$additional_matches" | tr -d ' ')"
    total_match_count=$((recording_match_count + additional_match_count))
    if [[ "$total_match_count" -eq 0 && "$source_relative" != */* && "$base" == .* ]]; then
      continue
    fi
    if [[ "$total_match_count" -ne 1 ]]; then
      echo "Live rule source evidence verification failed (1 unmatched or ambiguous file)." >&2
      [[ "$diagnostic" -eq 0 ]] || echo "source=$rule_name:$source_relative matches=$total_match_count" >&2
      exit 1
    fi
    if [[ "$recording_match_count" -eq 1 ]]; then
      live_source_count=$((live_source_count + 1))
    else
      live_additional_count=$((live_additional_count + 1))
    fi
  done < <(find "$source_root" -type d -name '.*' -prune -o -type f -print0)
done < "$source_map"

echo "Live source WAV files: $live_source_count"
echo "Live additional files: $live_additional_count"
echo "Ledger-matched final artifacts: $verified_artifact_count"
echo "Verified WAV artifacts: $wav_count"
echo "Verified 128kbps-profile M4A artifacts: $m4a_count"
echo "Verified raw additional-file artifacts: $verified_additional_count"
echo "Every in-scope live source and durable destination artifact passed independent verification."
