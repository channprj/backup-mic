#!/bin/bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_APP="/Users/channprj/Applications/Backup Mic.app"
LEGACY_TARGET_APP="/Users/channprj/Applications/DJI Mic Backup.app"
TARGET_PARENT="/Users/channprj/Applications"
EXPECTED_IDENTIFIER="com.channprj.BackupMic"
LEGACY_IDENTIFIER="com.channprj.DJIMicBackup"
EXPECTED_EXECUTABLE="backup-mic"
LEGACY_EXECUTABLE="dji-mic-backup"
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
  local identifier bundle_version minimum_system executable architectures codesign_details
  [[ -d "$app" && ! -L "$app" ]] || return 1
  codesign --verify --deep --strict --verbose=2 "$app"
  codesign_details="$(codesign -d --verbose=4 "$app" 2>&1)"
  [[ "$codesign_details" == *"Signature=adhoc"* ]] || return 1
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
install_temp="$(mktemp -d "$TARGET_PARENT/.backup-mic-install.XXXXXX")"
chmod 700 "$install_temp"
staged_app="$install_temp/Backup Mic.app"
rollback_root="$install_temp/Previous Apps"
current_rollback_app="$rollback_root/Backup Mic.app"
legacy_rollback_app="$rollback_root/DJI Mic Backup.app"
failed_app="$install_temp/Backup Mic.failed.app"
mkdir "$rollback_root"
current_moved=0
legacy_moved=0
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
  if [[ "$current_moved" -eq 1 && -e "$current_rollback_app" ]]; then
    mv "$current_rollback_app" "$TARGET_APP"
  fi
  if [[ "$legacy_moved" -eq 1 && -e "$legacy_rollback_app" ]]; then
    mv "$legacy_rollback_app" "$LEGACY_TARGET_APP"
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

"$PROJECT_ROOT/scripts/stop-local-apps.sh" \
  "$EXPECTED_IDENTIFIER" \
  "$LEGACY_IDENTIFIER" \
  "$TARGET_APP/Contents/MacOS/$EXPECTED_EXECUTABLE" \
  "$LEGACY_TARGET_APP/Contents/MacOS/$LEGACY_EXECUTABLE"

if [[ -e "$TARGET_APP" ]]; then
  [[ -d "$TARGET_APP" && ! -L "$TARGET_APP" ]] || {
    echo "The exact installation target is not a regular app bundle." >&2
    exit 1
  }
  mv "$TARGET_APP" "$current_rollback_app"
  current_moved=1
fi

if [[ -e "$LEGACY_TARGET_APP" ]]; then
  [[ -d "$LEGACY_TARGET_APP" && ! -L "$LEGACY_TARGET_APP" ]] || {
    echo "The exact legacy installation target is not a regular app bundle." >&2
    exit 1
  }
  mv "$LEGACY_TARGET_APP" "$legacy_rollback_app"
  legacy_moved=1
fi

mv "$staged_app" "$TARGET_APP"
new_installed=1
verify_app "$TARGET_APP"
installed_hash="$(shasum -a 256 "$TARGET_APP/Contents/MacOS/$EXPECTED_EXECUTABLE" | awk '{print $1}')"
if [[ "$installed_hash" != "$release_hash" ]]; then
  echo "Installed executable does not match the verified release executable." >&2
  exit 1
fi
if [[ -e "$LEGACY_TARGET_APP" ]]; then
  echo "The exact legacy app bundle is still present after installation." >&2
  exit 1
fi

if [[ "$current_moved" -eq 1 || "$legacy_moved" -eq 1 ]]; then
  move_to_trash "$rollback_root"
else
  rmdir "$rollback_root"
fi
completed=1
rmdir "$install_temp" || echo "Installation succeeded, but empty staging remains at: $install_temp" >&2

echo "Installed app: $TARGET_APP"
echo "Version: $expected_version"
echo "Executable SHA-256: $installed_hash"
echo "Prior exact Backup Mic bundles were moved to macOS Trash when they existed."
