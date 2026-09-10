# Changelog

All notable changes to this project will be documented in this file.

## [0.0.3] - 2026-09-10

### Bug Fixes

- Gate cask publish via env so workflow parses by Blankeos
- Preserve marks when typing inside formatted runs and strip bullets on backspace in indented lists by Blankeos
- Unique ids for bubble mark buttons so I/U/S work by Blankeos
- Fix linewise run detection and visual charwise caret rendering by Blankeos
- Route linewise selections to unit-level delete by Blankeos
- Vim/helix mode-scoped keys, word objects at EOL, indent ops on display buffer, tab bar footer fix by Blankeos

### Features

- Rich details summaries, hidden HTML comments, live emphasis closers by Blankeos
- Parse <strong>/<b>/<em>/<i> inline HTML, keep <leader> visible; add Copy Absolute Path to palette by Blankeos
- Ship a bundled macOS app with Homebrew cask and desktop registration by Blankeos
- Better edit link a11y by Blankeos
- Notion-style link following with hover cards, frontmatter property header, jump list, and toasts by Blankeos
- Add :bn/:bp ex commands and link toolbar improvements by Blankeos
- Add :q/:q!/:qa and :wqa ex commands, vim-style search commit by Blankeos
- Implement Helix linewise selection (`x`/`X`) with sticky line-select semantics by Blankeos
- Add code-block conveniences for markdown editing by Blankeos
- Add vim `za` to toggle `<details>` and fix backspace around disclosures by Blankeos
- Add keyboard navigation for grip menu and improve details handling by Blankeos
- Segmented inline code layout, source view toggle, palette shortcuts by Blankeos
- Bundle IBM Plex Sans and JetBrains Mono fonts by Blankeos
- Add in-flow side bearings for inline code pills by Blankeos
- Animate details chevron rotation on collapse toggle by Blankeos
- Add horizontal code-block scrolling, caret reveal, and improved inline code pills by Blankeos
- Add single-instance daemon with fast CLI forwarding, code-block copy button, and quote/alert paragraph breaks by Blankeos
- Preserve code fence indent and honor GFM `<details open>` semantics by Blankeos
- Render semantic HTML blocks and nested list code fences by Blankeos

## [0.0.2] - 2026-09-03

### Bug Fixes

- Install xkbcommon/xcb system libs for Linux builds, native ARM runner by Blankeos
- Add profile.dist so release builds work by Blankeos
- Re-enable dist for publish=false crate by Blankeos
- Let checkout persist credentials so origin fetch works by Blankeos
- Auth origin fetch in publish-registries prepare by Blankeos
- Pin tinyvec + gpui-component rev so cargo install --path . works by Blankeos
- Restore known-good lockfile, fix package include paths by Blankeos
- Stable remote timeline with buffered playback by Blankeos
- Preserve nested list items and multi-line table cells; add dirty tracking with quit promptfix: preserve nested lists and multi-line table cells; add dirty-state tracking with quit confirmation by Blankeos
- Activate app and window on launch by Blankeos
- Preserve live tree in undo snapshots and fix list indentation GFM round-trip by Blankeos
- Checkbox pos by Blankeos
- Drag handle position for headings by Blankeos
- Layout shifts when empty blocks by Blankeos

### Chores

- Dont publish to cratesio by Blankeos
- Release for 0.0.1 first publish by Blankeos
- First ver for release to claim by Blankeos
- Ready for releasing by Blankeos

### Features

- Stream remote clips progressively with live buffering UI by Blankeos
- Drag-scrubbing, icon play/pause, space-to-toggle, caret-focus borders by Blankeos
- Add inline video playback via yscv-video by Blankeos
- Video preview tiles, slash menu additions, and media toolbar slash fix by Blankeos
- Load remote http(s) images via GPUI SharedUri by wiring ReqwestClient as the GPUI HTTP client (fixes remote photos never rendering with the default null client) by Blankeos
- Editor UX upgrades, media/mermaid/crosslink rendering, theme generator by Blankeos
- Manual cmd-k t chord handling, per-field font resets, and settings polish by Blankeos
- Themes picker, media toolbar, `cmd-k` chords, and editor polish by Blankeos
- Command palette, detached CLI, and editor polish by Blankeos
- Auto-link bare URLs on space, enter, and paste by Blankeos
- Add copy/paste actions with rich GFM clipboard support by Blankeos
- Add link interaction, selection tab handling, and quit action by Blankeos
- Add select-all, word/line backspace refinements, and tree-native block drag-drop by Blankeos
- Sibling-aware ordered list markers and multi-click drag selection by Blankeos
- Rework editing on tree-based document model with block drag & drop by Blankeos
- Improve quote/alert editing, slash command application, and mark toggling by Blankeos
- Refine Notion-style wysiwyg editing behaviors by Blankeos
- Per-slot fonts, syntax highlighting, and empty-paragraph editing by Blankeos
- Good progress so far, but very bad. we'll polish overtime by Blankeos
- Progress so far. by Blankeos

### Refactor

- Store caret and selection in display coordinates by Blankeos


