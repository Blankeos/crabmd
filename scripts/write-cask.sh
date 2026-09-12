#!/usr/bin/env bash
# Generate the Homebrew cask for CrabMD.app (macOS sole distribution).
#
#   ./scripts/write-cask.sh --version 0.0.3 --sha-arm HEX --sha-intel HEX --out crabmd.rb
#
# The cask ships /Applications/CrabMD.app (Spotlight/Dock/Open-With) plus a
# crabmd CLI shim. Postflight strips quarantine for immediate first launch
# and safely migrates legacy formula/npm self-installed copies in
# ~/Applications/CrabMD.app: only when the bundle is provably ours
# (origin marker + CFBundleIdentifier + path distinction), never blindly.
set -euo pipefail

VERSION=""
SHA_ARM=""
SHA_INTEL=""
OUT=""

usage() {
    echo "usage: $0 --version VER --sha-arm HEX --sha-intel HEX --out PATH" >&2
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) VERSION="${2:-}"; shift 2 ;;
        --sha-arm) SHA_ARM="${2:-}"; shift 2 ;;
        --sha-intel) SHA_INTEL="${2:-}"; shift 2 ;;
        --out) OUT="${2:-}"; shift 2 ;;
        -h | --help) usage ;;
        *) echo "unknown arg: $1" >&2; usage ;;
    esac
done

[[ -n "$VERSION" && -n "$SHA_ARM" && -n "$SHA_INTEL" && -n "$OUT" ]] || usage

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
    echo "invalid --version (expected SemVer like 0.0.3): $VERSION" >&2
    exit 1
fi
if [[ ! "$SHA_ARM" =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "invalid --sha-arm (expected 64 hex chars)" >&2
    exit 1
fi
if [[ ! "$SHA_INTEL" =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "invalid --sha-intel (expected 64 hex chars)" >&2
    exit 1
fi

mkdir -p "$(dirname "$OUT")"
# Quoted heredoc: no shell expansion inside the Ruby template (shell $vars and
# Ruby interpolation stay literal). Placeholders are replaced with sed below.
cat > "$OUT" <<'RUBY_EOF'
cask "crabmd" do
  arch arm: "aarch64", intel: "x86_64"

  version "__VERSION__"
  sha256 arm:   "__SHA_ARM__",
         intel: "__SHA_INTEL__"

  url "https://github.com/Blankeos/crabmd/releases/download/v#{version}/CrabMD-#{arch}-apple-darwin.zip"
  name "CrabMD"
  desc "Fast native GPUI markdown writer"
  homepage "https://github.com/Blankeos/crabmd"

  livecheck do
    url :url
    strategy :github_latest
  end

  # The retired formula shipped the same crabmd binary without the app
  # bundle. Keep the conflict so brew errors clearly instead of forking app
  # identity (two Dock icons, Spotlight reopen doing nothing).
  conflicts_with formula: "crabmd"
  depends_on macos: :big_sur

  app "CrabMD.app"
  binary "#{appdir}/CrabMD.app/Contents/MacOS/crabmd", target: "crabmd"

  # Structured steps only (legacy postflight Ruby is rejected by brew style,
  # even in third-party taps). The migration below never blindly deletes:
  # the outer guard requires the legacy origin marker, and the shell
  # re-checks path distinction + bundle identity before removing.
  postflight_steps do
    run "/usr/bin/xattr",
        args:         ["-dr", "com.apple.quarantine", "{{appdir}}/CrabMD.app"],
        must_succeed: false
    if_path_exists "Applications/CrabMD.app/Contents/Resources/origin", base: :home do
      run "/bin/sh",
          args:           ["-c", <<~SH],
            legacy="$HOME/Applications/CrabMD.app"
            managed="{{appdir}}/CrabMD.app"
            [ "$legacy" != "$managed" ] || exit 0
            if [ -e "$legacy" ] && [ -e "$managed" ] && [ "$legacy" -ef "$managed" ]; then exit 0; fi
            [ -d "$legacy" ] || exit 0
            [ -f "$legacy/Contents/Info.plist" ] || exit 0
            id=$(/usr/bin/defaults read "$legacy/Contents/Info" CFBundleIdentifier 2>/dev/null || true)
            [ "$id" = "ai.blankeos.crabmd" ] || exit 0
            [ -f "$legacy/Contents/Resources/origin" ] || exit 0
            rm -rf "$legacy"
          SH
          must_succeed:   false,
          writable_paths: ["Applications"],
          writable_base:  :home
    end
  end

  # app + binary artifacts are removed automatically on
  # brew uninstall --cask (managed /Applications copy + CLI shim).
  # Settings in ~/.config/crabmd are retained; use --zap to discard them.
  uninstall quit: "ai.blankeos.crabmd"

  zap trash: [
    "~/.config/crabmd",
    "~/Library/Saved Application State/ai.blankeos.crabmd.savedState",
  ]

  caveats "The Homebrew formula and npm package are retired on macOS."
end
RUBY_EOF

# Replace placeholders (values are validated hex/SemVer above, safe for sed).
sed -i.bak \
  -e "s/__VERSION__/${VERSION}/g" \
  -e "s/__SHA_ARM__/${SHA_ARM}/g" \
  -e "s/__SHA_INTEL__/${SHA_INTEL}/g" \
  "$OUT"
rm -f "${OUT}.bak"
echo "$OUT"
