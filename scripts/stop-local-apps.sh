#!/bin/bash

set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "Usage: $0 IDENTIFIER LEGACY_IDENTIFIER EXECUTABLE LEGACY_EXECUTABLE" >&2
  exit 2
fi

identifier="$1"
legacy_identifier="$2"
expected_command="$3"
legacy_command="$4"

running_pids() {
  local pid command
  while read -r pid command; do
    if [[ "$command" == "$expected_command" || "$command" == "$legacy_command" ]]; then
      echo "$pid"
    fi
  done < <(ps -axo pid=,command=)
}

wait_for_exit() {
  local _
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    [[ -z "$(running_pids)" ]] && return 0
    sleep 0.25
  done
  return 1
}

/usr/bin/osascript -l JavaScript \
  -e 'ObjC.import("AppKit"); function run(argv) { for (let i = 0; i < argv.length; i += 1) { const apps = $.NSRunningApplication.runningApplicationsWithBundleIdentifier($(argv[i])); for (let j = 0; j < apps.count; j += 1) { apps.objectAtIndex(j).terminate; } } }' \
  -- "$identifier" "$legacy_identifier" >/dev/null 2>&1 || true

if wait_for_exit; then
  exit 0
fi

while read -r pid; do
  kill -TERM "$pid" 2>/dev/null || true
done < <(running_pids)

if wait_for_exit; then
  exit 0
fi

echo "A running Backup Mic app did not quit; installation was not started." >&2
exit 1
