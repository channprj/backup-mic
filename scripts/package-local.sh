#!/bin/bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_ROOT="$PROJECT_ROOT/src-tauri/target/release/bundle"

cd "$PROJECT_ROOT"
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
executable="$app/Contents/MacOS/dji-mic-backup"

codesign --verify --deep --strict --verbose=2 "$app"
identifier="$(plutil -extract CFBundleIdentifier raw -o - "$app/Contents/Info.plist")"
minimum_system="$(plutil -extract LSMinimumSystemVersion raw -o - "$app/Contents/Info.plist")"
architectures="$(lipo -archs "$executable")"
executable_hash="$(shasum -a 256 "$executable" | awk '{print $1}')"
dmg_hash="$(shasum -a 256 "$dmg" | awk '{print $1}')"
hdiutil verify "$dmg"

if [[ "$identifier" != "com.channprj.DJIMicBackup" ]]; then
  echo "Unexpected bundle identifier: $identifier" >&2
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
echo "Minimum macOS: $minimum_system"
echo "Architectures: $architectures"
echo "Executable SHA-256: $executable_hash"
echo "DMG SHA-256: $dmg_hash"
