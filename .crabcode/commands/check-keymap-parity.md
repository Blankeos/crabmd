---
description: Refresh Helix/Vim keymap parity report for crabmd
---

Compare crabmd's Helix and Vim bindings against upstream. Do not invent keys.

1. Fetch and read https://docs.helix-editor.com/keymap.html and https://vim.rtorr.com. Use those pages as the only source of upstream meaning.
2. Read crabmd bindings from source: `src/mode.rs`, `src/motion.rs`, `src/editor.rs` (`bind_keys` and handlers), and `README.md`.
3. Write or update the living report at `docs/keymap-parity.md` (create `docs/` if needed).

Report shape — concise, straight to the point:

- One-line summary: Helix X% (a/b), Vim Y% (c/d) of **scoped** keys (see scope).
- Two tables (Helix, then Vim), columns: Key | Upstream meaning | crabmd | status (`yes` / `partial` / `no`).
- Keep **upstream order** (easier to diff). Do not sort by status.

Scope: **navigation + search + basic change** that a markdown writer should have.

Do NOT score LSP, windows, buffers, pickers, tree-sitter textobjects, macros, registers, diffs, or tabs. List those once under a short **Out of scope** heading as N/A so the percent is not tanked by features crabmd will never implement.

In-scope Helix: movement (`h j k l w b e W B E f t F T G gg ge gh gl gs`), search (`/ ? n N *`), select (`v x X J`), changes (`r i a I A o O u U d c y p`), unimpaired (`]p [p`), command (`:w`).

In-scope Vim: cursor movement from the vim.rtorr cheat sheet that a markdown writer needs (`h j k l w b e ge gg G 0 ^ $ f t F T ; , n N / ? { }` and the obvious neighbors on that list), visual (`v V`), editing (`r J x d dd`), insert (`i a I A o O`), `:w`.

Percent = `(yes + 0.5 * partial) / in-scope count`, one decimal place. Count only in-scope keys.

Notes that must not be invented if the docs say otherwise:

- Helix `f`/`F`/`t`/`T` are **not** confined to the current line; Vim's are.
- Helix `gg` = `goto_file_start` (start of file); count `Ngg` = line N.
- Helix `ge` = `goto_last_line` (end of file), **not** vim `ge` (backward word-end).
- Helix `G` = `goto_line` (line number, else a line prompt).
- Vim `gg` = first line, `G` = last line, `5gg`/`5G` = line 5.
