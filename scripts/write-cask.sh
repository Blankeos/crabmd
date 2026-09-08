#!/usr/bin/env bash
# Generate the Homebrew cask for CrabMD.app.
#
#   ./scripts/write-cask.sh --version 0.0.2 --sha-arm HEX --sha-intel HEX --out crabmd.rb
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

mkdir -p "$(dirname "$OUT")"
cat > "$OUT" <<EOF
cask "crabmd" do
  arch arm: "aarch64", intel: "x86_64"

  version "${VERSION}"
  sha256 arm:   "${SHA_ARM}",
         intel: "${SHA_INTEL}"

  url "https://github.com/Blankeos/crabmd/releases/download/v#{version}/CrabMD-#{arch}-apple-darwin.zip"
  name "CrabMD"
  desc "Fast native GPUI markdown writer"
  homepage "https://github.com/Blankeos/crabmd"

  livecheck do
    url :url
    strategy :github_latest
  end

  conflicts_with formula: "crabmd"
  depends_on macos: ">= :big_sur"

  app "CrabMD.app"
  binary "#{appdir}/CrabMD.app/Contents/MacOS/crabmd", target: "crabmd"

  # Self-signed / ad-hoc; strip quarantine so Gatekeeper does not block launch.
  postflight_steps do
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{appdir}}/CrabMD.app"]
  end

  uninstall quit: "ai.blankeos.crabmd"

  zap trash: [
    "~/.config/crabmd",
    "~/Library/Saved Application State/ai.blankeos.crabmd.savedState",
  ]
end
EOF
echo "$OUT"
