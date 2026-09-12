default:
    just --list

dev *args:
    cargo r -- -w {{ if args == "" { "examples/kitchen-sink.md" } else { args } }}

check:
    cargo c

test:
    cargo t

themes:
    cargo r -- --list-themes

gen-themes:
    bun run scripts/gen-themes.ts

dpreview *args:
    ./target/debug/crabmd -w {{args}}

preview *args:
    ./target/release/crabmd -w {{args}}

[doc('Wrap the release binary in target/release/CrabMD.app')]
macos-app:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "$(uname -s)" != Darwin ]]; then
        echo "macos-app is macOS-only" >&2
        exit 1
    fi
    cargo build --release
    version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
    ./scripts/macos-app.sh \
        --bin target/release/crabmd \
        --version "$version" \
        --out target/release/CrabMD.app
    echo "→ target/release/CrabMD.app"

# npm is retired: npm/README.md is a standalone deprecation notice.
# Do NOT overwrite it from the main README (would erase the banner).
# Kept as a no-op with a warning so old muscle memory fails loudly.
sync_readme:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "sync_readme is retired: npm/README.md is a standalone deprecation notice, not a copy of README.md" >&2
    exit 1

[doc('Release: bump versions, commit, and tag from main (just tag [patch|minor|major])')]
tag bump="":
    sh scripts/tag_and_release.sh {{ bump }}

[doc('Startup + idle CPU bench (release binary). Pass --write-perf to append PERF.md')]
bench-perf *args:
    python3 scripts/bench-perf.py {{args}}
