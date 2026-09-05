//! One markdown buffer. GFM `source` is truth; paint and the caret walk a
//! visible projection. No textarea — WYSIWYG overlay on every block.

use std::cell::RefCell;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    actions, canvas, deferred, div, img, point, prelude::FluentBuilder as _, px, relative, rgb,
    svg,
    AnyElement, Animation, AnimationExt as _, ElementId,
    App, AppContext as _, Bounds, ClickEvent, ClipboardEntry, ClipboardItem, Context, CursorStyle,
    DragMoveEvent, Empty, Entity, EntityInputHandler, ExternalPaths, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, PromptLevel, Render, ScrollHandle, SharedString,
    SharedUri, StatefulInteractiveElement as _, Styled, UTF16Selection, Window,
    Transformation,
    ease_in_out, radians,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    scroll::ScrollableMask,
    switch::Switch,
    v_flex, Icon, Sizable as _, Theme, ThemeMode, ThemeTokens,
};

use crate::config::{self, Config, EditorKind};
use crate::display::{
    floor_char_boundary, list_sibling_index, ordered_marker, project, Affinity, BlockExtra,
    Projection, CODE_LANGS, COLUMN_PX,
};
use crate::document::{
    alert_icon_name, extract_links, parse_ranges, take_bare_url, BlockKind, PaintRange,
};
use crate::images;
use crate::mermaid::{self, MermaidStore};
use crate::mode::{self, Caret, ExCommand, Mode};
use crate::motion::{
    after_caret_same_line, apply_motion, block_caret_range, extend_visual_line, find_char,
    first_non_blank_in, last_line_start, line_start_n, logical_line_range, paragraph_jump,
    push_count, search_next, search_prev, take_count, visual_line_range, whichwrap, word_range_at,
    FindKind, Motion,
};
use crate::palette::{LineField, PaletteAction, PaletteMode, PaletteState};
use crate::slash::{self, SlashItem};
use crate::surface::{self, Hit};
use crate::syntax;
use crate::theme::{self, Appearance, Palette};
use crate::undo::{Snapshot, UndoStack};
use crate::wysiwyg::{self, Mark};

actions!(
    crabmd,
    [
        Save,
        LeaveInsert,
        InsertAtCaret,
        InsertAfterCaret,
        InsertLineStart,
        InsertLineEnd,
        OpenBelow,
        OpenAbove,
        CharLeft,
        CharRight,
        LineDown,
        LineUp,
        WordForward,
        WordBack,
        WordEnd,
        WordForwardWs,
        WordBackWs,
        WordEndWs,
        LineStart,
        LineFirstNonBlank,
        LineEnd,
        FirstDoc,
        LastDoc,
        PendingG,
        Digit0,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        SelectLine,
        DeleteOp,
        DeleteChar,
        DeleteToEnd,
        VisualChar,
        VisualLine,
        Undo,
        Redo,
        SlashPrev,
        SlashNext,
        SlashApply,
        BlockBackspace,
        ToggleSettings,
        OpenPalette,
        OpenCommand,
        OpenSearch,
        SearchBack,
        SearchNext,
        SearchPrev,
        JoinLines,
        ReplaceChar,
        FindForward,
        FindBackward,
        FindTill,
        FindTillBack,
        RepeatFind,
        ReverseFind,
        BracketOpen,
        BracketClose,
        InsertNewline,
        InsertHardBreak,
        IndentTab,
        OutdentTab,
        ToggleBold,
        ToggleItalic,
        ToggleStrike,
        ToggleCode,
        ToggleUnderline,
        ToggleLink,
        InsertSlash,
        DeleteWordBack,
        DeleteLineBack,
        CutSelection,
        CopySelection,
        PasteClipboard,
        SelectAll,
        ToggleSource,
        IndentShift,
        DedentShift,
        AutoIndent,
        ChangeOp,
        MatchObject,
        QuitApp,
    ]
);

/// `>` / `<` / `=` operator kind for `indent_key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IndentOpKind {
    Indent,
    Dedent,
    Auto,
}

/// Text-object operator for `pending_obj`: Vim `v`/`d`/`c` + `i`/`a`, or
/// Helix `m` + `i`/`a`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObjOp {
    Select,
    Delete,
    Change,
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", Save, Some("Workspace")),
        KeyBinding::new("ctrl-s", Save, Some("Workspace")),
        KeyBinding::new("cmd-,", ToggleSettings, Some("Workspace")),
        KeyBinding::new("ctrl-,", ToggleSettings, Some("Workspace")),
        KeyBinding::new("escape", LeaveInsert, Some("Workspace")),
        KeyBinding::new("escape", LeaveInsert, Some("Input")),
        KeyBinding::new("escape", LeaveInsert, Some("Command")),
        KeyBinding::new("i", InsertAtCaret, Some("Normal")),
        KeyBinding::new("a", InsertAfterCaret, Some("Normal")),
        KeyBinding::new("shift-i", InsertLineStart, Some("Normal")),
        KeyBinding::new("shift-a", InsertLineEnd, Some("Normal")),
        KeyBinding::new("o", OpenBelow, Some("Normal")),
        KeyBinding::new("shift-o", OpenAbove, Some("Normal")),
        KeyBinding::new("h", CharLeft, Some("Normal")),
        KeyBinding::new("l", CharRight, Some("Normal")),
        KeyBinding::new("j", LineDown, Some("Normal")),
        KeyBinding::new("k", LineUp, Some("Normal")),
        KeyBinding::new("w", WordForward, Some("Normal")),
        KeyBinding::new("b", WordBack, Some("Normal")),
        KeyBinding::new("e", WordEnd, Some("Normal")),
        KeyBinding::new("shift-w", WordForwardWs, Some("Normal")),
        KeyBinding::new("shift-b", WordBackWs, Some("Normal")),
        KeyBinding::new("shift-e", WordEndWs, Some("Normal")),
        KeyBinding::new("0", Digit0, Some("Normal")),
        KeyBinding::new("^", LineFirstNonBlank, Some("Normal")),
        KeyBinding::new("$", LineEnd, Some("Normal")),
        KeyBinding::new("g", PendingG, Some("Normal")),
        KeyBinding::new("shift-g", LastDoc, Some("Vim")),
        KeyBinding::new("x", SelectLine, Some("Helix")),
        KeyBinding::new("x", DeleteChar, Some("Vim")),
        KeyBinding::new("d", DeleteOp, Some("Normal")),
        KeyBinding::new("shift-d", DeleteToEnd, Some("Vim")),
        KeyBinding::new("v", VisualChar, Some("Normal")),
        KeyBinding::new("shift-v", VisualLine, Some("Vim")),
        KeyBinding::new("u", Undo, Some("Normal")),
        KeyBinding::new("shift-u", Redo, Some("Helix")),
        KeyBinding::new("ctrl-r", Redo, Some("Vim")),
        KeyBinding::new("1", Digit1, Some("Normal")),
        KeyBinding::new("2", Digit2, Some("Normal")),
        KeyBinding::new("3", Digit3, Some("Normal")),
        KeyBinding::new("4", Digit4, Some("Normal")),
        KeyBinding::new("5", Digit5, Some("Normal")),
        KeyBinding::new("6", Digit6, Some("Normal")),
        KeyBinding::new("7", Digit7, Some("Normal")),
        KeyBinding::new("8", Digit8, Some("Normal")),
        KeyBinding::new("9", Digit9, Some("Normal")),
        KeyBinding::new("up", SlashPrev, Some("Input")),
        KeyBinding::new("down", SlashNext, Some("Input")),
        KeyBinding::new("ctrl-p", SlashPrev, Some("Input")),
        KeyBinding::new("ctrl-n", SlashNext, Some("Input")),
        KeyBinding::new("ctrl-k", SlashPrev, Some("Input")),
        KeyBinding::new("ctrl-j", SlashNext, Some("Input")),
        KeyBinding::new("enter", SlashApply, Some("Input")),
        KeyBinding::new("backspace", BlockBackspace, Some("Input")),
        KeyBinding::new("shift-backspace", BlockBackspace, Some("Input")),
        KeyBinding::new(":", OpenCommand, Some("Normal")),
        KeyBinding::new("shift-;", OpenCommand, Some("Normal")),
        KeyBinding::new("/", OpenSearch, Some("Vim && Normal")),
        KeyBinding::new("shift-/", SearchBack, Some("Normal")),
        KeyBinding::new("cmd-f", OpenSearch, Some("Workspace")),
        KeyBinding::new("ctrl-f", OpenSearch, Some("Workspace")),
        KeyBinding::new("/", InsertSlash, Some("Helix")),
        KeyBinding::new("/", InsertSlash, Some("Notion")),
        KeyBinding::new("n", SearchNext, Some("Normal")),
        KeyBinding::new("shift-n", SearchPrev, Some("Normal")),
        KeyBinding::new("shift-j", JoinLines, Some("Normal")),
        KeyBinding::new("r", ReplaceChar, Some("Normal")),
        KeyBinding::new("f", FindForward, Some("Normal")),
        KeyBinding::new("shift-f", FindBackward, Some("Normal")),
        KeyBinding::new("t", FindTill, Some("Normal")),
        KeyBinding::new("shift-t", FindTillBack, Some("Normal")),
        KeyBinding::new(";", RepeatFind, Some("Normal")),
        KeyBinding::new(",", ReverseFind, Some("Normal")),
        KeyBinding::new("[", BracketOpen, Some("Normal")),
        KeyBinding::new("]", BracketClose, Some("Normal")),
        KeyBinding::new("enter", InsertNewline, Some("Workspace")),
        KeyBinding::new("shift-enter", InsertHardBreak, Some("Workspace")),
        KeyBinding::new("tab", IndentTab, Some("Workspace")),
        KeyBinding::new("shift-tab", OutdentTab, Some("Workspace")),
        KeyBinding::new("cmd-b", ToggleBold, Some("Workspace")),
        KeyBinding::new("ctrl-b", ToggleBold, Some("Workspace")),
        KeyBinding::new("cmd-i", ToggleItalic, Some("Workspace")),
        KeyBinding::new("ctrl-i", ToggleItalic, Some("Workspace")),
        KeyBinding::new("cmd-u", ToggleUnderline, Some("Workspace")),
        KeyBinding::new("ctrl-u", ToggleUnderline, Some("Workspace")),
        KeyBinding::new("cmd-e", ToggleCode, Some("Workspace")),
        KeyBinding::new("ctrl-e", ToggleCode, Some("Workspace")),
        KeyBinding::new("cmd-shift-s", ToggleStrike, Some("Workspace")),
        KeyBinding::new("cmd-shift-p", OpenPalette, Some("Workspace")),
        KeyBinding::new("ctrl-shift-p", OpenPalette, Some("Workspace")),
        KeyBinding::new("cmd-shift-k", ToggleLink, Some("Workspace")),
        KeyBinding::new("ctrl-shift-k", ToggleLink, Some("Workspace")),
        KeyBinding::new("cmd-x", CutSelection, Some("Workspace")),
        KeyBinding::new("ctrl-x", CutSelection, Some("Workspace")),
        KeyBinding::new("cmd-c", CopySelection, Some("Workspace")),
        KeyBinding::new("ctrl-c", CopySelection, Some("Workspace")),
        KeyBinding::new("cmd-v", PasteClipboard, Some("Workspace")),
        KeyBinding::new("ctrl-v", PasteClipboard, Some("Workspace")),
        KeyBinding::new("cmd-shift-v", ToggleSource, Some("Workspace")),
        KeyBinding::new("ctrl-shift-v", ToggleSource, Some("Workspace")),
        KeyBinding::new("cmd-a", SelectAll, Some("Workspace")),
        KeyBinding::new("ctrl-a", SelectAll, Some("Workspace")),
        KeyBinding::new("%", SelectAll, Some("Helix && Normal")),
        KeyBinding::new(">", IndentShift, Some("Normal")),
        KeyBinding::new("<", DedentShift, Some("Normal")),
        KeyBinding::new("=", AutoIndent, Some("Normal")),
        KeyBinding::new("c", ChangeOp, Some("Normal")),
        KeyBinding::new("m", MatchObject, Some("Helix && Normal")),
        KeyBinding::new("backspace", BlockBackspace, Some("Workspace")),
        KeyBinding::new("shift-backspace", BlockBackspace, Some("Workspace")),
        KeyBinding::new("alt-backspace", DeleteWordBack, Some("Workspace")),
        KeyBinding::new("cmd-backspace", DeleteLineBack, Some("Workspace")),
        KeyBinding::new("ctrl-backspace", DeleteWordBack, Some("Workspace")),
        KeyBinding::new("cmd-z", Undo, Some("Workspace")),
        KeyBinding::new("ctrl-z", Undo, Some("Workspace")),
        KeyBinding::new("cmd-shift-z", Redo, Some("Workspace")),
        KeyBinding::new("ctrl-shift-z", Redo, Some("Workspace")),
        KeyBinding::new("cmd-q", QuitApp, Some("Workspace")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectGranularity {
    Char,
    Word,
    Line,
}

/// Excel-style rectangular cell selection inside one table block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TableRect {
    block_ix: usize,
    r0: usize,
    c0: usize,
    r1: usize,
    c1: usize,
}

impl TableRect {
    fn normalize(self) -> Self {
        Self {
            block_ix: self.block_ix,
            r0: self.r0.min(self.r1),
            c0: self.c0.min(self.c1),
            r1: self.r0.max(self.r1),
            c1: self.c0.max(self.c1),
        }
    }

    fn contains(self, row: usize, col: usize) -> bool {
        let n = self.normalize();
        row >= n.r0 && row <= n.r1 && col >= n.c0 && col <= n.c1
    }

    fn is_multi(self) -> bool {
        let n = self.normalize();
        n.r0 != n.r1 || n.c0 != n.c1
    }

    fn row_count(self) -> usize {
        let n = self.normalize();
        n.r1.saturating_sub(n.r0) + 1
    }

    fn col_count(self) -> usize {
        let n = self.normalize();
        n.c1.saturating_sub(n.c0) + 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
enum SettingsPane {
    Editor,
    Theme,
    Font,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontSlot {
    Ui,
    Markdown,
    Buffer,
}

/// Which field of a font slot a per-field reset button owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontField {
    Family,
    Size,
}

struct FontInputs {
    ui_family: Entity<InputState>,
    ui_size: Entity<InputState>,
    markdown_family: Entity<InputState>,
    markdown_size: Entity<InputState>,
    buffer_family: Entity<InputState>,
    buffer_size: Entity<InputState>,
    _subs: Vec<gpui::Subscription>,
}

impl FontInputs {
    fn new(config: &Config, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let mk =
            |value: String, placeholder: &str, window: &mut Window, cx: &mut Context<Workspace>| {
                let ph = placeholder.to_string();
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(value)
                        .placeholder(ph)
                })
            };
        let ui_family = mk(config.ui_font.family.clone(), "UI font family", window, cx);
        let ui_size = mk(config.ui_font.size.to_string(), "13", window, cx);
        let markdown_family = mk(
            config.markdown_font.family.clone(),
            "Markdown font family",
            window,
            cx,
        );
        let markdown_size = mk(config.markdown_font.size.to_string(), "15", window, cx);
        let buffer_family = mk(
            config.buffer_font.family.clone(),
            "Code / buffer font family",
            window,
            cx,
        );
        let buffer_size = mk(config.buffer_font.size.to_string(), "14", window, cx);

        let mut _subs = Vec::new();
        let wire = |slot: FontSlot,
                    family: &Entity<InputState>,
                    size: &Entity<InputState>,
                    cx: &mut Context<Workspace>,
                    subs: &mut Vec<gpui::Subscription>| {
            subs.push(
                cx.subscribe(family, move |this, input, ev: &InputEvent, cx| {
                    if matches!(ev, InputEvent::Change) {
                        let family = input.read(cx).value().to_string();
                        this.apply_font_family(slot, family, cx);
                    }
                }),
            );
            subs.push(cx.subscribe(size, move |this, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    let raw = input.read(cx).value().to_string();
                    // Numeric-only: ignore (never apply) anything without digits.
                    let digits: String =
                        raw.chars().filter(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = digits.parse::<u32>() {
                        this.apply_font_size(slot, n, cx);
                    }
                }
            }));
        };
        wire(FontSlot::Ui, &ui_family, &ui_size, cx, &mut _subs);
        wire(
            FontSlot::Markdown,
            &markdown_family,
            &markdown_size,
            cx,
            &mut _subs,
        );
        wire(
            FontSlot::Buffer,
            &buffer_family,
            &buffer_size,
            cx,
            &mut _subs,
        );

        Self {
            ui_family,
            ui_size,
            markdown_family,
            markdown_size,
            buffer_family,
            buffer_size,
            _subs,
        }
    }
}

/// 1-based `(line, col)` in GFM source -> byte offset. Columns count
/// characters and clamp to the line end.
fn source_offset_for_line_col(source: &str, line: usize, col: usize) -> usize {
    let line = line.max(1);
    let col = col.max(1);
    let mut offset = 0;
    let mut cur = 1;
    for raw in source.split_inclusive('\n') {
        if cur == line {
            let line_body = raw.strip_suffix('\n').unwrap_or(raw);
            let mut ix = 0;
            let mut c = 1;
            for ch in line_body.chars() {
                if c >= col {
                    break;
                }
                ix += ch.len_utf8();
                c += 1;
            }
            return offset + ix;
        }
        offset += raw.len();
        cur += 1;
    }
    source.len()
}

pub struct Workspace {
    path: PathBuf,    doc: crate::tree::Doc,
    source: String,
    caret: usize,
    sel: Option<Range<usize>>,
    mode: Mode,
    pending_g: bool,
    pending_d: bool,
    pending_count: Option<usize>,
    visual_anchor: Option<usize>,
    slash_index: usize,
    last_slash_query: String,
    focus: FocusHandle,
    palette: Palette,
    dirty: bool,
    /// Source as last saved/opened. Undo back to this clears the dirty dot.
    clean_source: String,
    status: SharedString,
    command: Option<LineField>,
    search: Option<(LineField, bool)>,
    last_search: Option<(String, bool)>,
    cmd_palette: Option<PaletteState>,
    view_source: bool,
    affinity: Affinity,
    marked: Option<Range<usize>>,
    sticky: crate::display::Marks,
    link_open: bool,
    link_draft: LineField,
    palette_scroll: ScrollHandle,
    slash_scroll: ScrollHandle,
    hits: Vec<Hit>,
    pending_replace: Option<usize>,
    pending_find: Option<(FindKind, usize)>,
    last_find: Option<(FindKind, char)>,
    pending_bracket: Option<i8>,
    /// `>` / `<` / `=` operator waiting for a repeat (`>>`) or a motion
    /// (`>j`, `>G`, `gg=G`). Cleared by `clear_pending` / esc.
    pending_op: Option<IndentOpKind>,
    /// Text-object flow (`viw`, `di"`, `ci{`, Helix `miw`): the operator plus
    /// whether `i` (inner) or `a` (around) was chosen. Cleared on esc.
    pending_obj: Option<(ObjOp, Option<bool>)>,
    /// `c` waiting for a repeat (`cc`) or `i`/`a` (change object).
    pending_change: bool,
    config: Config,
    undo: UndoStack,
    insert_origin: Option<Snapshot>,
    settings_open: bool,
    settings_pane: SettingsPane,
    scroll_handle: ScrollHandle,
    /// Keep the caret in view after the next paint. Clicks leave scroll alone.
    follow_caret: bool,
    surface_h: gpui::Pixels,
    /// Preferred screen X for vertical caret motion (sticky column). Cleared
    /// on horizontal moves / clicks.
    goal_x: Option<Pixels>,
    mouse_anchor: Option<usize>,
    /// Initial word/line unit from a multi-click, used while dragging.
    mouse_unit: Option<Range<usize>>,
    mouse_dragging: bool,
    mouse_granularity: SelectGranularity,
    /// Anchor cell when starting a drag inside a table (block_ix, row, col).
    table_anchor: Option<(usize, usize, usize)>,
    /// Rectangular cell selection (Excel-style). Independent of linear `sel`.
    table_sel: Option<TableRect>,
    block_dragging: Option<usize>,
    block_drag_gap: Option<usize>,
    block_menu: Option<usize>,
    /// Copy-button flash per code block, keyed by block display start
    /// with the click time. Shows a check ~1.2s, then reverts to copy.
    code_copied: std::collections::HashMap<usize, std::time::Instant>,
    /// Horizontal scroll state per fenced code block, keyed by block
    /// display start. Persisted across frames so wheel/drag/caret nudges
    /// don't reset on every keystroke.
    code_scroll: std::collections::HashMap<usize, ScrollHandle>,
    /// Keys touched this frame (prune stale `code_scroll` entries).
    code_scroll_seen: Vec<usize>,
    loading: bool,
    fonts: FontInputs,
    /// Read-only display of the current theme (picked via the palette).
    theme_input: Entity<InputState>,
    /// Selected image/video block (display offset) with its edit toolbar.
    media_sel: Option<usize>,
    /// Collapsed `<details open>` blocks, keyed by open-row source start.
    /// `<details>` without `open` is collapsed by default (GFM) — those use
    /// `expanded_html` when the user opens them in this session.
    collapsed_html: std::collections::HashSet<usize>,
    /// Session-expanded `<details>` (no `open` attr), keyed by source start.
    expanded_html: std::collections::HashSet<usize>,
    media_alt: Entity<InputState>,
    media_src: Entity<InputState>,
    /// Zed-style `cmd-k` chord: armed by `cmd-k` at capture level, consumed
    /// by the next key (`t` opens the Themes picker, esc cancels).
    pending_chord: bool,
    /// Theme before the palette opened. Hover/arrows preview without
    /// persisting; esc / dismiss restores this, enter / click commits.
    palette_theme_backup: Option<String>,
    /// Live Mermaid diagrams, keyed by block source. Themed at render time,
    /// so a theme switch clears the cache.
    mermaid: MermaidStore,
    /// Inline video clips, keyed by resolved path (or `remote:<url>` once
    /// downloaded to temp). Pure-Rust `yscv-video` decode — no GStreamer /
    /// FFmpeg system libs, so the bundle stays self-contained.
    video: crate::video::VideoStore,
}

impl Workspace {
    pub fn new(
        path: PathBuf,
        source: String,
        palette: Palette,
        config: Config,
        initial: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        apply_palette(&palette, cx);

        let empty_doc = source.trim().is_empty();
        let fonts = FontInputs::new(&config, window, cx);
        let theme_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(config.theme.clone())
        });
        let media_alt =
            cx.new(|cx| InputState::new(window, cx).placeholder("Alt text".to_string()));
        let media_src =
            cx.new(|cx| InputState::new(window, cx).placeholder("image.png".to_string()));
        let doc = crate::tree::Doc::from_gfm(&source);
        // Normalize once: `source` after any edit is `doc.to_gfm()`, so the
        // clean baseline must be in the same form or typing-then-deleting the
        // same text would never clear the dirty dot.
        let normalized = doc.to_gfm();
        let mut this = Self {
            path,
            doc,
            source: normalized.clone(),
            caret: 0,
            sel: None,
            mode: Mode::Normal,
            pending_g: false,
            pending_d: false,
            pending_count: None,
            visual_anchor: None,
            slash_index: 0,
            last_slash_query: String::new(),
            focus: cx.focus_handle(),
            palette,
            dirty: false,
            clean_source: normalized,
            status: "ready".into(),
            command: None,
            search: None,
            last_search: None,
            cmd_palette: None,
            view_source: false,
            affinity: Affinity::Inside,
            marked: None,
            sticky: crate::display::Marks::default(),
            link_open: false,
            link_draft: LineField::new(),
            palette_scroll: ScrollHandle::new(),
            slash_scroll: ScrollHandle::new(),
            hits: Vec::new(),
            pending_replace: None,
            pending_find: None,
            last_find: None,
            pending_bracket: None,
            pending_op: None,
            pending_obj: None,
            pending_change: false,
            config,
            undo: UndoStack::default(),
            insert_origin: None,
            settings_open: false,
            settings_pane: SettingsPane::Editor,
            scroll_handle: ScrollHandle::new(),
            follow_caret: true,
            surface_h: px(0.),
            goal_x: None,
            mouse_anchor: None,
            mouse_unit: None,
            mouse_dragging: false,
            mouse_granularity: SelectGranularity::Char,
            table_anchor: None,
            table_sel: None,
            block_dragging: None,
            block_drag_gap: None,
            block_menu: None,
            code_copied: std::collections::HashMap::new(),
            code_scroll: std::collections::HashMap::new(),
            code_scroll_seen: Vec::new(),
            loading: false,
            fonts,
            theme_input,
            media_sel: None,
            collapsed_html: std::collections::HashSet::new(),
            expanded_html: std::collections::HashSet::new(),
            media_alt,
            media_src,
            pending_chord: false,
            palette_theme_backup: None,
            mermaid: MermaidStore::default(),
            video: crate::video::VideoStore::default(),
        };

        if let Some((line, col)) = initial {
            // Zed-style `file:line:col`: source offsets map to the nearest
            // visible position (hidden markup like `> ` has no caret home).
            let src = source_offset_for_line_col(&this.source, line, col);
            let d = this.proj().to_display(src);
            this.caret = d.min(this.proj().display.len());
            if !this.config.editor.is_modal() {
                this.enter_insert(Caret::Offset(this.caret), window, cx);
            } else {
                this.mode = Mode::Normal;
                this.refresh(window, cx);
            }
        } else if empty_doc || !this.config.editor.is_modal() {
            // Always open at the very top (caret 0), never the bottom.
            this.enter_insert(Caret::Start, window, cx);
        } else {
            this.mode = Mode::Normal;
            this.refresh_raw(window, cx);
        }
        // Window-close guard lives on the tab shell (one prompt for all
        // tabs); no per-workspace hook here.
        this
    }

    pub fn view(
        path: PathBuf,
        source: String,
        palette: Palette,
        config: Config,
        initial: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new(path, source, palette, config, initial, window, cx))
    }

    fn key_context(&self) -> &'static str {
        // Keep "Workspace" out of the context so Workspace-scoped bindings
        // (notably enter → InsertNewline) do not fire while the find/command
        // bar owns the keyboard. Capture handlers still see every key.
        if self.command.is_some() || self.search.is_some() || self.cmd_palette.is_some() {
            return "Command";
        }
        match (self.config.editor, self.mode) {
            (EditorKind::Notion, _) => "Workspace Notion",
            (EditorKind::Helix, Mode::Insert) => "Workspace Insert Helix",
            (EditorKind::Vim, Mode::Insert) => "Workspace Insert Vim",
            (EditorKind::Helix, Mode::Select) => "Workspace Normal Helix Select",
            (EditorKind::Vim, Mode::Visual) => "Workspace Normal Vim Visual",
            (EditorKind::Vim, Mode::VisualLine) => "Workspace Normal Vim VisualLine",
            (EditorKind::Helix, _) => "Workspace Normal Helix",
            (EditorKind::Vim, _) => "Workspace Normal Vim",
        }
    }

    fn is_notion(&self) -> bool {
        self.config.editor == EditorKind::Notion
    }

    fn is_modal_nav(&self) -> bool {
        self.config.editor.is_modal() && !self.mode.is_insert()
    }

    fn wrap_cols(&self) -> Option<usize> {
        // Preview always soft-wraps; the toggle only affects source view.
        if self.view_source && !self.config.wrap_motions {
            return None;
        }
        // Prefer the live column width from the surface when painted so the
        // char-column fallback matches soft-wrap more closely. Only full
        // (unsegmented) blocks carry the text-area width — word pieces are
        // content-sized — so skip small hits.
        let width = self
            .hits
            .iter()
            .filter(|h| h.doc_len > 40)
            .find_map(|h| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.layout.bounds()))
                    .ok()
                    .map(|b| f32::from(b.size.width))
            })
            .filter(|w| *w > 8.0)
            .unwrap_or(COLUMN_PX - 64.0);
        let ch = (self.config.markdown_font.size as f32 * 0.55).max(6.0);
        Some((width / ch).max(8.0) as usize)
    }

    fn proj(&self) -> Projection {
        self.doc.project()
    }

    fn sync_gfm(&mut self) {
        self.source = self.doc.to_gfm();
    }

    fn commit_caret(&mut self, caret: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.caret = caret.min(self.proj().display.len());
        self.sel = None;
        self.mark_dirty();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn ui_font_px(&self) -> gpui::Pixels {
        px(self.config.ui_font.size.clamp(8, 48) as f32)
    }

    fn markdown_font_px(&self) -> gpui::Pixels {
        px(self.config.markdown_font.size.clamp(8, 48) as f32)
    }

    fn buffer_font_px(&self) -> gpui::Pixels {
        px(self.config.buffer_font.size.clamp(8, 48) as f32)
    }

    fn font_px(&self) -> gpui::Pixels {
        self.markdown_font_px()
    }

    fn paint_ranges(&self) -> Vec<PaintRange> {
        parse_ranges(&self.source)
    }

    fn compute_raw(&self) -> Range<usize> {
        0..0
    }

    fn clamp_caret(&mut self) {
        let display = self.proj().display;
        let n = display.len();
        self.caret = floor_char_boundary(&display, self.caret.min(n));
        if let Some(sel) = self.sel.as_mut() {
            sel.start = floor_char_boundary(&display, sel.start.min(n));
            sel.end = floor_char_boundary(&display, sel.end.min(n));
            if sel.start == sel.end {
                self.sel = None;
            }
        }
    }

    fn caret_src(&self) -> usize {
        self.proj().to_source(self.caret, self.affinity)
    }

    fn set_caret_src(&mut self, src: usize) {
        self.caret = self.proj().to_display(src);
    }

    fn mark_dirty(&mut self) {
        self.refresh_dirty();
    }

    fn refresh_dirty(&mut self) {
        self.dirty = self.source != self.clean_source;
        self.status = if self.dirty { "unsaved" } else { "ready" }.into();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot::of(
            &self.doc,
            &self.source,
            self.caret,
            self.sel.clone(),
            self.mode,
        )
    }

    fn push_doc_undo(&mut self) {
        // Commit any coalesced typing session first so Undo restores the
        // pre-edit caret/text (e.g. list Enter → cmd-z lands back on the item).
        self.finish_insert_undo();
        self.undo.push(self.snapshot());
    }

    fn refresh_raw(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(window, cx);
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clamp_caret();
        if self.mode.extends_selection() {
            self.snap_visual_sel();
        }
        self.follow_caret = true;
        self.request_short_block_scroll();
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn snap_visual_sel(&mut self) {
        let Some(anchor) = self.visual_anchor else {
            return;
        };
        let p = self.proj();
        let da = anchor.min(p.display.len());
        let dc = self.caret.min(p.display.len());
        if self.mode == Mode::VisualLine {
            let a = da.min(dc);
            let b = da.max(dc);
            let start = logical_line_range(&p.display, a).start;
            let mut end = logical_line_range(&p.display, b).end;
            if end < p.display.len() && p.display.as_bytes()[end] == b'\n' {
                end += 1;
            }
            self.sel = Some(start..end);
        } else {
            let a = da.min(dc);
            let b = da.max(dc);
            if a == b {
                self.sel = None;
            } else {
                self.sel = Some(a..b);
            }
        }
    }

    fn land(&mut self, next: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.caret = next.min(self.proj().display.len());
        if self.mode.extends_selection() {
            if self.visual_anchor.is_none() {
                self.visual_anchor = Some(self.caret);
            }
            self.snap_visual_sel();
        } else {
            self.sel = None;
            self.visual_anchor = None;
            // Drop rectangular table selection on caret moves (unless still in that cell).
            if let Some(rect) = self.table_sel {
                let still = self
                    .proj()
                    .table_cell_at(self.caret)
                    .and_then(|(b, c)| {
                        let ix = self
                            .proj()
                            .blocks
                            .iter()
                            .position(|x| x.source == b.source)?;
                        Some(ix == rect.block_ix && rect.contains(c.row, c.col))
                    })
                    .unwrap_or(false);
                if !still {
                    self.table_sel = None;
                    self.table_anchor = None;
                }
            }
        }
        self.refresh_raw(window, cx);
    }

    /// Same-frame scroll for short blocks (new empty line at EOF). Tall
    /// lists/paragraphs skip this — `scroll_to_item` would snap to their top.
    fn request_short_block_scroll(&self) {
        let p = self.proj();
        let d = self.caret;
        let Some(ix) = wysiwyg::units(&p).iter().position(|u| {
            let r = wysiwyg::unit_display(&p, *u);
            d >= r.start && d <= r.end
        }) else {
            return;
        };
        let viewport = self.scroll_handle.bounds();
        if let Some(bounds) = self.scroll_handle.bounds_for_item(ix) {
            if viewport.size.height > px(0.) && bounds.size.height > viewport.size.height {
                return;
            }
            if viewport.size.height > px(0.) {
                let offset = self.scroll_handle.offset();
                let top = bounds.top() + offset.y;
                let bottom = bounds.bottom() + offset.y;
                if top >= viewport.top() && bottom <= viewport.bottom() {
                    return;
                }
            }
        }
        self.scroll_handle.scroll_to_item(ix);
    }

    /// Nudge scroll so the caret line is inside the viewport. Never snaps a
    /// tall list/paragraph to its top (`scroll_to_item` does that).
    fn ensure_caret_visible(&mut self, cx: &mut Context<Self>) {
        let viewport = self.scroll_handle.bounds();
        if viewport.size.height <= px(0.) {
            return;
        }
        if self.hits.is_empty() {
            return;
        }
        let d = self.caret;
        let Some((caret_top, line_h)) = surface::caret_screen_y(&self.hits, d) else {
            self.follow_caret = false;
            return;
        };
        self.follow_caret = false;
        let caret_bottom = caret_top + line_h;
        let margin = (line_h * 0.5).max(px(4.));
        let mut offset = self.scroll_handle.offset();
        let mut changed = false;
        if caret_top < viewport.top() + margin {
            offset.y += viewport.top() + margin - caret_top;
            changed = true;
        } else if caret_bottom > viewport.bottom() - margin {
            offset.y -= caret_bottom - (viewport.bottom() - margin);
            changed = true;
        }
        if changed {
            self.scroll_handle.set_offset(offset);
            cx.notify();
        }
        // Horizontal reveal for fenced code blocks, which own an x-scroller.
        // Arrow-key caret motion sets `follow_caret`, so walking past either
        // edge pulls the viewport along (right edge keeps clearance for the
        // floating lang/copy pill).
        let p = self.proj();
        let in_code = p.blocks.iter().find(|b| {
            matches!(b.extra, BlockExtra::Code { .. })
                && d >= b.display.start
                && d <= b.display.end
        });
        if let Some(b) = in_code {
            let key = b.display.start;
            if let Some(handle) = self.code_scroll.get(&key).cloned() {
                let viewport = handle.bounds();
                if viewport.size.width > px(0.) {
                    if let Some(caret_x) = surface::caret_screen_x(&self.hits, d) {
                        let mut offset = handle.offset();
                        let left_pad = px(4.);
                        let right_pad = px(112.);
                        let mut hx = false;
                        if caret_x > viewport.right() - right_pad {
                            offset.x -= caret_x - (viewport.right() - right_pad);
                            hx = true;
                        } else if caret_x < viewport.left() + left_pad {
                            offset.x += viewport.left() + left_pad - caret_x;
                            hx = true;
                        }
                        if hx {
                            handle.set_offset(offset);
                            cx.notify();
                        }
                    }
                }
            }
        }
    }

    /// Nudge a code block's x-scroller while drag-selecting past its edges.
    /// Fires on mouse-move over the block's scroll viewport; stationary holds
    /// don't repeat (no timer) — moving further re-fires.
    fn autoscroll_code_x(
        &mut self,
        key: usize,
        range: Range<usize>,
        x: Pixels,
        cx: &mut Context<Self>,
    ) {
        if !self.mouse_dragging {
            return;
        }
        let d = self.sel.as_ref().map(|s| s.end).unwrap_or(self.caret);
        if d < range.start || d > range.end {
            return;
        }
        let Some(handle) = self.code_scroll.get(&key).cloned() else {
            return;
        };
        let viewport = handle.bounds();
        if viewport.size.width <= px(0.) {
            return;
        }
        let offset = handle.offset();
        let max_x = handle.max_offset().x.max(px(0.));
        let margin = px(32.);
        let step = px(32.);
        let mut nx = offset.x;
        if x > viewport.right() - margin {
            nx = (offset.x - step).max(-max_x);
        } else if x < viewport.left() + margin {
            nx = (offset.x + step).min(px(0.));
        } else {
            return;
        }
        if nx != offset.x {
            handle.set_offset(gpui::Point { x: nx, y: offset.y });
            cx.notify();
        }
    }

    fn persist_config(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = config::save(&self.config) {
            self.status = format!("config: {err}").into();
            cx.notify();
        }
    }

    fn uses_block_caret(&self) -> bool {
        self.config.editor.is_modal() && !self.mode.is_insert() && !self.mode.is_visual()
    }

    fn finish_insert_undo(&mut self) {
        let Some(mut origin) = self.insert_origin.take() else {
            return;
        };
        let current = self.snapshot();
        if origin.source != current.source {
            // Insert session started from Normal; undoing it should return there.
            origin.mode = Mode::Normal;
            self.undo.push(origin);
        }
    }

    fn apply_snapshot(&mut self, snap: Snapshot, window: &mut Window, cx: &mut Context<Self>) {
        // Restore the live tree. Re-parsing GFM is lossy (e.g. Tab-indented
        // lists become indented code whose "language" is `- bullet`).
        // Undo/redo restores the caret recorded in the snapshot (the position
        // from before the undone edit); dirty state is pure content comparison
        // so it never moves the caret on its own.
        self.doc = snap.doc;
        self.source = snap.source;
        self.caret = snap.caret.min(self.doc.project().display.len());
        self.sel = snap.sel;
        self.insert_origin = None;
        self.clear_pending();
        self.visual_anchor = self.sel.as_ref().map(|s| s.start);
        self.refresh_dirty();
        if !self.config.editor.is_modal() {
            self.enter_insert(Caret::Offset(self.caret), window, cx);
        } else {
            self.mode = Mode::Normal;
            self.sel = None;
            self.visual_anchor = None;
            self.refresh_raw(window, cx);
        }
        self.sync_title(window);
    }

    fn clear_pending(&mut self) {
        self.pending_g = false;
        self.pending_d = false;
        self.pending_count = None;
        self.pending_replace = None;
        self.pending_find = None;
        self.pending_bracket = None;
        self.pending_op = None;
        self.pending_obj = None;
        self.pending_change = false;
    }

    fn on_press_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_newline(false, window, cx);
    }

    fn insert_newline(&mut self, hard: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some()
            || self.command.is_some()
            || self.settings_open
            || self.cmd_palette.is_some()
            || self.view_source
        {
            return;
        }
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            return;
        }
        if self.link_open {
            self.commit_link(window, cx);
            return;
        }
        if let Some(item) = self.current_slash_pick() {
            self.apply_slash(item, window, cx);
            return;
        }
        let prev_caret = self.caret;
        // Closed `<details>`: Enter (or shift-enter) must land on the block
        // *below* the disclosure, not inside the hidden body where the caret
        // disappears. Offset 0 keeps the normal above-insert.
        let on_closed_details = {
            let p = self.proj();
            match p.block_at_display(self.caret) {
                Some(b)
                    if matches!(b.extra, BlockExtra::Details { .. })
                        && self.is_details_collapsed(b)
                        && self.caret > b.display.start =>
                {
                    true
                }
                _ => false,
            }
        };
        self.push_doc_undo();
        if on_closed_details {
            if let Some(c) = self.doc.enter_closed_details(self.caret) {
                self.caret = c;
                self.sync_gfm();
                self.caret = self.caret.min(self.proj().display.len());
                self.sel = None;
                self.mark_dirty();
                self.try_auto_link_preceding_url(prev_caret);
                self.refresh(window, cx);
                self.sync_title(window);
                return;
            }
        }
        self.caret = self.doc.enter(self.caret, hard);
        self.sync_gfm();
        self.caret = self.caret.min(self.proj().display.len());
        self.sel = None;
        self.mark_dirty();

        // Check if the token before Enter was a bare URL. If so, auto-link it.
        // `push_doc_undo` above already recorded the state before Enter, and
        // `try_auto_link_preceding_url` pushes another snapshot of the state
        // with the Enter applied but before the link transformation.
        // Therefore, first Cmd-Z undoes the link, and second Cmd-Z undoes Enter.
        self.try_auto_link_preceding_url(prev_caret);

        self.refresh(window, cx);
        self.sync_title(window);
    }

    pub fn click_display(
        &mut self,
        d: usize,
        shift: bool,
        cmd: bool,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        self.goal_x = None;
        if self.link_open {
            self.link_open = false;
            self.link_draft.clear();
        }
        if !shift {
            let p = self.proj();
            if let Some((range, url)) = p.link_at(d) {
                if cmd {
                    self.open_link_url(url, window, cx);
                    return;
                } else if click_count == 1 {
                    // Clicking on a link selects the link and opens the link bubble
                    self.caret = range.end;
                    self.sel = Some(range);
                    self.mouse_dragging = false;
                    self.clamp_caret();
                    self.follow_caret = false;
                    self.focus.focus(window, cx);
                    cx.notify();
                    return;
                }
            }
        }
        // Images / videos: single click selects the block and opens the
        // alt + path toolbar. Clicking anywhere else dismisses it.
        if click_count == 1 && !shift && !cmd {
            let hit = {
                let p = self.proj();
                p.block_at_display(d).map(|b| {
                    let media = matches!(&b.extra, BlockExtra::Image { .. })
                        || matches!(&b.extra, BlockExtra::Html)
                            && self
                                .source
                                .get(b.source.clone())
                                .is_some_and(|raw| images::parse_video_src(raw).is_some());
                    (b.display.start, media)
                })
            };
            if let Some((start, true)) = hit {
                self.select_media(start, window, cx);
                return;
            }
            if self.media_sel.is_some() {
                self.media_sel = None;
            }
        }
        if click_count >= 2 && !shift {            let p = self.proj();
            let d = d.min(p.display.len());
            let (range, gran) = if click_count >= 3 {
                (
                    crate::motion::logical_line_range(&p.display, d),
                    SelectGranularity::Line,
                )
            } else {
                (word_range_at(&p.display, d), SelectGranularity::Word)
            };
            if range.start < range.end {
                self.caret = range.end;
                self.mouse_anchor = Some(range.start);
                self.mouse_unit = Some(range.clone());
                self.sel = Some(range);
                self.mouse_granularity = gran;
                self.affinity = Affinity::Inside;
                // Keep dragging so double-click-then-drag extends by words/lines.
                self.mouse_dragging = true;
                self.clamp_caret();
                self.follow_caret = false;
                self.focus.focus(window, cx);
                cx.notify();
                return;
            }
        }
        self.mouse_granularity = SelectGranularity::Char;
        self.mouse_unit = None;
        // Table: start Excel-style cell selection from this cell.
        let table_hit = {
            let p = self.proj();
            p.table_cell_at(d).and_then(|(block, cell)| {
                let ix = p.blocks.iter().position(|b| b.source == block.source)?;
                Some((ix, cell.row, cell.col, cell.display.clone()))
            })
        };
        if let Some((ix, row, col, _disp)) = table_hit {
            if shift {
                if let Some((aix, ar, ac)) = self.table_anchor {
                    if aix == ix {
                        self.table_sel = Some(
                            TableRect {
                                block_ix: ix,
                                r0: ar,
                                c0: ac,
                                r1: row,
                                c1: col,
                            }
                            .normalize(),
                        );
                        self.caret = d;
                        self.sel = Some(self.table_sel_display_range());
                        self.mouse_dragging = true;
                        self.clamp_caret();
                        self.follow_caret = false;
                        self.focus.focus(window, cx);
                        cx.notify();
                        return;
                    }
                }
            }
            self.table_anchor = Some((ix, row, col));
            self.table_sel = Some(TableRect {
                block_ix: ix,
                r0: row,
                c0: col,
                r1: row,
                c1: col,
            });
            if !shift {
                self.caret = d;
                self.mouse_anchor = Some(d);
                self.affinity = Affinity::Inside;
                if !self.mode.is_insert() && self.config.editor.is_modal() {
                    self.mode = Mode::Normal;
                    self.sel = None;
                    self.visual_anchor = None;
                } else {
                    self.sel = None;
                }
                self.mouse_dragging = true;
                self.clamp_caret();
                self.follow_caret = false;
                self.focus.focus(window, cx);
                cx.notify();
                return;
            }
        } else {
            self.table_anchor = None;
            self.table_sel = None;
        }
        if shift {
            if self.visual_anchor.is_none() {
                self.visual_anchor = Some(self.caret);
            }
            self.caret = d;
            self.mode = if self.config.editor == EditorKind::Helix {
                Mode::Select
            } else if self.config.editor.is_modal() {
                Mode::Visual
            } else {
                self.mode
            };
            self.snap_visual_sel();
        } else {
            self.caret = d;
            self.mouse_anchor = Some(d);
            self.affinity = Affinity::Inside;
            if !self.mode.is_insert() && self.config.editor.is_modal() {
                self.mode = Mode::Normal;
                self.sel = None;
                self.visual_anchor = None;
            } else {
                self.sel = None;
            }
        }
        self.mouse_dragging = true;
        self.clamp_caret();
        self.follow_caret = false;
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub fn drag_display(&mut self, d: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        if self.link_open {
            self.link_open = false;
            self.link_draft.clear();
        }
        if !self.mouse_dragging {
            return;
        }
        let p = self.proj();
        let d = d.min(p.display.len());
        // Excel-style table rectangle while dragging inside the same table.
        if let Some((aix, ar, ac)) = self.table_anchor {
            if let Some((block, cell)) = p.table_cell_at(d) {
                if p.blocks.get(aix).is_some_and(|b| b.source == block.source) {
                    if cell.row == ar && cell.col == ac {
                        // Same cell: text selection, not a cell rectangle.
                        self.table_sel = Some(TableRect {
                            block_ix: aix,
                            r0: ar,
                            c0: ac,
                            r1: ar,
                            c1: ac,
                        });
                        let anchor = self.mouse_anchor.unwrap_or(self.caret);
                        self.visual_anchor = Some(anchor);
                        self.caret = d;
                        self.snap_visual_sel();
                        self.clamp_caret();
                        self.follow_caret = false;
                        self.focus.focus(window, cx);
                        cx.notify();
                        return;
                    }
                    self.table_sel = Some(
                        TableRect {
                            block_ix: aix,
                            r0: ar,
                            c0: ac,
                            r1: cell.row,
                            c1: cell.col,
                        }
                        .normalize(),
                    );
                    self.caret = d;
                    if self.table_sel.is_some_and(|t| t.is_multi()) {
                        self.sel = Some(self.table_sel_display_range());
                        if self.config.editor.is_modal() && !self.mode.is_insert() {
                            self.mode = if self.config.editor == EditorKind::Helix {
                                Mode::Select
                            } else {
                                Mode::Visual
                            };
                        }
                    } else {
                        self.sel = None;
                    }
                    self.clamp_caret();
                    self.follow_caret = false;
                    self.focus.focus(window, cx);
                    cx.notify();
                    return;
                }
            }
            // Left the table — fall through to linear selection.
            self.table_sel = None;
            self.table_anchor = None;
        }
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            self.mode = if self.config.editor == EditorKind::Helix {
                Mode::Select
            } else {
                Mode::Visual
            };
        }
        match self.mouse_granularity {
            SelectGranularity::Char => {
                let anchor = self.mouse_anchor.unwrap_or(self.caret);
                self.visual_anchor = Some(anchor);
                self.caret = d;
                self.snap_visual_sel();
            }
            SelectGranularity::Word | SelectGranularity::Line => {
                let unit = self.mouse_unit.clone().unwrap_or(d..d);
                let range = extend_select_unit(&p.display, unit, d, self.mouse_granularity);
                self.sel = Some(range.clone());
                self.caret = if d < range.start {
                    range.start
                } else {
                    range.end
                };
                self.visual_anchor = Some(range.start);
            }
        }
        self.clamp_caret();
        self.follow_caret = false;
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Union of display ranges for cells in `table_sel` (for clipboard / bubble).
    fn table_sel_display_range(&self) -> Range<usize> {
        let Some(rect) = self.table_sel.map(|r| r.normalize()) else {
            return self.caret..self.caret;
        };
        let p = self.proj();
        let Some(block) = p.blocks.get(rect.block_ix) else {
            return self.caret..self.caret;
        };
        let BlockExtra::Table { cells, .. } = &block.extra else {
            return self.caret..self.caret;
        };
        let mut start = usize::MAX;
        let mut end = 0usize;
        for c in cells {
            if rect.contains(c.row, c.col) {
                start = start.min(c.display.start);
                end = end.max(c.display.end);
            }
        }
        if start == usize::MAX {
            self.caret..self.caret
        } else {
            start..end
        }
    }

    fn slash_query(&self) -> Option<String> {
        let p = self.proj();
        let d = self.caret;
        let block = p.block_at_display(d)?;
        let body = &p.display[block.display.clone()];
        let local = floor_char_boundary(body, d.saturating_sub(block.display.start).min(body.len()));
        let line_start = body[..local]
            .rfind(['\n', crate::display::TABLE_CELL_BR])
            .map(|i| i + 1)
            .unwrap_or(0);
        crate::document::slash_query(&body[line_start..local]).map(|s| s.to_string())
    }

    fn slash_items(&self) -> Vec<&'static SlashItem> {
        let q = self.slash_query().unwrap_or_default();
        slash::filter(&q)
    }

    fn slash_is_open(&self) -> bool {
        (self.mode.is_insert() || self.is_notion())
            && self.slash_query().is_some()
            && !self.slash_items().is_empty()
    }

    fn current_slash_pick(&self) -> Option<&'static SlashItem> {
        if !self.slash_is_open() {
            return None;
        }
        let items = self.slash_items();
        slash::selected(&items, self.slash_index)
    }

    fn apply_slash(
        &mut self,
        item: &'static SlashItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push_doc_undo();
        self.caret = self.doc.apply_slash(self.caret, item.template);
        self.sync_gfm();
        self.caret = self.caret.min(self.proj().display.len());
        // Fresh `<details>` starts expanded in-session so the user can fill
        // the summary/body; the file stays without `open` (collapsed on reload
        // per GFM) unless they add `open` in source view.
        if item.template.contains("<details") {
            let p = self.proj();
            if let Some(b) = p.block_at_display(self.caret) {
                if matches!(b.extra, BlockExtra::Details { .. }) {
                    self.expanded_html.insert(b.source.start);
                }
            }
        }
        self.slash_index = 0;
        self.last_slash_query.clear();
        self.mark_dirty();
        self.refresh_raw(window, cx);
    }

    fn close_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.proj();
        if let Some((display, _)) = {
            let d = self.caret.min(p.display.len());
            p.block_at_display(d).and_then(|block| {
                let body = p.display.get(block.display.clone())?;
                let local = floor_char_boundary(
                    body,
                    d.saturating_sub(block.display.start).min(body.len()),
                );
                let line_start = body[..local]
                    .rfind(['\n', crate::display::TABLE_CELL_BR])
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let rel = body[line_start..local].rfind('/')?;
                let d0 = block.display.start + line_start + rel;
                let d1 = block.display.start + local;
                Some((d0..d1, ()))
            })
        } {
            self.push_doc_undo();
            self.caret = self.doc.delete_display(display);
            self.sync_gfm();
        }
        self.slash_index = 0;
        self.last_slash_query.clear();
        self.mark_dirty();
        self.refresh_raw(window, cx);
    }

    fn leave_insert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_notion() {
            self.follow_caret = true;
            cx.notify();
            return;
        }
        if self.mode.is_visual() {
            self.mode = Mode::Normal;
            self.sel = None;
            self.visual_anchor = None;
            self.refresh(window, cx);
            return;
        }
        if !self.mode.is_insert() {
            return;
        }
        self.finish_insert_undo();
        self.mode = Mode::Normal;
        self.clear_pending();
        self.visual_anchor = None;
        self.sel = None;
        self.refresh(window, cx);
    }

    fn enter_insert(&mut self, caret: Caret, window: &mut Window, cx: &mut Context<Self>) {
        if self.insert_origin.is_none() {
            self.insert_origin = Some(self.snapshot());
        }
        self.mode = Mode::Insert;
        self.sel = None;
        self.visual_anchor = None;
        self.clear_pending();
        match caret {
            Caret::Start => self.caret = 0,
            Caret::End => self.caret = self.proj().display.len(),
            Caret::Offset(n) => self.caret = n.min(self.proj().display.len()),
        }
        self.refresh(window, cx);
    }

    fn apply_buffer_motion(&mut self, motion: Motion, window: &mut Window, cx: &mut Context<Self>) {
        self.move_caret(motion, false, window, cx);
    }

    fn move_caret(
        &mut self,
        motion: Motion,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.command.is_some()
            || self.search.is_some()
            || self.link_open
            || self.cmd_palette.is_some()
            || self.view_source
        {
            return;
        }
        window.prevent_default();
        let count = take_count(&mut self.pending_count);
        self.pending_g = false;
        let vertical = matches!(motion, Motion::Up | Motion::Down);
        if !vertical {
            self.goal_x = None;
        }
        let p = self.proj();
        let d = self.caret;
        let wrap = self.wrap_cols();
        let mut next_d = if vertical {
            let mut at = d;
            for _ in 0..count.max(1) {
                if let Some(n) = self.layout_move_vertical(at, motion == Motion::Down) {
                    at = n;
                } else {
                    let step = apply_motion(&p.display, at, motion, 1, wrap);
                    if step == at {
                        break;
                    }
                    at = step;
                }
            }
            at
        } else {
            apply_motion(&p.display, d, motion, count, wrap)
        };
        let insert_like = self.mode.is_insert() || self.is_notion();
        if insert_like && next_d == d {
            if let Some(w) = whichwrap(&p.display, d, motion) {
                next_d = w;
            }
        }
        if motion == Motion::Right {
            let at_mark_end = p.marks_at(d, Affinity::Inside).any()
                && !p.marks_at(next_d, Affinity::Inside).any();
            self.affinity = if at_mark_end {
                Affinity::Outside
            } else {
                Affinity::Inside
            };
        } else {
            self.affinity = Affinity::Inside;
        }
        let next = next_d;
        if extend {
            if self.is_modal_nav() && !self.mode.is_visual() {
                self.mode = if self.config.editor == EditorKind::Helix {
                    Mode::Select
                } else {
                    Mode::Visual
                };
            }
            if self.visual_anchor.is_none() {
                self.visual_anchor = Some(self.caret);
            }
            self.caret = next;
            self.snap_visual_sel();
            self.refresh_raw(window, cx);
        } else {
            self.land(next, window, cx);
        }
        self.pending_d = false;
        // Linewise `>`/`<`/`=` only combines with j/k/G/gg (handled in
        // their actions); any other motion abandons the armed operator.
        self.pending_op = None;
    }

    /// Prefer real TextLayout wraps over the char-column estimate so up/down
    /// stay on the painted visual row within a soft-wrapped block. Resolves
    /// across word-piece hits via the shared nearest-glyph lookup, then
    /// requires row progress inside the same block (edge rows fall back to
    /// char-column motion).
    fn layout_move_vertical(&mut self, from: usize, down: bool) -> Option<usize> {
        let hit = surface::piece_for_offset(&self.hits, from)?;
        let local = from.saturating_sub(hit.display_start).min(hit.doc_len);
        let layout = hit.layout.clone();
        let pos = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            layout.position_for_index(local)
        }))
        .ok()
        .flatten()?;
        let line_h =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| layout.line_height())).ok()?;
        if line_h <= px(0.) {
            return None;
        }
        let goal = self.goal_x.get_or_insert(pos.x);
        let target_y = if down {
            pos.y + line_h + (line_h * 0.5)
        } else {
            pos.y - (line_h * 0.5)
        };
        let target = point(*goal, target_y);
        let next = surface::index_for_point(&self.hits, target)?;
        if next == from {
            return None;
        }
        // Same block, and strictly onto another visual row.
        let p = self.proj();
        let blk = p.block_at_display(from)?;
        if next < blk.display.start || next > blk.display.end {
            return None;
        }
        let hit2 = surface::piece_for_offset(&self.hits, next)?;
        let local2 = next.saturating_sub(hit2.display_start).min(hit2.doc_len);
        let pos2 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hit2.layout.position_for_index(local2)
        }))
        .ok()
        .flatten()?;
        let dy: f32 = (pos2.y - pos.y).into();
        if down && dy <= 0.5 {
            return None;
        }
        if !down && dy >= -0.5 {
            return None;
        }
        Some(next)
    }

    fn go_doc_edge(&mut self, last: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            return;
        }
        let count = self.pending_count.take();
        self.clear_pending();
        self.mode = Mode::Normal;
        self.visual_anchor = None;
        self.sel = None;
        let pos = if let Some(c) = count {
            line_start_n(&self.source, c)
        } else if last {
            last_line_start(&self.source)
        } else {
            0
        };
        let d = project(&self.source).to_display(pos);
        self.land(d, window, cx);
    }

    fn click_range(
        &mut self,
        range: Range<usize>,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = range.start.min(self.source.len());
        let end = range.end.min(self.source.len());
        if shift {
            if self.visual_anchor.is_none() {
                self.visual_anchor = Some(self.caret);
            }
            self.caret = start;
            self.mode = if self.config.editor == EditorKind::Helix {
                Mode::Select
            } else {
                Mode::Visual
            };
            self.snap_visual_sel();
        } else {
            self.caret = start;
            self.mouse_anchor = Some(start);
            if !self.mode.is_insert() && self.config.editor.is_modal() {
                self.mode = Mode::Normal;
                self.sel = None;
                self.visual_anchor = None;
            }
            let _ = end;
        }
        self.mouse_dragging = true;
        self.refresh_raw(window, cx);
    }

    fn drag_to_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mouse_dragging {
            return;
        }
        let anchor = self.mouse_anchor.unwrap_or(self.caret);
        let at = if range.start >= anchor {
            range.end
        } else {
            range.start
        };
        self.visual_anchor = Some(anchor);
        self.caret = at.min(self.source.len());
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            self.mode = if self.config.editor == EditorKind::Helix {
                Mode::Select
            } else {
                Mode::Visual
            };
        }
        self.snap_visual_sel();
        self.refresh_raw(window, cx);
    }

    fn on_leave_insert(&mut self, _: &LeaveInsert, window: &mut Window, cx: &mut Context<Self>) {
        if self.command.is_some() {
            self.cancel_command(window, cx);
            return;
        }
        if self.search.is_some() {
            self.cancel_search(window, cx);
            return;
        }
        if self.cmd_palette.is_some() {
            self.cancel_palette(window, cx);
            return;
        }
        if self.view_source {
            self.view_source = false;
            cx.notify();
            return;
        }
        if self.pending_replace.is_some()
            || self.pending_find.is_some()
            || self.pending_bracket.is_some()
            || self.pending_op.is_some()
            || self.pending_obj.is_some()
            || self.pending_change
        {
            self.clear_pending();
            cx.notify();
            return;
        }
        if self.settings_open {
            self.settings_open = false;
            cx.notify();
            return;
        }
        if self.pending_chord {
            self.pending_chord = false;
            self.status = if self.dirty { "unsaved" } else { "ready" }.into();
            cx.notify();
            return;
        }
        if self.slash_is_open() {
            self.close_slash(window, cx);
            return;
        }
        window.prevent_default();
        self.leave_insert(window, cx);
    }

    fn on_insert_caret(&mut self, _: &InsertAtCaret, window: &mut Window, cx: &mut Context<Self>) {
        // `i` after `d`/`c` (or in Vim visual) starts a text object (`diw`).
        if self.arm_obj_inner(true, window, cx) {
            cx.stop_propagation();
            return;
        }
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.enter_insert(Caret::Offset(self.caret), window, cx);
    }

    fn on_insert_after(
        &mut self,
        _: &InsertAfterCaret,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // `a` after `d`/`c` (or in Vim visual) = around (`daw`).
        if self.arm_obj_inner(false, window, cx) {
            cx.stop_propagation();
            return;
        }
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let off = {
            let p = self.proj();
            let d = self.caret;
            // Stay on this logical line — never jump to the next block.
            let d2 = after_caret_same_line(&p.display, d);
            p.to_source(d2, Affinity::Inside)
        };
        self.enter_insert(Caret::Offset(off), window, cx);
    }

    fn on_insert_line_start(
        &mut self,
        _: &InsertLineStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_modal_nav() {
            return;
        }
        let p = self.proj();
        let d = self.caret;
        let range = logical_line_range(&p.display, d);
        let off = first_non_blank_in(&p.display, range);
        let off = p.to_source(off, Affinity::Inside);
        self.enter_insert(Caret::Offset(off), window, cx);
    }

    fn on_insert_line_end(
        &mut self,
        _: &InsertLineEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_modal_nav() {
            return;
        }
        let p = self.proj();
        let d = self.caret;
        let end = logical_line_range(&p.display, d).end;
        let off = p.to_source(end, Affinity::Inside);
        self.enter_insert(Caret::Offset(off), window, cx);
    }

    fn open_line(&mut self, above: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.push_doc_undo();
        let caret = self.doc.open_line(self.caret, above);
        self.sync_gfm();
        self.mark_dirty();
        self.enter_insert(Caret::Offset(caret), window, cx);
    }

    fn on_open_below(&mut self, _: &OpenBelow, window: &mut Window, cx: &mut Context<Self>) {
        self.open_line(false, window, cx);
    }
    fn on_open_above(&mut self, _: &OpenAbove, window: &mut Window, cx: &mut Context<Self>) {
        self.open_line(true, window, cx);
    }
    fn on_char_left(&mut self, _: &CharLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_g {
            self.pending_g = false;
            self.apply_buffer_motion(Motion::LineStart, window, cx);
            return;
        }
        self.apply_buffer_motion(Motion::Left, window, cx);
    }
    fn on_char_right(&mut self, _: &CharRight, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_g {
            self.pending_g = false;
            self.apply_buffer_motion(Motion::LineEnd, window, cx);
            return;
        }
        self.apply_buffer_motion(Motion::Right, window, cx);
    }
    fn on_line_down(&mut self, _: &LineDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.slash_is_open() {
            self.on_slash_next(&SlashNext, window, cx);
            return;
        }
        if self.pending_op.is_some() && self.is_modal_nav() {
            window.prevent_default();
            self.consume_pending_op_for_lines(1, false, false, window, cx);
            return;
        }
        if self.mode == Mode::VisualLine {
            window.prevent_default();
            let sel = self
                .sel
                .clone()
                .unwrap_or_else(|| visual_line_range(&self.source, self.caret));
            let next = extend_visual_line(&self.source, sel, 1);
            self.sel = Some(next.clone());
            self.caret = next.end.min(self.source.len());
            self.refresh_raw(window, cx);
            return;
        }
        self.apply_buffer_motion(Motion::Down, window, cx);
    }
    fn on_line_up(&mut self, _: &LineUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.slash_is_open() {
            self.on_slash_prev(&SlashPrev, window, cx);
            return;
        }
        if self.pending_op.is_some() && self.is_modal_nav() {
            window.prevent_default();
            self.consume_pending_op_for_lines(-1, false, false, window, cx);
            return;
        }
        if self.mode == Mode::VisualLine {
            window.prevent_default();
            let sel = self
                .sel
                .clone()
                .unwrap_or_else(|| visual_line_range(&self.source, self.caret));
            let next = extend_visual_line(&self.source, sel, -1);
            self.sel = Some(next.clone());
            self.caret = next.start;
            self.refresh_raw(window, cx);
            return;
        }
        self.apply_buffer_motion(Motion::Up, window, cx);
    }
    fn on_word_forward(&mut self, _: &WordForward, window: &mut Window, cx: &mut Context<Self>) {
        // `w` completes a text object (`viw`, `diw`, `miw`).
        if self.pending_obj.is_some_and(|(_, inner)| inner.is_some()) && self.is_modal_nav() {
            window.prevent_default();
            cx.stop_propagation();
            self.commit_text_object('w', window, cx);
            return;
        }
        self.apply_buffer_motion(Motion::WordForward, window, cx);
    }
    fn on_word_back(&mut self, _: &WordBack, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_buffer_motion(Motion::WordBack, window, cx);
    }
    fn on_word_end(&mut self, _: &WordEnd, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_g && self.config.editor == EditorKind::Helix {
            self.pending_g = false;
            self.pending_count = None;
            self.go_doc_edge(true, window, cx);
            return;
        }
        self.apply_buffer_motion(Motion::WordEnd, window, cx);
    }
    fn on_word_forward_ws(
        &mut self,
        _: &WordForwardWs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // `W` completes a WORD object (`viW`, `daW`).
        if self.pending_obj.is_some_and(|(_, inner)| inner.is_some()) && self.is_modal_nav() {
            window.prevent_default();
            cx.stop_propagation();
            self.commit_text_object('W', window, cx);
            return;
        }
        self.apply_buffer_motion(Motion::WordForwardWs, window, cx);
    }
    fn on_word_back_ws(&mut self, _: &WordBackWs, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_buffer_motion(Motion::WordBackWs, window, cx);
    }
    fn on_word_end_ws(&mut self, _: &WordEndWs, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_buffer_motion(Motion::WordEndWs, window, cx);
    }
    fn on_line_start(&mut self, _: &LineStart, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_buffer_motion(Motion::LineStart, window, cx);
    }
    fn on_line_first_non_blank(
        &mut self,
        _: &LineFirstNonBlank,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_buffer_motion(Motion::LineFirstNonBlank, window, cx);
    }
    fn on_line_end(&mut self, _: &LineEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_buffer_motion(Motion::LineEnd, window, cx);
    }
    fn on_first_doc(&mut self, _: &FirstDoc, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_op.is_some() && self.is_modal_nav() {
            window.prevent_default();
            self.consume_pending_op_for_lines(-1, false, true, window, cx);
            return;
        }
        self.go_doc_edge(false, window, cx);
    }
    fn on_last_doc(&mut self, _: &LastDoc, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_op.is_some() && self.is_modal_nav() {
            window.prevent_default();
            self.consume_pending_op_for_lines(1, true, false, window, cx);
            return;
        }
        self.go_doc_edge(true, window, cx);
    }
    fn on_pending_g(&mut self, _: &PendingG, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.pending_d = false;
        if self.pending_g {
            if self.pending_op.is_some() {
                self.consume_pending_op_for_lines(-1, false, true, window, cx);
                return;
            }
            self.go_doc_edge(false, window, cx);
        } else {
            self.pending_g = true;
            cx.notify();
        }
    }

    fn on_digit(&mut self, d: u8, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if d == 0 && self.pending_count.is_none() {
            self.apply_buffer_motion(Motion::LineStart, window, cx);
            return;
        }
        self.pending_count = push_count(self.pending_count, d);
        cx.notify();
    }
    fn on_digit0(&mut self, _: &Digit0, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(0, window, cx);
    }
    fn on_digit1(&mut self, _: &Digit1, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(1, window, cx);
    }
    fn on_digit2(&mut self, _: &Digit2, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(2, window, cx);
    }
    fn on_digit3(&mut self, _: &Digit3, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(3, window, cx);
    }
    fn on_digit4(&mut self, _: &Digit4, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(4, window, cx);
    }
    fn on_digit5(&mut self, _: &Digit5, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(5, window, cx);
    }
    fn on_digit6(&mut self, _: &Digit6, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(6, window, cx);
    }
    fn on_digit7(&mut self, _: &Digit7, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(7, window, cx);
    }
    fn on_digit8(&mut self, _: &Digit8, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(8, window, cx);
    }
    fn on_digit9(&mut self, _: &Digit9, window: &mut Window, cx: &mut Context<Self>) {
        self.on_digit(9, window, cx);
    }

    fn delete_selection_or_char(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Tree mutation on display offsets: splicing GFM `source` with display
        // offsets corrupts lists/tables (markers are hidden in display).
        let range = if let Some(sel) = self.sel.clone() {
            sel
        } else {
            block_caret_range(&self.proj().display, self.caret)
        };
        self.push_doc_undo();
        let caret = if range.start == range.end {
            self.doc.delete_char(self.caret)
        } else {
            self.doc.delete_display(range)
        };
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    fn delete_current_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.unit_ix_at(self.caret);
        self.push_doc_undo();
        let caret = self.doc.delete_unit(ix);
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    /// Unit (block / list item) index containing display offset `d`.
    fn unit_ix_at(&self, d: usize) -> usize {
        let p = self.proj();
        let us = crate::tree::units(&p);
        us.iter()
            .position(|u| {
                let r = crate::tree::unit_display(&p, *u);
                d >= r.start && d <= r.end
            })
            .unwrap_or(0)
    }

    fn on_select_line(&mut self, _: &SelectLine, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.editor != EditorKind::Helix || !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.clear_pending();
        if self.mode != Mode::Select {
            self.mode = Mode::Select;
            let range = visual_line_range(&self.source, self.caret);
            self.visual_anchor = Some(range.start);
            self.sel = Some(range.clone());
            self.caret = range.end.min(self.source.len());
        } else {
            let sel = self
                .sel
                .clone()
                .unwrap_or_else(|| visual_line_range(&self.source, self.caret));
            let next = extend_visual_line(&self.source, sel, 1);
            self.sel = Some(next.clone());
            self.caret = next.end.min(self.source.len());
        }
        self.refresh_raw(window, cx);
    }

    fn on_delete_op(&mut self, _: &DeleteOp, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if self.mode.is_visual() {
            self.delete_selection_or_char(window, cx);
            return;
        }
        if self.config.editor == EditorKind::Helix {
            self.delete_selection_or_char(window, cx);
            return;
        }
        if self.pending_d {
            self.pending_d = false;
            self.delete_current_line(window, cx);
        } else {
            self.pending_g = false;
            self.pending_d = true;
            cx.notify();
        }
    }

    fn on_delete_char(&mut self, _: &DeleteChar, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.editor != EditorKind::Vim || !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.clear_pending();
        self.delete_selection_or_char(window, cx);
    }

    fn on_delete_to_end(&mut self, _: &DeleteToEnd, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.editor != EditorKind::Vim || !self.is_modal_nav() {
            return;
        }
        let start = self.caret;
        let p = self.proj();
        let end = crate::tree::units(&p)
            .iter()
            .find_map(|u| {
                let r = crate::tree::unit_display(&p, *u);
                (start >= r.start && start <= r.end).then_some(r.end)
            })
            .unwrap_or(p.display.len());
        self.push_doc_undo();
        let caret = self.doc.delete_display(start..end);
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    fn on_visual_char(&mut self, _: &VisualChar, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.clear_pending();
        self.visual_anchor = Some(self.caret);
        self.mode = if self.config.editor == EditorKind::Helix {
            Mode::Select
        } else {
            Mode::Visual
        };
        self.sel = None;
        self.refresh_raw(window, cx);
    }

    fn on_visual_line(&mut self, _: &VisualLine, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.editor != EditorKind::Vim || !self.is_modal_nav() {
            return;
        }
        self.clear_pending();
        if self.mode == Mode::VisualLine {
            let sel = self
                .sel
                .clone()
                .unwrap_or_else(|| visual_line_range(&self.source, self.caret));
            let next = extend_visual_line(&self.source, sel, 1);
            self.sel = Some(next.clone());
            self.caret = next.end.min(self.source.len());
        } else {
            let range = visual_line_range(&self.source, self.caret);
            self.mode = Mode::VisualLine;
            self.visual_anchor = Some(range.start);
            self.sel = Some(range.clone());
            self.caret = range.end.min(self.source.len());
        }
        self.refresh_raw(window, cx);
    }

    fn on_undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            // A focused panel input owns undo; never touch the doc behind it.
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.finish_insert_undo();
        let current = self.snapshot();
        if let Some(prev) = self.undo.undo(current) {
            self.apply_snapshot(prev, window, cx);
        }
    }
    fn on_redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let current = self.snapshot();
        if let Some(next) = self.undo.redo(current) {
            self.apply_snapshot(next, window, cx);
        }
    }

    fn on_slash_prev(&mut self, _: &SlashPrev, window: &mut Window, cx: &mut Context<Self>) {
        if self.slash_is_open() {
            window.prevent_default();
            let n = self.slash_items().len();
            self.slash_index = slash::move_index(self.slash_index, n, -1);
            self.scroll_slash_to_selected();
            cx.notify();
            return;
        }
        if self.is_modal_nav() {
            window.prevent_default();
            self.apply_buffer_motion(Motion::Up, window, cx);
            return;
        }
        cx.propagate();
    }
    fn on_slash_next(&mut self, _: &SlashNext, window: &mut Window, cx: &mut Context<Self>) {
        if self.slash_is_open() {
            window.prevent_default();
            let n = self.slash_items().len();
            self.slash_index = slash::move_index(self.slash_index, n, 1);
            self.scroll_slash_to_selected();
            cx.notify();
            return;
        }
        if self.is_modal_nav() {
            window.prevent_default();
            self.apply_buffer_motion(Motion::Down, window, cx);
            return;
        }
        cx.propagate();
    }
    fn on_slash_apply(&mut self, _: &SlashApply, window: &mut Window, cx: &mut Context<Self>) {
        if !self.slash_is_open() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if let Some(item) = self.current_slash_pick() {
            self.apply_slash(item, window, cx);
        }
    }

    fn on_insert_slash(&mut self, _: &InsertSlash, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            // The Alt/Path toolbar (or settings) owns `/` — typing a path
            // like `assets/clip.mp4` must reach the focused input instead
            // of inserting a slash block into the markdown behind it.
            cx.propagate();
            return;
        }
        if self.command.is_some()
            || self.search.is_some()
            || self.link_open
            || self.cmd_palette.is_some()
            || self.view_source
        {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if self.is_modal_nav() {
            self.enter_insert(Caret::Offset(self.caret), window, cx);
        }
        if self.insert_origin.is_none() {
            self.insert_origin = Some(self.snapshot());
        }
        self.caret = self
            .doc
            .insert_text(self.caret, self.sel.clone(), "/", self.sticky);
        self.sync_gfm();
        self.sel = None;
        self.mark_dirty();
        let q = self.slash_query().unwrap_or_default();
        if q != self.last_slash_query {
            self.last_slash_query = q;
            self.slash_index = 0;
            self.slash_scroll.scroll_to_item(0);
        }
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn on_delete_word_back(
        &mut self,
        _: &DeleteWordBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay_input_focused(window, cx) {
            // Focused panel input handles its own keys; never edit the
            // markdown behind the panel.
            cx.propagate();
            return;
        }
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        if self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
            self.push_doc_undo();
            let d0 = sel.start.min(sel.end);
            let d1 = sel.end.max(sel.start);
            let caret = self.doc.delete_display(d0..d1);
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
            return;
        }
        let p = self.proj();
        let d = self.caret;
        if d == 0 {
            return;
        }

        // Notion: like cmd-backspace, but word-granularity. Never cross the
        // current block/item — clearing the last word leaves an empty slot;
        // a second opt-backspace at the empty start joins/exits structurally.
        if self.is_notion() {
            let Some(block) = p.block_at_display(d) else {
                return;
            };
            let unit_start = if let BlockExtra::List { items, .. } = &block.extra {
                items
                    .iter()
                    .find(|it| d >= it.display.start && d <= it.display.end)
                    .map(|it| it.display.start)
                    .unwrap_or(block.display.start)
            } else {
                block.display.start
            };
            if d <= unit_start {
                self.on_block_backspace(&BlockBackspace, window, cx);
                return;
            }
            let start_d = apply_motion(&p.display, d, Motion::WordBack, 1, None).max(unit_start);
            if start_d >= d {
                return;
            }
            self.push_doc_undo();
            let caret = self.doc.delete_display(start_d..d);
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
            return;
        }

        let start_d = apply_motion(&p.display, d, Motion::WordBack, 1, None);
        self.push_doc_undo();
        let caret = self.doc.delete_display(start_d..d);
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    fn on_delete_line_back(
        &mut self,
        _: &DeleteLineBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        if self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
            self.push_doc_undo();
            let d0 = sel.start.min(sel.end);
            let d1 = sel.end.max(sel.start);
            let caret = self.doc.delete_display(d0..d1);
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
            return;
        }
        if self.is_notion() {
            let p = self.proj();
            let d = self.caret;
            let Some(block) = p.block_at_display(d) else {
                return;
            };
            let (unit_start, unit_end) = if let BlockExtra::List { items, .. } = &block.extra {
                items
                    .iter()
                    .find(|it| d >= it.display.start && d <= it.display.end)
                    .map(|it| (it.display.start, it.display.end))
                    .unwrap_or((block.display.start, block.display.end))
            } else {
                (block.display.start, block.display.end)
            };
            if d <= unit_start {
                self.on_block_backspace(&BlockBackspace, window, cx);
                return;
            }
            // Delete from the start of the current visual line within the unit.
            let body = &p.display[unit_start..unit_end.min(p.display.len())];
            let local = d.saturating_sub(unit_start).min(body.len());
            let line_start = body[..local].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let start_d = unit_start + line_start;
            if start_d >= d {
                self.on_block_backspace(&BlockBackspace, window, cx);
                return;
            }
            self.push_doc_undo();
            // From the unit start: clear the whole unit (avoids leaving a stray
            // last character when the caret sits on it after a code fence).
            // Mid-line cmd-backspace still deletes only to the caret.
            let caret = if start_d <= unit_start {
                self.doc.delete_display(unit_start..unit_end)
            } else {
                self.doc.delete_display(start_d..d)
            };
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
            return;
        }
        let p = self.proj();
        let d = self.caret;
        let start_d = logical_line_range(&p.display, d).start;
        if start_d >= d {
            // At line start — delete previous newline (join) via normal backspace.
            self.on_block_backspace(&BlockBackspace, window, cx);
            return;
        }
        self.push_doc_undo();
        let caret = self.doc.delete_display(start_d..d);
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    fn on_block_backspace(
        &mut self,
        _: &BlockBackspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        if !self.mode.is_insert() && !self.is_notion() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
            self.push_doc_undo();
            let d0 = sel.start.min(sel.end);
            let d1 = sel.end.max(sel.start);
            let caret = self.doc.delete_display(d0..d1);
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
            return;
        }
        self.push_doc_undo();
        let caret = if let Some(c) = self.doc.backspace(self.caret) {
            c
        } else {
            self.doc.delete_char(self.caret)
        };
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    fn on_open_search(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search(false, window, cx);
    }
    fn on_search_back(&mut self, _: &SearchBack, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search(true, window, cx);
    }
    fn open_search(&mut self, backward: bool, window: &mut Window, cx: &mut Context<Self>) {
        window.prevent_default();
        self.clear_pending();
        self.command = None;
        self.search = Some((LineField::new(), backward));
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn cancel_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search = None;
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn submit_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Enter cycles forward (shift+enter is handled at capture level and
        // calls `cycle_search(false)` directly). Keep the bar open so repeated
        // enter keeps cycling; esc closes.
        self.cycle_search(true, window, cx);
    }
    /// Cycle the open search bar without closing it: commit the typed query
    /// to `last_search`, then jump. Used by enter (forward) / shift+enter
    /// (backward) and the footer ‹ › buttons.
    fn cycle_search(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let backward = self.search.as_ref().map(|s| s.1).unwrap_or(false);
        let typed = self
            .search
            .as_ref()
            .map(|s| s.0.as_str().to_string())
            .unwrap_or_default();
        let q = if typed.is_empty() {
            match &self.last_search {
                Some((prev, _)) => prev.clone(),
                None => return,
            }
        } else {
            typed
        };
        self.last_search = Some((q, backward));
        // `jump_search` takes an absolute direction; the bar's `backward`
        // flag (opened with `?`) flips the first jump only. Cycling with
        // enter always means forward-from-caret / backward-from-caret.
        self.jump_search(forward, true, window, cx);
        cx.notify();
    }
    /// Live query for highlights + footer count: the open bar's text when
    /// non-empty, otherwise the last submitted search.
    fn active_search_query(&self) -> Option<String> {
        if let Some((buf, _)) = self.search.as_ref() {
            if !buf.is_empty() {
                return Some(buf.as_str().to_string());
            }
        }
        self.last_search.as_ref().map(|s| s.0.clone())
    }
    /// All non-overlapping matches of the active query in display text.
    /// Capped so a huge file + 1-char query can't stall the frame.
    fn search_all_matches(&self) -> Vec<std::ops::Range<usize>> {
        let Some(q) = self.active_search_query() else {
            return Vec::new();
        };
        if q.is_empty() {
            return Vec::new();
        }
        let display = self.proj().display;
        let mut out = Vec::new();
        let mut from = 0usize;
        while from <= display.len() && out.len() < 2000 {
            let Some(rel) = display[from..].find(&q) else {
                break;
            };
            let start = from + rel;
            out.push(start..start + q.len());
            from = start + q.len().max(1);
        }
        out
    }
    /// `(current_1_based, total)` for the footer counter. Current is the
    /// match at-or-after the caret (the one enter would land on).
    fn search_position(&self) -> Option<(usize, usize)> {
        let matches = self.search_all_matches();
        if matches.is_empty() {
            return None;
        }
        let total = matches.len();
        let cur = matches
            .iter()
            .position(|r| r.start >= self.caret)
            .unwrap_or(0);
        Some((cur + 1, total))
    }
    fn jump_search(
        &mut self,
        forward: bool,
        wrap: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((query, _)) = self.last_search.clone() else {
            return;
        };
        let p = self.proj();
        let from = self.caret;
        let found = if forward {
            let start = if from < p.display.len() { from + 1 } else { 0 };
            search_next(&p.display, start, &query, wrap)
                .or_else(|| search_next(&p.display, 0, &query, false))
        } else {
            search_prev(&p.display, from, &query, wrap)
        };
        if let Some(range) = found {
            self.mode = Mode::Normal;
            self.visual_anchor = None;
            self.sel = None;
            self.caret = range.start;
            self.refresh(window, cx);
        } else {
            self.status = format!("no match: {query}").into();
        }
        cx.notify();
    }
    fn on_search_next(&mut self, _: &SearchNext, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let back = self.last_search.as_ref().map(|s| s.1).unwrap_or(false);
        self.jump_search(!back, true, window, cx);
    }
    fn on_search_prev(&mut self, _: &SearchPrev, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let back = self.last_search.as_ref().map(|s| s.1).unwrap_or(false);
        self.jump_search(back, true, window, cx);
    }

    fn on_join_lines(&mut self, _: &JoinLines, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let count = take_count(&mut self.pending_count);
        self.clear_pending();
        self.push_doc_undo();
        if let Some(sel) = self.sel.clone() {
            let caret = self.doc.join_range(sel);
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
        } else {
            let caret = self.doc.join_lines(self.caret, count);
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
        }
    }

    fn on_replace_char(&mut self, _: &ReplaceChar, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let count = take_count(&mut self.pending_count);
        self.pending_g = false;
        self.pending_d = false;
        self.pending_find = None;
        self.pending_replace = Some(count);
        cx.notify();
    }
    fn commit_replace(&mut self, ch: char, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.pending_replace.take().unwrap_or(1);
        self.push_doc_undo();
        let caret = if let Some(sel) = self.sel.clone() {
            self.doc.replace_range(sel, ch)
        } else {
            self.doc.replace_chars(self.caret, count, ch)
        };
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    fn on_find(&mut self, kind: FindKind, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let count = take_count(&mut self.pending_count);
        self.pending_g = false;
        self.pending_d = false;
        self.pending_replace = None;
        self.pending_find = Some((kind, count));
        cx.notify();
    }
    fn on_find_forward(&mut self, _: &FindForward, window: &mut Window, cx: &mut Context<Self>) {
        self.on_find(FindKind::Forward, window, cx);
    }
    fn on_find_backward(&mut self, _: &FindBackward, window: &mut Window, cx: &mut Context<Self>) {
        self.on_find(FindKind::Backward, window, cx);
    }
    fn on_find_till(&mut self, _: &FindTill, window: &mut Window, cx: &mut Context<Self>) {
        // `t` doubles as the `cmd-k t` chord follow-up. Keymap dispatch runs
        // before capture handlers, so in Normal mode this action fires first
        // (which is why the chord only worked in Notion) — consume the chord
        // here and stop propagation so capture doesn't see it twice.
        if self.pending_chord {
            self.pending_chord = false;
            window.prevent_default();
            cx.stop_propagation();
            self.open_themes_picker(window, cx);
            return;
        }
        self.on_find(FindKind::Till, window, cx);
    }
    fn on_find_till_back(&mut self, _: &FindTillBack, window: &mut Window, cx: &mut Context<Self>) {
        // Same chord race as `t` (covers `cmd-k shift-t`).
        if self.pending_chord {
            self.pending_chord = false;
            window.prevent_default();
            cx.stop_propagation();
            self.open_themes_picker(window, cx);
            return;
        }
        self.on_find(FindKind::TillBack, window, cx);
    }
    fn commit_find(&mut self, ch: char, window: &mut Window, cx: &mut Context<Self>) {
        let Some((kind, count)) = self.pending_find.take() else {
            return;
        };
        self.last_find = Some((kind, ch));
        self.perform_find(kind, ch, count, window, cx);
    }
    fn perform_find(
        &mut self,
        kind: FindKind,
        ch: char,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line_only = self.config.editor == EditorKind::Vim;
        if let Some(next) = find_char(&self.source, self.caret, ch, kind, count, line_only) {
            self.land(next, window, cx);
        }
        cx.notify();
    }
    fn on_repeat_find(&mut self, _: &RepeatFind, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let count = take_count(&mut self.pending_count);
        if let Some((kind, ch)) = self.last_find {
            self.perform_find(kind, ch, count, window, cx);
        }
    }
    fn on_reverse_find(&mut self, _: &ReverseFind, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let count = take_count(&mut self.pending_count);
        if let Some((kind, ch)) = self.last_find {
            self.perform_find(kind.reverse(), ch, count, window, cx);
        }
    }

    fn on_bracket_open(&mut self, _: &BracketOpen, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.pending_find = None;
        self.pending_replace = None;
        self.pending_bracket = Some(-1);
        cx.notify();
    }
    fn on_bracket_close(&mut self, _: &BracketClose, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.pending_find = None;
        self.pending_replace = None;
        self.pending_bracket = Some(1);
        cx.notify();
    }
    fn commit_bracket(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.pending_bracket.take() else {
            return;
        };
        let count = take_count(&mut self.pending_count);
        let p = self.proj();
        let d = self.caret;
        let next_d = match key {
            "p" => {
                let src_next = paragraph_jump(&p.display, d, dir, count);
                src_next
            }
            "h" => {
                let mut ix = p
                    .blocks
                    .iter()
                    .position(|b| d >= b.display.start && d <= b.display.end)
                    .unwrap_or(0);
                let mut left = count.max(1);
                if dir >= 0 {
                    while left > 0 && ix + 1 < p.blocks.len() {
                        ix += 1;
                        if matches!(p.blocks[ix].kind, BlockKind::Heading(_)) {
                            left -= 1;
                        }
                    }
                } else {
                    while left > 0 && ix > 0 {
                        ix -= 1;
                        if matches!(p.blocks[ix].kind, BlockKind::Heading(_)) {
                            left -= 1;
                        }
                    }
                }
                p.blocks.get(ix).map(|b| b.display.start).unwrap_or(d)
            }
            _ => {
                self.pending_count = None;
                cx.notify();
                return;
            }
        };
        let next = p.to_source(next_d, Affinity::Inside);
        self.land(next, window, cx);
    }

    fn reset_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = config::default_ui_font();
        let md = config::default_markdown_font();
        let buf = config::default_buffer_font();
        self.config.ui_font = ui.clone();
        self.config.markdown_font = md.clone();
        self.config.buffer_font = buf.clone();
        self.fonts.ui_family.update(cx, |s, cx| {
            s.set_value(ui.family, window, cx);
        });
        self.fonts.ui_size.update(cx, |s, cx| {
            s.set_value(ui.size.to_string(), window, cx);
        });
        self.fonts.markdown_family.update(cx, |s, cx| {
            s.set_value(md.family, window, cx);
        });
        self.fonts.markdown_size.update(cx, |s, cx| {
            s.set_value(md.size.to_string(), window, cx);
        });
        self.fonts.buffer_family.update(cx, |s, cx| {
            s.set_value(buf.family, window, cx);
        });
        self.fonts.buffer_size.update(cx, |s, cx| {
            s.set_value(buf.size.to_string(), window, cx);
        });
        self.persist_config(cx);
        cx.notify();
    }

    /// Reset one font field (family or size) back to its default.
    fn reset_font_field(
        &mut self,
        slot: FontSlot,
        field: FontField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let def = match slot {
            FontSlot::Ui => config::default_ui_font(),
            FontSlot::Markdown => config::default_markdown_font(),
            FontSlot::Buffer => config::default_buffer_font(),
        };
        match (slot, field) {
            (FontSlot::Ui, FontField::Family) => {
                self.config.ui_font.family = def.family.clone();
                self.fonts.ui_family.update(cx, |s, cx| {
                    s.set_value(def.family, window, cx);
                });
            }
            (FontSlot::Ui, FontField::Size) => {
                self.config.ui_font.size = def.size;
                self.fonts.ui_size.update(cx, |s, cx| {
                    s.set_value(def.size.to_string(), window, cx);
                });
            }
            (FontSlot::Markdown, FontField::Family) => {
                self.config.markdown_font.family = def.family.clone();
                self.fonts.markdown_family.update(cx, |s, cx| {
                    s.set_value(def.family, window, cx);
                });
            }
            (FontSlot::Markdown, FontField::Size) => {
                self.config.markdown_font.size = def.size;
                self.fonts.markdown_size.update(cx, |s, cx| {
                    s.set_value(def.size.to_string(), window, cx);
                });
            }
            (FontSlot::Buffer, FontField::Family) => {
                self.config.buffer_font.family = def.family.clone();
                self.fonts.buffer_family.update(cx, |s, cx| {
                    s.set_value(def.family, window, cx);
                });
            }
            (FontSlot::Buffer, FontField::Size) => {
                self.config.buffer_font.size = def.size;
                self.fonts.buffer_size.update(cx, |s, cx| {
                    s.set_value(def.size.to_string(), window, cx);
                });
            }
        }
        self.persist_config(cx);
        cx.notify();
    }

    fn apply_font_family(&mut self, slot: FontSlot, family: String, cx: &mut Context<Self>) {
        let family = family.trim().to_string();
        if family.is_empty() {
            return;
        }
        let target = match slot {
            FontSlot::Ui => &mut self.config.ui_font.family,
            FontSlot::Markdown => &mut self.config.markdown_font.family,
            FontSlot::Buffer => &mut self.config.buffer_font.family,
        };
        if *target == family {
            return;
        }
        *target = family;
        self.persist_config(cx);
        cx.notify();
    }

    fn apply_font_size(&mut self, slot: FontSlot, size: u32, cx: &mut Context<Self>) {
        let size = size.clamp(8, 48);
        let target = match slot {
            FontSlot::Ui => &mut self.config.ui_font.size,
            FontSlot::Markdown => &mut self.config.markdown_font.size,
            FontSlot::Buffer => &mut self.config.buffer_font.size,
        };
        if *target == size {
            return;
        }
        *target = size;
        self.persist_config(cx);
        cx.notify();
    }

    fn on_toggle_settings(
        &mut self,
        _: &ToggleSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_settings(window, cx);
    }
    fn on_open_palette(&mut self, _: &OpenPalette, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_palette(window, cx);
    }
    /// Zed-style `cmd-k t` chord (handled at capture level, before keymap
    /// dispatch): `cmd-k` arms it, `t` opens the Themes picker straight
    /// away, esc cancels, anything else behaves normally. `cmd-shift-p`
    /// remains the full command palette.
    fn open_themes_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteMode::Themes, window, cx);
    }
    /// `cmd-k cmd-t` races the Workspace `cmd-t` → NewTab binding (keymap
    /// dispatch runs before the editor capture handler), so `WorkspaceShell`
    /// asks the active tab first: consume a pending `cmd-k` chord and open
    /// Themes instead of a new tab. Returns true when consumed.
    pub(crate) fn consume_chord_for_new_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.pending_chord {
            return false;
        }
        self.pending_chord = false;
        window.prevent_default();
        self.open_themes_picker(window, cx);
        true
    }
    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        if self.settings_open {
            self.cmd_palette = None;
        }
        if !self.settings_open {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }
    fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteMode::Root, window, cx);
    }
    fn on_toggle_source(
        &mut self,
        _: &ToggleSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.prevent_default();
        // Dismiss transient UI so the raw source gets the full surface.
        self.cmd_palette = None;
        self.palette_theme_backup = None;
        self.command = None;
        self.search = None;
        self.toggle_source(window, cx);
    }
    fn toggle_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.view_source = !self.view_source;
        if self.view_source {
            self.status = "source view — esc/cmd-shift-v to exit".into();
        } else {
            self.status = if self.dirty { "unsaved" } else { "ready" }.into();
        }
        self.focus.focus(window, cx);
        cx.notify();
    }
    /// Open the palette, optionally straight into a submenu (settings theme
    /// picker opens Themes directly).
    fn open_palette(&mut self, mode: PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.cmd_palette.is_some() {
            self.cancel_palette(window, cx);
            return;
        }
        if self.view_source {
            window.prevent_default();
            self.view_source = false;
            self.status = if self.dirty { "unsaved" } else { "ready" }.into();
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }
        window.prevent_default();
        self.clear_pending();
        self.command = None;
        self.search = None;
        self.link_open = false;
        self.settings_open = false;
        let mut state = PaletteState::open_in(mode);
        if mode == PaletteMode::Themes {
            // Park the cursor on the current theme instead of row 0.
            if let Some(ix) = theme::index_of(&self.config.theme) {
                state.index = ix;
            }
        }
        self.cmd_palette = Some(state);
        self.palette_theme_backup = Some(self.config.theme.clone());
        self.scroll_palette_to_selected();
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cmd_palette = None;
        self.palette_theme_backup = None;
        self.focus.focus(window, cx);
        cx.notify();
    }
    /// Dismiss without committing: restore the theme live-previewed in the
    /// Themes list (preview never touches disk, so this just re-applies).
    fn cancel_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(orig) = self.palette_theme_backup.take() {
            if self.config.theme != orig {
                self.preview_theme(&orig, window, cx);
            }
        }
        self.cmd_palette = None;
        self.focus.focus(window, cx);
        cx.notify();
    }
    /// Back out of a submenu to root without committing theme previews.
    fn palette_back_to_root(&mut self, cx: &mut Context<Self>) {
        if let Some(orig) = self.palette_theme_backup.clone() {
            if self.config.theme != orig {
                // Re-apply only; keep backup so a later esc still knows origin.
                if let Ok(palette) = theme::load_named(&orig) {
                    apply_palette(&palette, cx);
                    self.palette = palette;
                    self.config.theme = orig;
                }
            }
        }
        if let Some(state) = self.cmd_palette.as_mut() {
            state.set_mode(PaletteMode::Root);
        }
        cx.notify();
    }
    fn submit_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.cmd_palette.as_ref() else {
            return;
        };
        let items = state.items(self.view_source);
        let Some(item) = items.get(state.index).copied() else {
            return;
        };
        self.run_palette_action(item.action, window, cx);
    }
    fn run_palette_action(
        &mut self,
        action: PaletteAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            PaletteAction::OpenThemes => {
                if self.palette_theme_backup.is_none() {
                    self.palette_theme_backup = Some(self.config.theme.clone());
                }
                if let Some(state) = self.cmd_palette.as_mut() {
                    state.set_mode(PaletteMode::Themes);
                    // Park the cursor on the current theme instead of row 0.
                    if let Some(ix) = theme::index_of(&self.config.theme) {
                        let len = state.items(self.view_source).len();
                        if ix < len {
                            state.index = ix;
                        }
                    }
                }
                self.scroll_palette_to_selected();
                cx.notify();
            }
            PaletteAction::OpenEditors => {
                if let Some(state) = self.cmd_palette.as_mut() {
                    state.set_mode(PaletteMode::Editors);
                }
                cx.notify();
            }
            PaletteAction::ToggleFullWidth => {
                self.config.full_width = !self.config.full_width;
                self.persist_config(cx);
                self.status = if self.config.full_width {
                    "full width".into()
                } else {
                    "column width".into()
                };
                self.close_palette(window, cx);
            }
            PaletteAction::ToggleSource => {
                self.close_palette(window, cx);
                self.toggle_source(window, cx);
            }
            PaletteAction::OpenSettings => {
                // Discard any uncommitted theme preview before leaving.
                if let Some(orig) = self.palette_theme_backup.take() {
                    if self.config.theme != orig {
                        self.preview_theme(&orig, window, cx);
                    }
                }
                self.cmd_palette = None;
                self.settings_open = true;
                self.focus.focus(window, cx);
                cx.notify();
            }
            PaletteAction::SetTheme(name) => {
                self.close_palette(window, cx);
                self.set_theme(name, window, cx);
            }
            PaletteAction::SetEditor(kind) => {
                self.close_palette(window, cx);
                self.set_editor(kind, window, cx);
            }
        }
    }
    fn set_editor(&mut self, editor: EditorKind, window: &mut Window, cx: &mut Context<Self>) {
        let was_notion = self.is_notion();
        self.config.editor = editor;
        self.persist_config(cx);
        if editor == EditorKind::Notion {
            // Keep the current position instead of jumping to the end.
            let at = self.caret;
            self.enter_insert(Caret::Offset(at), window, cx);
        } else if was_notion {
            self.leave_insert(window, cx);
        }
        cx.notify();
    }
    fn set_wrap_motions(&mut self, wrap: bool, _window: &mut Window, cx: &mut Context<Self>) {
        self.config.wrap_motions = wrap;
        self.persist_config(cx);
        cx.notify();
    }
    fn set_full_width(&mut self, full: bool, cx: &mut Context<Self>) {
        self.config.full_width = full;
        self.persist_config(cx);
        self.status = if full { "full width".into() } else { "column width".into() };
        cx.notify();
    }
    /// Live theme preview while hovering / moving through the Themes list.
    /// Preview-only: applies immediately without persisting. Enter / click
    /// commits via `set_theme`; esc / dismiss restores the backup.
    fn preview_palette_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let action = self
            .cmd_palette
            .as_ref()
            .filter(|s| s.mode == PaletteMode::Themes)
            .and_then(|s| s.items(self.view_source).get(s.index).copied())
            .map(|item| item.action);
        if let Some(PaletteAction::SetTheme(name)) = action {
            if !self.config.theme.eq_ignore_ascii_case(name) {
                self.preview_theme(name, window, cx);
            }
        }
    }
    /// Preview/revert path: swap the palette with zero disk I/O, no settings-
    /// input churn and no forced refresh (`cx.notify` from the caller coalesces
    /// into the single repaint for this step). `set_theme` below is the only
    /// committing path.
    fn preview_theme(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        match theme::load_named(name) {
            Ok(palette) => {
                apply_palette(&palette, cx);
                self.palette = palette;
                self.mermaid.clear();
                self.config.theme = name.to_string();
                window.refresh();
            }
            Err(err) => {
                self.status = format!("{err}").into();
                cx.notify();
            }
        }
    }
    /// Keep the keyboard-selected palette row in view (up/down + filtering).
    /// Hover doesn't scroll — the mouse user owns the wheel.
    fn scroll_palette_to_selected(&self) {
        let Some(state) = self.cmd_palette.as_ref() else {
            return;
        };
        let n = state.items(self.view_source).len();
        if n == 0 {
            return;
        }
        self.palette_scroll
            .scroll_to_item(state.index.min(n.saturating_sub(1)));
    }
    /// Keep the keyboard-selected slash row in view (up/down + filtering).
    /// Hover doesn't scroll — the mouse user owns the wheel.
    fn scroll_slash_to_selected(&self) {
        let n = self.slash_items().len();
        if n == 0 {
            return;
        }
        self.slash_scroll
            .scroll_to_item(self.slash_index.min(n.saturating_sub(1)));
    }
    /// Floating-style placement for the `/` popover: prefer below the caret
    /// block, flip above when there isn't room (judged against the minimum
    /// height), and shrink the menu to whatever space the winning side has.
    /// Returns `(above, max_h)`.
    fn slash_placement(&self) -> (bool, Pixels) {
        const MIN_H: f32 = 120.;
        const MAX_H: f32 = 304.;
        const GAP: f32 = 8.;
        let viewport = self.scroll_handle.bounds();
        if viewport.size.height <= px(0.) {
            return (false, px(MAX_H));
        }
        let cap = (viewport.size.height - px(16.)).min(px(MAX_H));
        let cap = cap.max(px(80.));
        let p = self.proj();
        let d = self.caret;
        let ix = wysiwyg::units(&p).iter().position(|u| {
            let r = wysiwyg::unit_display(&p, *u);
            d >= r.start && d <= r.end
        });
        let Some(ix) = ix else {
            return (false, cap);
        };
        let Some(b) = self.scroll_handle.bounds_for_item(ix) else {
            return (false, cap);
        };
        let offset = self.scroll_handle.offset();
        let block_top = b.top() + offset.y;
        let block_bottom = b.bottom() + offset.y;
        let below = viewport.bottom() - block_bottom - px(GAP);
        let above = block_top - viewport.top() - px(GAP);
        let flip = below < px(MIN_H) && above > below;
        let avail = if flip { above } else { below };
        // Shrink to the winning side; never exceed the viewport.
        let h = avail.max(px(0.)).min(cap);
        // Floor so the menu stays usable in tiny spaces (may overflow by a
        // few px only when there is no room on either side).
        (flip, h.max(px(80.)).min(cap.max(px(80.))))
    }
    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.write_to_disk(cx) {
            self.status = "saved".into();
        }
        self.sync_title(window);
    }
    fn write_to_disk(&mut self, cx: &mut Context<Self>) -> bool {
        let ok = match std::fs::write(&self.path, self.source.as_bytes()) {
            Ok(()) => {
                self.clean_source = self.source.clone();
                self.dirty = false;
                true
            }
            Err(err) => {
                self.status = format!("save failed: {err}").into();
                false
            }
        };
        cx.notify();
        ok
    }
    fn on_open_command(&mut self, _: &OpenCommand, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        self.clear_pending();
        self.command = Some(LineField::new());
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn cancel_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command = None;
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn submit_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.command.take() else {
            return;
        };
        self.focus.focus(window, cx);
        match mode::parse_ex(input.as_str()) {
            ExCommand::Cancel => {}
            ExCommand::Write => {
                if self.write_to_disk(cx) {
                    self.status = "written".into();
                }
                self.sync_title(window);
            }
            ExCommand::WriteQuit => {
                if self.write_to_disk(cx) {
                    self.status = "written".into();
                    self.sync_title(window);
                    cx.quit();
                } else {
                    self.sync_title(window);
                }
            }
            ExCommand::Unknown(s) => {
                self.status = format!("unknown command: {s}").into();
            }
        }
        cx.notify();
    }
    fn set_theme(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        match theme::load_named(name) {
            Ok(palette) => {
                apply_palette(&palette, cx);
                self.palette = palette;
                self.mermaid.clear();
                self.config.theme = name.to_string();
                let label = name.to_string();
                self.theme_input.update(cx, |s, cx| {
                    s.set_value(label, window, cx);
                });
                self.persist_config(cx);
                window.refresh();
                cx.notify();
            }
            Err(err) => {
                self.status = format!("{err}").into();
                cx.notify();
            }
        }
    }
    fn sync_title(&self, window: &mut Window) {
        window.set_window_title(&window_title(&self.path, self.dirty));
    }

    /// Tab-shell accessors (multi-buffer UI in `crate::tabs`).
    pub fn file_path(&self) -> &PathBuf {
        &self.path
    }

    pub fn tab_title(&self) -> String {
        self.path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn save_now(&mut self, cx: &mut Context<Self>) -> bool {
        self.write_to_disk(cx)
    }

    pub fn sync_title_now(&self, window: &mut Window) {
        self.sync_title(window);
    }

    /// Zed-style jump for an already-open tab.
    pub fn jump_to(
        &mut self,
        line: usize,
        col: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let src = source_offset_for_line_col(&self.source, line, col);
        let d = self.proj().to_display(src);
        self.caret = d.min(self.proj().display.len());
        self.sel = None;
        if !self.config.editor.is_modal() {
            self.enter_insert(Caret::Offset(self.caret), window, cx);
        } else {
            self.mode = Mode::Normal;
            self.refresh(window, cx);
        }
        self.sync_title(window);
    }

    fn toggle_task(
        &mut self,
        block_ix: usize,
        item_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mouse_dragging = false;
        self.mouse_unit = None;
        self.sel = None;
        self.follow_caret = false;
        self.push_doc_undo();
        self.doc.toggle_task(block_ix, item_ix);
        self.sync_gfm();
        self.mark_dirty();
        self.sync_title(window);
        self.clamp_caret();
        if self.mode.extends_selection() {
            self.snap_visual_sel();
        }
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn insert_image_line(&mut self, filename: &str, window: &mut Window, cx: &mut Context<Self>) {
        let alt = Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image");
        self.push_doc_undo();
        let caret = self
            .doc
            .insert_image(self.caret, alt.to_string(), filename.to_string());
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    /// Clicking an image/video selects it and loads the toolbar inputs.
    fn select_media(&mut self, display_start: usize, window: &mut Window, cx: &mut Context<Self>) {
        let found = {
            let p = self.proj();
            p.block_at_display(display_start).map(|b| match &b.extra {
                BlockExtra::Image { alt, src } => Some((alt.clone(), src.clone())),
                BlockExtra::Html => self
                    .source
                    .get(b.source.clone())
                    .and_then(|raw| {
                        images::parse_video_src(raw)
                            .or_else(|| images::parse_html_src(raw))
                            .or_else(|| images::parse_img_src(raw).map(|(s, _)| s))
                    })
                    .map(|src| (String::new(), src)),
                _ => None,
            })
        };
        let Some(Some((alt, src))) = found else {
            return;
        };
        self.media_sel = Some(display_start);
        self.caret = display_start;
        self.sel = None;
        self.mouse_dragging = false;
        self.clamp_caret();
        self.follow_caret = false;
        self.media_alt.update(cx, |st, cx| {
            st.set_value(alt, window, cx);
        });
        self.media_src.update(cx, |st, cx| {
            st.set_value(src, window, cx);
        });
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Apply the toolbar's alt + path to the selected image/video.
    fn apply_media_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(at) = self.media_sel else {
            return;
        };
        let alt = self.media_alt.read(cx).value().to_string();
        let src = self.media_src.read(cx).value().trim().to_string();
        if src.is_empty() {
            return;
        }
        let is_html = self
            .proj()
            .block_at_display(at)
            .is_some_and(|b| matches!(&b.extra, BlockExtra::Html));
        self.push_doc_undo();
        if is_html {
            if let Some(caret) = self.doc.update_html_video_src(at, src) {
                self.sync_gfm();
                self.commit_caret(caret, window, cx);
            }
        } else {
            let caret = self.doc.update_image(at, alt, src);
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
        }
        self.media_sel = None;
    }

    fn try_paste_image(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(clip) = cx.read_from_clipboard() else {
            return false;
        };
        let mut did = false;
        for entry in clip.entries() {
            let ClipboardEntry::Image(image) = entry else {
                continue;
            };
            let ext = image.format.extension();
            let preferred = images::timestamped_name(&images::now_stamp(), ext);
            match images::write_beside(&self.path, &preferred, &image.bytes) {
                Ok(name) => {
                    self.insert_image_line(&name, window, cx);
                    did = true;
                }
                Err(err) => {
                    self.status = format!("image paste failed: {err}").into();
                    cx.notify();
                }
            }
        }
        did
    }

    fn import_paths(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        for path in paths {
            if !images::is_media_path(path) {
                continue;
            }
            match images::place_dropped(&self.path, path) {
                Ok(name) => self.insert_image_line(&name, window, cx),
                Err(err) => {
                    self.status = format!("drop failed: {err}").into();
                    cx.notify();
                }
            }
        }
    }
}

impl Workspace {
    /// True when a panel/input owns the keyboard: settings is open, or a
    /// media-toolbar input is focused. Doc-mutating actions must propagate
    /// so the focused input handles its own keys.
    fn overlay_input_focused(&self, window: &mut Window, cx: &App) -> bool {
        if self.settings_open {
            return true;
        }
        [&self.media_alt, &self.media_src].iter().any(|e| {
            e.read(cx).focus_handle(cx).is_focused(window)
        })
    }

    /// Eat non-digit keystrokes when a font-size input is focused (settings).
    /// Family inputs still take any text; sizes are numeric-only.
    fn eat_non_digit_in_size(
        &self,
        key: &str,
        mods: gpui::Modifiers,
        window: &mut Window,
        cx: &App,
    ) -> bool {
        if mods.platform || mods.control || mods.alt || mods.shift {
            return false;
        }
        if key.chars().count() != 1 {
            return false;
        }
        let ch = key.chars().next().unwrap_or('\0');
        if ch.is_control() || ch.is_ascii_digit() {
            return false;
        }
        let size_focused = [
            &self.fonts.ui_size,
            &self.fonts.markdown_size,
            &self.fonts.buffer_size,
        ]
        .iter()
        .any(|e| e.read(cx).focus_handle(cx).is_focused(window));
        if size_focused {
            window.prevent_default();
            true
        } else {
            false
        }
    }

    fn handle_capture_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = ev.keystroke.key.as_str();
        let mods = ev.keystroke.modifiers;

        // Zed-style `cmd-k` chord starter at capture level (GPUI keymap
        // dispatch proved unreliable for this chord, so it is handled
        // manually here): `cmd-k` then `t` opens Themes. Note capture runs
        // *after* keymap dispatch, so the `t` follow-up races the Normal-mode
        // `t` (till) binding — `on_find_till`/`on_find_till_back` consume a
        // pending chord first (see below); this capture path covers
        // Notion/insert where `t` has no binding.
        if (key == "k" || key == "K")
            && (mods.platform || mods.control)
            && !mods.shift
            && !mods.alt
            && !self.pending_chord
            && self.cmd_palette.is_none()
            && self.command.is_none()
            && self.search.is_none()
            && !self.link_open
            && !self.view_source
            && !self.settings_open
            && !self.overlay_input_focused(window, cx)
        {
            window.prevent_default();
            self.pending_chord = true;
            self.status = "cmd-k … (t themes)".into();
            cx.notify();
            return true;
        }

        // Chord follow-up: `t` opens Themes (plain or with cmd/ctrl still
        // held, so `cmd-k t` and `cmd-k cmd-t` both work), esc cancels
        // (handled by LeaveInsert), anything else falls through to normal
        // handling.
        if self.pending_chord {
            self.pending_chord = false;
            let clean = self.command.is_none()
                && self.search.is_none()
                && self.cmd_palette.is_none()
                && !self.link_open
                && !self.view_source
                && !self.settings_open;
            if clean && key.eq_ignore_ascii_case("t") && !mods.alt {
                window.prevent_default();
                self.open_themes_picker(window, cx);
                return true;
            }
            self.status = if self.dirty { "unsaved" } else { "ready" }.into();
            // Fall through: process this key normally.
        }

        // Let cmd/ctrl-c/x/v reach Copy/Cut/Paste actions (including overlays).
        if matches!(key, "c" | "x" | "v")
            && (mods.platform || mods.control)
            && !mods.alt
            && !mods.shift
        {
            return false;
        }

        // Settings font sizes are numeric-only at the key level.
        if self.settings_open
            && self.command.is_none()
            && self.search.is_none()
            && self.cmd_palette.is_none()
            && !self.link_open
            && self.eat_non_digit_in_size(key, mods, window, cx)
        {
            return true;
        }

        if self.cmd_palette.is_some() {
            window.prevent_default();
            if (key == "p")
                && (mods.platform || mods.control)
                && mods.shift
                && !mods.alt
            {
                self.cancel_palette(window, cx);
                return true;
            }
            // Caret-aware single-line editing (opt/cmd-backspace, opt/cmd-arrows).
            if let Some(state) = self.cmd_palette.as_mut() {
                let edited = state.query.delete_key(key, mods);
                let moved = !edited && state.query.caret_key(key, mods);
                if edited || moved {
                    if edited {
                        state.index = 0;
                    }
                    cx.notify();
                    if edited {
                        self.scroll_palette_to_selected();
                    }
                    return true;
                }
            }
            if mods.control || mods.platform || mods.alt {
                return true;
            }
            let view_source = self.view_source;
            let len = self
                .cmd_palette
                .as_ref()
                .map(|s| s.items(view_source).len())
                .unwrap_or(0);
            match key {
                "escape" => {
                    let back = self
                        .cmd_palette
                        .as_ref()
                        .is_some_and(|s| s.mode != PaletteMode::Root);
                    if back {
                        self.palette_back_to_root(cx);
                    } else {
                        self.cancel_palette(window, cx);
                    }
                }
                "enter" => self.submit_palette(window, cx),
                "up" => {
                    if let Some(state) = self.cmd_palette.as_mut() {
                        state.move_by(-1, len);
                    }
                    self.preview_palette_theme(window, cx);
                    self.scroll_palette_to_selected();
                    cx.notify();
                }
                "down" => {
                    if let Some(state) = self.cmd_palette.as_mut() {
                        state.move_by(1, len);
                    }
                    self.preview_palette_theme(window, cx);
                    self.scroll_palette_to_selected();
                    cx.notify();
                }
                "backspace" => {
                    // Empty query in a submenu goes back to root.
                    let back = self.cmd_palette.as_ref().is_some_and(|state| {
                        state.query.is_empty() && state.mode != PaletteMode::Root
                    });
                    if back {
                        self.palette_back_to_root(cx);
                    } else if let Some(state) = self.cmd_palette.as_mut() {
                        state.index = 0;
                        cx.notify();
                    }
                }
                "space" => {
                    if let Some(state) = self.cmd_palette.as_mut() {
                        state.query.insert_char(' ');
                        state.index = 0;
                    }
                    self.scroll_palette_to_selected();
                    cx.notify();
                }
                k if k.chars().count() == 1 => {
                    if let Some(ch) = k.chars().next() {
                        if !ch.is_control() {
                            if let Some(state) = self.cmd_palette.as_mut() {
                                state.query.insert_char(ch);
                                state.index = 0;
                            }
                            self.scroll_palette_to_selected();
                            cx.notify();
                        }
                    }
                }
                _ => {}
            }
            return true;
        }

        if self.link_open {
            window.prevent_default();
            if self.link_draft.delete_key(key, mods) || self.link_draft.caret_key(key, mods) {
                cx.notify();
                return true;
            }
            match key {
                "escape" => {
                    self.link_open = false;
                    self.link_draft.clear();
                    cx.notify();
                }
                "enter" => self.commit_link(window, cx),
                "backspace" => {
                    self.link_draft.backspace();
                    cx.notify();
                }
                "space" => {
                    self.link_draft.insert_char(' ');
                    cx.notify();
                }
                k if k.chars().count() == 1 && !mods.control && !mods.platform => {
                    if let Some(ch) = k.chars().next() {
                        if !ch.is_control() {
                            self.link_draft.insert_char(ch);
                            cx.notify();
                        }
                    }
                }
                _ => {}
            }
            return true;
        }

        if self.pending_replace.is_some() && self.is_modal_nav() {
            window.prevent_default();
            if key == "escape" {
                self.pending_replace = None;
                cx.notify();
                return true;
            }
            if mods.control || mods.platform || mods.alt {
                return true;
            }
            let ch = if key == "space" {
                Some(' ')
            } else if key.chars().count() == 1 {
                key.chars().next()
            } else {
                None
            };
            if let Some(ch) = ch {
                if !ch.is_control() {
                    self.commit_replace(ch, window, cx);
                }
            }
            return true;
        }

        if self.pending_find.is_some() && self.is_modal_nav() {
            window.prevent_default();
            if key == "escape" {
                self.pending_find = None;
                cx.notify();
                return true;
            }
            if mods.control || mods.platform || mods.alt {
                return true;
            }
            let ch = if key == "space" {
                Some(' ')
            } else if key.chars().count() == 1 {
                key.chars().next()
            } else {
                None
            };
            if let Some(ch) = ch {
                if !ch.is_control() {
                    self.commit_find(ch, window, cx);
                }
            }
            return true;
        }

        if self.pending_bracket.is_some() && self.is_modal_nav() {
            window.prevent_default();
            if key == "escape" {
                self.pending_bracket = None;
                cx.notify();
                return true;
            }
            self.commit_bracket(key, window, cx);
            return true;
        }

        // Text-object completion (`di"`, `ci{`, `va)`, …): delimiter keys
        // have no Normal bindings, so they arrive here. `i`/`a`/`w`/`W`
        // are consumed in their actions (which run first); esc cancels via
        // LeaveInsert → clear_pending.
        if self.pending_obj.is_some_and(|(_, inner)| inner.is_some()) && self.is_modal_nav() {
            if !mods.control && !mods.platform && !mods.alt && key.chars().count() == 1 {
                if let Some(ch) = key.chars().next() {
                    if matches!(
                        ch,
                        '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | 'b' | 'B'
                    ) {
                        window.prevent_default();
                        self.commit_text_object(ch, window, cx);
                        return true;
                    }
                }
            }
        }

        if self.pending_g && self.is_modal_nav() && key == "s" && !mods.control && !mods.platform {
            window.prevent_default();
            self.pending_g = false;
            self.apply_buffer_motion(Motion::LineFirstNonBlank, window, cx);
            return true;
        }

        if self.command.is_some() {
            window.prevent_default();
            if let Some(buf) = self.command.as_mut() {
                if buf.delete_key(key, mods) || buf.caret_key(key, mods) {
                    cx.notify();
                    return true;
                }
            }
            if mods.control || mods.platform || mods.alt {
                return true;
            }
            match key {
                "escape" => self.cancel_command(window, cx),
                "enter" => self.submit_command(window, cx),
                "backspace" => {
                    if let Some(buf) = self.command.as_mut() {
                        buf.backspace();
                    }
                    cx.notify();
                }
                "space" => {
                    if let Some(buf) = self.command.as_mut() {
                        buf.insert_char(' ');
                    }
                    cx.notify();
                }
                k if k.chars().count() == 1 => {
                    if let Some(ch) = k.chars().next() {
                        if !ch.is_control() {
                            if let Some(buf) = self.command.as_mut() {
                                buf.insert_char(ch);
                            }
                            cx.notify();
                        }
                    }
                }
                _ => {}
            }
            return true;
        }

        if self.search.is_some() {
            window.prevent_default();
            if let Some((buf, _)) = self.search.as_mut() {
                if buf.delete_key(key, mods) || buf.caret_key(key, mods) {
                    cx.notify();
                    return true;
                }
            }
            if mods.control || mods.platform || mods.alt {
                return true;
            }
            match key {
                "escape" => self.cancel_search(window, cx),
                "enter" => {
                    if mods.shift {
                        self.cycle_search(false, window, cx);
                    } else {
                        self.submit_search(window, cx);
                    }
                }
                "backspace" => {
                    if let Some((buf, _)) = self.search.as_mut() {
                        buf.backspace();
                    }
                    cx.notify();
                }
                "space" => {
                    if let Some((buf, _)) = self.search.as_mut() {
                        buf.insert_char(' ');
                    }
                    cx.notify();
                }
                k if k.chars().count() == 1 => {
                    if let Some(ch) = k.chars().next() {
                        if !ch.is_control() {
                            if let Some((buf, _)) = self.search.as_mut() {
                                buf.insert_char(ch);
                            }
                            cx.notify();
                        }
                    }
                }
                _ => {}
            }
            return true;
        }

        if key == "enter"
            && !mods.control
            && !mods.platform
            && !mods.alt
            && (self.mode.is_insert() || self.is_notion())
            && self.command.is_none()
            && self.search.is_none()
            && self.cmd_palette.is_none()
            && !self.link_open
            && !self.view_source
        {
            window.prevent_default();
            self.insert_newline(mods.shift, window, cx);
            return true;
        }

        let insert_like = self.mode.is_insert() || self.is_notion();
        // Focused video block: `space` toggles play/pause instead of
        // inserting a space (YouTube-style). Skipped when an overlay owns
        // the keyboard, the slash menu is open, or the clip isn't ready —
        // `video_key_at` already requires a decoded clip.
        if key == "space"
            && !mods.platform
            && !mods.control
            && !mods.alt
            && self.command.is_none()
            && self.search.is_none()
            && self.cmd_palette.is_none()
            && !self.link_open
            && !self.view_source
            && !self.settings_open
            && !self.slash_is_open()
            && !self.overlay_input_focused(window, cx)
            && self.pending_replace.is_none()
            && self.pending_find.is_none()
            && self.pending_bracket.is_none()
            && !self.pending_g
        {
            if let Some(vkey) = self.video_key_at(self.caret) {
                window.prevent_default();
                self.toggle_video(&vkey, cx);
                return true;
            }
        }
        let can_nav = (self.is_modal_nav() || insert_like)
            && self.command.is_none()
            && self.search.is_none()
            && self.cmd_palette.is_none()
            && !self.link_open
            && !self.view_source
            && self.pending_replace.is_none()
            && self.pending_find.is_none()
            && self.pending_bracket.is_none();
        if can_nav {
            if self.slash_is_open() && (key == "up" || key == "down") {
                window.prevent_default();
                if key == "up" {
                    self.on_slash_prev(&SlashPrev, window, cx);
                } else {
                    self.on_slash_next(&SlashNext, window, cx);
                }
                return true;
            }
            let word = mods.alt || (mods.control && !mods.platform);
            let motion = match key {
                "left" if word => Some(Motion::WordBack),
                "right" if word => Some(Motion::WordForward),
                "left" if mods.platform => Some(Motion::LineStart),
                "right" if mods.platform => Some(Motion::LineEnd),
                "left" => Some(Motion::Left),
                "right" => Some(Motion::Right),
                "up" => Some(Motion::Up),
                "down" => Some(Motion::Down),
                "home" => Some(Motion::LineStart),
                "end" => Some(Motion::LineEnd),
                _ => None,
            };
            if let Some(motion) = motion {
                window.prevent_default();
                self.move_caret(motion, mods.shift, window, cx);
                return true;
            }
        }

        false
    }

    /// Positioned `/` popover: preferred side is below the `/` block, flips
    /// above when there is no room (see `slash_placement`). Height shrinks
    /// to fit the winning side; width is capped so it never leaves the view.
    fn render_slash_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let (above, _) = self.slash_placement();
        let mut anchor = div().absolute().left_0().occlude();
        anchor = if above {
            // bottom:100% pins the menu's bottom edge to the block's top
            // edge, so it sits fully above regardless of menu height.
            anchor.bottom(relative(1.)).mb_1()
        } else {
            anchor.top(px(28.))
        };
        deferred(
            anchor
                .on_scroll_wheel(|_, _, cx| {
                    cx.stop_propagation();
                })
                .child(self.render_slash_menu(cx)),
        )
        .with_priority(1)
        .into_any_element()
    }

    fn render_slash_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let items = self.slash_items();
        if items.is_empty() {
            return div().into_any_element();
        }
        let selected = slash::clamp_index(self.slash_index, items.len());
        let (_, max_h) = self.slash_placement();
        let p = &self.palette;
        v_flex()
            .w(px(340.))
            .max_w(relative(0.9))
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.background_panel)
            .shadow_lg()
            .overflow_hidden()
            .child(
                v_flex()
                    .id("slash-list")
                    .w_full()
                    // Clamp the scroll container itself (not the outer panel)
                    // so the viewport height is well-defined: thumb size and
                    // position are computed from this box vs its content.
                    .max_h(max_h)
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.slash_scroll)
                    .py_1()
                    .children(items.into_iter().enumerate().map(|(i, item)| {
                        let active = i == selected;
                        div()
                            .id(("slash-item", i))
                            // Never compress rows to fit: they must overflow
                            // so the scrollbar thumb reflects real content.
                            .flex_shrink_0()
                            .px_3()
                            .py_1()
                            .cursor_pointer()
                            .when(active, |el| el.bg(p.background_element))
                            .hover(|el| el.bg(p.background_element))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.apply_slash(item, window, cx);
                                }),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(icon_el(
                                        item.icon,
                                        if active { p.primary } else { p.text_muted },
                                    ))
                                    .child(div().flex_1().text_color(p.markdown_text).child(item.label))
                                    .child(div().text_xs().text_color(p.text_muted).child(item.hint)),
                            )
                    }))
                    // Scrollbar last: the overlay is an extra tracked
                    // child, so it must not shift item indices used
                    // by keyboard scroll-into-view.
                    .vertical_scrollbar(&self.slash_scroll),
            )
            .into_any_element()
    }

    fn render_link_chips(&self, source: &str, cx: &mut Context<Self>) -> AnyElement {
        let links = extract_links(source);
        if links.is_empty() {
            return div().into_any_element();
        }
        let p = &self.palette;
        h_flex()
            .w_full()
            .mt_1()
            .gap_2()
            .flex_wrap()
            .children(links.into_iter().enumerate().map(|(i, link)| {
                let url = link.url.clone();
                let label = if url.len() > 48 {
                    format!("↗ {}…", &url[..45])
                } else {
                    format!("↗ {url}")
                };
                div()
                    .id(("link-chip", i))
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.))
                    .bg(p.background_element)
                    .text_xs()
                    .text_color(p.markdown_link)
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.open_link_url(&url, window, cx);
                        }),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(icon_el("external-link", p.markdown_link))
                            .child(label),
                    )
            }))
            .into_any_element()
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette.clone();
        let pane = self.settings_pane;
        let editor = self.config.editor;
        let wrap = self.config.wrap_motions;
        let full_width = self.config.full_width;

        let nav_row = |label: &'static str, which: SettingsPane, cx: &mut Context<Self>| {
            let active = pane == which;
            div()
                .id(("settings-nav", which as usize))
                .w_full()
                .px_3()
                .py_1()
                .rounded(px(6.))
                .cursor_pointer()
                .when(active, |el| el.bg(p.background_element))
                .hover(|el| el.bg(p.background_element.opacity(0.6)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.settings_pane = which;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(if active {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if active { p.primary } else { p.markdown_text })
                        .child(label),
                )
        };

        let content = match pane {
            SettingsPane::Editor => {
                let entity = cx.entity();
                let entity2 = entity.clone();
                let card = |icon: &'static str,
                            label: &'static str,
                            desc: &'static str,
                            kind: EditorKind,
                            cx: &mut Context<Self>| {
                    let selected = editor == kind;
                    h_flex()
                        .id(("editor-card", kind as usize))
                        .w_full()
                        .p_3()
                        .gap_3()
                        .items_center()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(if selected { p.primary } else { p.border })
                        .when(selected, |el| el.bg(p.background_element.opacity(0.5)))
                        .hover(|el| el.bg(p.background_element.opacity(0.7)))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.set_editor(kind, window, cx);
                            }),
                        )
                        .child(icon_el(
                            icon,
                            if selected { p.primary } else { p.text_muted },
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(if selected {
                                            p.primary
                                        } else {
                                            p.markdown_text
                                        })
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.text_muted)
                                        .child(desc),
                                ),
                        )
                        .when(selected, |el| {
                            el.child(icon_el("check", p.primary))
                        })
                };
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Editor"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(p.text_muted)
                            .child("Helix and Vim keep a caret in the focused raw block. Notion is WYSIWYG."),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(card(
                                "helix",
                                "Helix",
                                "x select line · d delete · v select · u undo · U redo",
                                EditorKind::Helix,
                                cx,
                            ))
                            .child(card(
                                "vim",
                                "Vim",
                                "x delete char · dd line · v/V visual · u undo · ctrl-r redo",
                                EditorKind::Vim,
                                cx,
                            ))
                            .child(card(
                                "notion",
                                "Notion",
                                "Rendered blocks; edit visible text. j/k move between blocks.",
                                EditorKind::Notion,
                                cx,
                            )),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_3()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.markdown_text)
                                            .child("Wrap lines"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.text_muted)
                                            .child("Preview always wraps. This only affects the Markdown source view: on follows wrapped visual lines with j/k, off uses logical lines."),
                                    ),
                            )
                            .child(
                                Switch::new("wrap-motions")
                                    .small()
                                    .checked(wrap)
                                    .on_click(move |checked, window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.set_wrap_motions(*checked, window, cx);
                                        });
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_3()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.markdown_text)
                                            .child("Full width"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.text_muted)
                                            .child("When on, markdown fills the window. Off: centered column."),
                                    ),
                            )
                            .child(
                                Switch::new("full-width")
                                    .small()
                                    .checked(full_width)
                                    .on_click(move |checked, _, cx| {
                                        entity2.update(cx, |this, cx| {
                                            this.set_full_width(*checked, cx);
                                        });
                                    }),
                            ),
                    )
                    .into_any_element()
            }
            SettingsPane::Theme => v_flex()
                .w_full()
                .gap_3()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Theme"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(p.text_muted)
                        .child("Pick in the command palette — type to filter, preview as you move."),
                )
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_8()
                                .px_2()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(p.border)
                                .bg(p.background_element.opacity(0.5))
                                .flex()
                                .items_center()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(p.markdown_text)
                                        .child(self.config.theme.clone()),
                                ),
                        )
                        .child(
                            Button::new("theme-browse")
                                .small()
                                .h_8()
                                .label("Choose…")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.settings_open = false;
                                    this.cmd_palette =
                                        Some(PaletteState::open_in(PaletteMode::Themes));
                                    this.focus.focus(window, cx);
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.text_muted)
                        .child("Shortcut: cmd-k then t."),
                )
                .into_any_element(),
            SettingsPane::Font => {
                let row = |label: &'static str,
                           family: &Entity<InputState>,
                           size: &Entity<InputState>,
                           hint: &'static str,
                           slot: FontSlot,
                           fam_custom: bool,
                           size_custom: bool,
                           p: &crate::theme::Palette,
                           cx: &mut Context<Self>| {
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(label),
                        )
                        .child(div().text_xs().text_color(p.text_muted).child(hint))
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .relative()
                                        // cleanable off: the slot-reset icon
                                        // owns the input's right corner.
                                        .child(Input::new(family).cleanable(false))
                                        // Per-field reset on the family input
                                        // (shows only when family differs).
                                        .when(fam_custom, |el| {
                                            el.child(reset_btn(
                                                ("font-reset-fam", slot as usize),
                                                slot,
                                                FontField::Family,
                                                p,
                                                cx,
                                            ))
                                        }),
                                )
                                .child(
                                    div()
                                        .w(px(84.))
                                        .relative()
                                        .child(Input::new(size).cleanable(false))
                                        // Per-field reset: tiny circular arrow
                                        // inside the size input (shows only
                                        // when the size differs).
                                        .when(size_custom, |el| {
                                            el.child(reset_btn(
                                                ("font-reset", slot as usize),
                                                slot,
                                                FontField::Size,
                                                p,
                                                cx,
                                            ))
                                        }),
                                ),
                        )
                };
                let p = &self.palette;
                let ui_def = config::default_ui_font();
                let md_def = config::default_markdown_font();
                let buf_def = config::default_buffer_font();
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_4()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Fonts"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(p.text_muted)
                            .child("UI · Markdown · Buffer (code). Family + size in px (digits only)."),
                    )
                    .child(row(
                        "UI",
                        &self.fonts.ui_family,
                        &self.fonts.ui_size,
                        "Titlebar, footer, settings",
                        FontSlot::Ui,
                        self.config.ui_font.family != ui_def.family,
                        self.config.ui_font.size != ui_def.size,
                        p,
                        cx,
                    ))
                    .child(row(
                        "Markdown",
                        &self.fonts.markdown_family,
                        &self.fonts.markdown_size,
                        "Paragraphs, headings, lists, quotes",
                        FontSlot::Markdown,
                        self.config.markdown_font.family != md_def.family,
                        self.config.markdown_font.size != md_def.size,
                        p,
                        cx,
                    ))
                    .child(row(
                        "Buffer",
                        &self.fonts.buffer_family,
                        &self.fonts.buffer_size,
                        "Fenced code blocks",
                        FontSlot::Buffer,
                        self.config.buffer_font.family != buf_def.family,
                        self.config.buffer_font.size != buf_def.size,
                        p,
                        cx,
                    ))
                    .child(
                        Button::new("fonts-reset")
                            .ghost()
                            .xsmall()
                            .label("Reset all to defaults")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reset_fonts(window, cx);
                            })),
                    )
                    .into_any_element()
            }
        };

        div()
            .id("settings-overlay")
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.background.opacity(0.55))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.settings_open = false;
                    this.focus.focus(window, cx);
                    cx.notify();
                }),
            )
            .on_scroll_wheel(|_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                h_flex()
                    .id("settings-panel")
                    .w(px(560.))
                    .h(px(480.))
                    .max_w(relative(0.92))
                    .max_h(relative(0.9))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.background_panel)
                    .shadow_lg()
                    .overflow_hidden()
                    .font_family(self.config.ui_font.family.clone())
                    .text_size(self.ui_font_px())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        v_flex()
                            .w(px(148.))
                            .flex_shrink_0()
                            .h_full()
                            .px_2()
                            .py_3()
                            .gap_1()
                            .border_r_1()
                            .border_color(p.border)
                            .bg(p.background)
                            .child(
                                div()
                                    .px_3()
                                    .pb_2()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.text_muted)
                                    .child("Settings"),
                            )
                            .child(nav_row("Editor", SettingsPane::Editor, cx))
                            .child(nav_row("Theme", SettingsPane::Theme, cx))
                            .child(nav_row("Font", SettingsPane::Font, cx)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .pt_4()
                            .gap_2()
                            .child(
                                h_flex().w_full().min_w_0().px_5().justify_end().child(
                                    Button::new("settings-done")
                                        .ghost()
                                        .xsmall()
                                        .label("Done")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_settings(window, cx);
                                        })),
                                ),
                            )
                            .child(
                                div()
                                    .id("settings-content")
                                    .flex_1()
                                    .min_w_0()
                                    .min_h(px(0.))
                                    .w_full()
                                    .px_5()
                                    .pb_4()
                                    .overflow_y_scrollbar()
                                    .overflow_x_hidden()
                                    .child(content),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_palette(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(state) = self.cmd_palette.as_ref() else {
            return div().into_any_element();
        };
        let p = self.palette.clone();
        let items = state.items(self.view_source);
        let selected = state.index;
        let query = state.query.render();
        let query_empty = state.query.is_empty();
        let mode = state.mode;
        let title = match mode {
            PaletteMode::Root => "Commands",
            PaletteMode::Themes => "Themes",
            PaletteMode::Editors => "Editor",
        };

        div()
            .id("palette-overlay")
            .absolute()
            .size_full()
            .flex()
            .justify_center()
            .pt(px(88.))
            .bg(p.background.opacity(0.45))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.cancel_palette(window, cx);
                }),
            )
            .on_scroll_wheel(|_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                v_flex()
                    .id("palette-panel")
                    .w(px(520.))
                    .max_w(relative(0.92))
                    .max_h(px(420.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.background_panel)
                    .shadow_lg()
                    .overflow_hidden()
                    .font_family(self.config.ui_font.family.clone())
                    .text_size(self.ui_font_px())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .w_full()
                            .px_3()
                            .py_2()
                            .items_center()
                            .gap_2()
                            .border_b_1()
                            .border_color(p.border)
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.text_muted)
                                    .child(title),
                            )
                            .child(div().flex_1()),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .px_3()
                            .py_2()
                            .items_center()
                            .gap_2()
                            .border_b_1()
                            .border_color(p.border)
                            .child(div().flex_1().text_sm().child(
                                if query_empty {
                                    div()
                                        .text_color(p.text_muted.opacity(0.7))
                                        .child("Type a command…")
                                } else {
                                    div().text_color(p.markdown_text).child(query)
                                },
                            )),
                    )
                    .child(
                        v_flex()
                            .id("palette-list")
                            .w_full()
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .track_scroll(&self.palette_scroll)
                            .py_1()
                            .px_1()
                            .children(if items.is_empty() {
                                vec![div()
                                    .px_3()
                                    .py_2()
                                    .text_sm()
                                    .text_color(p.text_muted)
                                    .child("No matching commands")
                                    .into_any_element()]
                            } else {
                                items
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, item)| {
                                        let active = i == selected;
                                        let action = item.action;
                                        div()
                                            .id(("palette-item", i))
                                            .w_full()
                                            .px_3()
                                            .py_2()
                                            .rounded(px(8.))
                                            .cursor_pointer()
                                            .when(active, |el| el.bg(p.background_element))
                                            .hover(|el| el.bg(p.background_element.opacity(0.7)))
                                            .on_mouse_move(cx.listener(
                                                move |this, _, window, cx| {
                                                    let changed = this
                                                        .cmd_palette
                                                        .as_mut()
                                                        .map(|state| {
                                                            if state.index != i {
                                                                state.index = i;
                                                                true
                                                            } else {
                                                                false
                                                            }
                                                        })
                                                        .unwrap_or(false);
                                                    if changed {
                                                        // Live theme preview while hovering Themes.
                                                        this.preview_palette_theme(window, cx);
                                                        cx.notify();
                                                    }
                                                },
                                            ))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _, window, cx| {
                                                    cx.stop_propagation();
                                                    this.run_palette_action(action, window, cx);
                                                }),
                                            )
                                            .child(
                                                h_flex()
                                                    .w_full()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .text_sm()
                                                            .font_weight(FontWeight::NORMAL)
                                                            .text_color(if active {
                                                                p.primary
                                                            } else {
                                                                p.markdown_text
                                                            })
                                                            .child(item.label),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(p.text_muted)
                                                            .child(item.right()),
                                                    ),
                                            )
                                            .into_any_element()
                                    })
                                    .collect()
                            })
                            // Scrollbar last: the overlay is an extra tracked
                            // child, so it must not shift item indices used
                            // by keyboard scroll-into-view.
                            .vertical_scrollbar(&self.palette_scroll),
                    ),
            )
            .into_any_element()
    }

    fn render_edit(
        &mut self,
        display: std::ops::Range<usize>,
        text: &str,
        heading: bool,
        heading_level: Option<u8>,
        placeholder: Option<&str>,
        wrap: bool,
        fit_content: bool,
        syntax_lang: Option<&str>,
        _mono: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.proj();
        let pal = self.palette.clone();
        let d_caret = self.caret;
        let d_sel = self.sel.clone();
        let d_marked = self.marked.clone();
        let local_caret = if d_caret >= display.start && d_caret <= display.end {
            Some(d_caret - display.start)
        } else {
            None
        };
        let local_sel = surface::clip_range(d_sel, display.clone());
        let local_marked = surface::clip_range(d_marked, display.clone());
        let runs = surface::mark_runs(&p, display.clone());
        let mut hs = surface::highlights(text.len(), &runs, local_sel, local_marked, &pal, heading);
        if let Some(lang) = syntax_lang {
            hs.extend(syntax::highlights(lang, text, &pal));
            hs = surface::flatten(text.len(), hs);
        }
        // Dimmed search-occurrence highlights (current match brighter). The
        // footer counter + ‹ › buttons cycle them; enter/shift+enter too.
        if let Some(q) = self.active_search_query() {
            if !q.is_empty() && !text.is_empty() {
                let mut from = 0usize;
                while from <= text.len() {
                    let Some(rel) = text[from..].find(&q) else {
                        break;
                    };
                    let s = from + rel;
                    let e = (s + q.len()).min(text.len());
                    if e > s {
                        let global = display.start + s;
                        let current =
                            self.caret >= global && self.caret < global + q.len().max(1);
                        hs.push((
                            s..e,
                            gpui::HighlightStyle {
                                background_color: Some(if current {
                                    pal.primary.opacity(0.55)
                                } else {
                                    pal.primary.opacity(0.22)
                                }),
                                ..Default::default()
                            },
                        ));
                    }
                    from = e.max(from + 1);
                    if hs.len() > 2100 {
                        break;
                    }
                }
                hs = surface::flatten(text.len(), hs);
            }
        }
        let ime = local_caret.is_some() && (self.mode.is_insert() || self.is_notion());
        let block_caret = self.uses_block_caret();
        let view = cx.entity();
        let focus = self.focus.clone();
        let (family, size) = if _mono {
            (
                self.config.buffer_font.family.clone(),
                self.buffer_font_px(),
            )
        } else {
            let base = self.config.markdown_font.size.clamp(8, 48) as f32;
            let scale = match heading_level {
                Some(1) => 2.0,
                Some(2) => 1.5,
                Some(3) => 1.25,
                Some(4) => 1.0,
                Some(5) => 0.875,
                Some(6) => 0.85,
                _ => 1.0,
            };
            (self.config.markdown_font.family.clone(), px(base * scale))
        };
        let font = Some(gpui::SharedString::from(family));
        let font_px = Some(size);
        let code_ranges: Vec<_> = runs
            .iter()
            .filter(|(_, m)| m.code)
            .map(|(r, _)| r.clone())
            .collect();
        let code_font = if code_ranges.is_empty() {
            None
        } else {
            Some((
                gpui::SharedString::from(self.config.buffer_font.family.clone()),
                code_ranges,
            ))
        };
        // Mixed-size inline code (85% of body). Mono blocks stay uniform.
        let code_px = if _mono {
            None
        } else {
            font_px.filter(|_| code_font.is_some()).map(|s| s * crate::surface::INLINE_CODE_SCALE)
        };
        surface::edit_text(
            view.clone(),
            focus,
            &mut self.hits,
            display.start,
            text.to_string(),
            hs,
            local_caret,
            block_caret,
            wrap,
            fit_content,
            ime,
            &pal,
            placeholder,
            font,
            font_px,
            heading,
            code_font,
            code_px,
            {
                let view = view.clone();
                move |d, shift, cmd, clicks, window, cx| {
                    view.update(cx, |this, cx| {
                        this.click_display(d, shift, cmd, clicks, window, cx)
                    });
                }
            },
            {
                let view = view.clone();
                move |d, window, cx| {
                    view.update(cx, |this, cx| this.drag_display(d, window, cx));
                }
            },
        )
    }

    fn render_block(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = self.proj();
        let pal = self.palette.clone();
        // Preview always wraps; "Wrap lines" only affects source view.
        let wrap = true;
        let block = p.blocks[ix].clone();
        let text = p
            .display
            .get(block.display.clone())
            .unwrap_or("")
            .to_string();
        let empty = text.trim().is_empty() && !block.extra.is_atomic();
        let caret_here = {
            let d = self.caret;
            d >= block.display.start && d <= block.display.end
        };
        // Only show the empty-block hint on the focused row (less noisy).
        // `<details>` gets a "Summary" hint so `/details` inserts an empty
        // summary the user can fill in (no baked-in "Summary" text).
        let placeholder = if empty && matches!(block.extra, BlockExtra::Details { .. }) {
            Some("Summary")
        } else if empty && caret_here {
            Some("Type to write, or / for blocks")
        } else {
            None
        };
        let heading = matches!(block.kind, BlockKind::Heading(_))
            || matches!(block.extra, BlockExtra::HtmlHeading(_));
        let heading_level = match block.kind {
            BlockKind::Heading(l) => Some(l),
            _ => match block.extra {
                BlockExtra::HtmlHeading(l) => Some(l),
                _ => None,
            },
        };
        let is_code = matches!(block.kind, BlockKind::Code);
        let syntax_lang = match &block.extra {
            BlockExtra::Code { lang, .. } if !lang.is_empty() => Some(lang.as_str()),
            _ => None,
        };
        let body = self.render_edit(
            block.display.clone(),
            &text,
            heading,
            heading_level,
            placeholder,
            wrap && !matches!(block.kind, BlockKind::Code | BlockKind::Html),
            is_code,
            syntax_lang,
            is_code,
            cx,
        );
        let modal_open = self.cmd_palette.is_some()
            || self.settings_open
            || self.command.is_some()
            || self.search.is_some();
        let slash = self.slash_is_open()
            && !modal_open
            && p.block_at_display(self.caret)
                .map(|b| b.source == block.source)
                .unwrap_or(false);
        match &block.extra {
            BlockExtra::Alert(kind) => {
                let color = pal.alert_color(*kind);
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .border_l_4()
                    .border_color(color)
                    .bg(pal.background_element)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(icon_el(alert_icon_name(*kind), color))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(color)
                                    .child(kind.as_str().to_string()),
                            ),
                    )
                    .child(body)
                    .into_any_element()
            }
            BlockExtra::Quote => div()
                .w_full()
                .min_w_0()
                .border_l_2()
                .border_color(pal.markdown_block_quote)
                .px_3()
                .child(body)
                .into_any_element(),
            BlockExtra::Code { lang, .. } => {
                let caret = self.caret;
                let family = self.config.buffer_font.family.clone();
                let mermaid = lang.eq_ignore_ascii_case("mermaid");
                let code_key = block.display.start;
                let code_range = block.display.clone();
                let handle = self
                    .code_scroll
                    .entry(code_key)
                    .or_insert_with(ScrollHandle::new)
                    .clone();
                self.code_scroll_seen.push(code_key);
                v_flex()
                    .relative()
                    .w_full()
                    .min_w_0()
                    .rounded(px(6.))
                    .bg(pal.background_element)
                    .px_3()
                    .py_2()
                    .font_family(family)
                    .text_size(self.buffer_font_px())
                    .text_color(pal.markdown_code_block)
                    // Floating picker: painted last so it stays
                    // clickable above the scroll body, with a
                    // translucent pill so code shows through.
                    .when(mermaid, |el| el.child(self.render_mermaid(&text, cx)))
                    .child(
                        // `overflow_x_scroll` only scrolls when the container
                        // is a flex container: the default `Display::Block`
                        // child fills the parent width instead of overflowing.
                        // `.flex()` (not just `.flex_row()`, which only sets
                        // direction) plus `flex_none()` on the inner lets it
                        // take its natural unwrapped width. Same pattern as
                        // Zed's markdown code blocks.
                        // The `ScrollableMask` sibling owns wheel input in the
                        // capture phase: horizontal-dominant gestures scroll
                        // here AND stop propagation, so the page never moves
                        // vertically mid-swipe. Vertical-dominant gestures
                        // pass through to the page untouched.
                        div().w_full().relative()
                            .child(
                                div()
                                    .id(("code-scroll", code_key))
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_x_scroll()
                                    .restrict_scroll_to_axis()
                                    .track_scroll(&handle)
                                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _window, cx| {
                                        this.autoscroll_code_x(code_key, code_range.clone(), ev.position.x, cx);
                                    }))
                                    .child(div().flex_none().child(body)),
                            )
                            .child(
                                ScrollableMask::new(gpui::Axis::Horizontal, &handle)
                                    .id(("code-scroll-mask", code_key)),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(4.))
                            .right(px(6.))
                            .rounded(px(4.))
                            .bg(pal.background.opacity(0.65))
                            // Swallow clicks so they hit the buttons
                            // instead of focusing the code below.
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .child(self.render_lang_chip(lang, caret, cx))
                                    .child(self.render_copy_btn(
                                        block.display.start,
                                        text.clone(),
                                        cx,
                                    )),
                            ),
                    )
                    .into_any_element()
            }
            BlockExtra::Rule => div()
                .w_full()
                .h(px(2.))
                .my_3()
                .bg(pal.markdown_horizontal_rule)
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, {
                    let start = block.display.start;
                    let view = cx.entity();
                    move |ev: &MouseDownEvent, window, cx| {
                        view.update(cx, |this, cx| {
                            this.click_display(
                                start,
                                ev.modifiers.shift,
                                ev.modifiers.platform || ev.modifiers.control,
                                ev.click_count,
                                window,
                                cx,
                            )
                        });
                    }
                })
                .into_any_element(),
            BlockExtra::Image { alt, src } => {
                self.render_image_hit(alt, src, block.display.start, caret_here, cx)
            }
            BlockExtra::Heading(level) | BlockExtra::HtmlHeading(level) => {
                let mut el = v_flex()
                    .relative()
                    .w_full()
                    .min_w_0()
                    .when(*level <= 2, |el| el.pt_2())
                    .child(body);
                if slash {
                    el = el.child(self.render_slash_popover(cx));
                }
                el.into_any_element()
            }
            BlockExtra::Details { .. } => self.render_details_row(ix, body, slash, cx),
            BlockExtra::DetailsClose => div().into_any_element(),
            BlockExtra::List { items, ordered } => self.render_list(ix, items, *ordered, body, cx),
            BlockExtra::Table { .. } => self.render_table_block(ix, cx),
            BlockExtra::Text | BlockExtra::Html => {
                // HTML media blocks render with the same attachment UI as
                // markdown images: `<video src=…>` as a video card,
                // `<img src=…>` as an image, anything else as an HTML card
                // whose handle click primes the src editor.
                // NOTE: pulldown-cmark only emits `HtmlBlock` for block-level
                // tags — a lone `<video>` line often arrives as a Paragraph
                // with inline HTML, which used to show as raw text. So scan
                // the raw source for media tags regardless of the extra.
                if let Some(raw) = self.source.get(block.source.clone()) {
                    if images::parse_video_src(raw).is_some() || is_bare_video(raw) {
                        if let Some(src) =
                            images::parse_video_src(raw).or_else(|| images::parse_html_src(raw))
                        {
                            return self.render_video_hit("", &src, block.display.start, caret_here, cx);
                        }
                    }
                    if let Some((src, alt)) = images::parse_img_src(raw) {
                        // Only hijack Text blocks when the whole block is just
                        // the image tag (else inline `<img>` inside prose keeps
                        // its text rendering).
                        let t = raw.trim();
                        if matches!(&block.extra, BlockExtra::Html)
                            || (t.starts_with('<') && t.contains("<img"))
                        {
                            return self.render_image_hit(&alt, &src, block.display.start, caret_here, cx);
                        }
                    }
                    if matches!(&block.extra, BlockExtra::Html) {
                        return self.render_html_block(raw, block.display.start, cx);
                    }
                }
                let mut el = v_flex().relative().w_full().min_w_0().child(body);
                if slash {
                    // `deferred` paints after later sibling blocks so the menu
                    // floats above the document instead of sitting underneath.
                    el = el.child(self.render_slash_popover(cx));
                }
                el.into_any_element()
            }
        }
    }

    fn render_image_hit(
        &mut self,
        alt: &str,
        src: &str,
        display_start: usize,
        caret_here: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if images::is_video_src(src) {
            return self.render_video_hit(alt, src, display_start, caret_here, cx);
        }
        // Ambiguous remote `![](https://…)` with no video extension (GitHub
        // asset URLs): probe download → magic sniff → decode. `probe` runs
        // once per URL; `NotVideo` falls through to image UI below, anything
        // else (ready / loading / video error) renders the video card so the
        // clip plays inline instead of showing as a static image.
        if images::is_remote_src(src) {
            let pkey = crate::video::VideoStore::remote_key(src);
            if !self.video.is_not_video(&pkey) {
                self.probe_remote_video(src, cx);
                if self.video.get(&pkey).is_some()
                    || self.video.error(&pkey).is_some()
                    || !self.video.is_not_video(&pkey)
                {
                    return self.render_video_hit(alt, src, display_start, caret_here, cx);
                }
            }
        }
        let pal = &self.palette;
        let path = images::resolve_beside(&self.path, src);
        let remote = !path.exists() && images::is_remote_src(src);
        let missing = !path.exists() && !remote;
        let selected = self.media_sel == Some(display_start);
        // Caret inside this block (keyboard walk) highlights the frame too —
        // same primary border as the Edit-selected state.
        let focused = selected || caret_here;
        let alt_owned = alt.to_string();
        let src_owned = src.to_string();
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .cursor_pointer()
            .when(focused, |el| {
                el.border_1()
                    .border_color(pal.primary)
                    .rounded(px(8.))
                    .p_1()
            })
            .on_mouse_down(MouseButton::Left, {
                let view = cx.entity();
                move |ev: &MouseDownEvent, window, cx| {
                    view.update(cx, |this, cx| {
                        this.click_display(
                            display_start,
                            ev.modifiers.shift,
                            ev.modifiers.platform || ev.modifiers.control,
                            ev.click_count,
                            window,
                            cx,
                        )
                    });
                }
            })
            .when(path.exists(), |el| {
                el.child(
                    img(path.clone())
                        .max_w_full()
                        .max_h(px(480.))
                        .rounded(px(6.)),
                )
            })
            .when(remote, |el| {
                // Remote `http(s)` photos load through GPUI's asset pipeline
                // (`SharedUri` → http_client download + cache). `min_h` keeps
                // a visible slot while the fetch is in flight; the caption
                // keeps the URL visible even if the host blocks the fetch.
                let uri: SharedUri = src.to_string().into();
                el.child(
                    img(uri)
                        .max_w_full()
                        .max_h(px(480.))
                        .min_h(px(120.))
                        .rounded(px(6.)),
                )
            })
            .when(missing, |el| {
                el.child(
                    h_flex()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .items_center()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(pal.warning)
                        .bg(pal.background_panel)
                        .child(icon_el("image", pal.warning))
                        .child(
                            v_flex().flex_1().min_w_0()
                                .child(
                                    div().text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(pal.markdown_text)
                                        .child("Missing image"),
                                )
                                .child(
                                    div().text_xs()
                                        .text_color(pal.text_muted)
                                        .child(format!("{src} — not found beside this file")),
                                ),
                        ),
                )
            })
            // Knob bar: always-visible alt/url readout + Edit + Open, so a
            // photo's URL/alt are one click away without guessing select.
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(pal.text_muted)
                            .child(if alt.is_empty() {
                                src.to_string()
                            } else {
                                format!("{alt} · {src}")
                            }),
                    )
                    .when(remote, |el| {
                        el.child(
                            Button::new(("img-open", display_start))
                                .ghost()
                                .xsmall()
                                .label("Open")
                                .on_click({
                                    let url = src_owned.clone();
                                    cx.listener(move |_, _, _, cx| {
                                        cx.open_url(&url);
                                    })
                                }),
                        )
                    })
                    .child(
                        Button::new(("img-edit", display_start))
                            .ghost()
                            .xsmall()
                            .label(if selected { "Editing…" } else { "Edit" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_media_knob(
                                    display_start,
                                    alt_owned.clone(),
                                    src_owned.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .when(selected, |el| {
                el.child(self.render_media_toolbar(true, display_start, cx))
            })
            .into_any_element()
    }

    /// Prime the Alt + Path toolbar for an image without going through
    /// click-hit-testing (used by the always-visible Edit knob).
    fn select_media_knob(
        &mut self,
        display_start: usize,
        alt: String,
        src: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.media_sel = Some(display_start);
        self.caret = display_start;
        self.sel = None;
        self.mouse_dragging = false;
        self.clamp_caret();
        self.follow_caret = false;
        self.media_alt.update(cx, |st, cx| {
            st.set_value(alt, window, cx);
        });
        self.media_src.update(cx, |st, cx| {
            st.set_value(src, window, cx);
        });
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// GitHub-style video (`![alt](clip.mp4)` or `<video src=…>`) with
    /// INLINE playback: pure-Rust `yscv-video` decodes MP4 (H.264/HEVC)
    /// off-thread into `RenderImage` frames — no GStreamer/FFmpeg system
    /// libs, so the bundle stays self-contained (this is why we deliberately
    /// do NOT use awesome-gpui's `gpui-video-player`: system GStreamer +
    /// per-video playbin pipelines break the self-contained bundle).
    /// Playback is silent (yscv exposes audio metadata only); `Open` still
    /// shells to the system player for audio. Remote URLs download to temp
    /// (`curl`) and decode through the same path. No HTML renderer exists
    /// in the GPUI ecosystem, so `<iframe>` etc. stay as cards.
    fn render_video_hit(
        &mut self,
        alt: &str,
        src: &str,
        display_start: usize,
        caret_here: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pal = self.palette.clone();
        let path = images::resolve_beside(&self.path, src);
        let selected = self.media_sel == Some(display_start);
        let focused = selected || caret_here;
        let remote = !path.exists() && images::is_remote_src(src);
        let name = Path::new(src)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(src);
        let src_owned = src.to_string();
        let alt_owned = alt.to_string();

        // Missing local file — same guidance card as before.
        if !path.exists() && !remote {
            let title = "Missing video".to_string();
            return v_flex()
                .w_full()
                .min_w_0()
                .gap_1()
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, {
                    let view = cx.entity();
                    move |ev: &MouseDownEvent, window, cx| {
                        view.update(cx, |this, cx| {
                            this.click_display(
                                display_start,
                                ev.modifiers.shift,
                                ev.modifiers.platform || ev.modifiers.control,
                                ev.click_count,
                                window,
                                cx,
                            )
                        });
                    }
                })
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(if focused { pal.primary } else { pal.border })
                        .bg(pal.background_panel)
                        .child(
                            div()
                                .w_full()
                                .h(px(220.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(gpui::black().opacity(0.85))
                                .child(icon_el_px(
                                    "image",
                                    gpui::white().opacity(0.4),
                                    px(40.),
                                )),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .p_3()
                                .gap_2()
                                .items_center()
                                .child(
                                    v_flex().flex_1().min_w_0().gap_1()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(pal.markdown_text)
                                                .child(title),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(pal.text_muted)
                                                .child(format!(
                                                    "{src} — not found beside this file; edit the path below"
                                                )),
                                        ),
                                )
                                .child(
                                    Button::new(("video-edit", display_start))
                                        .ghost()
                                        .xsmall()
                                        .label(if selected { "Editing…" } else { "Edit" })
                                        .on_click({
                                            let src = src_owned.clone();
                                            let alt = alt_owned.clone();
                                            cx.listener(move |this, _, window, cx| {
                                                this.select_media_knob(
                                                    display_start,
                                                    alt.clone(),
                                                    src.clone(),
                                                    window,
                                                    cx,
                                                );
                                            })
                                        }),
                                ),
                        ),
                )
                .when(selected, |el| {
                    el.child(self.render_media_toolbar(false, display_start, cx))
                })
                .into_any_element();
        }

        // Decode key: local path, or the remote URL (decoded from temp).
        let key = if remote {
            crate::video::VideoStore::remote_key(src)
        } else {
            crate::video::VideoStore::key_for(&path)
        };

        // Kick off decode at most once per clip. Progressive: the first
        // ~12 frames publish ASAP so the clip plays while the rest decodes.
        if self.video.begin(key.clone()) {
            let view = cx.entity();
            if remote {
                self.start_remote_stream(&src, &key, false, view, cx);
            } else {
                Self::stream_decode_into(path.clone(), key.clone(), view, cx);
            }
        }

        // Decode error — card with the message + external fallback.
        if let Some(err) = self.video.error(&key) {
            let first = err.lines().next().unwrap_or("decode failed").to_string();
            let url_owned = src_owned.clone();
            let path_owned = path.clone();
            let inner = v_flex()
                .w_full()
                .min_w_0()
                .p_3()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(icon_el("play", pal.error))
                        .child(
                            v_flex().flex_1().min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(pal.markdown_text)
                                        .child("Video — could not play inline"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(pal.text_muted)
                                        .child(format!("{first} — try Open for audio/full playback")),
                                ),
                        )
                        .child(
                            Button::new(("video-open", display_start))
                                .xsmall()
                                .label("Open")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    if remote {
                                        cx.open_url(&url_owned);
                                    } else {
                                        open_video_src(&url_owned, &path_owned, cx);
                                    }
                                })),
                        )
                        .child(
                            Button::new(("video-edit", display_start))
                                .ghost()
                                .xsmall()
                                .label(if selected { "Editing…" } else { "Edit" })
                                .on_click({
                                    let src = src_owned.clone();
                                    let alt = alt_owned.clone();
                                    cx.listener(move |this, _, window, cx| {
                                        this.select_media_knob(
                                            display_start,
                                            alt.clone(),
                                            src.clone(),
                                            window,
                                            cx,
                                        );
                                    })
                                }),
                        ),
                )
                .into_any_element();
            return self.render_video_card(inner, display_start, focused, cx);
        }

        // Ready — the inline player.
        if let Some((clip, stamps)) = self.video.get(&key) {
            let n = clip.frames.len();
            let (playing, frame) = self.video.play_state(&key);
            let frame = frame.min(n.saturating_sub(1));
            let image = clip.frames[frame].clone();
            let (cw, ch) = (clip.width.max(1) as f32, clip.height.max(1) as f32);
            // Fit inside a 560-wide letterbox; fixed height avoids reflow.
            let box_h = (560.0 * ch / cw).clamp(180.0, 400.0);
            let (iw, ih) = if cw / ch > 560.0 / box_h {
                (560.0, 560.0 * ch / cw)
            } else {
                (box_h * cw / ch, box_h)
            };
            let cur_us = stamps.get(frame).copied().unwrap_or(0);
            // Stable timeline: pinned to the container totals known before
            // the first frame decodes, so the playhead / duration don't
            // jump as each 12-frame chunk lands. `buffered_frac` is the
            // YouTube-style grey track behind it.
            let total_n = clip.timeline_frames();
            let total_us = clip.timeline_duration_us();
            let buffered_frac = clip.buffered_frac();
            let frac = if total_n <= 1 {
                1.0
            } else {
                frame as f32 / (total_n - 1) as f32
            };
            let time = format!(
                "{} / {}",
                crate::video::fmt_time(cur_us),
                crate::video::fmt_time(total_us)
            );
            let truncated = clip.truncated;
            let preview = clip.preview;
            // Live buffered bytes while the full clip streams (preview
            // playback keeps the badge up until the full decode swaps in).
            let buffered = if remote {
                self.video.progress_of(&key)
            } else {
                None
            };
            let pal2 = pal.clone();
            // Bounds tracker: the canvas paints nothing but records the
            // scrub bar's window bounds each frame, so click + drag can map
            // the (window-space) mouse position to a frame.
            let bar_bounds: Rc<RefCell<Option<Bounds<Pixels>>>> = Rc::new(RefCell::new(None));
            let bar_bounds_paint = bar_bounds.clone();
            let bar_bounds_drag = bar_bounds.clone();
            let bar_bounds_outside = bar_bounds.clone();
            let view = cx.entity();
            let view_drag = view.clone();
            let view_outside = view.clone();
            let key_seek = key.clone();
            let key_drag = key.clone();
            let key_outside = key.clone();
            // Seek fractions are over the stable `total_n` timeline; the
            // store clamps to the decoded run, so seeking past the buffer
            // lands at the buffer end (YouTube behavior) instead of past it.
            let n_seek = total_n;
            let n_drag = total_n;
            let scrub = div()
                .id(("video-scrub", display_start))
                .w_full()
                .h(px(22.))
                .flex()
                .items_center()
                .cursor_pointer()
                // Stop here so seeking never moves the caret / opens the
                // slash menu on the video block.
                .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let bb = bar_bounds.clone();
                    let f = {
                        let b = bb.borrow();
                        b.as_ref().map(|b| {
                            let w = b.size.width.as_f32().max(1.0);
                            ((ev.position.x.as_f32() - b.origin.x.as_f32()) / w).clamp(0.0, 1.0)
                        })
                    };
                    if let Some(f) = f {
                        let frame = (f * (n_seek.saturating_sub(1)) as f32).round() as usize;
                        view.update(cx, |this, cx| {
                            this.video.scrub(&key_seek, frame);
                            cx.notify();
                        });
                    }
                })
                .on_mouse_move(move |ev: &MouseMoveEvent, _, cx| {
                    // Slow drags inside the bar: only while held. Fast
                    // flings that leave the bar are handled by
                    // `on_drag_move` below (keeps working outside the
                    // element / window while the button stays down).
                    if !ev.dragging() {
                        return;
                    }
                    cx.stop_propagation();
                    let bb = bar_bounds_drag.clone();
                    let f = {
                        let b = bb.borrow();
                        b.as_ref().map(|b| {
                            let w = b.size.width.as_f32().max(1.0);
                            ((ev.position.x.as_f32() - b.origin.x.as_f32()) / w).clamp(0.0, 1.0)
                        })
                    };
                    if let Some(f) = f {
                        let frame = (f * (n_drag.saturating_sub(1)) as f32).round() as usize;
                        view_drag.update(cx, |this, cx| {
                            this.video.scrub(&key_drag, frame);
                            cx.notify();
                        });
                    }
                })
                // Platform drag: keeps scrubbing when the pointer races off
                // the bar (or out of the window / over another app) while
                // held. `on_drag_move` fires for every move until mouse-up,
                // regardless of hit-testing — the fraction clamps so fast
                // flings still land on the first/last frame.
                .on_drag(
                    DragVideoScrub {
                        key: key.clone(),
                        total: total_n,
                    },
                    |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    },
                )
                .on_drag_move(move |ev: &DragMoveEvent<DragVideoScrub>, _, cx| {
                    let (dkey, ntotal) = {
                        let d = ev.drag(cx);
                        (d.key.clone(), d.total)
                    };
                    if dkey != key_outside {
                        return;
                    }
                    let x = ev.event.position.x.as_f32();
                    let f = bar_bounds_outside
                        .borrow()
                        .as_ref()
                        .map(|b| {
                            let w = b.size.width.as_f32().max(1.0);
                            ((x - b.origin.x.as_f32()) / w).clamp(0.0, 1.0)
                        })
                        .or_else(|| {
                            let b = ev.bounds;
                            let w = b.size.width.as_f32().max(1.0);
                            Some(((x - b.origin.x.as_f32()) / w).clamp(0.0, 1.0))
                        });
                    if let Some(f) = f {
                        let frame = (f * (ntotal.saturating_sub(1)) as f32).round() as usize;
                        view_outside.update(cx, |this, cx| {
                            this.video.scrub(&dkey, frame);
                            cx.notify();
                        });
                    }
                })
                .child(
                    div()
                        .relative()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .bg(pal2.border)
                        // Buffered (decoded) run — YouTube-style grey track
                        // behind the playhead. Grows as batches land; the
                        // playhead (`frac`) stays pinned to the stable total.
                        .when(buffered_frac.is_some(), |el| {
                            let b = buffered_frac.unwrap_or(0.0);
                            el.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(relative(b))
                                    .rounded_full()
                                    .bg(pal2.text_muted),
                            )
                        })
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(relative(frac))
                                .rounded_full()
                                .bg(pal2.primary),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(-4.))
                                .bottom(px(-4.))
                                .left(relative(frac))
                                .ml(px(-7.))
                                .w(px(14.))
                                .h(px(14.))
                                .rounded_full()
                                .bg(gpui::white())
                                .border_2()
                                .border_color(pal2.primary),
                        )
                        .child(canvas(
                            move |bounds, _, _| {
                                *bar_bounds_paint.borrow_mut() = Some(bounds);
                            },
                            |_, _, _, _| {},
                        )),
                );
            let play_icon = if playing { "pause" } else { "play" };
            let frame_key = key.clone();
            let frame_view = cx.entity();
            let inner = v_flex()
                .w_full()
                .min_w_0()
                .child(
                    div()
                        .id(("video-frame", display_start))
                        .w_full()
                        .h(px(box_h))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::black())
                        .cursor_pointer()
                        // Clicking the picture toggles play/pause (footer
                        // stays for Open/Edit). Stop here so the card
                        // doesn't also move the caret / open the toolbar —
                        // we set the caret ourselves so `space` keeps
                        // working afterwards.
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            frame_view.update(cx, |this, cx| {
                                this.caret = display_start;
                                this.sel = None;
                                this.mouse_dragging = false;
                                this.follow_caret = false;
                                this.focus.focus(window, cx);
                                this.toggle_video(&frame_key, cx);
                            });
                        })
                        .child(img(image).w(px(iw)).h(px(ih))),
                )
                .child(
                    h_flex()
                        .w_full()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .items_center()
                        // Swallow clicks on the transport so they never reach
                        // the card (which would move the caret / pop the slash
                        // menu). No tooltip on the icon button either.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new(("video-play", display_start))
                                .ghost()
                                .xsmall()
                                .icon(icon_el(play_icon, pal.text_muted))
                                .on_click({
                                    let key = key.clone();
                                    cx.listener(move |this, _, _, cx| {
                                        this.toggle_video(&key, cx);
                                    })
                                }),
                        )
                        .child(div().flex_1().min_w_0().child(scrub))
                        .child(
                            div()
                                .text_xs()
                                .text_color(pal.text_muted)
                                .child(time),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .px_3()
                        .pb_3()
                        .gap_2()
                        .items_center()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            v_flex().flex_1().min_w_0()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(pal.text_muted)
                                        .child(if preview {
                                            match buffered {
                                                Some((r, Some(t))) if t > 0 => format!(
                                                    "{name} · preview playing · buffering {}%",
                                                    (r.min(t) * 100 / t).min(100)
                                                ),
                                                Some((r, _)) if r > 0 => format!(
                                                    "{name} · preview playing · buffering {}",
                                                    crate::video::fmt_bytes(r)
                                                ),
                                                _ => format!(
                                                    "{name} · preview playing · buffering full clip…"
                                                ),
                                            }
                                        } else if truncated {
                                            format!(
                                                "{name} · preview of first {n} frames · Open for full"
                                            )
                                        } else if alt.is_empty() {
                                            format!("{name} · silent preview")
                                        } else {
                                            format!("{name} · {alt} · silent preview")
                                        }),
                                ),
                        )
                        .when(remote, |el| {
                            el.child(
                                Button::new(("video-open", display_start))
                                    .ghost()
                                    .xsmall()
                                    .label("Open")
                                    .on_click({
                                        let url = src_owned.clone();
                                        cx.listener(move |_, _, _, cx| {
                                            cx.open_url(&url);
                                        })
                                    }),
                            )
                        })
                        .child(
                            Button::new(("video-edit", display_start))
                                .ghost()
                                .xsmall()
                                .label(if selected { "Editing…" } else { "Edit" })
                                .on_click({
                                    let src = src_owned.clone();
                                    let alt = alt_owned.clone();
                                    cx.listener(move |this, _, window, cx| {
                                        this.select_media_knob(
                                            display_start,
                                            alt.clone(),
                                            src.clone(),
                                            window,
                                            cx,
                                        );
                                    })
                                }),
                        ),
                )
                .into_any_element();
            return self.render_video_card(inner, display_start, focused, cx);
        }

        // Still decoding (or downloading a remote) — placeholder tile.
        // Remote shows a live buffering bar (browser-style) instead of a
        // stuck spinner: bytes come from the `.part` poller.
        let progress = if remote {
            self.video.progress_of(&key)
        } else {
            None
        };
        let fetching_remote =
            remote && !crate::video::remote_cache_path(src).exists();
        // Download done but no frames yet → the pure-Rust software decode
        // is grinding, not the network. Say so (previously this showed a
        // "Buffering 1.4MB" label that looked complete but wasn't playable).
        let label = match progress {
            Some((r, Some(t))) if t > 0 && fetching_remote => format!(
                "Buffering {}% · {} / {}",
                (r.min(t) * 100 / t).min(100),
                crate::video::fmt_bytes(r),
                crate::video::fmt_bytes(t)
            ),
            Some((r, _)) if r > 0 && fetching_remote => {
                format!("Buffering {} received… · {name}", crate::video::fmt_bytes(r))
            }
            _ if fetching_remote => "Fetching remote video…".to_string(),
            _ => "Decoding video…".to_string(),
        };
        let bar_frac = match progress {
            Some((r, Some(t))) if t > 0 => (r as f32 / t as f32).clamp(0.0, 1.0),
            _ => -1.0,
        };
        let inner = v_flex()
            .w_full()
            .min_w_0()
            .child(
                div()
                    .w_full()
                    .h(px(220.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::black().opacity(0.85))
                    .child(icon_el_px("play", gpui::white().opacity(0.4), px(40.))),
            )
            .child(
                v_flex()
                    .w_full()
                    .p_3()
                    .pb_2()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(pal.text_muted)
                            .child(format!("{label} · {name}")),
                    )
                    .when(bar_frac >= 0.0, |el| {
                        el.child(
                            div()
                                .w_full()
                                .h(px(3.))
                                .rounded_full()
                                .bg(pal.border)
                                .child(
                                    div()
                                        .h_full()
                                        .rounded_full()
                                        .bg(pal.primary)
                                        .w(relative(bar_frac)),
                                ),
                        )
                    }),
            )
            .into_any_element();
        self.render_video_card(inner, display_start, focused, cx)
    }

    /// Video card shell: bordered panel + click-to-select + the Alt/Path
    /// toolbar when Edit-selected. `focused` (Edit-selected OR caret inside
    /// the block) drives the primary border; the toolbar stays Edit-only.
    fn render_video_card(
        &self,
        inner: AnyElement,
        display_start: usize,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pal = &self.palette;
        let selected = self.media_sel == Some(display_start);
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, {
                let view = cx.entity();
                move |ev: &MouseDownEvent, window, cx| {
                    view.update(cx, |this, cx| {
                        this.click_display(
                            display_start,
                            ev.modifiers.shift,
                            ev.modifiers.platform || ev.modifiers.control,
                            ev.click_count,
                            window,
                            cx,
                        )
                    });
                }
            })
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if focused { pal.primary } else { pal.border })
                    .bg(pal.background_panel)
                    .child(inner),
            )
            .when(selected, |el| {
                el.child(self.render_media_toolbar(false, display_start, cx))
            })
            .into_any_element()
    }

    /// Progressive decode into the store: a decode thread ships
    /// `DECODE_BATCH`-sized batches over a channel while a `cx.spawn`
    /// forwarder publishes each batch + notifies, so the first ~12 frames
    /// paint/play while the (slow pure-Rust software) decode of the rest
    /// still runs. `on_done` reports the terminal state for error cards.
    fn stream_decode_into(
        path: PathBuf,
        key: String,
        view: Entity<Self>,
        cx: &mut Context<Self>,
    ) {
        use std::time::Duration;
        let (tx, rx) = std::sync::mpsc::sync_channel::<crate::video::DecodeBatch>(8);
        std::thread::spawn(move || {
            let _ = crate::video::decode_file_batches(&path, tx);
        });
        cx.spawn(async move |_, cx| {
            let mut published = 0usize;
            loop {
                match rx.try_recv() {
                    Ok(batch) => {
                        let finished = batch.finished;
                        let _ = cx.update(|cx| {
                            view.update(cx, |this, cx| {
                                if this.video.append_batch(&key, batch) {
                                    published += 1;
                                    if finished {
                                        this.video.normalize_timestamps(&key);
                                    }
                                    cx.notify();
                                }
                            })
                        });
                        if finished {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        cx.background_executor().timer(Duration::from_millis(80)).await;
                    }
                }
            }
            // Zero frames decoded → surface the error card (the decode
            // thread dropped `tx` without sending). Otherwise the partial
            // clip stays playable — but clear a dangling `preview` flag
            // (error tail after partial batches) so the timer loop stops
            // at the buffer end instead of stalling forever.
            let _ = cx.update(|cx| {
                view.update(cx, |this, cx| {
                    if published == 0 && this.video.get(&key).is_none() {
                        this.video.finish(
                            key.clone(),
                            Err("no frames decoded".to_string()),
                        );
                        cx.notify();
                    } else {
                        this.video.finalize_preview(&key);
                        // Wake the card one last time (preview flag cleared).
                        cx.notify();
                    }
                })
            });
        })
        .detach();
    }

    /// Browser-style streaming for a remote clip. Assumes `begin()` already
    /// claimed the slot (so this runs once per URL): a single download
    /// streams to a `.part` target while a poller reports live bytes into
    /// `VideoStore` progress (buffering bar), then the file sniffs +
    /// decodes *progressively* — first frames publish ASAP and the clip
    /// plays while the rest decodes. (No separate Range preview: for small
    /// clips the preview IS the file, so it doubled downloads; for
    /// moov-at-end files it always failed and its latency sat on the
    /// critical path.)
    /// `is_probe` marks non-video URLs `NotVideo` (extensionless `![](…)`
    /// falls back to image UI); the known-video path surfaces decode errors
    /// via `finish(Err)` like local files.
    fn start_remote_stream(
        &mut self,
        url: &str,
        key: &str,
        is_probe: bool,
        view: Entity<Self>,
        cx: &mut Context<Self>,
    ) {
        use std::time::Duration;
        self.video.set_progress(key.to_string(), 0, None);
        // Progress poller: reads the growing `.part` file + HEAD total and
        // notifies so the buffering bar animates. Runs until the full clip
        // lands (preview Ready doesn't stop it — the badge stays up).
        {
            let view = view.clone();
            let url = url.to_string();
            let key = key.to_string();
            cx.spawn(async move |_, cx| {
                // HEAD first so the bar has a total ASAP.
                let url_head = url.clone();
                let total = cx
                    .background_spawn(async move { crate::video::fetch_remote_total(&url_head) })
                    .await;
                let _ = cx.update(|cx| {
                    view.update(cx, |this, cx| {
                        let received = this.video.progress_of(&key).map(|(r, _)| r).unwrap_or(0);
                        this.video.set_progress(key.clone(), received, total);
                        cx.notify();
                    })
                });
                for _ in 0..600 {
                    cx.background_executor().timer(Duration::from_millis(150)).await;
                    let done = cx.update(|cx| {
                        view.update(cx, |this, cx| {
                                let partial = crate::video::remote_part_path(&url);
                                let received = partial.metadata().map(|m| m.len()).unwrap_or(0);
                                let total = this
                                    .video
                                    .progress_of(&key)
                                    .and_then(|(_, t)| t)
                                    .or(total);
                                // The final cache file appearing means curl
                                // renamed `.part` — report it as complete.
                                let final_len = crate::video::remote_cache_path(&url)
                                    .metadata()
                                    .map(|m| m.len())
                                    .unwrap_or(0);
                                let received = received.max(if final_len > 0 {
                                    // Full file landed; the finish lands next.
                                    this.video.set_progress(
                                        key.clone(),
                                        total.unwrap_or(final_len).max(final_len),
                                        total,
                                    );
                                    cx.notify();
                                    return true;
                                } else {
                                    received
                                });
                                this.video.set_progress(key.clone(), received, total);
                                cx.notify();
                                // Keep polling through the preview; stop once
                                // the full clip (or an error) resolves and no
                                // download is in flight.
                                let resolved = this.video.get(&key).is_some()
                                    && !this.video.is_preview(&key)
                                    || this.video.error(&key).is_some()
                                    || this.video.is_not_video(&key);
                                resolved && final_len > 0
                            })
                        });
                    if done {
                        break;
                    }
                }
            })
            .detach();
        }
        // Single download → sniff → progressive decode. The decode
        // publishes its first batch (~12 frames) ASAP; the error tail
        // only fires when nothing decoded at all.
        {
            let url = url.to_string();
            let key = key.to_string();
            cx.spawn(async move |_, cx| {
                let downloaded = cx
                    .background_spawn(async move {
                        let mut noop = |_, _| {};
                        let tmp = crate::video::download_remote_streaming(&url, &mut noop)?;
                        let bytes =
                            std::fs::read(&tmp).map_err(|e| format!("read cache: {e}"))?;
                        if !crate::video::is_video_bytes(&bytes) {
                            let _ = std::fs::remove_file(&tmp);
                            return Err(crate::video::NOT_VIDEO.to_string());
                        }
                        Ok(tmp)
                    })
                    .await;
                match downloaded {
                    Err(e) => {
                        let _ = cx.update(|cx| {
                            view.update(cx, |this, cx| {
                                if e == crate::video::NOT_VIDEO && is_probe {
                                    this.video.mark_not_video(key.clone());
                                } else {
                                    this.video.finish(key.clone(), Err(e));
                                }
                                cx.notify();
                            })
                        });
                    }
                    Ok(tmp) => {
                        let _ = cx.update(|cx| {
                            view.update(cx, |this, cx| {
                                Self::stream_decode_into(tmp, key.clone(), view.clone(), cx);
                            })
                        });
                    }
                }
            })
            .detach();
        }
    }

    /// Probe an ambiguous remote URL (bare link or extensionless `![](…)`)
    /// as video: download → magic sniff → decode. Returns the store key.
    /// Non-video URLs are marked `NotVideo` (and the temp file removed) so
    /// callers fall back to image/link UI; genuine video errors surface via
    /// `finish(Err)` like the known-video path. Runs once per URL.
    fn probe_remote_video(&mut self, url: &str, cx: &mut Context<Self>) -> String {
        let key = crate::video::VideoStore::remote_key(url);
        if self.video.begin(key.clone()) {
            self.start_remote_stream(url, &key, true, cx.entity(), cx);
        }
        key
    }

    /// Resolve the inline-player key for the video block at display offset
    /// `d`, when that block actually renders as a video. Mirrors the
    /// `render_video_hit` source resolution (markdown image vs `<video>`
    /// HTML). Returns `None` for missing/error/loading clips so callers
    /// (space-to-toggle) only hijack keys when there's a ready clip.
    fn video_key_at(&self, d: usize) -> Option<String> {
        let p = self.proj();
        let b = p.block_at_display(d)?;
        if let BlockExtra::Image { src, .. } = &b.extra {
            // Extensionless remote `![](https://…)` plays inline once the
            // probe decodes it — mirror `render_image_hit`'s fallback.
            if !images::is_video_src(src) {
                if images::is_remote_src(src) {
                    let key = crate::video::VideoStore::remote_key(src);
                    if self.video.get(&key).is_some() {
                        return Some(key);
                    }
                }
                return None;
            }
            let path = images::resolve_beside(&self.path, src);
            let key = if !path.exists() && images::is_remote_src(src) {
                crate::video::VideoStore::remote_key(src)
            } else {
                crate::video::VideoStore::key_for(&path)
            };
            return self.video.get(&key).is_some().then_some(key);
        }
        let raw = self.source.get(b.source.clone())?;
        if images::parse_video_src(raw).is_none() && !is_bare_video(raw) {
            return None;
        }
        let src = images::parse_video_src(raw).or_else(|| images::parse_html_src(raw))?;
        let path = images::resolve_beside(&self.path, &src);
        let key = if !path.exists() && images::is_remote_src(&src) {
            crate::video::VideoStore::remote_key(&src)
        } else {
            crate::video::VideoStore::key_for(&path)
        };
        self.video.get(&key).is_some().then_some(key)
    }

    /// Play/pause toggle for an inline clip. Starting playback spawns a
    /// frame-pacing loop (`background_executor().timer`, same pattern as
    /// Zed's blink manager); the `generation` guard kills stale loops.
    fn toggle_video(&mut self, key: &str, cx: &mut Context<Self>) {
        let (playing, gen) = self.video.toggle(key);
        cx.notify();
        if playing {
            let dt = self.video.frame_dt(key);
            let key = key.to_string();
            let view = cx.entity();
            cx.spawn(async move |_, cx| {
                loop {
                    cx.background_executor().timer(dt).await;
                    let cont = cx.update(|cx| {
                        view.update(cx, |this, cx| {
                            let live = this.video.advance_if_active(&key, gen);
                            cx.notify();
                            live
                        })
                    });
                    if !cont {
                        break;
                    }
                }
            })
            .detach();
        }
    }

    /// `<details>` open row: chevron + editable summary, Notion-style.
    /// No card — just a paragraph-like toggle row. GFM is collapsed by
    /// default; `<details open>` starts expanded. Clicking the chevron
    /// toggles; clicking the text edits. `stop_propagation` keeps the
    /// chevron click from selecting the summary (checkbox-bug parity).
    fn render_details_row(
        &mut self,
        ix: usize,
        body: AnyElement,
        slash: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.proj();
        let pal = self.palette.clone();
        let block = p.blocks[ix].clone();
        let collapsed = self.is_details_collapsed(&block);
        let view = cx.entity();
        let src_start = block.source.start;
        // Notion-style: one chevron-down glyph, rotated -90° when collapsed.
        // The animation id folds in `collapsed` so every toggle remounts and
        // replays the rotation (one-shot `with_animation` runs on mount).
        let (from, to) = if collapsed {
            (0.0, -std::f32::consts::FRAC_PI_2)
        } else {
            (-std::f32::consts::FRAC_PI_2, 0.0)
        };
        let chev = svg()
            .path(crate::assets::path("chevron-down"))
            .size(px(10.))
            .text_color(pal.text_muted)
            .with_transformation(Transformation::rotate(radians(to)))
            .with_animation(
                ElementId::NamedInteger(
                    "details-chev".into(),
                    ((src_start as u64) << 1) | collapsed as u64,
                ),
                Animation::new(Duration::from_millis(180)).with_easing(ease_in_out),
                move |el, delta| {
                    el.with_transformation(Transformation::rotate(radians(
                        from + (to - from) * delta,
                    )))
                },
            );
        let mut el = h_flex()
            .w_full()
            .min_w_0()
            .items_start()
            .gap_1()
            .child(
                div()
                    .w(px(20.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(pal.text_muted)
                    .cursor_pointer()
                    .rounded(px(4.))
                    .hover(|el| el.bg(pal.background_element))
                    .child(chev)
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        view.update(cx, |this, cx| {
                            this.toggle_details(src_start, cx);
                        });
                    }),
            )
            .child(div().flex_1().min_w_0().child(body));
        if slash {
            el = el.child(self.render_slash_popover(cx));
        }
        el.into_any_element()
    }

    /// GFM default: `<details>` collapsed unless it has `open` or the user
    /// expanded it this session; `<details open>` expanded unless collapsed.
    fn is_details_collapsed(&self, block: &crate::display::ProjBlock) -> bool {
        let open = matches!(&block.extra, BlockExtra::Details { open: true, .. });
        if open {
            self.collapsed_html.contains(&block.source.start)
        } else {
            !self.expanded_html.contains(&block.source.start)
        }
    }

    /// Collapse/expand a `<details>` block. Collapsing pulls the caret out
    /// of the now-hidden body onto the summary row.
    fn toggle_details(&mut self, src_start: usize, cx: &mut Context<Self>) {
        let open = self
            .proj()
            .blocks
            .iter()
            .find(|b| b.source.start == src_start)
            .map(|b| matches!(&b.extra, BlockExtra::Details { open: true, .. }))
            .unwrap_or(false);
        if open {
            if self.collapsed_html.remove(&src_start) {
                cx.notify();
                return;
            }
            self.collapsed_html.insert(src_start);
        } else if self.expanded_html.remove(&src_start) {
            // Was expanded → collapse; pull caret onto the summary row.
        } else {
            self.expanded_html.insert(src_start);
            cx.notify();
            return;
        }
        let p = self.proj();
        if let Some(ix) = p.blocks.iter().position(|b| b.source.start == src_start) {
            if let Some((_, end_ix)) = crate::display::details_block_range(&p, ix) {
                let end_d = p.blocks[end_ix].display.end;
                if self.caret > p.blocks[ix].display.end && self.caret <= end_d {
                    self.caret = p.blocks[ix].display.end;
                    self.sel = None;
                }
            }
        }
        cx.notify();
    }

    /// Generic HTML blocks (`<iframe>`, `<details>`, raw `<div>`…): a small
    /// card with the tag name, a one-line source preview, and a draggable
    /// handle. GPUI has no HTML renderer (awesome-gpui lists no webview /
    /// HTML crate), so these stay as cards — with `Open` for remote
    /// `src`/`href` and the path toolbar for editing. Clicking the handle
    /// selects the block and loads its `src` (when it has one) into the
    /// path toolbar so it can be edited in place.
    fn render_html_block(&self, raw: &str, display_start: usize, cx: &mut Context<Self>) -> AnyElement {
        let pal = &self.palette;
        let selected = self.media_sel == Some(display_start);
        let tag = images::html_tag(raw).unwrap_or_else(|| "html".to_string());
        let src = images::parse_html_src(raw).unwrap_or_default();
        let remote = images::is_remote_src(&src);
        let preview = raw.lines().next().unwrap_or("").trim().chars().take(88).collect::<String>();
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, {
                let view = cx.entity();
                let src = src.clone();
                move |ev: &MouseDownEvent, window, cx| {
                    view.update(cx, |this, cx| {
                        // Handle click: select + prime the src editor.
                        this.select_html(display_start, src.clone(), window, cx);
                        let _ = ev;
                    });
                }
            })
            .child(
                h_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if selected { pal.primary } else { pal.border })
                    .bg(pal.background_panel)
                    .child(icon_el("code", if selected { pal.primary } else { pal.text_muted }))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(pal.markdown_text)
                                    .child(format!("<{tag}> HTML")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(pal.text_muted)
                                    .child(if preview.is_empty() { "empty block — click to edit".to_string() } else { preview }),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(pal.text_muted)
                            .child("⠿"),
                    )
                    .when(remote, |el| {
                        el.child(
                            Button::new(("html-open", display_start))
                                .ghost()
                                .xsmall()
                                .label("Open")
                                .on_click({
                                    let url = src.clone();
                                    cx.listener(move |_, _, _, cx| {
                                        cx.open_url(&url);
                                    })
                                }),
                        )
                    }),
            )
            .when(selected, |el| {
                el.child(self.render_media_toolbar(false, display_start, cx))
            })
            .into_any_element()
    }

    /// Select an HTML block from its handle and prime the toolbar inputs.
    fn select_html(&mut self, display_start: usize, src: String, window: &mut Window, cx: &mut Context<Self>) {
        self.media_sel = Some(display_start);
        self.caret = display_start;
        self.sel = None;
        self.mouse_dragging = false;
        self.clamp_caret();
        self.follow_caret = false;
        self.media_alt.update(cx, |st, cx| {
            st.set_value(String::new(), window, cx);
        });
        self.media_src.update(cx, |st, cx| {
            st.set_value(src, window, cx);
        });
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Alt + path toolbar under a selected image/video. Clicks inside stop
    /// at the panel so they never re-select (which would reset the inputs).
    fn render_media_toolbar(
        &self,
        show_alt: bool,
        display_start: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &self.palette;
        let field = |label: &'static str, input: &Entity<InputState>| {
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.text_muted)
                        .child(label),
                )
                .child(Input::new(input).cleanable(false))
        };
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.background_panel)
            .font_family(self.config.ui_font.family.clone())
            .text_size(self.ui_font_px())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when(show_alt, |el| el.child(field("Alt text", &self.media_alt)))
            .child(field("Path", &self.media_src))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new(("media-done", display_start))
                            .ghost()
                            .xsmall()
                            .label("Done")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.media_sel = None;
                                this.focus.focus(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("media-apply", display_start))
                            .xsmall()
                            .label("Apply")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_media_edit(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    /// Live Mermaid preview above the editable source: ready diagram as an
    /// image, error card with the message, or a "Rendering…" placeholder
    /// while the off-thread render runs. Mirrors `zorite`'s `MermaidStore`.
    fn render_mermaid(&mut self, source: &str, cx: &mut Context<Self>) -> AnyElement {
        let pal = self.palette.clone();
        let key: SharedString = source.to_string().into();
        if let Some((image, w, h)) = self.mermaid.get(&key) {
            return v_flex()
                .w_full()
                .min_w_0()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(pal.border)
                .bg(pal.background_panel)
                .child(
                    img(image)
                        .w(px(w.clamp(40.0, 1200.0)))
                        .h(px(h.clamp(20.0, 900.0))),
                )
                .into_any_element();
        }
        if let Some(err) = self.mermaid.error(&key) {
            let first = err.lines().next().unwrap_or("render failed").to_string();
            return h_flex()
                .w_full()
                .p_3()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .border_color(pal.error)
                .bg(pal.background_panel)
                .child(icon_el("diagram", pal.error))
                .child(
                    v_flex().flex_1().min_w_0()
                        .child(
                            div().text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(pal.markdown_text)
                                .child("Mermaid — could not render"),
                        )
                        .child(
                            div().text_xs()
                                .text_color(pal.text_muted)
                                .child(format!("{first} — edit the source below")),
                        ),
                )
                .into_any_element();
        }
        // Kick off the off-thread render at most once per source text.
        if self.mermaid.begin(key.clone()) {
            let src = source.to_string();
            let theme = mermaid::theme_for(&pal);
            let view = cx.entity();
            cx.spawn(async move |_weak, cx| {
                let result = cx
                    .background_spawn(async move {
                        let svg = gpui::SvgRenderer::new(std::sync::Arc::new(
                            crate::assets::Assets,
                        ));
                        mermaid::render_to_image(&src, theme, &svg, mermaid::RASTER_SCALE)
                    })
                    .await;
                let _ = cx.update(|cx| {
                    view.update(cx, |this, cx| {
                        this.mermaid.finish(key, result);
                        cx.notify();
                    })
                });
            })
            .detach();
        }
        h_flex()
            .w_full()
            .p_3()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(pal.border)
            .bg(pal.background_panel)
            .child(icon_el("diagram", pal.text_muted))
            .child(
                div().text_xs().text_color(pal.text_muted).child("Rendering diagram…"),
            )
            .into_any_element()
    }

    fn render_lang_chip(&self, lang: &str, caret: usize, cx: &mut Context<Self>) -> AnyElement {
        let label = if lang.is_empty() { "plain" } else { lang };
        Button::new(("lang", caret))
            .ghost()
            .xsmall()
            .label(label.to_string())
            .dropdown_menu({
                let entity = cx.entity();
                move |menu, _, _| {
                    let mut menu = menu;
                    for lang in CODE_LANGS {
                        let entity = entity.clone();
                        let label = if lang.is_empty() { "plain" } else { *lang };
                        let lang = lang.to_string();
                        menu =
                            menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.push_doc_undo();
                                    if let Some(caret) =
                                        this.doc.set_code_lang(this.caret, &lang)
                                    {
                                        this.sync_gfm();
                                        this.commit_caret(caret, window, cx);
                                    }
                                });
                            }));
                    }
                    menu
                }
            })
            .into_any_element()
    }

    /// GitHub-style copy button for a code block. Copies the block body,
    /// flashes a check ~1.2s, then reverts to the copy icon.
    fn render_copy_btn(&self, key: usize, code: String, cx: &mut Context<Self>) -> AnyElement {
        use std::time::{Duration, Instant};
        let pal = self.palette.clone();
        let copied = self
            .code_copied
            .get(&key)
            .map(|t| t.elapsed() < Duration::from_millis(1500))
            .unwrap_or(false);
        Button::new(("copy", key))
            .ghost()
            .xsmall()
            .icon(icon_el(
                if copied { "check" } else { "copy" },
                if copied { pal.success } else { pal.text_muted },
            ))
            .tooltip(if copied { "Copied!" } else { "Copy code" })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                let stamp = Instant::now();
                this.code_copied.insert(key, stamp);
                cx.notify();
                // Revert the flash unless a newer click superseded it.
                let view = cx.entity();
                cx.spawn(async move |_, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(1200))
                        .await;
                    let _ = cx.update(|cx| {
                        view.update(cx, |this, cx| {
                            if this.code_copied.get(&key) == Some(&stamp) {
                                this.code_copied.remove(&key);
                                cx.notify();
                            }
                        })
                    });
                })
                .detach();
            }))
            .into_any_element()
    }

    fn render_list(
        &mut self,
        ix: usize,
        items: &[crate::display::ListItem],
        ordered: bool,
        _body: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.proj();
        let pal = self.palette.clone();
        // Preview always wraps; "Wrap lines" only affects source view.
        let wrap = true;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .children(items.iter().enumerate().map(|(i, item)| {
                let text = p
                    .display
                    .get(item.display.clone())
                    .unwrap_or("")
                    .to_string();
                let bullet = if let Some(checked) = item.checked {
                    if checked {
                        "☑".to_string()
                    } else {
                        "☐".to_string()
                    }
                } else if ordered {
                    ordered_marker(item.indent, list_sibling_index(items, i))
                } else {
                    "•".to_string()
                };
                let indent = px((item.indent as f32) * 16.);
                let edit = self.render_edit(
                    item.display.clone(),
                    &text,
                    false,
                    None,
                    None,
                    wrap,
                    false,
                    None,
                    false,
                    cx,
                );
                h_flex()
                    .id(("li", item.display.start))
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap_2()
                    .pl(indent)
                    .child(
                        div()
                            .w(px(28.))
                            .pt(px(2.))
                            .text_color(pal.markdown_list_item)
                            .when(item.checked.is_some(), |el| {
                                el.cursor_pointer().on_mouse_down(MouseButton::Left, {
                                    let view = cx.entity();
                                    let block_ix = ix;
                                    move |_, window, cx| {
                                        cx.stop_propagation();
                                        view.update(cx, |this, cx| {
                                            this.toggle_task(block_ix, i, window, cx);
                                        });
                                    }
                                })
                            })
                            .child(bullet),
                    )
                    .child(div().flex_1().min_w_0().child(edit))
                    .into_any_element()
            }))
            .into_any_element()
    }

    fn render_table_block(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = self.proj();
        let pal = self.palette.clone();
        // Preview always wraps; "Wrap lines" only affects source view.
        let wrap = true;
        let caret = self.caret;
        let table_sel = self.table_sel;
        let dragging = self.mouse_dragging;
        let BlockExtra::Table { cells, rows, cols } = &p.blocks[ix].extra else {
            return div().into_any_element();
        };
        let cols = (*cols).max(1);
        let rows = (*rows).max(1);
        let mut grid: Vec<Vec<Option<crate::display::TableCell>>> = vec![vec![None; cols]; rows];
        for c in cells {
            if c.row < rows && c.col < cols {
                grid[c.row][c.col] = Some(c.clone());
            }
        }
        let in_table = p
            .block_at_display(caret)
            .is_some_and(|b| b.source == p.blocks[ix].source)
            || table_sel.is_some_and(|t| t.block_ix == ix);
        let modal_open = self.cmd_palette.is_some()
            || self.settings_open
            || self.command.is_some()
            || self.search.is_some();
        let tools_at = if in_table && !dragging && !modal_open {
            p.table_cell_at(caret)
                .filter(|(b, _)| b.source == p.blocks[ix].source)
                .map(|(_, c)| (c.row, c.col))
        } else {
            None
        };
        let grid_el = v_flex()
            .w_full()
            .min_w_0()
            .border_1()
            .border_color(pal.border)
            .rounded(px(8.))
            .children(grid.into_iter().enumerate().map(|(r, row)| {
                h_flex()
                    .w_full()
                    .items_stretch()
                    .children(row.into_iter().enumerate().map(|(c, cell)| {
                        let (disp, header, real) = if let Some(cell) = cell {
                            (cell.display, cell.header, true)
                        } else {
                            (0..0, r == 0, false)
                        };
                        let rect = table_sel
                            .filter(|t| t.block_ix == ix && t.is_multi())
                            .map(|t| t.normalize());
                        let in_sel = real && rect.is_some_and(|n| n.contains(r, c));
                        let edge_t = in_sel && rect.is_some_and(|n| r == n.r0);
                        let edge_b = in_sel && rect.is_some_and(|n| r == n.r1);
                        let edge_l = in_sel && rect.is_some_and(|n| c == n.c0);
                        let edge_r = in_sel && rect.is_some_and(|n| c == n.c1);
                        let text = p
                            .display
                            .get(disp.clone())
                            .unwrap_or("")
                            .replace('\t', "")
                            // Render in-cell breaks as real newlines. Both are
                            // 1 byte, so display offsets used for caret/hit
                            // mapping stay aligned.
                            .replace(crate::display::TABLE_CELL_BR, "\n")
                            .to_string();
                        // Suppress linear text-sel paint when using rectangular table_sel
                        // (linear ranges include in-between cells in reading order).
                        let saved_sel = if table_sel.is_some_and(|t| t.is_multi()) {
                            let s = self.sel.take();
                            let edit = self.render_edit(
                                disp, &text, header, None, None, wrap, false, None, false, cx,
                            );
                            self.sel = s;
                            edit
                        } else {
                            self.render_edit(disp, &text, header, None, None, wrap, false, None, false, cx)
                        };
                        let edit = saved_sel;
                        let show_tools = tools_at == Some((r, c));
                        let sel_color = pal.primary;
                        div()
                            .id(("td", r * 100 + c))
                            .relative()
                            .flex_1()
                            .flex_basis(relative(1. / cols as f32))
                            .min_w_0()
                            .h_full()
                            .min_h(px(28.))
                            .px_2()
                            .py_1()
                            .border_r_1()
                            .border_b_1()
                            .border_color(pal.border)
                            .when(header, |el| {
                                el.bg(pal.background_element)
                                    .font_weight(FontWeight::SEMIBOLD)
                            })
                            .child(edit)
                            .when(in_sel, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .when(edge_t, |e| e.border_t_2())
                                        .when(edge_b, |e| e.border_b_2())
                                        .when(edge_l, |e| e.border_l_2())
                                        .when(edge_r, |e| e.border_r_2())
                                        .border_color(sel_color),
                                )
                            })
                            .when(show_tools, |el| {
                                el.child(
                                    deferred(
                                        div()
                                            .absolute()
                                            .top(px(-52.))
                                            .left(px(-4.))
                                            .child(self.render_table_tools(cx)),
                                    )
                                    .with_priority(2),
                                )
                            })
                            .into_any_element()
                    }))
                    .into_any_element()
            }));
        div()
            .relative()
            .w_full()
            .min_w_0()
            .child(grid_el)
            .into_any_element()
    }

    /// Combined selection bubble + grid controls, floating over the table.
    fn render_table_tools(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = &self.palette;
        let n = self.table_sel.map(|t| t.normalize());
        let del_rows = n.map(|t| t.row_count()).unwrap_or(1);
        let del_cols = n.map(|t| t.col_count()).unwrap_or(1);
        let del_row_label = if del_rows > 1 { "Del rows" } else { "Del row" };
        let del_col_label = if del_cols > 1 { "Del cols" } else { "Del col" };
        v_flex()
            .gap_1()
            .p_1()
            .rounded(px(8.))
            .border_1()
            .border_color(pal.border)
            .bg(pal.background_panel)
            .shadow_sm()
            .child(
                h_flex()
                    .gap_1()
                    .child(self.mark_btn("B", Mark::Bold, cx))
                    .child(self.mark_btn("I", Mark::Italic, cx))
                    .child(self.mark_btn("U", Mark::Underline, cx))
                    .child(self.mark_btn("S", Mark::Strike, cx))
                    .child(self.mark_btn("<>", Mark::Code, cx)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(self.table_btn("row-above", "Row ↑", false, true, cx))
                    .child(self.table_btn("row-below", "Row ↓", false, false, cx))
                    .child(self.table_btn("col-left", "Col ←", true, true, cx))
                    .child(self.table_btn("col-right", "Col →", true, false, cx))
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                Button::new("tbl-del-row")
                                    .ghost()
                                    .xsmall()
                                    .label(del_row_label)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.table_delete_row(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                Button::new("tbl-del-col")
                                    .ghost()
                                    .xsmall()
                                    .label(del_col_label)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.table_delete_col(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                Button::new("tbl-del")
                                    .ghost()
                                    .xsmall()
                                    .label("Delete table")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.delete_current_table(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_table_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        // Kept for callers; tools now live in render_table_tools.
        self.render_table_tools(cx)
    }

    fn table_btn(
        &self,
        id: &'static str,
        label: &'static str,
        col: bool,
        before: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new(id)
                    .ghost()
                    .xsmall()
                    .label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.table_insert(col, before, window, cx);
                    })),
            )
            .into_any_element()
    }

    fn table_insert(
        &mut self,
        col: bool,
        before: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push_doc_undo();
        let caret = if col {
            self.doc.insert_table_col(self.caret, before)
        } else {
            self.doc.insert_table_row(self.caret, before)
        };
        let Some(caret) = caret else {
            return;
        };
        self.sync_gfm();
        self.table_sel = None;
        self.commit_caret(caret, window, cx);
    }

    fn table_delete_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_doc_undo();
        let caret = if let Some(rect) = self.table_sel.map(|t| t.normalize()) {
            self.doc.delete_table_rows(self.caret, rect.r0, rect.r1)
        } else {
            self.doc.delete_table_row(self.caret)
        };
        let Some(caret) = caret else {
            return;
        };
        self.sync_gfm();
        self.table_sel = None;
        self.commit_caret(caret, window, cx);
    }

    fn table_delete_col(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_doc_undo();
        let caret = if let Some(rect) = self.table_sel.map(|t| t.normalize()) {
            self.doc.delete_table_cols(self.caret, rect.c0, rect.c1)
        } else {
            self.doc.delete_table_col(self.caret)
        };
        let Some(caret) = caret else {
            return;
        };
        self.sync_gfm();
        self.table_sel = None;
        self.commit_caret(caret, window, cx);
    }

    fn delete_current_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_doc_undo();
        let Some(caret) = self.doc.delete_table(self.caret) else {
            return;
        };
        self.sync_gfm();
        self.table_sel = None;
        self.table_anchor = None;
        self.commit_caret(caret, window, cx);
    }

    fn render_bubble(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(_sel) = self.sel.clone().filter(|s| s.start != s.end) else {
            if self.link_open {
                return self.render_link_field(cx);
            }
            return div().into_any_element();
        };
        let p = &self.palette;
        let linked_url: Option<String> = if let Some(s) = self.sel.as_ref() {
            (s.start..s.end)
                .find_map(|i| self.proj().link_at(i).map(|(_, u)| u.to_string()))
                .or_else(|| self.proj().link_at(self.caret).map(|(_, u)| u.to_string()))
        } else {
            self.proj().link_at(self.caret).map(|(_, u)| u.to_string())
        };
        let is_linked = linked_url.is_some();

        h_flex()
            .gap_1()
            .px_2()
            .py_1()
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.background_panel)
            .shadow_sm()
            .child(self.mark_btn("B", Mark::Bold, cx))
            .child(self.mark_btn("I", Mark::Italic, cx))
            .child(self.mark_btn("U", Mark::Underline, cx))
            .child(self.mark_btn("S", Mark::Strike, cx))
            .child(self.mark_btn("<>", Mark::Code, cx))
            .child(
                Button::new("mk-link")
                    .ghost()
                    .xsmall()
                    .label("Link")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_toggle_link(&ToggleLink, window, cx);
                    })),
            )
            .when_some(linked_url.clone(), |el, url| {
                el.child(
                    Button::new("go-link")
                        .ghost()
                        .xsmall()
                        .icon(Icon::default().path(crate::assets::path("arrow-up-right")))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_link_url(&url, window, cx);
                        })),
                )
            })
            .when(is_linked, |el| {
                el.child(
                    Button::new("rm-link")
                        .ghost()
                        .xsmall()
                        .label("Unlink")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.remove_link_action(window, cx);
                        })),
                )
            })
            .when(self.link_open, |el| el.child(self.render_link_field(cx)))
            .into_any_element()
    }

    fn selection_bubble_for_block(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        // Never float the selection bubble above a modal overlay (palette,
        // settings, : command, search): the backdrop occludes the doc and
        // the modal's deferred pass already paints above the document.
        if self.cmd_palette.is_some()
            || self.settings_open
            || self.command.is_some()
            || self.search.is_some()
        {
            return None;
        }
        let p = self.proj();
        let this = p.blocks.get(ix)?;
        // Tables own their floating toolbar (marks + grid controls).
        if matches!(this.extra, BlockExtra::Table { .. }) {
            return None;
        }
        let show = if let Some(sel) = self.sel.as_ref().filter(|s| s.start != s.end) {
            let d0 = p.to_display(sel.start.min(sel.end));
            p.block_at_display(d0)
                .is_some_and(|b| b.source == this.source)
        } else if self.link_open {
            let d = self.caret;
            p.block_at_display(d)
                .is_some_and(|b| b.source == this.source)
        } else {
            false
        };
        if !show || self.mouse_dragging {
            return None;
        }
        // Float above the selected block (same deferred trick as the slash menu).
        Some(
            deferred(
                div()
                    .absolute()
                    .top(px(-40.))
                    .left(px(48.))
                    .child(self.render_bubble(cx)),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    fn mark_btn(&self, label: &'static str, mark: Mark, cx: &mut Context<Self>) -> AnyElement {
        div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new(("mk", label.len()))
                    .ghost()
                    .xsmall()
                    .label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_mark_action(mark, window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_link_field(&self, _cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        h_flex()
            .items_center()
            .gap_1()
            .child(div().text_xs().text_color(p.text_muted).child("url"))
            .child(
                div()
                    .text_sm()
                    .text_color(p.markdown_text)
                    .child(self.link_draft.render()),
            )
            .into_any_element()
    }

    fn on_insert_newline(
        &mut self,
        _: &InsertNewline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert_newline(false, window, cx);
    }
    fn on_insert_hard_break(
        &mut self,
        _: &InsertHardBreak,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert_newline(true, window, cx);
    }
    fn on_indent_tab(&mut self, _: &IndentTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
            self.finish_insert_undo();
            let before = self.snapshot();
            if let Some(range) = self.doc.tab_selection(sel, false) {
                self.undo.push(before);
                self.sync_gfm();
                self.sel = Some(range);
                self.mark_dirty();
                self.refresh(window, cx);
                self.sync_title(window);
                return;
            }
        }
        self.finish_insert_undo();
        let before = self.snapshot();
        if let Some(caret) = self.doc.tab(self.caret, false) {
            self.undo.push(before);
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
            return;
        }
        // Table cell navigation still uses the GFM helper.
        let p = self.proj();
        if matches!(
            p.block_at_display(self.caret).map(|b| &b.extra),
            Some(BlockExtra::Table { .. })
        ) {
            if let Some(caret) = self.doc.table_tab(self.caret, false) {
                self.push_doc_undo();
                self.sync_gfm();
                self.commit_caret(caret, window, cx);
            }
        }
    }
    fn on_outdent_tab(&mut self, _: &OutdentTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
            self.finish_insert_undo();
            let before = self.snapshot();
            if let Some(range) = self.doc.tab_selection(sel, true) {
                self.undo.push(before);
                self.sync_gfm();
                self.sel = Some(range);
                self.mark_dirty();
                self.refresh(window, cx);
                self.sync_title(window);
                return;
            }
        }
        self.finish_insert_undo();
        let before = self.snapshot();
        if let Some(caret) = self.doc.tab(self.caret, true) {
            self.undo.push(before);
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
            return;
        }
        let p = self.proj();
        if matches!(
            p.block_at_display(self.caret).map(|b| &b.extra),
            Some(BlockExtra::Table { .. })
        ) {
            if let Some(caret) = self.doc.table_tab(self.caret, true) {
                self.push_doc_undo();
                self.sync_gfm();
                self.commit_caret(caret, window, cx);
            }
        }
    }
    fn on_toggle_bold(&mut self, _: &ToggleBold, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        self.toggle_mark_action(Mark::Bold, window, cx);
    }
    fn on_toggle_italic(&mut self, _: &ToggleItalic, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        self.toggle_mark_action(Mark::Italic, window, cx);
    }
    fn on_toggle_strike(&mut self, _: &ToggleStrike, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        self.toggle_mark_action(Mark::Strike, window, cx);
    }
    fn on_toggle_code(&mut self, _: &ToggleCode, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        self.toggle_mark_action(Mark::Code, window, cx);
    }
    fn on_toggle_underline(
        &mut self,
        _: &ToggleUnderline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        self.toggle_mark_action(Mark::Underline, window, cx);
    }
    fn on_toggle_link(&mut self, _: &ToggleLink, window: &mut Window, cx: &mut Context<Self>) {
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        if self.sel.as_ref().is_none_or(|s| s.start == s.end) && !self.link_open {
            return;
        }
        self.link_open = true;
        self.link_draft.clear();
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn on_cut_selection(&mut self, _: &CutSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) else {
            return;
        };
        window.prevent_default();
        if !self.write_selection_clipboard(sel.start, sel.end, cx) {
            return;
        }
        self.push_doc_undo();
        let a = sel.start.min(sel.end);
        let b = sel.end.max(sel.start);
        self.caret = self.doc.delete_display(a..b);
        self.sync_gfm();
        self.sel = None;
        self.mark_dirty();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn on_copy_selection(
        &mut self,
        _: &CopySelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if self.link_open {
            if !self.link_draft.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(
                    self.link_draft.as_str().to_string(),
                ));
            }
            return;
        }
        if let Some(buf) = self.command.as_ref() {
            if !buf.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(buf.as_str().to_string()));
            }
            return;
        }
        if let Some((buf, _)) = self.search.as_ref() {
            if !buf.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(buf.as_str().to_string()));
            }
            return;
        }
        if let Some(state) = self.cmd_palette.as_ref() {
            if !state.query.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(
                    state.query.as_str().to_string(),
                ));
            }
            return;
        }
        let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) else {
            return;
        };
        self.write_selection_clipboard(sel.start, sel.end, cx);
    }

    fn write_selection_clipboard(&self, start: usize, end: usize, cx: &mut Context<Self>) -> bool {
        let p = self.proj();
        let a = start.min(end).min(p.display.len());
        let b = start.max(end).min(p.display.len());
        if a >= b {
            return false;
        }
        let display = p.display.get(a..b).unwrap_or("").to_string();
        let gfm = self.doc.gfm_range(a..b);
        if gfm.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(display));
        } else {
            cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(display, gfm));
        }
        true
    }

    fn on_paste_clipboard(
        &mut self,
        _: &PasteClipboard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay_input_focused(window, cx) {
            // Let the focused panel input handle paste natively.
            cx.propagate();
            return;
        }
        window.prevent_default();
        if self.try_paste_image(window, cx) {
            return;
        }
        let Some(clip) = cx.read_from_clipboard() else {
            return;
        };
        let Some(raw) = clip.text() else {
            return;
        };
        let text = raw.replace("\r\n", "\n").replace('\r', "\n");
        if text.is_empty() {
            return;
        }
        if self.link_open {
            self.link_draft.insert_str(&text.replace('\n', ""));
            cx.notify();
            return;
        }
        if let Some(buf) = self.command.as_mut() {
            buf.insert_str(&text.replace('\n', ""));
            cx.notify();
            return;
        }
        if let Some((buf, _)) = self.search.as_mut() {
            buf.insert_str(&text.replace('\n', ""));
            cx.notify();
            return;
        }
        if let Some(state) = self.cmd_palette.as_mut() {
            state.query.insert_str(&text.replace('\n', ""));
            state.index = 0;
            cx.notify();
            return;
        }
        if self.view_source {
            return;
        }
        let gfm = match clip.metadata() {
            Some(meta) if !meta.is_empty() && !meta.trim_start().starts_with('{') => meta.clone(),
            _ => text.clone(),
        };
        self.paste_rich(&text, &gfm, window, cx);
    }

    fn paste_rich(&mut self, text: &str, gfm: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.push_doc_undo();
        if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
            let a = sel.start.min(sel.end);
            let b = sel.end.max(sel.start);
            self.caret = self.doc.delete_display(a..b);
            self.sel = None;
        }
        let in_code = self
            .proj()
            .block_at_display(self.caret)
            .is_some_and(|b| matches!(b.kind, BlockKind::Code));
        if in_code {
            self.caret = self.doc.insert_text(self.caret, None, text, self.sticky);
        } else {
            self.caret = self.doc.paste_gfm(self.caret, gfm);
        }
        self.sync_gfm();
        self.sel = None;
        self.mark_dirty();

        // If the pasted text was or ended with a bare URL, auto-link it with a two-step undo stack.
        // `push_doc_undo` above already recorded the state before pasting (step 1).
        // `try_auto_link_preceding_url` will push the plaintext-pasted state before linkifying (step 2).
        // So first Cmd-Z undoes the link (leaving plaintext URL), second Cmd-Z undoes the paste.
        if !in_code {
            self.try_auto_link_preceding_url(self.caret);
        }

        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn on_select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_input_focused(window, cx) {
            cx.propagate();
            return;
        }
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        window.prevent_default();
        self.clear_pending();
        let end = self.proj().display.len();
        self.sel = Some(0..end);
        self.caret = end;
        self.visual_anchor = Some(0);
        self.mouse_anchor = Some(0);
        self.refresh(window, cx);
    }

    fn on_indent_shift(&mut self, _: &IndentShift, window: &mut Window, cx: &mut Context<Self>) {
        self.indent_key(IndentOpKind::Indent, window, cx);
    }
    fn on_dedent_shift(&mut self, _: &DedentShift, window: &mut Window, cx: &mut Context<Self>) {
        self.indent_key(IndentOpKind::Dedent, window, cx);
    }
    fn on_auto_indent(&mut self, _: &AutoIndent, window: &mut Window, cx: &mut Context<Self>) {
        self.indent_key(IndentOpKind::Auto, window, cx);
    }

    /// `>` / `<` / `=` in Normal/Visual:
    /// - with a visual selection: apply to the selection (Helix `>` after
    ///   `x`, Vim visual `>`);
    /// - repeated (`>>`, `<<`, `==`): current line, honoring counts;
    /// - otherwise: arm `pending_op` for a motion (`>j`, `>G`, `gg=G`).
    fn indent_key(&mut self, kind: IndentOpKind, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        if self.mode.is_visual() {
            if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
                self.clear_pending();
                // Like Vim, shifting exits visual (gv reselects; anchor would
                // otherwise snap a stale selection back on refresh).
                self.mode = Mode::Normal;
                self.visual_anchor = None;
                self.apply_indent_range(sel, kind, window, cx);
                return;
            }
        }
        if self.pending_op == Some(kind) {
            let count = take_count(&mut self.pending_count);
            self.clear_pending();
            let mut range = visual_line_range(&self.source, self.caret);
            for _ in 1..count {
                range = extend_visual_line(&self.source, range, 1);
            }
            self.apply_indent_range(range, kind, window, cx);
        } else {
            self.pending_g = false;
            self.pending_d = false;
            self.pending_replace = None;
            self.pending_find = None;
            self.pending_bracket = None;
            self.pending_op = Some(kind);
            cx.notify();
        }
    }

    /// Apply an indent operator to a display range. List items go through
    /// the tree-aware `tab` ops (nesting); anything else gets 2-space
    /// shifts; `=` aligns each line with its previous non-blank line.
    fn apply_indent_range(
        &mut self,
        range: Range<usize>,
        kind: IndentOpKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let display_len = self.proj().display.len();
        let a = range.start.min(range.end).min(display_len);
        let b = range.start.max(range.end).min(display_len);
        // Collect logical line starts covered by the range first (offsets
        // from the pre-edit snapshot; applied bottom-up / top-down below).
        let display = self.proj().display;
        let mut starts = Vec::new();
        let mut line = logical_line_range(&display, a);
        // Include the line containing `b` (exclusive end may sit at its start).
        loop {
            starts.push(line.start);
            if line.end >= b || line.end >= display.len() {
                break;
            }
            let next = logical_line_range(&display, line.end + 1);
            if next.start <= line.start {
                break;
            }
            line = next;
            if starts.len() > 2000 {
                break;
            }
        }
        if starts.is_empty() {
            return;
        }
        self.push_doc_undo();
        match kind {
            IndentOpKind::Indent | IndentOpKind::Dedent => {
                let outdent = kind == IndentOpKind::Dedent;
                // Bottom-up so earlier line starts stay valid.
                for s in starts.iter().rev() {
                    let at = (*s).min(self.proj().display.len());
                    if self.doc.tab(at, outdent).is_some() {
                        continue;
                    }
                    if outdent {
                        let disp = self.proj().display;
                        let line = logical_line_range(&disp, at);
                        let text = disp.get(line.clone()).unwrap_or("");
                        let strip = if text.starts_with("  ") {
                            2
                        } else if text.starts_with(' ') || text.starts_with('\t') {
                            1
                        } else {
                            0
                        };
                        if strip > 0 {
                            self.doc.delete_display(at..at + strip);
                        }
                    } else {
                        let sticky = self.sticky.clone();
                        self.doc.insert_text(at, None, "  ", sticky);
                    }
                }
                self.sync_gfm();
                // Land on the first non-blank of the first touched line.
                let first = starts[0].min(self.proj().display.len());
                let disp = self.proj().display;
                self.commit_caret(first_non_blank_in(&disp, logical_line_range(&disp, first)), window, cx);
            }
            IndentOpKind::Auto => {
                // Top-down: each line takes the previous non-blank line's
                // indent (spaces; tabs count as 2).
                let mut delta: isize = 0;
                for (i, s) in starts.iter().enumerate() {
                    let at = ((*s) as isize + delta).max(0) as usize;
                    let at = at.min(self.proj().display.len());
                    let disp = self.proj().display;
                    let line = logical_line_range(&disp, at);
                    let text = disp.get(line.clone()).unwrap_or("").to_string();
                    if text.trim().is_empty() {
                        continue;
                    }
                    let cur_w = indent_width(&text);
                    let want = if i == 0 {
                        // First line: match the previous non-blank line above it.
                        prev_indent_before(&disp, line.start)
                    } else {
                        // Previous line in the (already re-indented) buffer.
                        prev_indent_before(&disp, line.start)
                    };
                    let want = want.unwrap_or(cur_w);
                    if want == cur_w {
                        continue;
                    }
                    if want > cur_w {
                        let pad = " ".repeat(want - cur_w);
                        let sticky = self.sticky.clone();
                        self.doc.insert_text(at, None, &pad, sticky);
                        delta += (want - cur_w) as isize;
                    } else {
                        // Remove plain leading spaces/tabs only (list-aware
                        // tab() would fight marker indents; auto-indent is
                        // aimed at code/text lines).
                        let mut rm = 0usize;
                        let mut w = 0usize;
                        for c in text.chars() {
                            if w >= cur_w - want {
                                break;
                            }
                            if c == ' ' {
                                rm += 1;
                                w += 1;
                            } else if c == '\t' {
                                rm += 1;
                                w += 2;
                            } else {
                                break;
                            }
                        }
                        if rm > 0 {
                            self.doc.delete_display(at..at + rm);
                            delta -= rm as isize;
                        }
                    }
                }
                self.sync_gfm();
                let first = ((starts[0]) as isize + 0).max(0) as usize;
                let first = first.min(self.proj().display.len());
                let disp = self.proj().display;
                self.commit_caret(first_non_blank_in(&disp, logical_line_range(&disp, first)), window, cx);
            }
        }
    }

    /// Consume `pending_op` for linewise motions (`>j` / `>k` / `>G` /
    /// `gg` with `=` armed). Returns true when the motion was consumed as
    /// an operator range.
    fn consume_pending_op_for_lines(
        &mut self,
        dir: i8,
        to_end: bool,
        to_start: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(kind) = self.pending_op.take() else {
            return false;
        };
        let count = take_count(&mut self.pending_count);
        self.pending_g = false;
        self.pending_d = false;
        self.pending_replace = None;
        self.pending_find = None;
        self.pending_bracket = None;
        let range = if to_end {
            let first = visual_line_range(&self.source, self.caret);
            first.start..self.source.len()
        } else if to_start {
            0..visual_line_range(&self.source, self.caret).end
        } else {
            // Operator + motion covers the motion (`>j` = current + next).
            let mut r = visual_line_range(&self.source, self.caret);
            for _ in 0..count {
                r = extend_visual_line(&self.source, r, dir);
            }
            r
        };
        self.apply_indent_range(range, kind, window, cx);
        true
    }

    fn on_change_op(&mut self, _: &ChangeOp, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        // Visual change: delete the selection, enter insert (like Vim `s`).
        if self.mode.is_visual() {
            if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
                self.clear_pending();
                self.push_doc_undo();
                let a = sel.start.min(sel.end);
                let b = sel.end.max(sel.start);
                self.caret = self.doc.delete_display(a..b);
                self.sync_gfm();
                self.sel = None;
                self.mark_dirty();
                self.enter_insert(Caret::Offset(self.caret), window, cx);
                self.sync_title(window);
                return;
            }
        }
        // `cc`: whole current line, then insert.
        if self.pending_change {
            self.clear_pending();
            self.delete_current_line(window, cx);
            // `delete_current_line` leaves Normal + a valid caret: drop in.
            self.enter_insert(Caret::Offset(self.caret), window, cx);
            self.sync_title(window);
            return;
        }
        self.pending_g = false;
        self.pending_d = false;
        self.pending_op = None;
        self.pending_replace = None;
        self.pending_find = None;
        self.pending_bracket = None;
        self.pending_obj = None;
        self.pending_change = true;
        cx.notify();
    }

    /// Helix `m`: match mode — `miw` / `maw` / `mi"` / `ma{` … select the
    /// text object (Select mode, like Helix).
    fn on_match_object(&mut self, _: &MatchObject, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.editor != EditorKind::Helix || !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        self.pending_g = false;
        self.pending_d = false;
        self.pending_op = None;
        self.pending_replace = None;
        self.pending_find = None;
        self.pending_bracket = None;
        self.pending_change = false;
        self.pending_obj = Some((ObjOp::Select, None));
        cx.notify();
    }

    /// Consume `i` / `a` after a text-object operator (`d`, `c`, visual `v`,
    /// Helix `m`) as inner/around. Returns true when the key was consumed.
    fn arm_obj_inner(&mut self, inner: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.is_modal_nav() || self.view_source || self.cmd_palette.is_some() {
            return false;
        }
        // Already choosing an object — a second i/a just flips inner/around.
        if let Some((_, slot)) = self.pending_obj.as_mut() {
            window.prevent_default();
            *slot = Some(inner);
            cx.notify();
            return true;
        }
        let op = if self.pending_d {
            ObjOp::Delete
        } else if self.pending_change {
            ObjOp::Change
        } else if self.mode == Mode::Visual && self.config.editor == EditorKind::Vim {
            // Vim `vi…` / `va…`: object-select inside the visual session.
            ObjOp::Select
        } else {
            return false;
        };
        window.prevent_default();
        self.pending_g = false;
        self.pending_d = false;
        self.pending_change = false;
        self.pending_op = None;
        self.pending_replace = None;
        self.pending_find = None;
        self.pending_bracket = None;
        self.pending_obj = Some((op, Some(inner)));
        cx.notify();
        true
    }

    /// Resolve + apply a text object (`w`, `"`, `'`, `` ` ``, `(`, `[`, `{`,
    /// `<` and closers; `b` = parens, `B` = braces).
    fn commit_text_object(&mut self, obj: char, window: &mut Window, cx: &mut Context<Self>) {
        let Some((op, inner_opt)) = self.pending_obj.take() else {
            return;
        };
        let inner = inner_opt.unwrap_or(true);
        self.pending_g = false;
        self.pending_d = false;
        self.pending_change = false;
        self.pending_op = None;
        self.pending_replace = None;
        self.pending_find = None;
        self.pending_bracket = None;
        let p = self.proj();
        let display = p.display.clone();
        let caret = self.caret.min(display.len());
        let range: Option<Range<usize>> = match obj {
            'w' | 'W' => {
                let big = obj == 'W';
                let r = if big {
                    crate::motion::big_word_range_at(&display, caret)
                } else {
                    word_range_at(&display, caret)
                };
                if r.start >= r.end {
                    None
                } else if inner {
                    Some(r)
                } else {
                    // `aw`: word + trailing blanks, else leading blanks.
                    let mut end = r.end;
                    while end < display.len()
                        && (display.as_bytes()[end] == b' ' || display.as_bytes()[end] == b'\t')
                    {
                        end += 1;
                    }
                    if end == r.end {
                        let mut start = r.start;
                        while start > 0
                            && (display.as_bytes()[start - 1] == b' '
                                || display.as_bytes()[start - 1] == b'\t')
                        {
                            start -= 1;
                        }
                        Some(start..r.end)
                    } else {
                        Some(r.start..end)
                    }
                }
            }
            '(' | ')' | 'b' => Self::delim_object(&display, caret, '(', ')', inner),
            '[' | ']' => Self::delim_object(&display, caret, '[', ']', inner),
            '{' | '}' | 'B' => Self::delim_object(&display, caret, '{', '}', inner),
            '<' | '>' => Self::delim_object(&display, caret, '<', '>', inner),
            '"' | '\'' | '`' => {
                crate::motion::quote_around(&display, caret, obj).map(|(o, c)| {
                    if inner {
                        o + obj.len_utf8()..c
                    } else {
                        o..c + obj.len_utf8()
                    }
                })
            }
            _ => None,
        };
        let Some(range) = range.filter(|r| r.start < r.end) else {
            self.status = "no match".into();
            cx.notify();
            return;
        };
        match op {
            ObjOp::Select => {
                self.mode = if self.config.editor == EditorKind::Helix {
                    Mode::Select
                } else {
                    Mode::Visual
                };
                self.visual_anchor = Some(range.start);
                self.caret = range.end.min(display.len());
                self.sel = Some(range);
                self.refresh_raw(window, cx);
            }
            ObjOp::Delete => {
                window.prevent_default();
                self.push_doc_undo();
                let caret = self.doc.delete_display(range);
                self.sync_gfm();
                self.mode = Mode::Normal;
                self.sel = None;
                self.visual_anchor = None;
                self.commit_caret(caret, window, cx);
            }
            ObjOp::Change => {
                window.prevent_default();
                self.push_doc_undo();
                let a = range.start;
                self.doc.delete_display(range);
                self.sync_gfm();
                self.sel = None;
                self.visual_anchor = None;
                self.mark_dirty();
                self.enter_insert(Caret::Offset(a), window, cx);
                self.sync_title(window);
            }
        }
    }

    fn delim_object(
        display: &str,
        caret: usize,
        open: char,
        close: char,
        inner: bool,
    ) -> Option<Range<usize>> {
        let (o, c) = crate::motion::pair_around(display, caret, open, close)?;
        if inner {
            let s = o + open.len_utf8();
            (s < c).then(|| s..c)
        } else {
            Some(o..c + close.len_utf8())
        }
    }

    fn render_source_view(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.hits.clear();
        let pal = self.palette.clone();
        let text = self.source.clone();
        let hs = crate::syntax::highlights("markdown", &text, &pal);
        let view = cx.entity();
        let focus = self.focus.clone();
        let family = Some(gpui::SharedString::from(
            self.config.buffer_font.family.clone(),
        ));
        let font_px = Some(self.buffer_font_px());
        // Source view honors the "Wrap lines" toggle (preview always wraps).
        let src_wrap = self.config.wrap_motions;
        let body = surface::edit_text(
            view,
            focus,
            &mut self.hits,
            0,
            text,
            hs,
            None,
            false,
            src_wrap,
            false,
            false,
            &pal,
            Some(""),
            family,
            font_px,
            false,
            None,
            None,
            |_, _, _, _, _, _| {},
            |_, _, _| {},
        );
        let full_width = self.config.full_width;
        vec![div()
            .id("source-view")
            .when(!full_width, |el| el.w(px(COLUMN_PX)).mx_auto())
            .when(full_width, |el| el.w_full())
            .max_w_full()
            .min_w_0()
            .px_8()
            .py_2()
            .child(body)
            .into_any_element()]
    }

    fn render_surface(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.view_source {
            return self.render_source_view(cx);
        }
        if !cx.has_active_drag() {
            self.block_dragging = None;
            self.block_drag_gap = None;
        }
        if cx.has_active_drag() {
            self.block_menu = None;
        }
        self.hits.clear();
        self.code_scroll_seen.clear();
        let full_width = self.config.full_width;
        let p = self.proj();
        let us = wysiwyg::units(&p);
        let n = us.len();
        // Body ranges of collapsed `<details>` blocks — those rows render empty.
        // All `<details>` ranges (for body indent); collapsed state comes
        // from `is_details_collapsed` (GFM closed-by-default).
        let all_details: Vec<(usize, usize)> = p
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b.extra, BlockExtra::Details { .. }))
            .filter_map(|(ix, _)| crate::display::details_block_range(&p, ix))
            .collect();
        let hidden: Vec<(usize, usize)> = p
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                matches!(b.extra, BlockExtra::Details { .. }) && self.is_details_collapsed(b)
            })
            .filter_map(|(ix, _)| crate::display::details_block_range(&p, ix))
            .collect();
        let mut kids = Vec::new();
        // Which rows are hidden (collapsed `<details>` bodies + `</details>`
        // chrome). Precomputed so the loop can skip the last-visible-row
        // margin and keep hidden rows at true zero height.
        let gone: Vec<bool> = us
            .iter()
            .copied()
            .map(|unit| {
                matches!(p.blocks[unit.block].extra, BlockExtra::DetailsClose)
                    || hidden
                        .iter()
                        .any(|(a, b)| unit.block > *a && unit.block <= *b)
            })
            .collect();
        let last_visible = gone.iter().rposition(|g| !g);
        for (ui, unit) in us.iter().copied().enumerate() {
            let view = cx.entity();
            let group = SharedString::from(format!("row-{ui}"));
            let dragging = self.block_dragging == Some(ui);
            let row_gone = gone[ui];
            let content: AnyElement = if row_gone {
                div().into_any_element()
            } else if let Some(item_ix) = unit.item {
                self.render_list_item_row(unit.block, item_ix, cx)
            } else {
                self.render_block(unit.block, cx)
            };
            let disp = wysiwyg::unit_display(&p, unit);
            let start = disp.start;
            let end = disp.end;
            let list_item = unit.item;
            // Anything inside `<details>…</details>` renders slightly indented
            // (Notion-style nesting). Depth-counted so nested disclosures
            // indent further. The open row itself stays flush.
            let details_depth = all_details
                .iter()
                .filter(|(a, b)| unit.block > *a && unit.block < *b)
                .count();
            // Split-out nested fences keep their source fence indent as extra
            // row padding so code under lists/details looks nested.
            let code_indent = match &p.blocks[unit.block].extra {
                BlockExtra::Code { indent, .. } => (*indent).min(8) as f32 * 3.0,
                _ => 0.0,
            };
            let nest_extra = details_depth as f32 * 16.0 + code_indent;
            kids.push(
                div()
                    .id(("row", ui))
                    .group(group.clone())
                    .relative()
                    // Never compress rows to fit: they must overflow so the
                    // scrollbar thumb reflects real content (same as slash).
                    .flex_shrink_0()
                    .when(!full_width, |el| el.w(px(COLUMN_PX)).mx_auto())
                    .when(full_width, |el| el.w_full())
                    .max_w_full()
                    .min_w_0()
                    .px_8()
                    .when(nest_extra > 0.0 && !row_gone, |el| el.pl(px(32.0 + nest_extra)))
                    .when(list_item.is_some() && !row_gone, |el| el.py_1())
                    .when(list_item.is_none() && !row_gone, |el| el.py_2())
                    // Inter-row spacing lives here (not parent `gap_1`) so
                    // hidden rows collapse to true zero height: a closed
                    // `<details>` measures exactly one row. Skipped on the
                    // last visible row (gap never trails).
                    .when(!row_gone && Some(ui) != last_visible, |el| el.mb_1())
                    .when(dragging, |el| el.opacity(0.45))
                    .on_mouse_down(MouseButton::Left, {
                        let view = view.clone();
                        move |ev: &MouseDownEvent, window, cx| {
                            view.update(cx, |this, cx| {
                                this.block_menu = None;
                                let d = surface::index_for_point(&this.hits, ev.position)
                                    .unwrap_or_else(|| {
                                        let _ = start;
                                        end
                                    });
                                this.click_display(
                                    d,
                                    ev.modifiers.shift,
                                    ev.modifiers.platform || ev.modifiers.control,
                                    ev.click_count,
                                    window,
                                    cx,
                                );
                            });
                        }
                    })
                    .can_drop(|v, _, _| v.downcast_ref::<DragBlock>().is_some())
                    // Gap is owned by the surface-level scroll-aware handler —
                    // per-row bounds fought it and showed dishonest edges.
                    .on_drop(cx.listener(|this, drag: &DragBlock, window, cx| {
                        this.drop_block_at(drag.ix, window, cx);
                    }))
                    .child(content)
                    .child(self.render_drop_edge(ui, n, cx))
                    .child(self.render_block_handle(ui, group, list_item.is_some(), cx))
                    .children(self.render_handle_menu(ui, list_item.is_some(), cx))
                    .children(self.selection_bubble_for_unit(unit.block, list_item, cx))
                    .into_any_element(),
            );
        }
        let overscroll = self.overscroll_px();
        if overscroll > px(0.) {
            kids.push(
                div()
                    .w_full()
                    .h(overscroll)
                    .flex_shrink_0()
                    .into_any_element(),
            );
        }
        let seen = std::mem::take(&mut self.code_scroll_seen);
        self.code_scroll.retain(|k, _| seen.contains(k));
        kids
    }

    fn unit_handle_top(&self, ix: usize, list_item: bool) -> gpui::Pixels {
        if list_item {
            // Row has py_1 (4px). Checkbox / bullet container is 22px tall (center at 4 + 11 = 15px).
            // Handle is 20px tall (center at top + 10px). Center alignment: 15 - 10 = 5px.
            return px(5.);
        }
        let p = self.proj();
        let us = wysiwyg::units(&p);
        let Some(unit) = us.get(ix) else {
            return px(10.);
        };
        let Some(block) = p.blocks.get(unit.block) else {
            return px(10.);
        };
        match &block.extra {
            BlockExtra::Heading(level) => {
                let base = self.config.markdown_font.size.clamp(8, 48) as f32;
                let scale = match level {
                    1 => 2.0,
                    2 => 1.5,
                    3 => 1.25,
                    4 => 1.0,
                    5 => 0.875,
                    6 => 0.85,
                    _ => 1.0,
                };
                let font_size = base * scale;
                // Text line-height in GPUI (~1.3-1.4x font size) + visual baseline adjustment
                let line_height = font_size * 1.35;
                let pt = if *level <= 2 { 8.0 } else { 0.0 };
                // Row has py_2 (8px). Content starts at 8px from row top.
                // Center of the first text line is: 8 + pt + line_height / 2.
                // Handle is 20px tall (16px icon + 2px padding each side).
                // Center the handle with: line_center - 10 (+ 3px downward nudge for visual cap-height centering).
                let top = 8.0 + pt + (line_height / 2.0) - 10.0 + 3.0;
                px(top.max(0.0).round())
            }
            _ => px(10.),
        }
    }

    fn render_block_handle(
        &self,
        ix: usize,
        group: SharedString,
        list_item: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pal = &self.palette;
        let muted = pal.text_muted;
        let p = self.proj();
        let us = wysiwyg::units(&p);
        let preview = us
            .get(ix)
            .map(|u| {
                let t = p
                    .display
                    .get(wysiwyg::unit_display(&p, *u))
                    .unwrap_or("")
                    .trim();
                if t.is_empty() {
                    "Empty".to_string()
                } else {
                    let mut s = t.lines().next().unwrap_or(t).to_string();
                    if s.chars().count() > 40 {
                        s = s.chars().take(40).collect();
                        s.push('…');
                    }
                    s
                }
            })
            .unwrap_or_else(|| "Block".into());
        let drag = DragBlock {
            ix,
            preview,
            bg: pal.background_panel,
            fg: pal.markdown_text,
            border: pal.border,
        };
        let force = self.block_dragging == Some(ix) || self.block_menu == Some(ix);
        let view = cx.entity();
        let handle = div()
            .id(("grip", ix))
            .p(px(2.))
            .rounded(px(4.))
            .cursor(CursorStyle::OpenHand)
            .hover(|el| el.bg(pal.background_element))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click({
                let view = view.clone();
                move |_: &ClickEvent, _, cx| {
                    view.update(cx, |this, cx| {
                        if this.block_dragging.is_some() {
                            return;
                        }
                        this.block_menu = if this.block_menu == Some(ix) {
                            None
                        } else {
                            Some(ix)
                        };
                        cx.notify();
                    });
                }
            })
            .on_drag(drag, {
                let view = view.clone();
                move |drag, _, _, cx| {
                    view.update(cx, |this, cx| {
                        this.mouse_dragging = false;
                        this.block_menu = None;
                        this.block_dragging = Some(drag.ix);
                        // None until the pointer actually picks a gap — avoids a
                        // no-op drop when the user hasn't moved yet (gap==from).
                        this.block_drag_gap = None;
                        cx.notify();
                    });
                    cx.new(|_| drag.clone())
                }
            })
            .child(icon_el("grip-vertical", muted));

        let top = self.unit_handle_top(ix, list_item);

        div()
            .id(("grip-wrap", ix))
            .absolute()
            .left(px(2.))
            .top(top)
            .opacity(if force { 1. } else { 0. })
            .group_hover(group, |s| s.opacity(1.))
            .child(handle)
            .into_any_element()
    }

    fn render_handle_menu(
        &self,
        ix: usize,
        list_item: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.block_menu != Some(ix) || cx.has_active_drag() {
            return None;
        }
        let top = self.unit_handle_top(ix, list_item);
        let pal = &self.palette;
        let err = pal.error;
        let muted = pal.text_muted;
        let text = pal.markdown_text;
        let panel = pal.background_panel;
        let border = pal.border;
        let hover = pal.background_element;
        Some(
            deferred(
                v_flex()
                    .id(("grip-menu", ix))
                    .absolute()
                    .left(px(28.))
                    .top(top)
                    .w(px(168.))
                    .py_1()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .bg(panel)
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .id(("grip-dup", ix))
                            .w_full()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|el| el.bg(hover))
                            .on_mouse_down(MouseButton::Left, {
                                let view = cx.entity();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.block_menu = None;
                                        this.duplicate_block_at(ix, window, cx);
                                    });
                                }
                            })
                            .child(icon_el("copy", muted))
                            .child(div().text_sm().text_color(text).child("Duplicate")),
                    )
                    .child(
                        h_flex()
                            .id(("grip-del", ix))
                            .w_full()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|el| el.bg(hover))
                            .on_mouse_down(MouseButton::Left, {
                                let view = cx.entity();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.block_menu = None;
                                        this.delete_block_at(ix, window, cx);
                                    });
                                }
                            })
                            .child(icon_el("trash-2", err))
                            .child(div().text_sm().text_color(err).child("Delete")),
                    ),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    fn render_list_item_row(
        &mut self,
        block_ix: usize,
        item_ix: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.proj();
        let pal = self.palette.clone();
        // Preview always wraps; "Wrap lines" only affects source view.
        let wrap = true;
        let block = p.blocks[block_ix].clone();
        let BlockExtra::List { items, ordered } = &block.extra else {
            return div().into_any_element();
        };
        let Some(item) = items.get(item_ix) else {
            return div().into_any_element();
        };
        let text = p
            .display
            .get(item.display.clone())
            .unwrap_or("")
            .to_string();
        let checked = item.checked;
        let indent_level = item.indent;
        let indent = px((item.indent as f32) * 16.);
        let ordered = *ordered;
        let sibling = list_sibling_index(items, item_ix);
        let marker = match checked {
            Some(true) => None,
            Some(false) => None,
            None if ordered => Some(ordered_marker(indent_level, sibling)),
            None => None,
        };
        let edit = self.render_edit(
            item.display.clone(),
            &text,
            false,
            None,
            None,
            wrap,
            false,
            None,
            false,
            cx,
        );
        h_flex()
            .id(("li", item.display.start))
            .w_full()
            .min_w_0()
            .items_start()
            .gap_2()
            .pl(indent)
            .child(
                div()
                    .w(px(28.))
                    .h(px(22.))
                    .mt(px(2.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(pal.markdown_list_item)
                    .when(checked.is_some(), |el| {
                        el.cursor_pointer().on_mouse_down(MouseButton::Left, {
                            let view = cx.entity();
                            move |_, window, cx| {
                                cx.stop_propagation();
                                view.update(cx, |this, cx| {
                                    this.toggle_task(block_ix, item_ix, window, cx);
                                });
                            }
                        })
                    })
                    .child(match checked {
                        Some(true) => {
                            icon_el("square-check", pal.markdown_list_item).into_any_element()
                        }
                        Some(false) => icon_el("square", pal.markdown_list_item).into_any_element(),
                        None => {
                            if let Some(m) = marker {
                                div().child(m).into_any_element()
                            } else {
                                div().child("•").into_any_element()
                            }
                        }
                    }),
            )
            .child(div().flex_1().min_w_0().child(edit))
            .into_any_element()
    }

    fn selection_bubble_for_unit(
        &self,
        block_ix: usize,
        item: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if item.is_some() {
            let p = self.proj();
            let block = p.blocks.get(block_ix)?;
            let BlockExtra::List { items, .. } = &block.extra else {
                return None;
            };
            let it = items.get(item?)?;
            let d = self
                .sel
                .as_ref()
                .filter(|s| s.start != s.end)
                .map(|s| p.to_display(s.start.min(s.end)))
                .unwrap_or(self.caret);
            if d < it.display.start || d > it.display.end {
                return None;
            }
        }
        self.selection_bubble_for_block(block_ix, cx)
    }

    fn drop_gap_is_live(from: usize, gap: usize, n: usize) -> bool {
        gap <= n && from < n && gap != from && gap != from + 1
    }

    fn render_drop_edge(&self, ix: usize, n: usize, cx: &mut Context<Self>) -> AnyElement {
        if !cx.has_active_drag() {
            return div().into_any_element();
        }
        let Some(from) = self.block_dragging else {
            return div().into_any_element();
        };
        let Some(gap) = self.block_drag_gap else {
            return div().into_any_element();
        };
        // Only paint gaps that move_unit / drop_block_at would accept.
        if !Self::drop_gap_is_live(from, gap, n) {
            return div().into_any_element();
        }
        let at_top = gap == ix;
        let at_bottom = gap == n && ix + 1 == n;
        if !at_top && !at_bottom {
            return div().into_any_element();
        }
        let pal = &self.palette;
        h_flex()
            .absolute()
            .left_0()
            .right_0()
            .when(at_top, |el| el.top(px(-5.)))
            .when(at_bottom, |el| el.bottom(px(-5.)))
            .items_center()
            .h(px(10.))
            .occlude()
            .child(
                div()
                    .size(px(8.))
                    .rounded_full()
                    .border_2()
                    .border_color(pal.primary)
                    .bg(pal.background),
            )
            .child(div().flex_1().h(px(2.)).rounded(px(1.)).bg(pal.primary))
            .into_any_element()
    }

    fn delete_block_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.block_menu = None;
        self.push_doc_undo();
        let caret = self.doc.delete_unit(ix);
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    fn duplicate_block_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.block_menu = None;
        self.push_doc_undo();
        let caret = self.doc.duplicate_unit(ix);
        self.sync_gfm();
        self.commit_caret(caret, window, cx);
    }

    fn drop_block_at(&mut self, from: usize, window: &mut Window, cx: &mut Context<Self>) {
        let gap = self.block_drag_gap;
        self.block_dragging = None;
        self.block_drag_gap = None;
        self.block_menu = None;
        let Some(gap) = gap else {
            cx.notify();
            return;
        };
        let n = wysiwyg::units(&self.proj()).len();
        if !Self::drop_gap_is_live(from, gap, n) {
            cx.notify();
            return;
        }
        self.push_doc_undo();
        // Tree-native move preserves empty paragraph slots; GFM round-trip
        // via move_block/commit_edit used to collapse them.
        if let Some(caret) = self.doc.move_unit(from, gap) {
            self.sync_gfm();
            self.commit_caret(caret, window, cx);
        } else {
            cx.notify();
        }
    }

    fn update_block_drag_gap(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let n = wysiwyg::units(&self.proj()).len();
        let from = self.block_dragging;
        let offset = self.scroll_handle.offset();

        // Prefer the nearest row boundary within a snap window so the painted
        // edge (drawn on the boundary) matches the gap we commit on drop.
        // Fall back to midpoint targeting when far from every boundary.
        let snap = px(14.);
        let mut best_boundary: Option<(Pixels, usize)> = None;
        let mut mid_gap = n;
        let mut mid_resolved = false;
        for i in 0..n {
            let Some(bounds) = self.scroll_handle.bounds_for_item(i) else {
                continue;
            };
            let top = bounds.origin.y + offset.y;
            let bottom = top + bounds.size.height;
            let mid = top + bounds.size.height / 2.;

            let d_top = (y - top).abs();
            if d_top <= snap {
                best_boundary = match best_boundary {
                    Some((d, _)) if d <= d_top => best_boundary,
                    _ => Some((d_top, i)),
                };
            }
            if i + 1 == n {
                let d_bot = (y - bottom).abs();
                if d_bot <= snap {
                    best_boundary = match best_boundary {
                        Some((d, _)) if d <= d_bot => best_boundary,
                        _ => Some((d_bot, n)),
                    };
                }
            }

            if !mid_resolved {
                if y < mid {
                    mid_gap = i;
                    mid_resolved = true;
                } else {
                    mid_gap = i + 1;
                }
            }
        }

        let raw = best_boundary.map(|(_, g)| g).unwrap_or(mid_gap);
        // Honest: never hold a no-op gap. Indicator + drop both key off this.
        let gap = match from {
            Some(from) if Self::drop_gap_is_live(from, raw, n) => Some(raw),
            Some(_) => None,
            None => Some(raw),
        };
        if self.block_drag_gap != gap {
            self.block_drag_gap = gap;
            cx.notify();
        }
    }

    fn overscroll_px(&self) -> gpui::Pixels {
        let viewport = self.scroll_handle.bounds().size.height;
        let viewport = if viewport > px(0.) {
            viewport
        } else {
            self.surface_h
        };
        if viewport <= px(0.) {
            return px(0.);
        }
        let last = wysiwyg::units(&self.proj()).len().saturating_sub(1);
        let last_h = self
            .scroll_handle
            .bounds_for_item(last)
            .map(|b| b.size.height)
            .unwrap_or(px(0.));
        (viewport - last_h).max(px(0.))
    }

    fn toggle_mark_action(&mut self, mark: Mark, window: &mut Window, cx: &mut Context<Self>) {
        if self.view_source || self.cmd_palette.is_some() {
            return;
        }
        // Multi-cell table selection: the linear `sel` spans in-between cells
        // in reading order, so apply to exactly the rectangle instead.
        if let Some(rect) = self.table_sel.map(|t| t.normalize()).filter(|t| t.is_multi()) {
            self.push_doc_undo();
            if self
                .doc
                .toggle_mark_table(rect.block_ix, rect.r0, rect.r1, rect.c0, rect.c1, mark)
            {
                self.sync_gfm();
                self.mark_dirty();
                self.refresh(window, cx);
                self.sync_title(window);
            }
            return;
        }
        if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
            self.push_doc_undo();
            if let Some(range) = self.doc.toggle_mark(sel, mark) {
                self.sync_gfm();
                self.caret = range.end.min(self.proj().display.len());
                self.sel = Some(range);
                self.mark_dirty();
                self.refresh(window, cx);
                self.sync_title(window);
            }
        } else {
            match mark {
                Mark::Bold => self.sticky.bold = !self.sticky.bold,
                Mark::Italic => self.sticky.italic = !self.sticky.italic,
                Mark::Strike => self.sticky.strike = !self.sticky.strike,
                Mark::Code => self.sticky.code = !self.sticky.code,
                Mark::Underline => self.sticky.underline = !self.sticky.underline,
            }
            cx.notify();
        }
    }

    fn commit_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.link_draft.as_str().to_string();
        self.link_open = false;
        self.link_draft.clear();
        let sel = self.sel.clone().unwrap_or(self.caret..self.caret);
        self.push_doc_undo();
        self.caret = self.doc.apply_link(sel, &url);
        self.sync_gfm();
        self.caret = self.caret.min(self.proj().display.len());
        self.sel = None;
        self.mark_dirty();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn open_link_url(&mut self, raw: &str, window: &mut Window, cx: &mut Context<Self>) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            eprintln!("crabmd: open_link_url: empty url");
            return;
        }
        // Crosslinks: `other.md`, `./other.md`, `sub/dir.md#anchor` open
        // in-place (same window). Anchors jump to the first matching heading.
        if let Some((target, anchor)) = Self::split_local_md(trimmed) {
            if target.is_empty() {
                // `#anchor` in the same file: jump to the matching heading.
                if let Some(a) = anchor.as_deref().filter(|a| !a.is_empty()) {
                    let slug = a.to_ascii_lowercase().replace(['-', '_'], " ");
                    let p = self.proj();
                    for b in &p.blocks {
                        if matches!(b.kind, BlockKind::Heading(_)) {
                            if let Some(text) = p.display.get(b.display.clone()) {
                                if text.to_ascii_lowercase().contains(&slug) {
                                    self.caret = b.display.start;
                                    self.sel = None;
                                    self.refresh(window, cx);
                                    return;
                                }
                            }
                        }
                    }
                }
                return;
            }
            let base = self
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let candidate = if std::path::Path::new(&target).is_absolute() {
                std::path::PathBuf::from(&target)
            } else {
                base.join(&target)
            };
            if self.open_local_file(&candidate, anchor.as_deref()) {
                self.refresh(window, cx);
                self.sync_title(window);
                return;
            }
        }
        let url = if trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
            || trimmed.starts_with("mailto:")
        {
            trimmed.to_string()
        } else {
            format!("https://{trimmed}")
        };
        open_in_browser(&url);
    }

    /// Split a link target into `(file, anchor)` when it points at a local
    /// markdown file. Returns None for URLs / mailto / bare domains.
    fn split_local_md(raw: &str) -> Option<(String, Option<String>)> {
        let t = raw.trim();
        if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("mailto:") {
            return None;
        }
        // Strip `<...>` autolink brackets pulldown sometimes keeps.
        let t = t.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(t);
        if t.is_empty() || t.contains(' ') || t.contains('\n') {
            return None;
        }
        let (file, anchor) = match t.split_once('#') {
            Some((f, a)) => (f, Some(a.to_string())),
            None => (t, None),
        };
        if file.is_empty() {
            // `#anchor` within the same file — handled as a heading jump by
            // the caller; not a crosslink open. Report as local with empty
            // file so callers can distinguish.
            return Some((String::new(), anchor));
        }
        let lower = file.to_ascii_lowercase();
        if !(lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdown")) {
            return None;
        }
        Some((file.to_string(), anchor))
    }

    /// Swap the buffer for a sibling markdown file (crosslink navigation).
    /// Returns false when the file can't be read (caller falls back to a
    /// browser open). Refuses when dirty to avoid losing edits.
    fn open_local_file(&mut self, candidate: &std::path::Path, anchor: Option<&str>) -> bool {
        if self.dirty {
            self.status = "unsaved changes — save first (cmd-s)".into();
            return true;
        }
        let resolved = std::fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());
        let Ok(raw) = std::fs::read_to_string(&resolved) else {
            self.status = format!("missing file: {}", candidate.display()).into();
            return true;
        };
        let doc = crate::tree::Doc::from_gfm(&raw);
        let normalized = doc.to_gfm();
        self.path = resolved;
        self.doc = doc;
        self.source = normalized.clone();
        self.clean_source = normalized;
        self.dirty = false;
        self.caret = 0;
        self.sel = None;
        self.mode = Mode::Normal;
        self.clear_pending();
        self.command = None;
        self.search = None;
        self.media_sel = None;
        self.link_open = false;
        self.undo = UndoStack::default();
        self.insert_origin = None;
        if let Some(a) = anchor.filter(|a| !a.is_empty()) {
            // Jump to the first heading containing the anchor slug.
            let slug = a.to_ascii_lowercase().replace(['-', '_'], " ");
            let p = self.proj();
            for b in &p.blocks {
                if matches!(b.kind, BlockKind::Heading(_)) {
                    if let Some(text) = p.display.get(b.display.clone()) {
                        if text.to_ascii_lowercase().contains(&slug) {
                            self.caret = b.display.start;
                            break;
                        }
                    }
                }
            }
        }
        self.status = "ready".into();
        true
    }

    /// Check if the word directly before `end_display` is a bare URL (http://, https://, or www.)
    /// that is not already marked as a link, and auto-link it with a dedicated undo step.
    ///
    /// When auto-linking triggers, a snapshot of the current state (prior to linkifying)
    /// is pushed to `undo`, so Cmd-Z first reverts the link transformation to plain text,
    /// and a second Cmd-Z reverts the preceding action (space, enter, or paste).
    fn try_auto_link_preceding_url(&mut self, end_display: usize) -> bool {
        let p = self.proj();
        let end = end_display.min(p.display.len());
        let before = &p.display[..end];

        // Find the start of the token right before `end` (skip trailing whitespace if any)
        let token_end = before
            .char_indices()
            .rev()
            .find(|(_, c)| !c.is_whitespace())
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        if token_end == 0 {
            return false;
        }

        // Find start of this token (bounded by whitespace or start of string)
        let token_start = before[..token_end]
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);

        let candidate = &before[token_start..token_end];
        let url_candidate = candidate.trim_start_matches(['(', '[', '<', '"', '\'']);
        let trim_lead_len = candidate.len() - url_candidate.len();
        let url_start = token_start + trim_lead_len;

        if !url_candidate.starts_with("http://")
            && !url_candidate.starts_with("https://")
            && !url_candidate.starts_with("www.")
        {
            return false;
        }

        let bare = take_bare_url(url_candidate);
        if bare.is_empty() || bare.len() < 4 {
            return false;
        }
        let url_end = url_start + bare.len();

        // Make sure it's not already linked
        if (url_start..url_end).any(|i| p.link_at(i).is_some()) {
            return false;
        }

        // Make sure it's not inside a code block
        if let Some(b) = p.block_at_display(url_start) {
            if matches!(b.kind, BlockKind::Code) {
                return false;
            }
        }

        let full_url = if bare.starts_with("www.") {
            format!("https://{bare}")
        } else {
            bare.clone()
        };

        // Snapshot current state (text typed/pasted) as an undo point, so first undo
        // reverts the link transformation back to plain text.
        let before_link = self.snapshot();
        self.doc.apply_link(url_start..url_end, &full_url);
        self.sync_gfm();
        self.undo.push(before_link);
        self.mark_dirty();
        self.status = "unsaved".into();
        true
    }

    fn remove_link_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.link_draft.clear();
        let p = self.proj();
        let sel = if let Some(s) = self.sel.clone().filter(|s| s.start != s.end) {
            s
        } else if let Some((range, _)) = p.link_at(self.caret) {
            range
        } else {
            return;
        };
        self.push_doc_undo();
        self.caret = self.doc.apply_link(sel, "");
        self.sync_gfm();
        self.caret = self.caret.min(self.proj().display.len());
        self.sel = None;
        self.mark_dirty();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn offset_from_utf16(text: &str, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in text.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(text: &str, offset: usize) -> usize {
        let mut utf16 = 0;
        let mut utf8 = 0;
        for ch in text.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }
}

fn extend_select_unit(
    display: &str,
    unit: Range<usize>,
    d: usize,
    gran: SelectGranularity,
) -> Range<usize> {
    let d = d.min(display.len());
    if d < unit.start {
        let other = match gran {
            SelectGranularity::Word => {
                let w = word_range_at(display, d);
                if w.start < w.end {
                    w
                } else {
                    d..d
                }
            }
            SelectGranularity::Line => crate::motion::logical_line_range(display, d),
            SelectGranularity::Char => d..d,
        };
        other.start..unit.end
    } else if d > unit.end {
        let other = match gran {
            SelectGranularity::Word => {
                let w = word_range_at(display, d);
                if w.start < w.end {
                    w
                } else {
                    d..d
                }
            }
            SelectGranularity::Line => crate::motion::logical_line_range(display, d),
            SelectGranularity::Char => d..d,
        };
        unit.start..other.end
    } else {
        unit
    }
}

#[derive(Clone)]
struct DragBlock {
    ix: usize,
    preview: String,
    bg: gpui::Hsla,
    fg: gpui::Hsla,
    border: gpui::Hsla,
}

/// Invisible drag payload for the video scrub bar. Starting a GPUI drag
/// (`on_drag`) routes all subsequent `on_drag_move` events to the scrub
/// bar — even when the pointer races off the bar, out of the window, or
/// over another app mid-gesture — for as long as the button stays down.
/// Same pattern as `gpui-component`'s `Slider` (`DragSlider`).
#[derive(Clone)]
struct DragVideoScrub {
    key: String,
    total: usize,
}

impl Render for DragVideoScrub {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

impl Render for DragBlock {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .border_1()
            .border_color(self.border)
            .bg(self.bg)
            .shadow_lg()
            .max_w(px(240.))
            .overflow_hidden()
            .child(icon_el("grip-vertical", self.fg).flex_shrink_0())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(self.fg)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(self.preview.clone()),
            )
    }
}

fn icon_el(name: &str, color: gpui::Hsla) -> Icon {
    icon_el_px(name, color, px(16.))
}

fn icon_el_px(name: &str, color: gpui::Hsla, size: gpui::Pixels) -> Icon {
    Icon::default()
        .path(crate::assets::path(name))
        .text_color(color)
        .w(size)
        .h(size)
}

/// Tiny per-field reset button overlaying an input's right corner (fonts).
/// `id` must be unique per input; clicking resets just that field.
fn reset_btn(
    id: (&'static str, usize),
    slot: FontSlot,
    field: FontField,
    p: &crate::theme::Palette,
    cx: &mut gpui::Context<Workspace>,
) -> gpui::AnyElement {
    div()
        .id(id)
        .absolute()
        .right(px(6.))
        .top(px(0.))
        .bottom(px(0.))
        .flex()
        .items_center()
        .cursor_pointer()
        .child(icon_el_px("rotate-ccw", p.text_muted, px(12.)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.reset_font_field(slot, field, window, cx);
            }),
        )
        .into_any_element()
}

pub fn window_title(path: &Path, dirty: bool) -> String {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled");
    if dirty {
        format!("• {name} — crabmd")
    } else {
        format!("{name} — crabmd")
    }
}

/// True when `raw` is a bare `<video …>` line (any attributes): pulldown
/// often files these as Paragraph inline-HTML rather than an HtmlBlock.
fn is_bare_video(raw: &str) -> bool {
    raw.trim_start().to_ascii_lowercase().starts_with("<video")
}

/// Open a video `src` in the system player: remote URLs via the OS opener,
/// local files via `open`/`xdg-open`/explorer so no GStreamer bundle is
/// needed. Failures land in `status` instead of silently doing nothing.
fn open_video_src(src: &str, path: &std::path::Path, cx: &mut App) {
    if crate::images::is_remote_src(src) {
        cx.open_url(src);
        return;
    }
    if !path.exists() {
        return;
    }
    let s = path.to_string_lossy().to_string();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&s).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", &s])
        .spawn();
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(&s).spawn();
}

pub fn apply_palette(palette: &Palette, cx: &mut App) {
    let mode = match palette.appearance {
        Appearance::Dark => ThemeMode::Dark,
        Appearance::Light => ThemeMode::Light,
    };
    Theme::change(mode, None, cx);
    {
        let theme = Theme::global_mut(cx);
        theme.background = palette.background;
        theme.foreground = palette.markdown_text;
        theme.primary = palette.primary;
        theme.primary_foreground = palette.background;
        // Softened menu/hover tint: full-strength `accent` (often a
        // loud pink) overwhelms popover rows, so wash it down.
        // Foreground stays body text for readability.
        theme.accent = palette.accent.opacity(0.28);
        theme.accent_foreground = palette.markdown_text;
        theme.secondary = palette.background_element;
        theme.secondary_foreground = palette.markdown_text;
        theme.muted = palette.background_element;
        theme.muted_foreground = palette.text_muted;
        theme.border = palette.border;
        theme.input = palette.border;
        theme.ring = palette.primary;
        theme.link = palette.markdown_link;
        theme.link_hover = palette.primary;
        theme.popover = palette.background_panel;
        theme.popover_foreground = palette.markdown_text;
        theme.danger = palette.error;
        theme.warning = palette.warning;
        theme.success = palette.success;
        theme.info = palette.info;
        theme.title_bar = palette.background_panel;
        theme.status_bar = palette.background_panel;
        theme.caret = palette.markdown_text;
        theme.selection = palette.primary;
        theme.button = palette.background_element;
        theme.button_foreground = palette.markdown_text;
        theme.button_hover = palette.border;
        theme.red = palette.error;
        theme.green = palette.success;
        theme.yellow = palette.warning;
        theme.cyan = palette.info;
        theme.blue = palette.secondary;
        theme.magenta = palette.accent;
        // Thumb-only overlay scrollbars (mac-like): transparent track so no
        // gutter covers content, thumb in border color.
        theme.scrollbar = palette.background.opacity(0.0);
        theme.scrollbar_thumb = palette.border;
        theme.focus_ring = false;
        theme.tokens = ThemeTokens::from(theme.colors);
    }
    Theme::sync_base(cx);
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let p = self.proj();
        let start = Self::offset_from_utf16(&p.display, range_utf16.start);
        let end = Self::offset_from_utf16(&p.display, range_utf16.end);
        *adjusted =
            Some(Self::offset_to_utf16(&p.display, start)..Self::offset_to_utf16(&p.display, end));
        Some(p.display.get(start..end).unwrap_or("").to_string())
    }

    fn selected_text_range(
        &mut self,
        ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if !ignore_disabled && self.is_modal_nav() {
            return None;
        }
        let p = self.proj();
        let d = self.caret;
        let range = if let Some(sel) = &self.sel {
            let a = sel.start.min(sel.end);
            let b = sel.end.max(sel.start);
            Self::offset_to_utf16(&p.display, a)..Self::offset_to_utf16(&p.display, b)
        } else {
            let u = Self::offset_to_utf16(&p.display, d);
            u..u
        };
        Some(UTF16Selection {
            range,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let marked = self.marked.as_ref()?;
        let p = self.proj();
        Some(
            Self::offset_to_utf16(&p.display, marked.start)
                ..Self::offset_to_utf16(&p.display, marked.end),
        )
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_modal_nav()
            || self.search.is_some()
            || self.command.is_some()
            || self.settings_open
            || self.cmd_palette.is_some()
            || self.view_source
        {
            return;
        }
        if text == "\n" || text == "\r\n" {
            self.insert_newline(false, window, cx);
            return;
        }
        let p = self.proj();
        let display_range = if let Some(r) = range_utf16 {
            Self::offset_from_utf16(&p.display, r.start)..Self::offset_from_utf16(&p.display, r.end)
        } else if let Some(sel) = &self.sel {
            sel.start.min(sel.end)..sel.end.max(sel.start)
        } else if let Some(m) = &self.marked {
            m.clone()
        } else {
            let d = self.caret;
            d..d
        };
        if text.is_empty() {
            if display_range.start == display_range.end {
                return;
            }
            if self.insert_origin.is_none() {
                self.insert_origin = Some(self.snapshot());
            }
            self.caret = if display_range.end == self.caret
                && display_range.end - display_range.start
                    == p.display[..display_range.end]
                        .chars()
                        .next_back()
                        .map(|c| c.len_utf8())
                        .unwrap_or(1)
            {
                if let Some(c) = self.doc.backspace(self.caret) {
                    c
                } else {
                    self.doc.delete_char(self.caret)
                }
            } else {
                self.doc.delete_display(display_range)
            };
            self.sync_gfm();
            self.sel = None;
            self.marked = None;
            self.mark_dirty();
            self.refresh(window, cx);
            self.sync_title(window);
            return;
        }
        if self.insert_origin.is_none() {
            self.insert_origin = Some(self.snapshot());
        }
        self.caret = if display_range.start == display_range.end {
            self.doc.insert_text(self.caret, None, text, self.sticky)
        } else {
            self.doc.delete_display(display_range.clone());
            self.doc
                .insert_text(display_range.start, None, text, self.sticky)
        };
        self.sync_gfm();
        self.sel = None;
        self.marked = None;
        self.mark_dirty();
        let q = self.slash_query().unwrap_or_default();
        if q != self.last_slash_query {
            self.last_slash_query = q;
            self.slash_index = 0;
            self.slash_scroll.scroll_to_item(0);
        }

        // If user typed a space (or ends with whitespace), check if preceding token is a bare URL.
        // Flush any active insert batch first so the typed space is its own undo step,
        // then auto-linking pushes another undo step so Cmd-Z first reverts the link.
        if text.chars().any(char::is_whitespace) {
            self.finish_insert_undo();
            self.try_auto_link_preceding_url(self.caret);
        }

        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range_utf16, new_text, window, cx);
        let p = self.proj();
        let d = self.caret;
        let start = d.saturating_sub(new_text.len());
        self.marked = if new_text.is_empty() {
            None
        } else {
            Some(start..d)
        };
        if let Some(sel) = new_selected_range {
            let a = start + Self::offset_from_utf16(new_text, sel.start);
            let b = start + Self::offset_from_utf16(new_text, sel.end);
            self.caret = b.min(p.display.len());
            let _ = a;
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: gpui::Bounds<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        let p = self.proj();
        let start = Self::offset_from_utf16(&p.display, range_utf16.start);
        let hit = surface::piece_for_offset(&self.hits, start)?;
        let len =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hit.layout.len())).ok()?;
        if start >= hit.display_start && start <= hit.display_start + len {
            let local = start.saturating_sub(hit.display_start);
            let pos = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                hit.layout.position_for_index(local)
            }))
            .ok()
            .flatten()?;
            let h = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                hit.layout.line_height()
            }))
            .unwrap_or(px(16.));
            return Some(gpui::Bounds::new(pos, gpui::size(px(2.), h)));
        }
        None
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let p = self.proj();
        let d = surface::index_for_point(&self.hits, point)?;
        Some(Self::offset_to_utf16(&p.display, d))
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        !self.is_modal_nav()
            && self.command.is_none()
            && self.search.is_none()
            && self.cmd_palette.is_none()
            && !self.link_open
            && !self.view_source
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let file_name = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string();
        let dirty = self.dirty;
        let theme_name = p.name.clone();
        let mode_label = self.mode.status(self.config.editor);
        let insert = self.mode.is_insert() || self.is_notion();
        let settings_open = self.settings_open;
        let palette_open = self.cmd_palette.is_some();
        let command = self.command.clone();
        let search = self.search.clone();
        let last_search = self.last_search.clone();
        let search_pos = self.search_position();
        let search_view = cx.entity();
        let status = if self.view_source {
            SharedString::from("source view — esc/cmd-shift-v to exit")
        } else {
            self.status.clone()
        };

        v_flex()
            .id("workspace")
            .relative()
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_leave_insert))
            .on_action(cx.listener(Self::on_insert_caret))
            .on_action(cx.listener(Self::on_insert_after))
            .on_action(cx.listener(Self::on_insert_line_start))
            .on_action(cx.listener(Self::on_insert_line_end))
            .on_action(cx.listener(Self::on_open_below))
            .on_action(cx.listener(Self::on_open_above))
            .on_action(cx.listener(Self::on_char_left))
            .on_action(cx.listener(Self::on_char_right))
            .on_action(cx.listener(Self::on_line_down))
            .on_action(cx.listener(Self::on_line_up))
            .on_action(cx.listener(Self::on_word_forward))
            .on_action(cx.listener(Self::on_word_back))
            .on_action(cx.listener(Self::on_word_end))
            .on_action(cx.listener(Self::on_word_forward_ws))
            .on_action(cx.listener(Self::on_word_back_ws))
            .on_action(cx.listener(Self::on_word_end_ws))
            .on_action(cx.listener(Self::on_line_start))
            .on_action(cx.listener(Self::on_line_first_non_blank))
            .on_action(cx.listener(Self::on_line_end))
            .on_action(cx.listener(Self::on_first_doc))
            .on_action(cx.listener(Self::on_last_doc))
            .on_action(cx.listener(Self::on_pending_g))
            .on_action(cx.listener(Self::on_digit0))
            .on_action(cx.listener(Self::on_digit1))
            .on_action(cx.listener(Self::on_digit2))
            .on_action(cx.listener(Self::on_digit3))
            .on_action(cx.listener(Self::on_digit4))
            .on_action(cx.listener(Self::on_digit5))
            .on_action(cx.listener(Self::on_digit6))
            .on_action(cx.listener(Self::on_digit7))
            .on_action(cx.listener(Self::on_digit8))
            .on_action(cx.listener(Self::on_digit9))
            .on_action(cx.listener(Self::on_select_line))
            .on_action(cx.listener(Self::on_delete_op))
            .on_action(cx.listener(Self::on_delete_char))
            .on_action(cx.listener(Self::on_delete_to_end))
            .on_action(cx.listener(Self::on_visual_char))
            .on_action(cx.listener(Self::on_visual_line))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            .on_action(cx.listener(Self::on_slash_prev))
            .on_action(cx.listener(Self::on_slash_next))
            .on_action(cx.listener(Self::on_slash_apply))
            .on_action(cx.listener(Self::on_insert_slash))
            .on_action(cx.listener(Self::on_delete_word_back))
            .on_action(cx.listener(Self::on_delete_line_back))
            .on_action(cx.listener(Self::on_block_backspace))
            .on_action(cx.listener(Self::on_toggle_settings))
            .on_action(cx.listener(Self::on_open_palette))
            .on_action(cx.listener(Self::on_open_command))
            .on_action(cx.listener(Self::on_open_search))
            .on_action(cx.listener(Self::on_search_back))
            .on_action(cx.listener(Self::on_search_next))
            .on_action(cx.listener(Self::on_search_prev))
            .on_action(cx.listener(Self::on_join_lines))
            .on_action(cx.listener(Self::on_replace_char))
            .on_action(cx.listener(Self::on_find_forward))
            .on_action(cx.listener(Self::on_find_backward))
            .on_action(cx.listener(Self::on_find_till))
            .on_action(cx.listener(Self::on_find_till_back))
            .on_action(cx.listener(Self::on_repeat_find))
            .on_action(cx.listener(Self::on_reverse_find))
            .on_action(cx.listener(Self::on_bracket_open))
            .on_action(cx.listener(Self::on_bracket_close))
            .on_action(cx.listener(Self::on_insert_newline))
            .on_action(cx.listener(Self::on_insert_hard_break))
            .on_action(cx.listener(Self::on_indent_tab))
            .on_action(cx.listener(Self::on_outdent_tab))
            .on_action(cx.listener(Self::on_toggle_bold))
            .on_action(cx.listener(Self::on_toggle_italic))
            .on_action(cx.listener(Self::on_toggle_strike))
            .on_action(cx.listener(Self::on_toggle_code))
            .on_action(cx.listener(Self::on_toggle_underline))
            .on_action(cx.listener(Self::on_toggle_link))
            .on_action(cx.listener(Self::on_cut_selection))
            .on_action(cx.listener(Self::on_copy_selection))
            .on_action(cx.listener(Self::on_paste_clipboard))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_toggle_source))
            .on_action(cx.listener(Self::on_indent_shift))
            .on_action(cx.listener(Self::on_dedent_shift))
            .on_action(cx.listener(Self::on_auto_indent))
            .on_action(cx.listener(Self::on_change_op))
            .on_action(cx.listener(Self::on_match_object))
            .on_action(cx.listener(|_, _: &QuitApp, _, cx| {
                // Quit the whole app (all windows). Each window's shell
                // close-guard prompts first when tabs are dirty.
                cx.quit();
            }))
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if this.handle_capture_key(ev, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if this.mouse_dragging {
                        this.mouse_dragging = false;
                        this.mouse_unit = None;
                        this.mouse_granularity = SelectGranularity::Char;
                        if this.sel.as_ref().is_some_and(|s| s.start != s.end)
                            && this.config.editor.is_modal()
                            && !this.mode.is_insert()
                            && !this.mode.is_visual()
                        {
                            this.mode = if this.config.editor == EditorKind::Helix {
                                Mode::Select
                            } else {
                                Mode::Visual
                            };
                        }
                        cx.notify();
                    }
                }),
            )
            .can_drop(|v, _, _| {
                v.downcast_ref::<ExternalPaths>().is_some()
                    || v.downcast_ref::<DragBlock>().is_some()
            })
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.import_paths(paths.paths(), window, cx);
            }))
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<DragBlock>, _, cx| {
                this.update_block_drag_gap(e.event.position.y, cx);
            }))
            .on_drop(cx.listener(|this, drag: &DragBlock, window, cx| {
                this.drop_block_at(drag.ix, window, cx);
            }))
            .size_full()
            .bg(p.background)
            .font_family(self.config.markdown_font.family.clone())
            .text_size(self.markdown_font_px())
            .text_color(p.markdown_text)
            .child(
                // Positioned wrapper owns the layout slot; the scrollbar
                // overlay is a sibling of the tracked scroll div (same as
                // gpui-component's `Scrollable`), so its layout bounds equal
                // the viewport. As a child of the scroll div it would scroll
                // with the content: thumb pinned at top, drag not following.
                // Column flex + flex_1 child (not percentage heights) so the
                // scroll div gets a definite viewport height and overflows.
                v_flex()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .min_w_0()
                    .relative()
                    .child(
                v_flex()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .min_w_0()
                    .on_children_prepainted({
                        let view = cx.entity();
                        move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                let h = this.scroll_handle.bounds().size.height;
                                if h > px(0.) && this.surface_h != h {
                                    this.surface_h = h;
                                    cx.notify();
                                }
                                if this.follow_caret {
                                    this.ensure_caret_visible(cx);
                                }
                            });
                        }
                    })
                    .id("surface")
                    .overflow_y_scroll()
                    .overflow_x_hidden()
                    .track_scroll(&self.scroll_handle)
                    .pt_10()
                    // No `gap_1` here: inter-row spacing is per-row `mb_1`
                    // (skipped on hidden rows) so collapsed `<details>` bodies
                    // contribute zero height instead of one gap per hidden row.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                            if let Some(d) = surface::index_for_point(&this.hits, ev.position) {
                                this.click_display(
                                    d,
                                    ev.modifiers.shift,
                                    ev.modifiers.platform || ev.modifiers.control,
                                    ev.click_count,
                                    window,
                                    cx,
                                );
                            }
                        }),
                    )
                    .children(self.render_surface(cx)),
                    )
                    // Sibling overlay: never a tracked child, so block
                    // indices for scroll_to_item / bounds_for_item stay clean.
                    .vertical_scrollbar(&self.scroll_handle),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .h(px(30.))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .gap_3()
                    .items_center()
                    .border_t_1()
                    .border_color(p.border)
                    .bg(p.background_panel)
                    .font_family(self.config.ui_font.family.clone())
                    .text_size(self.ui_font_px())
                    .when(command.is_none() && search.is_none(), |el| {
                        el.child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if insert { p.primary } else { p.text_muted })
                                .child(mode_label),
                        )
                        .child(
                            div().text_xs().text_color(p.text_muted).child(format!(
                                "{}{}",
                                file_name,
                                if dirty { " · unsaved" } else { "" }
                            )),
                        )
                    })
                    .when_some(command, |el, cmd: LineField| {
                        el.child(
                            h_flex()
                                .flex_1()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.primary)
                                        .child(":"),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(p.markdown_text)
                                        .child(cmd.render()),
                                ),
                        )
                    })
                    .when_some(search.clone(), |el, (q, back): (LineField, bool)| {
                        let prefix = if back { "?" } else { "/" };
                        let count = match search_pos {
                            Some((cur, total)) => format!("{cur}/{total}"),
                            None => {
                                let typed = q.as_str();
                                if typed.is_empty() {
                                    String::new()
                                } else {
                                    "0/0".to_string()
                                }
                            }
                        };
                        let prev_view = search_view.clone();
                        let next_view = search_view.clone();
                        el.child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.primary)
                                        .child(prefix),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(p.markdown_text)
                                        .child(q.render()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.text_muted)
                                        .child(count),
                                )
                                .child(
                                    div()
                                        .id("search-prev")
                                        .px_1()
                                        .rounded(px(4.))
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(p.text_muted)
                                        .hover(|el| el.text_color(p.primary))
                                        .child("‹")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            move |_, window, cx| {
                                                prev_view.update(cx, |this, cx| {
                                                    this.cycle_search(false, window, cx);
                                                });
                                            },
                                        ),
                                )
                                .child(
                                    div()
                                        .id("search-next")
                                        .px_1()
                                        .rounded(px(4.))
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(p.text_muted)
                                        .hover(|el| el.text_color(p.primary))
                                        .child("›")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            move |_, window, cx| {
                                                next_view.update(cx, |this, cx| {
                                                    this.cycle_search(true, window, cx);
                                                });
                                            },
                                        ),
                                ),
                        )
                    })
                    .when(search.is_none() && last_search.is_some(), |el| {
                        let (q, back) = last_search.clone().unwrap_or_default();
                        let prefix = if back { "?" } else { "/" };
                        let short = if q.chars().count() > 24 {
                            format!("{}…", q.chars().take(24).collect::<String>())
                        } else {
                            q.clone()
                        };
                        let count = match search_pos {
                            Some((cur, total)) => format!("{cur}/{total}"),
                            None => "0/0".to_string(),
                        };
                        let prev_view = search_view.clone();
                        let next_view = search_view.clone();
                        el.child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .flex_shrink_0()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.primary)
                                        .child(prefix),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.text_muted)
                                        .child(short),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.text_muted)
                                        .child(count),
                                )
                                .child(
                                    div()
                                        .id("search-prev-idle")
                                        .px_1()
                                        .rounded(px(4.))
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(p.text_muted)
                                        .hover(|el| el.text_color(p.primary))
                                        .child("‹")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            move |_, window, cx| {
                                                prev_view.update(cx, |this, cx| {
                                                    this.jump_search(false, true, window, cx);
                                                });
                                            },
                                        ),
                                )
                                .child(
                                    div()
                                        .id("search-next-idle")
                                        .px_1()
                                        .rounded(px(4.))
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(p.text_muted)
                                        .hover(|el| el.text_color(p.primary))
                                        .child("›")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            move |_, window, cx| {
                                                next_view.update(cx, |this, cx| {
                                                    this.jump_search(true, true, window, cx);
                                                });
                                            },
                                        ),
                                ),
                        )
                    })
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(p.text_muted).child(status))
                    .child(
                        Button::new("theme")
                            .ghost()
                            .xsmall()
                            .label(theme_name)
                            .dropdown_menu({
                                let entity = cx.entity();
                                move |menu, _, _| {
                                    let mut menu = menu;
                                    for name in theme::list_theme_names() {
                                        let entity = entity.clone();
                                        let label = name.to_string();
                                        menu =
                                            menu.item(PopupMenuItem::new(label.clone()).on_click(
                                                move |_, window, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.set_theme(&label, window, cx);
                                                    });
                                                },
                                            ));
                                    }
                                    menu
                                }
                            }),
                    )
                    .child(Button::new("save").ghost().xsmall().label("Save").on_click(
                        cx.listener(|this, _, window, cx| {
                            if this.write_to_disk(cx) {
                                this.status = "saved".into();
                            }
                            this.sync_title(window);
                        }),
                    ))
                    .child(
                        Button::new("settings")
                            .ghost()
                            .xsmall()
                            .icon(icon_el("settings", p.text_muted))
                            .tooltip("Settings")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_settings(window, cx);
                            })),
                    ),
            )
            // Modal overlays must paint above in-document `deferred` popovers
            // (selection bubble priority 2, slash menu priority 1): a plain
            // absolute child loses to any deferred element, so defer the
            // modals with a higher priority.
            .when(settings_open, |el| {
                el.child(deferred(self.render_settings(cx)).with_priority(100))
            })
            .when(palette_open, |el| {
                el.child(deferred(self.render_palette(cx)).with_priority(100))
            })
    }
}

/// Leading-indent width of a line (spaces; tabs count as 2).
fn indent_width(line: &str) -> usize {
    let mut w = 0usize;
    for c in line.chars() {
        if c == ' ' {
            w += 1;
        } else if c == '\t' {
            w += 2;
        } else {
            break;
        }
    }
    w
}

/// Indent width of the nearest non-blank logical line above `line_start`.
fn prev_indent_before(display: &str, line_start: usize) -> Option<usize> {
    let mut at = line_start.min(display.len());
    loop {
        if at == 0 {
            return None;
        }
        let prev_end = at - 1; // the `\n` ending the previous line (or before)
        let prev_line = logical_line_range(display, prev_end.min(display.len()));
        if prev_line.start >= at {
            return None;
        }
        let text = display.get(prev_line.clone()).unwrap_or("");
        if !text.trim().is_empty() {
            return Some(indent_width(text));
        }
        at = prev_line.start;
    }
}

fn open_in_browser(url: &str) {    // GPUI's open_url (NSWorkspace) is the primary launcher. Also try the
    // shell path as a fallback: `open` detaches from the GUI process in a way
    // `Command::spawn` from an app bundle sometimes does not, so wait for the
    // fast `open` exit instead of leaving a zombie child.
    for attempt in [&["/usr/bin/open", url][..], &["open", url][..]] {
        match std::process::Command::new(attempt[0])
            .arg(attempt[1])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
        {
            Ok(out) if out.status.success() => return,
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                eprintln!(
                    "crabmd: open {:?} exited {}: {}",
                    attempt,
                    out.status,
                    err.trim()
                );
            }
            Err(e) => eprintln!("crabmd: failed to spawn {:?}: {e:#}", attempt),
        }
    }
}

#[allow(dead_code)]
fn _rgb_anchor() -> gpui::Rgba {
    let _ = point(px(0.), px(0.));
    rgb(0x000000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(control: bool, alt: bool, platform: bool) -> gpui::Modifiers {
        gpui::Modifiers {
            control,
            alt,
            platform,
            ..Default::default()
        }
    }

    #[test]
    fn line_edit_backspace_variants() {
        let plain = mods(false, false, false);
        let mut s = LineField::new();
        s.insert_str("abc");
        assert!(s.delete_key("backspace", plain));
        assert_eq!(s.as_str(), "ab");
        // Empty buffer: fall through so callers (palette) can back out.
        let mut s = LineField::new();
        assert!(!s.delete_key("backspace", plain));

        let mut s = LineField::new();
        s.insert_str("foo bar baz");
        assert!(s.delete_key("backspace", mods(false, true, false)));
        assert_eq!(s.as_str(), "foo bar ");
        assert!(s.delete_key("backspace", mods(false, true, false)));
        assert_eq!(s.as_str(), "foo ");

        let mut s = LineField::new();
        s.insert_str("hello world");
        assert!(s.delete_key("backspace", mods(false, false, true)));
        assert_eq!(s.as_str(), "");
    }

    #[test]
    fn line_edit_caret_moves() {
        let plain = mods(false, false, false);
        let mut s = LineField::new();
        s.insert_str("abc");
        // Caret starts at the end; arrows move it instead of no-ops.
        assert!(s.caret_key("left", plain));
        assert!(s.caret_key("left", plain));
        s.insert_char('X');
        assert_eq!(s.as_str(), "aXbc");
        assert!(s.caret_key("right", mods(false, true, false)));
        assert_eq!(s.render(), "aXbc▌");
        assert!(s.caret_key("home", mods(false, false, true)));
        s.insert_char('Y');
        assert_eq!(s.as_str(), "YaXbc");
        assert!(!s.caret_key("up", plain));
    }
}
