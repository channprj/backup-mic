#!/bin/bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_APP="/Users/channprj/Applications/DJI Mic Backup.app"
TARGET_PARENT="/Users/channprj/Applications"
EXPECTED_IDENTIFIER="com.channprj.DJIMicBackup"
EXPECTED_EXECUTABLE="dji-mic-backup"
EXPECTED_MINIMUM_SYSTEM="13.0"

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 RELEASE_APP [EXPECTED_VERSION]" >&2
  exit 2
fi

release_app="$1"
expected_version="${2:-}"
if [[ -z "$expected_version" ]]; then
  [[ -f "$PROJECT_ROOT/VERSION" && ! -L "$PROJECT_ROOT/VERSION" ]] || {
    echo "VERSION is unavailable; run Headatever before installation." >&2
    exit 2
  }
  expected_version="$(tr -d '\r\n' < "$PROJECT_ROOT/VERSION")"
fi
if [[ ! "$expected_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
  echo "Expected version is not a valid release version." >&2
  exit 2
fi
if [[ ! -d "$release_app" || -L "$release_app" ]]; then
  echo "Release app is not a regular app bundle." >&2
  exit 2
fi
release_app="$(cd "$(dirname "$release_app")" && pwd -P)/$(basename "$release_app")"

verify_app() {
  local app="$1"
  local identifier bundle_version minimum_system executable architectures
  [[ -d "$app" && ! -L "$app" ]] || return 1
  codesign --verify --deep --strict --verbose=2 "$app"
  identifier="$(plutil -extract CFBundleIdentifier raw -o - "$app/Contents/Info.plist")"
  bundle_version="$(plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist")"
  minimum_system="$(plutil -extract LSMinimumSystemVersion raw -o - "$app/Contents/Info.plist")"
  executable="$app/Contents/MacOS/$EXPECTED_EXECUTABLE"
  [[ "$identifier" == "$EXPECTED_IDENTIFIER" && "$bundle_version" == "$expected_version" \
    && "$minimum_system" == "$EXPECTED_MINIMUM_SYSTEM" \
    && -f "$executable" && ! -L "$executable" && -x "$executable" ]] || return 1
  architectures="$(lipo -archs "$executable")"
  [[ " $architectures " == *" arm64 "* ]] || return 1
}

running_pids() {
  local pid command
  while read -r pid command; do
    if [[ "$command" == "$TARGET_APP/Contents/MacOS/$EXPECTED_EXECUTABLE" ]]; then
      echo "$pid"
    fi
  done < <(ps -axo pid=,command=)
}

move_to_trash() {
  local path="$1"
  /usr/bin/osascript -l JavaScript \
    -e 'ObjC.import("Foundation"); function run(argv) { const manager = $.NSFileManager.defaultManager; const url = $.NSURL.fileURLWithPath($(argv[0])); const resulting = Ref(); const error = Ref(); if (!manager.trashItemAtURLResultingItemURLError(url, resulting, error)) { throw new Error(ObjC.unwrap(error[0].localizedDescription)); } }' \
    -- "$path"
}

verify_app "$release_app" || {
  echo "Release app verification failed before installation." >&2
  exit 1
}

mkdir -p "$TARGET_PARENT"
install_temp="$(mktemp -d "$TARGET_PARENT/.dji-mic-install.XXXXXX")"
chmod 700 "$install_temp"
staged_app="$install_temp/DJI Mic Backup.app"
rollback_app="$install_temp/DJI Mic Backup.previous.app"
failed_app="$install_temp/DJI Mic Backup.failed.app"
previous_moved=0
new_installed=0
completed=0

rollback_on_exit() {
  local status=$?
  trap - EXIT INT TERM
  if [[ "$completed" -eq 1 ]]; then
    exit "$status"
  fi
  set +e
  if [[ "$new_installed" -eq 1 && -e "$TARGET_APP" ]]; then
    mv "$TARGET_APP" "$failed_app"
  fi
  if [[ "$previous_moved" -eq 1 && -e "$rollback_app" ]]; then
    mv "$rollback_app" "$TARGET_APP"
  fi
  if [[ -d "$install_temp" ]]; then
    if ! move_to_trash "$install_temp"; then
      echo "Rollback restored the prior app, but generated staging remains at: $install_temp" >&2
    fi
  fi
  echo "Local installation failed; the previous app was restored when available." >&2
  exit "$status"
}
trap rollback_on_exit EXIT INT TERM

/usr/bin/ditto --noqtn "$release_app" "$staged_app"
verify_app "$staged_app"
release_hash="$(shasum -a 256 "$release_app/Contents/MacOS/$EXPECTED_EXECUTABLE" | awk '{print $1}')"
staged_hash="$(shasum -a 256 "$staged_app/Contents/MacOS/$EXPECTED_EXECUTABLE" | awk '{print $1}')"
if [[ "$release_hash" != "$staged_hash" ]]; then
  echo "Staged executable does not match the verified release executable." >&2
  exit 1
fi

if [[ -n "$(running_pids)" ]]; then
  /usr/bin/osascript -e 'tell application id "com.channprj.DJIMicBackup" to quit' >/dev/null 2>&1 || true
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    [[ -z "$(running_pids)" ]] && break
    sleep 1
  done
  if [[ -n "$(running_pids)" ]]; then
    echo "The running DJI Mic Backup app did not quit; installation was not started." >&2
    exit 1
  fi
fi

if [[ -e "$TARGET_APP" ]]; then
  [[ -d "$TARGET_APP" && ! -L "$TARGET_APP" ]] || {
    echo "The exact installation target is not a regular app bundle." >&2
    exit 1
  }
  mv "$TARGET_APP" "$rollback_app"
  previous_moved=1
fi

mv "$staged_app" "$TARGET_APP"
new_installed=1
verify_app "$TARGET_APP"
installed_hash="$(shasum -a 256 "$TARGET_APP/Contents/MacOS/$EXPECTED_EXECUTABLE" | awk '{print $1}')"
if [[ "$installed_hash" != "$release_hash" ]]; then
  echo "Installed executable does not match the verified release executable." >&2
  exit 1
fi

if [[ "$previous_moved" -eq 1 ]]; then
  move_to_trash "$rollback_app"
fi
rmdir "$install_temp"
completed=1

echo "Installed app: $TARGET_APP"
echo "Version: $expected_version"
echo "Executable SHA-256: $installed_hash"
echo "The previous app bundle was moved to macOS Trash when one existed."
