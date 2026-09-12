#!/usr/bin/env bash
# Packaging validation for the cask-only macOS distribution.
#
#   bash scripts/test-packaging.sh
#
# Checks workflows, cask generator, bundle wrapper, and docs stay coherent:
# cask-only macOS, no formula/npm publish, safe legacy migration (bundle
# identity + path distinction, no blind deletes), Spotlight immediacy,
# clean uninstall, and race-safe tap publishing. No network, no commits.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PASS=0
FAIL=0
pass() { PASS=$((PASS + 1)); echo "  ok: $1"; }
fail() { FAIL=$((FAIL + 1)); echo "  FAIL: $1"; }

section() { echo "== $1"; }

# 1. dist-workspace.toml: shell-only, no tap/formula publishing.
section "dist-workspace.toml"
if grep -q 'installers = \["shell"\]' dist-workspace.toml; then pass "installers is shell-only"; else fail "installers must be [\"shell\"]"; fi
if grep -q 'installers = \[.*homebrew' dist-workspace.toml; then fail "homebrew installer still present"; else pass "no homebrew installer"; fi
if grep -Eq '^[[:space:]]*tap[[:space:]]*=' dist-workspace.toml; then fail "tap key still present (would regen formula job)"; else pass "no tap key"; fi
if grep -Eq '^[[:space:]]*publish-jobs' dist-workspace.toml; then fail "publish-jobs still present"; else pass "no publish-jobs"; fi
if grep -q 'aarch64-apple-darwin' dist-workspace.toml && grep -q 'x86_64-apple-darwin' dist-workspace.toml; then pass "both mac arch targets kept (cask needs both zips)"; else fail "mac arch targets missing"; fi
if grep -q 'aarch64-unknown-linux-gnu' dist-workspace.toml && grep -q 'x86_64-unknown-linux-gnu' dist-workspace.toml; then pass "both linux targets kept (shell installer)"; else fail "linux targets missing"; fi

# 2. release.yml: no formula publisher.
section "release.yml"
if grep -Eq '^  publish-homebrew-formula:' .github/workflows/release.yml; then fail "publish-homebrew-formula job still present"; else pass "no publish-homebrew-formula job"; fi
if grep -Eq '^[[:space:]]*path:[[:space:]]*Formula/' .github/workflows/release.yml; then fail "Formula artifact download remains"; else pass "no Formula artifact handling"; fi
if grep -q 'sole writer' .github/workflows/release.yml; then pass "hand-maintained note present"; else fail "missing hand-maintained note"; fi

# 3. macos-cask.yml: coherent triggers, versioning, race-safe tap push.
section "macos-cask.yml"
CASK_WF=".github/workflows/macos-cask.yml"
if grep -q 'workflow_run:' "$CASK_WF" && grep -q 'workflows: \["Release"\]' "$CASK_WF"; then pass "workflow_run after Release (GITHUB_TOKEN releases do not fire release events)"; else fail "missing workflow_run trigger"; fi
if grep -q 'types: \[published\]' "$CASK_WF"; then pass "release published trigger kept (manual publishes)"; else fail "missing release published trigger"; fi
if grep -q 'workflow_dispatch' "$CASK_WF"; then pass "manual dispatch kept (backfills)"; else fail "missing workflow_dispatch"; fi
if grep -q 'concurrency:' "$CASK_WF" && grep -q 'homebrew-tap-publish' "$CASK_WF"; then pass "tap publish serialized (concurrency group)"; else fail "missing tap concurrency group"; fi
if grep -q 'Formula/crabmd.rb' "$CASK_WF" && grep -q 'git -C homebrew-tap rm -f Formula/crabmd.rb' "$CASK_WF"; then pass "legacy Formula/crabmd.rb retired safely (single file)"; else fail "missing safe formula removal"; fi
if grep -q 'git add -A Casks/crabmd.rb Formula/crabmd.rb' "$CASK_WF" || grep -q 'git add Casks/crabmd.rb' "$CASK_WF"; then pass "cask + deletion committed atomically"; else fail "tap commit not atomic"; fi
if grep -q 'for attempt in 1 2 3' "$CASK_WF" && grep -q 'git pull --rebase' "$CASK_WF"; then pass "push retry loop (race-safe)"; else fail "missing push retry"; fi
if grep -q 'Cargo.toml' "$CASK_WF" && grep -q 'Version mismatch' "$CASK_WF"; then pass "tag vs Cargo.toml version check"; else fail "missing version coherence check"; fi
if grep -q 'crabmd-aarch64-apple-darwin.tar.xz' "$CASK_WF" && grep -q 'crabmd-x86_64-apple-darwin.tar.xz' "$CASK_WF"; then pass "both mac arch archives required"; else fail "arch artifacts incomplete"; fi
if grep -q 'CrabMD-aarch64-apple-darwin.zip' "$CASK_WF" && grep -q 'CrabMD-x86_64-apple-darwin.zip' "$CASK_WF"; then pass "both arch zips uploaded"; else fail "arch zips incomplete"; fi
if grep -q 'Contents/Resources/origin' "$CASK_WF"; then pass "cask bundle must not contain legacy origin marker"; else fail "missing origin-marker sanity check"; fi
if grep -q 'ai.blankeos.crabmd' "$CASK_WF"; then pass "bundle identity sanity check in pack"; else fail "missing bundle identity check"; fi

# 4. write-cask.sh + generated cask.
section "write-cask.sh / generated cask"
if [[ -x scripts/write-cask.sh ]]; then pass "write-cask.sh executable"; else fail "write-cask.sh not executable"; fi
TMP_CASK="$(mktemp -d)/crabmd.rb"
bash scripts/write-cask.sh --version 9.9.9 \
  --sha-arm aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --sha-intel bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
  --out "$TMP_CASK" >/dev/null
if ruby -c "$TMP_CASK" >/dev/null 2>&1; then pass "generated cask ruby syntax OK"; else fail "ruby -c failed"; fi
# Required stanzas / artifacts.
for needle in 'arch arm:' 'version "9.9.9"' 'sha256 arm:' 'url "https://github.com/Blankeos/crabmd/releases/download/v#{version}/CrabMD-#{arch}-apple-darwin.zip' 'conflicts_with formula: "crabmd"' 'depends_on macos: :big_sur' 'app "CrabMD.app"' 'binary "#{appdir}/CrabMD.app/Contents/MacOS/crabmd", target: "crabmd"' 'postflight_steps do' 'uninstall quit: "ai.blankeos.crabmd"' 'zap trash:' 'caveats "The Homebrew formula and npm package are retired'; do
  if grep -Fq "$needle" "$TMP_CASK"; then pass "cask contains: $needle"; else fail "cask missing: $needle"; fi
done
# Quarantine strip for immediate Spotlight/Gatekeeper.
if grep -q '/usr/bin/xattr' "$TMP_CASK" && grep -q 'com.apple.quarantine' "$TMP_CASK" && grep -q '{{appdir}}/CrabMD.app' "$TMP_CASK"; then pass "quarantine strip present (immediate launch)"; else fail "quarantine strip missing"; fi
# Safe migration: guard + identity + path distinction, no blind delete.
if grep -q 'if_path_exists "Applications/CrabMD.app/Contents/Resources/origin", base: :home' "$TMP_CASK"; then pass "migration guarded by legacy origin marker"; else fail "missing origin guard"; fi
if grep -q 'CFBundleIdentifier' "$TMP_CASK" && grep -q 'ai.blankeos.crabmd' "$TMP_CASK"; then pass "migration checks bundle identity"; else fail "missing bundle identity check"; fi
if grep -q '\[ "$legacy" != "$managed" \]' "$TMP_CASK"; then pass "migration distinguishes legacy vs managed appdir"; else fail "missing path-distinction check"; fi
if grep -q '\-ef "$managed"' "$TMP_CASK"; then pass "migration handles symlink/canonical equivalence (-ef)"; else fail "missing canonical equivalence check (-ef)"; fi
if grep -q 'rm -rf "$legacy"' "$TMP_CASK"; then pass "guarded removal present"; else fail "missing guarded rm"; fi
if grep -q 'postflight do' "$TMP_CASK"; then fail "legacy postflight Ruby must not be used (brew style rejects)"; else pass "no legacy postflight Ruby (structured steps only)"; fi
if grep -Eq 'remove +"~/Applications/CrabMD.app"|remove +"Applications/CrabMD.app"' "$TMP_CASK"; then fail "unconditional remove of user app (blind delete)"; else pass "no blind remove of user app"; fi
if grep -q 'depends_on macos: ">= :big_sur"' "$TMP_CASK"; then fail "old depends_on string form (use symbol :big_sur)"; else pass "depends_on uses modern symbol"; fi
# Style check in proper tap layout (generic cops are tap-aware there).
TAP_DIR="$(mktemp -d)/tap"
mkdir -p "$TAP_DIR/Casks"
cp "$TMP_CASK" "$TAP_DIR/Casks/crabmd.rb"
if brew style "$TAP_DIR/Casks/crabmd.rb" >/dev/null 2>&1; then pass "brew style clean"; else fail "brew style offenses (run: brew style $TAP_DIR/Casks/crabmd.rb)"; fi
# Input validation.
if bash scripts/write-cask.sh --version bad --sha-arm aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa --sha-intel bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb --out /tmp/should-fail.rb >/dev/null 2>&1; then fail "write-cask accepts bad version"; else pass "write-cask rejects bad version"; fi
if bash scripts/write-cask.sh --version 1.2.3 --sha-arm short --sha-intel bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb --out /tmp/should-fail.rb >/dev/null 2>&1; then fail "write-cask accepts bad sha"; else pass "write-cask rejects bad sha"; fi

# 5. macos-app.sh bundle sanity (uses a stub binary; no cargo build needed).
section "macos-app.sh bundle"
STUB_BIN="$(mktemp -d)/crabmd-stub"
printf '#!/bin/sh\necho hi\n' > "$STUB_BIN"
chmod +x "$STUB_BIN"
APP_OUT="$(mktemp -d)/CrabMD.app"
bash scripts/macos-app.sh --bin "$STUB_BIN" --version 9.9.9 --out "$APP_OUT" >/dev/null
if [[ -f "$APP_OUT/Contents/Info.plist" ]]; then pass "bundle has Info.plist"; else fail "bundle missing Info.plist"; fi
if grep -q "ai.blankeos.crabmd" "$APP_OUT/Contents/Info.plist"; then pass "bundle identifier correct"; else fail "bundle identifier wrong"; fi
if grep -q "9.9.9" "$APP_OUT/Contents/Info.plist"; then pass "bundle version stamped"; else fail "bundle version missing"; fi
if [[ -f "$APP_OUT/Contents/MacOS/crabmd" && -x "$APP_OUT/Contents/MacOS/crabmd" ]]; then pass "bundle executable present"; else fail "bundle executable missing"; fi
if [[ -f "$APP_OUT/Contents/Resources/AppIcon.icns" ]]; then pass "bundle icon present"; else fail "bundle icon missing"; fi
if [[ ! -e "$APP_OUT/Contents/Resources/origin" ]]; then pass "cask bundle has no legacy origin marker"; else fail "cask bundle must not contain origin marker"; fi
if grep -q "<string>md</string>" "$APP_OUT/Contents/Info.plist"; then pass "markdown file associations present"; else fail "file associations missing"; fi

# 6. Migration shell logic: safe cases (temp HOME, no real ~/Applications touched).
section "migration logic (safe cases)"
run_migration() {
  # $1 = fake HOME, $2 = fake appdir parent (e.g. /Applications or fake HOME/Applications for custom appdir test)
  local fake_home="$1" managed_parent="$2"
  local legacy="$fake_home/Applications/CrabMD.app" managed="$managed_parent/CrabMD.app"
  [ "$legacy" != "$managed" ] || return 0
  if [ -e "$legacy" ] && [ -e "$managed" ] && [ "$legacy" -ef "$managed" ]; then return 0; fi
  [ -d "$legacy" ] || return 0
  [ -f "$legacy/Contents/Info.plist" ] || return 0
  local id
  id=$(/usr/bin/defaults read "$legacy/Contents/Info" CFBundleIdentifier 2>/dev/null || true)
  [ "$id" = "ai.blankeos.crabmd" ] || return 0
  [ -f "$legacy/Contents/Resources/origin" ] || return 0
  rm -rf "$legacy"
}
make_fake_bundle() {
  # $1 = bundle path, $2 = bundle id, $3 = with_origin (yes/no)
  local bundle="$1" bid="$2" origin="$3"
  mkdir -p "$bundle/Contents/Resources" "$bundle/Contents/MacOS"
  printf 'x' > "$bundle/Contents/MacOS/crabmd"
  cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>${bid}</string>
</dict></plist>
PLIST
  if [[ "$origin" == "yes" ]]; then printf '/old/bin' > "$bundle/Contents/Resources/origin"; fi
}
FAKE_HOME="$(mktemp -d)/home"
mkdir -p "$FAKE_HOME/Applications"
# Case A: legacy (correct id + origin) is removed.
make_fake_bundle "$FAKE_HOME/Applications/CrabMD.app" "ai.blankeos.crabmd" "yes"
run_migration "$FAKE_HOME" "/Applications"
if [[ ! -e "$FAKE_HOME/Applications/CrabMD.app" ]]; then pass "legacy (id+origin) removed"; else fail "legacy was not removed"; fi
# Case B: unrelated app with same filename (different id + origin file!) is kept.
make_fake_bundle "$FAKE_HOME/Applications/CrabMD.app" "com.evil.other" "yes"
run_migration "$FAKE_HOME" "/Applications"
if [[ -d "$FAKE_HOME/Applications/CrabMD.app" ]]; then pass "unrelated bundle id kept (no blind delete)"; else fail "unrelated app was deleted!"; fi
rm -rf "$FAKE_HOME/Applications/CrabMD.app"
# Case C: manually copied cask (correct id, NO origin) is kept.
make_fake_bundle "$FAKE_HOME/Applications/CrabMD.app" "ai.blankeos.crabmd" "no"
run_migration "$FAKE_HOME" "/Applications"
if [[ -d "$FAKE_HOME/Applications/CrabMD.app" ]]; then pass "manual copy without marker kept"; else fail "manual copy was deleted!"; fi
rm -rf "$FAKE_HOME/Applications/CrabMD.app"
# Case D: custom --appdir == ~/Applications (managed == legacy) is never touched.
make_fake_bundle "$FAKE_HOME/Applications/CrabMD.app" "ai.blankeos.crabmd" "yes"
run_migration "$FAKE_HOME" "$FAKE_HOME/Applications"
if [[ -d "$FAKE_HOME/Applications/CrabMD.app" ]]; then pass "managed appdir never deleted"; else fail "managed path was deleted!"; fi
rm -rf "$FAKE_HOME/Applications/CrabMD.app"
# Case E: symlinked appdir resolving to the same bundle is never touched.
# e.g. custom --appdir via a symlink that canonicalizes to ~/Applications.
SYMLINK_PARENT="$(mktemp -d)/linkparent"
mkdir -p "$SYMLINK_PARENT"
make_fake_bundle "$FAKE_HOME/Applications/CrabMD.app" "ai.blankeos.crabmd" "yes"
ln -s "$FAKE_HOME/Applications" "$SYMLINK_PARENT/AppsLink"
run_migration "$FAKE_HOME" "$SYMLINK_PARENT/AppsLink"
if [[ -d "$FAKE_HOME/Applications/CrabMD.app" ]]; then pass "symlinked managed path never deleted (-ef)"; else fail "symlinked managed path was deleted!"; fi
rm -rf "$FAKE_HOME/Applications/CrabMD.app" "$SYMLINK_PARENT"

# 7. publish-registries.yml: npm retired.
section "publish-registries.yml (npm retired)"
PUB=".github/workflows/publish-registries.yml"
if grep -Eq 'run:[[:space:]]*npm publish' "$PUB"; then fail "npm publish command still present"; else pass "no npm publish command"; fi
if grep -Eq '^[[:space:]]*push:' "$PUB"; then fail "still runs on push"; else pass "no push trigger"; fi
if grep -qi 'retired' "$PUB"; then pass "retirement documented in workflow"; else fail "missing retirement note"; fi

# 8. Docs: cask-only macOS, migration order, uninstall, no obsolete installs.
section "README.md"
if grep -q 'brew install --cask blankeos/tap/crabmd' README.md; then pass "cask install documented"; else fail "cask install missing"; fi
if grep -q 'Migrate to the cask' README.md; then pass "migration section present"; else fail "migration section missing"; fi
if grep -q 'crabmd --uninstall-desktop' README.md && grep -q 'BEFORE removing' README.md; then pass "desktop uninstall BEFORE binary removal documented"; else fail "migration order not explicit"; fi
if grep -q 'Settings in `~/.config/crabmd` are retained' README.md; then pass "settings retained documented"; else fail "settings retention missing"; fi
if grep -q 'brew uninstall --cask crabmd' README.md; then pass "cask uninstall documented"; else fail "cask uninstall missing"; fi
if grep -q 'no ghost' README.md || grep -q 'no longer recreates' README.md; then pass "no-ghost behavior documented"; else fail "no-ghost note missing"; fi
if grep -Eq '^[[:space:]]*npm install -g crabmd[[:space:]]*$' README.md; then fail "obsolete npm install line still listed as install method"; else pass "no obsolete npm install line"; fi
if grep -Eq '^[[:space:]]*bun install -g crabmd[[:space:]]*$' README.md; then fail "obsolete bun install line still listed"; else pass "no obsolete bun line"; fi
if grep -Eq 'brew install blankeos/tap/crabmd[[:space:]]*$' README.md && ! grep -q 'without `--cask`' README.md; then fail "bare formula install still listed"; else pass "no bare formula install (only mentioned as retired)"; fi
if grep -q '^## Install$' README.md && [[ "$(grep -c '^## Install$' README.md)" -eq 1 ]]; then pass "single Install header"; else fail "duplicate/missing Install header"; fi

section "npm/README.md"
if grep -qi 'retired' npm/README.md; then pass "npm retirement banner present"; else fail "npm banner missing"; fi
if grep -q 'brew install --cask blankeos/tap/crabmd' npm/README.md; then pass "npm README points to cask"; else fail "npm README missing cask pointer"; fi
if grep -q 'crabmd --uninstall-desktop' npm/README.md && grep -q 'BEFORE' npm/README.md; then pass "npm README documents safe order"; else fail "npm README missing safe order"; fi

# 9. Release plumbing: tag script + justfile.
section "tag_and_release.sh / justfile"
if grep -Eq 'sed.*npm/package\.json|git add.*npm/package\.json' scripts/tag_and_release.sh; then fail "tag script still bumps npm version"; else pass "tag script no longer touches npm"; fi
if grep -q 'sync_readme is retired' justfile; then pass "just sync_readme retired (won't clobber npm banner)"; else fail "just sync_readme still copies README"; fi

# 10. Workflows parse.
section "workflow syntax"
if command -v actionlint >/dev/null 2>&1; then
  if actionlint .github/workflows/macos-cask.yml .github/workflows/release.yml .github/workflows/publish-registries.yml 2>&1 | grep -E 'error|failed' >/dev/null; then
    fail "actionlint errors"
    actionlint .github/workflows/macos-cask.yml .github/workflows/release.yml .github/workflows/publish-registries.yml 2>&1 | head -20
  else
    pass "actionlint clean (no errors)"
  fi
else
  echo "  skip: actionlint not installed"
fi

echo
echo "packaging tests: $PASS passed, $FAIL failed"
if [[ "$FAIL" -ne 0 ]]; then exit 1; fi
