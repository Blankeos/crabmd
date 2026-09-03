# Changelog

All notable changes to this project will be documented in this file.

## [0.0.2] - 2026-09-03

### Bug Fixes

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


