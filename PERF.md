# Performance notes

Startup + idle-CPU for crabmd (GPUI GUI). Refresh with:

```bash
just bench-perf --section version,idle --write-perf
# or, foreground window for idle sampling:
just bench-perf --section idle --settle 5 --sample 10
```

## Latest

_No runs recorded yet. Build a release binary (`cargo b -r`) and run `just bench-perf --write-perf`._

## Notes

- Default CLI **detaches** so the shell returns immediately; use `-w`/`--wait` for benches and `just dev`.
- Section B (PTY first-frame) is best-effort for GUI apps — prefer A (`--help` via hyperfine) + C (idle CPU/RSS).
- Peers to compare casually: Typora (best WYSIWYG), Obsidian (vaults), iA Writer (Mac prose), Bear, Craft.

## History

_(appended by `scripts/bench-perf.py --write-perf`)_
