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
