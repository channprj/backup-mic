#!/bin/bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
BUNDLE_ROOT="$PROJECT_ROOT/src-tauri/target/release/bundle"

if [[ $# -gt 1 ]]; then
  echo "Usage: $0 [EXPECTED_VERSION]" >&2
  exit 2
fi
expected_version="${1:-}"
if [[ -z "$expected_version" ]]; then
  [[ -f "$PROJECT_ROOT/VERSION" && ! -L "$PROJECT_ROOT/VERSION" ]] || {
    echo "VERSION is unavailable; run Headatever before packaging." >&2
    exit 2
  }
  expected_version="$(tr -d '\r\n' < "$PROJECT_ROOT/VERSION")"
fi
if [[ ! "$expected_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
  echo "Expected version is not a valid release version." >&2
  exit 2
fi

cd "$PROJECT_ROOT"
if [[ -d "$BUNDLE_ROOT" ]]; then
  if [[ -L "$PROJECT_ROOT/src-tauri/target" || -L "$BUNDLE_ROOT" \
    || "$(cd "$BUNDLE_ROOT" && pwd -P)" != "$BUNDLE_ROOT" \
    || -L "$BUNDLE_ROOT/macos" || -L "$BUNDLE_ROOT/dmg" ]]; then
    echo "Refusing to clean an unexpected or linked bundle directory." >&2
    exit 1
  fi
  rm -rf -- "$BUNDLE_ROOT/macos" "$BUNDLE_ROOT/dmg"
fi
pnpm tauri build --bundles app,dmg --ci

apps=("$BUNDLE_ROOT"/macos/*.app)
dmgs=("$BUNDLE_ROOT"/dmg/*.dmg)
if [[ ${#apps[@]} -ne 1 || ! -d "${apps[0]}" ]]; then
  echo "Expected exactly one macOS app bundle." >&2
  exit 1
fi
if [[ ${#dmgs[@]} -ne 1 || ! -f "${dmgs[0]}" ]]; then
  echo "Expected exactly one DMG." >&2
  exit 1
fi

app="${apps[0]}"
dmg="${dmgs[0]}"
executable="$app/Contents/MacOS/backup-mic"

if [[ "$app" != "$BUNDLE_ROOT/macos/Backup Mic.app" ]]; then
  echo "Unexpected app bundle name: $(basename "$app")" >&2
  exit 1
fi
if [[ "$(basename "$dmg")" != "Backup Mic_${expected_version}_aarch64.dmg" ]]; then
  echo "Unexpected DMG name: $(basename "$dmg")" >&2
  exit 1
fi
if [[ ! -f "$executable" || -L "$executable" || ! -x "$executable" ]]; then
  echo "Expected release executable is unavailable or unsafe." >&2
  exit 1
fi

codesign --verify --deep --strict --verbose=2 "$app"
codesign_details="$(codesign -d --verbose=4 "$app" 2>&1)"
if [[ "$codesign_details" != *"Signature=adhoc"* ]]; then
  echo "Local release app does not have the expected ad-hoc signature." >&2
  exit 1
fi
identifier="$(plutil -extract CFBundleIdentifier raw -o - "$app/Contents/Info.plist")"
bundle_version="$(plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist")"
minimum_system="$(plutil -extract LSMinimumSystemVersion raw -o - "$app/Contents/Info.plist")"
architectures="$(lipo -archs "$executable")"
executable_hash="$(shasum -a 256 "$executable" | awk '{print $1}')"
dmg_hash="$(shasum -a 256 "$dmg" | awk '{print $1}')"
hdiutil verify "$dmg"

if [[ "$identifier" != "com.channprj.BackupMic" ]]; then
  echo "Unexpected bundle identifier: $identifier" >&2
  exit 1
fi
if [[ "$bundle_version" != "$expected_version" ]]; then
  echo "Unexpected bundle version: $bundle_version (expected $expected_version)" >&2
  exit 1
fi
if [[ "$minimum_system" != "13.0" ]]; then
  echo "Unexpected minimum macOS version: $minimum_system" >&2
  exit 1
fi
if [[ " $architectures " != *" arm64 "* ]]; then
  echo "Release executable does not include arm64: $architectures" >&2
  exit 1
fi

echo "App: $app"
echo "DMG: $dmg"
echo "Identifier: $identifier"
echo "Version: $bundle_version"
echo "Minimum macOS: $minimum_system"
echo "Architectures: $architectures"
echo "Signature: ad-hoc (deep strict verified)"
echo "Executable SHA-256: $executable_hash"
echo "DMG SHA-256: $dmg_hash"
