#!/usr/bin/env bash
# Rust コア（crates/ffi）をビルドし、XCFramework と Swift バインディングを生成する。
#
#   apps/macos/scripts/build-core.sh            # release
#   PROFILE=debug apps/macos/scripts/build-core.sh
#
# 出力
#   apps/macos/build/MTCoreFFI.xcframework      静的ライブラリ + C ヘッダ + modulemap
#   apps/macos/Sources/MTCore/Generated/        UniFFI が生成した Swift
#   apps/macos/Sources/MultiTerm/Resources/     spec/ から写した Tera Term の文言
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
APP="$ROOT/apps/macos"
PROFILE="${PROFILE:-release}"
TARGETS=(aarch64-apple-darwin)   # Intel 対応時は x86_64-apple-darwin を足して lipo する
export MACOSX_DEPLOYMENT_TARGET=14.0

cd "$ROOT"

if [[ "$PROFILE" == "release" ]]; then PROFILE_FLAG=--release; PROFILE_DIR=release
else PROFILE_FLAG=; PROFILE_DIR=debug; fi

for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null 2>&1 || true
  cargo build -p mt-ffi $PROFILE_FLAG --target "$t"
done
LIB="$ROOT/target/${TARGETS[0]}/$PROFILE_DIR/libmt_ffi.a"

GEN="$APP/build/generated"
rm -rf "$GEN"
mkdir -p "$GEN/include"
cargo run -q -p mt-ffi --features bindgen --bin uniffi-bindgen-swift -- \
  --swift-sources --headers --modulemap \
  --module-name mt_ffiFFI --modulemap-filename module.modulemap \
  "$LIB" "$GEN"
mv "$GEN"/*.h "$GEN/module.modulemap" "$GEN/include/"

# XCFramework は xcodebuild を使わず直接組み立てる（Command Line Tools だけでもビルドできるように）
XCF="$APP/build/MTCoreFFI.xcframework"
SLICE="macos-arm64"
rm -rf "$XCF"
mkdir -p "$XCF/$SLICE/Headers"
cp "$LIB" "$XCF/$SLICE/"
cp "$GEN/include/"* "$XCF/$SLICE/Headers/"
cat > "$XCF/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AvailableLibraries</key>
	<array>
		<dict>
			<key>HeadersPath</key>
			<string>Headers</string>
			<key>LibraryIdentifier</key>
			<string>$SLICE</string>
			<key>LibraryPath</key>
			<string>libmt_ffi.a</string>
			<key>SupportedArchitectures</key>
			<array>
				<string>arm64</string>
			</array>
			<key>SupportedPlatform</key>
			<string>macos</string>
		</dict>
	</array>
	<key>CFBundlePackageType</key>
	<string>XFWK</string>
	<key>XCFrameworkFormatVersion</key>
	<string>1.0</string>
</dict>
</plist>
PLIST

mkdir -p "$APP/Sources/MTCore/Generated"
cp "$GEN"/*.swift "$APP/Sources/MTCore/Generated/"
# Tera Term の文言（spec/ は tools/extract-tt-ui の生成物）
RES="$APP/Sources/MultiTerm/Resources/spec"
rm -rf "$APP/Sources/MultiTerm/Resources"
mkdir -p "$RES"
cp "$ROOT/spec/lang/ja_JP.json" "$ROOT/spec/lang/en_US.json" "$ROOT/spec/serial-options.json" "$RES/"

echo "built: $XCF"
