#!/usr/bin/env bash
# Build Ferry.app and install it to ~/Applications.
#   ./build.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
APP="${1:-$HOME/Applications/Ferry.app}"

command -v cargo >/dev/null || { echo "Rust is required: https://rustup.rs"; exit 1; }

echo "-> building"
cargo build --release --manifest-path "$ROOT/src-tauri/Cargo.toml"

echo "-> bundling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/src-tauri/target/release/ferry" "$APP/Contents/MacOS/Ferry"
cp "$ROOT/src-tauri/icons/icon.icns"      "$APP/Contents/Resources/icon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Ferry</string>
  <key>CFBundleDisplayName</key><string>Ferry</string>
  <key>CFBundleExecutable</key><string>Ferry</string>
  <key>CFBundleIdentifier</key><string>dev.ahkamboh.ferry</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundleVersion</key><string>1.4.0</string>
  <key>CFBundleShortVersionString</key><string>1.4.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST

codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
echo "-> done: $APP  ($(du -sh "$APP" | cut -f1))"
echo "   open \"$APP\""
