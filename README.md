# crabmd

A fast native markdown **writer** for one person. One window, one file, write
back to disk. You type in the rendered document, not in a source buffer.

```sh
just dev
just dev path.md
```

If `path.md` does not exist, crabmd creates an empty markdown file and opens it.

## Install

```sh
brew install blankeos/tap/crabmd
npm install -g crabmd
bun install -g crabmd
cargo install --git https://github.com/Blankeos/crabmd --locked
curl --proto "=https" --tlsv1.2 -LsSf https://github.com/Blankeos/crabmd/releases/latest/download/crabmd-installer.sh | sh
```

## What it is

- A GPUI app (Rust, no browser shell)
- Block document: you type in the rendered page (WYSIWYG). GFM is only on disk.
- Helix / Vim: modal keys walk the **visible** text. Normal uses a block caret; Insert uses a thin caret
- Notion: always-on insert. `# ` / `- ` / `**bold**` convert; syntax stays hidden
- Slash menu: type `/` at the start of a block to insert GFM
- Bubble toolbar on selection: bold / italic / strike / code / link
- Tables are a cell grid (Tab between cells, slash inserts 3×3)
- GitHub Flavored Markdown, including GitHub alerts (`> [!NOTE]` and friends)
- OpenCode-compatible themes as JSON (default: `opencode`)
- Drop or paste images next to the markdown file

## What it is not

- Electron, Tauri, or any web preview pretending to be an app
- A code editor or source-first markdown editor
- An IDE: no LSP, no file tree, no terminal, no plugins, no notebook/workspace

## Run

```sh
just              # list recipes
just dev          # kitchen-sink
just dev notes.md
just dev --theme catppuccin-light notes.md
just check
just test
just themes

# already-built binaries
just dpreview notes.md
just preview notes.md     # release, faster startup
```

There is **no autosave**. The file stays dirty until an explicit write:

- `:w` / `:write` in Helix or Vim Normal (footer command-line)
- `:wq` writes and quits
- `⌘S` / `Ctrl+S`, or the Save button in the status bar

Opening a missing path still creates the empty file (that is open, not save).
Status bar: `unsaved` until write, then `saved` (⌘S) or `written` (`:w`).

## Editors

Settings → Editor, or `editor =` in `~/.config/crabmd/config.toml`
(`keymap =` is still read for older files). Status bar: `NOR` / `INS` /
`SEL` (Helix) / `VIS` / `V-LINE` (Vim) / `NOTION`.

Helix/Vim still treat the file as one buffer, but motions, visual, search,
join, and replace run on the **visible** projection (no `# ` / `**` under the
caret). GFM is rewritten on disk. Normal uses a **block caret**; Insert and
Notion use a thin caret. Insertable keys in Normal are swallowed unless bound.
There is no raw textarea — focused and unfocused blocks share the same paint.

`h`/`l` stay on the current **logical line** (no whichwrap). `w` / `b` / `e` /
`W` / `B` / `E` / `j` / `k` / `gg` / `G` walk the file. `j`/`k` count wrapped
visual lines when **Wrap lines** is on (default). Counts work: `5h`, `2k`,
`3w`, `10j`. `0` is line-start unless a count is already pending (`10`).

### Shared (Helix + Vim normal)

| Key | Action |
| --- | --- |
| `h` / `l` | character left / right (logical line of the full buffer, no whichwrap) |
| `j` / `k` | line down / up on the full buffer |
| `w` / `b` / `e` | word forward / back / end on the full buffer |
| `0` / `^` / `$` | line start / first non-blank / line end |
| `gg` | start of document. Count `Ngg` → line N |
| `f` / `F` / `t` / `T` | find / till char. Helix: rest of buffer. Vim: current line. `;` / `,` repeat |
| `/` / `?` | search prompt (footer). Enter jumps in the full buffer. Empty Enter repeats last. `n` / `N` wrap |
| `r` | replace next key (count `3rx`; Helix selection replaces every non-newline) |
| `J` | join with next line (blank separators eaten so a heading pulls the next paragraph) |
| `[p` / `]p` | previous / next paragraph (blank-line region / GFM block). `[h` / `]h` headings |
| `i` / `a` | insert at caret / after caret |
| `I` / `A` | insert at first non-blank of the line / end of the line |
| `o` / `O` | open a line below / above **inside this block**. New empty block only for rule / table / html / raw |
| `u` | undo (document-level) |
| `escape` | normal; in visual, collapse the selection to the caret |
| `:` | command-line (`w` / `write` / `wq`). Escape or empty Enter cancels |

### Helix

Status `NOR` / `SEL` / `INS`.

| Key | Action |
| --- | --- |
| `x` | select the current line (not delete). Repeat `x` extends to the next line |
| `d` | delete the selection; if empty, delete the character under the caret. Stay in normal. `dd` is not delete-block |
| `v` | select mode: further motions extend the selection |
| `U` | redo |
| `ge` | end of document (`goto_last_line`, **not** vim `ge`) |
| `G` | count → that line; no count → last line |
| `gh` / `gl` / `gs` | line start / end / first non-blank |

Delete-block is gone from `x`. `d` on a selection that covers the whole block source removes the block and keeps a trailing empty paragraph.

### Vim

Status `NOR` / `VIS` / `V-LINE` / `INS`.

| Key | Action |
| --- | --- |
| `x` | delete the character under the caret |
| `v` / `V` | visual character / visual line; motions extend |
| `d` | delete the visual selection; `dd` deletes the current **logical line** (not the whole block unless it is that one line) |
| `D` | delete to end of line |
| `ctrl-r` | redo |
| `G` | last line of the document; `5G` → line 5 |

### Notion

Status `NOTION`. No Helix/Vim normal mode — everything is insert. Blocks stay
rendered (no `# ` prefix, no fences). The focused block is a content editor:

- Paragraph: visible text
- Heading: large text; commit writes `#` / `##` / …
- List / task: item text; checks stay clickable when unfocused
- Alert: colored box, body only
- Code: body without fences; language is kept from the fence
- Quote: inner text, commit with `>`
- Table / HTML / raw: small raw exception (source editor)

`j` / `k` move between blocks. Slash menu still works at the start of an empty
block. Inline markers (`**bold**`) in a focused paragraph are still typed as
source — true inline WYSIWYG is not this pass.

Click a block to select it and place the caret (end of that block, or the
native textarea position if you click the focused raw source). Click does
**not** force Insert; `i` / `a` / `I` / `A` (or double-click intent) enter
insert. Click in normal stays normal with the caret there.

Backspace in Insert (and Notion) joins like one nvim/hx buffer. At document
column 0, if the previous characters are a block separator (two newlines), both
are deleted so `|## Quota` after `hello` becomes `hello## Quota`. Mid-line
backspace is a normal char delete.

## Helix (the editor)

Open the current buffer in crabmd:

```toml
[keys.normal]
space.m = ":sh crabmd %{file_path_absolute}"
```

`file_path_absolute` is the current document’s absolute path. On older Helix
builds that lack it, use `%{buffer_name}` (relative to Helix’s working directory):

```toml
[keys.normal]
space.m = ":sh crabmd %{buffer_name}"
```

## Slash inserts

Type `/` at the start of a block, then pick:

| Command | GFM |
| --- | --- |
| Heading 1–3 | `#` / `##` / `###` |
| Bullet / numbered / task list | `-` / `1.` / `- [ ]` |
| Code | fenced block |
| Table | GFM table |
| Quote | `>` |
| Divider | `---` |
| Alert | `> [!NOTE]` (also TIP, IMPORTANT, WARNING, CAUTION) |

The menu is keyboard-navigable: `↑`/`↓`, `ctrl-n`/`ctrl-p`, `ctrl-j`/`ctrl-k`
(bound on the textarea so the caret does not also move). Enter applies the
highlighted row; Escape closes the menu (empty block) and stays in insert.
Click still works. Rows have lucide icons from `assets/icons/`.

## Images

Drop an image file onto the window, or paste a bitmap from the clipboard.
crabmd writes it next to the current `.md` file:

- drop keeps the original filename when it does not collide (`shot.png`, then
  `shot-2.png`)
- paste uses `crabmd-image-YYYYMMDD-HHMMSS.png` (or the clipboard format)

and inserts `![name](filename)`. An unfocused image-only block renders the
bitmap when the file exists beside the markdown.

## Settings

The gear in the status bar opens a compact settings panel (sidebar + detail).
Escape or **Done** closes it; clicking the gear again toggles.

- **Editor:** Helix / Vim / Notion
- **Theme:** the OpenCode JSON set below; picking one applies immediately
- **Font:** monospace family for the raw editor (Menlo default, also Monaco,
  JetBrainsMono Nerd Font, Zed Mono, Courier New) and size 13–20 (default 15)
- **Wrap lines:** `j`/`k` count wrapped visual lines when on (default). Off:
  logical lines. The raw editor and rendered preview wrap with this setting.

Saved to `~/.config/crabmd/config.toml` (`editor`, `theme`, `wrap_motions`,
`font_family`, `font_size`). `--theme` on the CLI still overrides theme for
that session.

Keymap coverage vs Helix/Vim (scoped to navigation + search + basic change)
is in `docs/keymap-parity.md`. Refresh later with the crabcode command
`.crabcode/commands/check-keymap-parity.md`.

## App icon

`assets/app-icon.png` is the source of truth. `just dev` (`cargo r`) also
sets the macOS Dock icon at runtime. A bundled `.app` uses `assets/AppIcon.icns`.

The window uses a Zed-style custom titlebar (transparent native chrome, real
macOS traffic lights, filename + dirty dot). Drag the bar to move; double-click
zooms.

## Themes

JSON files in `themes/`, schema `https://opencode.ai/theme.json`. Shipped set
(small, not the whole catalog):

- `opencode` (default, dark) / `opencode-light`
- `catppuccin` / `catppuccin-light`
- `tokyonight` / `tokyonight-light`
- `github` / `github-light`

Switch from the status bar, or pass `--theme <name>`. Colors map
`markdownText`, `markdownHeading`, `markdownLink`, `markdownCode`,
`markdownBlockQuote`, and the rest of the OpenCode markdown keys onto the
writing surface (plus alert colors from `info` / `success` / `accent` /
`warning` / `error`).

## GFM

Parser: [pulldown-cmark](https://docs.rs/pulldown-cmark) with tables,
strikethrough, task lists, and `ENABLE_GFM` (GitHub alerts). Unfocused blocks
keep their original source so save stays valid GFM.

**Supported**

- Headings, paragraphs, emphasis, strong, strikethrough, inline code, links
- Bullet lists, ordered lists, task lists
- Fenced code, block quotes, GFM tables, thematic breaks
- GitHub alerts: NOTE, TIP, IMPORTANT, WARNING, CAUTION
- CommonMark autolinks (`<https://…>`); bare-URL autolinks if the parser emits them
- Round-trip of unknown leftovers (link reference definitions, raw HTML) as raw blocks
- Unfocused fenced code is highlighted by gpui-component’s TextView highlighter
  when the fence has a language tag (` ```rust `) and that language is enabled
  (Rust, JavaScript, Python, JSON in this build)

**Deferred**

- Inline WYSIWYG (bold as you type without seeing `**`)
- Nested-list indent editing as a tree (nested lists save as one list block)
- Footnotes, definition lists, math, wikilinks (not GFM)
- Publishing, sync, AI
- Click-at-exact-offset on an *unfocused* rendered block (click selects the
  block and puts the caret at the end; click in the focused editor uses the
  native caret)
- Macros, `.` repeat, text objects, yank/paste, windows, registers

A kitchen-sink file lives at `examples/kitchen-sink.md`.

## Stack

Rust, [GPUI](https://gpui.rs), [gpui-component](https://github.com/longbridge/gpui-component).
Linux and macOS first.

## License

MIT
