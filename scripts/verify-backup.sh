#!/bin/bash

set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "Usage: $0 DESTINATION SOURCE_VOLUME [SOURCE_VOLUME ...]" >&2
  exit 2
fi

destination="$1"
shift

if [[ ! -d "$destination" ]]; then
  echo "Backup destination is not a directory." >&2
  exit 2
fi
for source_root in "$@"; do
  if [[ ! -d "$source_root" ]]; then
    echo "A source volume is not mounted." >&2
    exit 2
  fi
done

temporary="$(mktemp -d -t dji-mic-verify)"
trap 'rm -rf "$temporary"' EXIT
source_hashes="$temporary/source-hashes"
destination_hashes="$temporary/destination-hashes"
missing_hashes="$temporary/missing-hashes"

for source_root in "$@"; do
  find "$source_root" -type d -name '.*' -prune -o -type f -iname '*.wav' -exec shasum -a 256 {} \;
done | awk '{print $1}' | LC_ALL=C sort > "$source_hashes"

find "$destination" -type d -name '.*' -prune -o -type f -iname '*.wav' -exec shasum -a 256 {} \; \
  | awk '{print $1}' \
  | LC_ALL=C sort > "$destination_hashes"

LC_ALL=C comm -23 "$source_hashes" "$destination_hashes" > "$missing_hashes"
source_count="$(wc -l < "$source_hashes" | tr -d ' ')"
destination_count="$(wc -l < "$destination_hashes" | tr -d ' ')"
missing_count="$(wc -l < "$missing_hashes" | tr -d ' ')"

echo "Source WAV files: $source_count"
echo "Destination WAV files: $destination_count"
echo "Missing verified hashes: $missing_count"

if [[ "$source_count" -eq 0 ]]; then
  echo "No source WAV files were found." >&2
  exit 1
fi
if [[ "$missing_count" -ne 0 ]]; then
  echo "Backup verification failed." >&2
  exit 1
fi

echo "Every source WAV has an independently verified destination copy."
