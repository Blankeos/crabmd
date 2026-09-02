default:
    just --list

dev *args:
    cargo r -- {{ if args == "" { "examples/kitchen-sink.md" } else { args } }}

check:
    cargo c

test:
    cargo t

themes:
    cargo r -- --list-themes

dpreview *args:
    ./target/debug/crabmd {{args}}

preview *args:
    ./target/release/crabmd {{args}}

sync_readme:
    cp README.md npm/README.md

[doc('Release: bump versions, commit, and tag from main (just tag [patch|minor|major])')]
tag bump="":
    sh scripts/tag_and_release.sh {{ bump }}
