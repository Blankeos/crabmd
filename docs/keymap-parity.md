# Keymap parity

Helix **86.7%** (38 yes + 2 partial / 45), Vim **92.9%** (39 yes / 42) of **scoped** keys.

Upstream: [Helix keymap](https://docs.helix-editor.com/keymap.html), [Vim cheat sheet](https://vim.rtorr.com). crabmd: `src/editor.rs` `bind_keys`, `src/motion.rs`, `src/mode.rs`. Refresh with `/check-keymap-parity`.

## Scope

Navigation, search, and basic change a markdown writer needs. One raw document (`source`); unfocused ranges stay rendered GFM.

## Out of scope (N/A, not scored)

LSP, windows, buffers, pickers, tree-sitter textobjects, macros, registers, diffs, tabs.

Also not scored (helix-only chrome): view mode `z`/`Z`, space mode, match mode `m`, jumplist, completion popups, shell pipes, insert-mode emacs chords.

## Helix

| Key | Upstream meaning | crabmd | status |
| --- | --- | --- | --- |
| `h` | Move left | char left; stays on the logical line of the full buffer | yes |
| `j` | Move down (visual line) | line down on the full buffer; wrap-lines setting | yes |
| `k` | Move up (visual line) | line up on the full buffer | yes |
| `l` | Move right | char right; stays on the logical line | yes |
| `w` | Next word start | word forward on the full buffer | yes |
| `b` | Previous word start | word back on the full buffer | yes |
| `e` | Next word end | word end on the full buffer | yes |
| `W` | Next WORD start | whitespace WORD forward | yes |
| `B` | Previous WORD start | whitespace WORD back | yes |
| `E` | Next WORD end | whitespace WORD end | yes |
| `t` | Find till next char (not line-confined) | pending-t; rest of buffer | yes |
| `f` | Find next char (not line-confined) | pending-f; rest of buffer | yes |
| `T` | Find till previous char | pending-T; rest of buffer | yes |
| `F` | Find previous char | pending-F; rest of buffer | yes |
| `G` | Go to line number, else a line prompt (`goto_line`) | count → that line; no count → last line | partial |
| `gg` | Line N else start of file (`goto_file_start`) | start of document; `Ngg` → line N | yes |
| `ge` | End of file (`goto_last_line`) | end of document (not vim `ge`) | yes |
| `gh` | Start of line | line start | yes |
| `gl` | End of line | line end | yes |
| `gs` | First non-whitespace of the line | first non-blank | yes |
| `/` | Search for regex pattern | `cmd-f` / `ctrl-f` opens search; `/` inserts a slash (slash commands) | partial |
| `?` | Search backward | footer `?` prompt | yes |
| `n` | Next search match | wrap | yes |
| `N` | Previous search match | wrap | yes |
| `*` | Current selection as search pattern (word bounds) | — | no |
| `v` | Select / extend mode | select mode; motions extend a document range | yes |
| `x` | Select current line; repeat extends | select line | yes |
| `X` | Extend selection to line bounds | — | no |
| `J` | Join lines inside selection | join current/selected lines (vim-like `J`; empty seps eaten) | partial |
| `r` | Replace with a character | pending-r; selection replaces non-newlines | yes |
| `i` | Insert before selection | insert at caret | yes |
| `a` | Insert after selection | insert after caret | yes |
| `I` | Insert at start of line | first non-blank | yes |
| `A` | Insert at end of line | line end | yes |
| `o` | Open line below | open below inside the block | yes |
| `O` | Open line above | open above inside the block | yes |
| `u` | Undo | document undo | yes |
| `U` | Redo | redo | yes |
| `d` | Delete selection | delete selection or char under caret | yes |
| `c` | Change selection (delete + insert) | — | no |
| `y` | Yank selection | — | no |
| `p` | Paste after selection | — | no |
| `]p` | Next paragraph | next blank-line / GFM block | yes |
| `[p` | Previous paragraph | previous blank-line / GFM block | yes |
| `:w` | Write file | `:w` / `:write` | yes |
| `:q` | Close buffer | `:q` closes the tab (`:q!` discards); `:wq` / `:x` saves and closes | yes |

Helix `;` is collapse selection and `,` keeps the primary selection — not scored. crabmd binds `;` / `,` to vim-style repeat-find in both keymaps. Extra, not scored: `]h` / `[h` next/prev ATX heading.

## Vim

Cursor list is the vim.rtorr movement a markdown writer needs (not `H`/`M`/`L` screen, `gd`, page/`zz`, or long-WORD `gE`).

| Key | Upstream meaning | crabmd | status |
| --- | --- | --- | --- |
| `h` | Cursor left | char left; no whichwrap | yes |
| `j` | Cursor down | line down on the full buffer | yes |
| `k` | Cursor up | line up on the full buffer | yes |
| `l` | Cursor right | char right; no whichwrap | yes |
| `w` | Next word start | word forward | yes |
| `b` | Previous word start | word back | yes |
| `e` | Next word end | word end | yes |
| `ge` | Previous word end | Helix `ge` (EOF) is not bound on the Vim keymap | no |
| `W` | Next WORD start | whitespace WORD forward | yes |
| `B` | Previous WORD start | whitespace WORD back | yes |
| `E` | Next WORD end | whitespace WORD end | yes |
| `gg` | First line of the document | start of document; `Ngg` → line N | yes |
| `G` | Last line; `5G` → line 5 | last line; count → that line | yes |
| `0` | Start of line | line start | yes |
| `^` | First non-blank of line | first non-blank | yes |
| `$` | End of line | line end | yes |
| `f` | Next char **on this line** | pending-f; current logical line only | yes |
| `t` | Before next char on this line | pending-t; current line | yes |
| `F` | Previous char on this line | pending-F | yes |
| `T` | After previous char on this line | pending-T | yes |
| `;` | Repeat f/t/F/T | same direction | yes |
| `,` | Repeat f/t/F/T opposite | opposite direction | yes |
| `/` | Search forward | footer `/` prompt; full buffer | yes |
| `?` | Search backward | footer `?` prompt | yes |
| `n` | Repeat search same direction | wrap | yes |
| `N` | Repeat search opposite | wrap | yes |
| `{` | Previous paragraph | use `[p` (not `{`) | no |
| `}` | Next paragraph | use `]p` (not `}`) | no |
| `v` | Visual character | visual char; document range | yes |
| `V` | Visual line | visual line | yes |
| `r` | Replace a single character | pending-r | yes |
| `J` | Join line below with a space | join; empty separator lines eaten | yes |
| `x` | Delete character | delete char under caret | yes |
| `d` | Delete (visual / `dd`) | visual delete; `dd` deletes the logical line | yes |
| `dd` | Delete current line | logical line (not the whole block unless it is one line) | yes |
| `i` | Insert before cursor | insert at caret | yes |
| `a` | Insert after cursor | insert after caret | yes |
| `I` | Insert at beginning of line | first non-blank | yes |
| `A` | Insert at end of line | line end | yes |
| `o` | Open line below | open below | yes |
| `O` | Open line above | open above | yes |
| `:w` | Write file | `:w` / `:write` | yes |
| `:q` | Close buffer | `:q` closes the tab (`:q!` discards); `:wq` / `:x` saves and closes | yes |

`[p` / `]p` work on the Vim keymap too (not scored here; `{` / `}` are the cheat-sheet keys and stay `no`).
