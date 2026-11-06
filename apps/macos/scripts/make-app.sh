#!/usr/bin/env bash
# MultiTerm.app を組み立てる（署名なし・開発用）。先に build-core.sh を実行しておくこと。
#
#   apps/macos/scripts/make-app.sh          # debug
#   CONFIG=release apps/macos/scripts/make-app.sh
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="${CONFIG:-debug}"
cd "$APP_DIR"

swift build -c "$CONFIG"
BIN="$(swift build -c "$CONFIG" --show-bin-path)/MultiTerm"

BUNDLE="$APP_DIR/build/MultiTerm.app"
rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
cp "$BIN" "$BUNDLE/Contents/MacOS/MultiTerm"
# SwiftPM のリソース（Tera Term の文言）
cp -R "$(dirname "$BIN")/MultiTerm_MultiTerm.bundle" "$BUNDLE/Contents/Resources/"
cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>MultiTerm</string>
	<key>CFBundleIdentifier</key>
	<string>dev.multiterm.MultiTerm</string>
	<key>CFBundleName</key>
	<string>Multi-Term</string>
	<key>CFBundleDisplayName</key>
	<string>Multi-Term</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>0.0.0</string>
	<key>CFBundleVersion</key>
	<string>1</string>
	<key>LSMinimumSystemVersion</key>
	<string>14.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSPrincipalClass</key>
	<string>NSApplication</string>
	<key>NSLocalNetworkUsageDescription</key>
	<string>LAN 内のホストへ SSH・Telnet・TCP で接続するために使います。</string>
</dict>
</plist>
PLIST
codesign --force --sign - "$BUNDLE" >/dev/null 2>&1 || true
echo "built: $BUNDLE"
