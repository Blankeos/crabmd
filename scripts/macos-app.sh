#!/usr/bin/env bash
# Wrap a crabmd binary in CrabMD.app (Spotlight / /Applications).
#
#   ./scripts/macos-app.sh --bin target/release/crabmd --version 0.0.2 --out CrabMD.app
set -euo pipefail

BIN=""
VERSION=""
OUT="CrabMD.app"
ICON=""

usage() {
    echo "usage: $0 --bin PATH --version VER [--out CrabMD.app] [--icon AppIcon.icns]" >&2
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --bin) BIN="${2:-}"; shift 2 ;;
        --version) VERSION="${2:-}"; shift 2 ;;
        --out) OUT="${2:-}"; shift 2 ;;
        --icon) ICON="${2:-}"; shift 2 ;;
        -h | --help) usage ;;
        *) echo "unknown arg: $1" >&2; usage ;;
    esac
done

[[ -n "$BIN" && -x "$BIN" ]] || { echo "missing executable --bin" >&2; exit 1; }
[[ -n "$VERSION" ]] || { echo "missing --version" >&2; exit 1; }

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ICON="${ICON:-"$ROOT/assets/AppIcon.icns"}"
[[ -f "$ICON" ]] || { echo "missing icon: $ICON" >&2; exit 1; }

rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
cp "$BIN" "$OUT/Contents/MacOS/crabmd"
chmod +x "$OUT/Contents/MacOS/crabmd"
cp "$ICON" "$OUT/Contents/Resources/AppIcon.icns"
printf 'APPL????' > "$OUT/Contents/PkgInfo"

cat > "$OUT/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>CrabMD</string>
  <key>CFBundleExecutable</key>
  <string>crabmd</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIdentifier</key>
  <string>ai.blankeos.crabmd</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>CrabMD</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.productivity</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeExtensions</key>
      <array>
        <string>md</string>
        <string>markdown</string>
        <string>mdown</string>
        <string>mkd</string>
      </array>
      <key>CFBundleTypeIconFile</key>
      <string>AppIcon</string>
      <key>CFBundleTypeName</key>
      <string>Markdown document</string>
      <key>CFBundleTypeRole</key>
      <string>Editor</string>
      <key>LSHandlerRank</key>
      <string>Alternate</string>
    </dict>
  </array>
</dict>
</plist>
EOF

if command -v codesign >/dev/null 2>&1; then
    codesign --force --deep --sign - "$OUT"
fi

echo "$OUT"
