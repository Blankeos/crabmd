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

dpreview *args:
    ./target/debug/crabmd -w {{args}}

preview *args:
    ./target/release/crabmd -w {{args}}

sync_readme:
    cp README.md npm/README.md

[doc('Release: bump versions, commit, and tag from main (just tag [patch|minor|major])')]
tag bump="":
    sh scripts/tag_and_release.sh {{ bump }}

[doc('Startup + idle CPU bench (release binary). Pass --write-perf to append PERF.md')]
bench-perf *args:
    python3 scripts/bench-perf.py {{args}}
