//! One markdown buffer. GFM `source` is truth; paint and the caret walk a
//! visible projection. No textarea — WYSIWYG overlay on every block.

use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::{
    actions, deferred, div, img, point, prelude::FluentBuilder as _, px, relative, rgb, AnyElement,
    App, AppContext as _, Bounds, ClickEvent, ClipboardEntry, ClipboardItem, Context, CursorStyle,
    DragMoveEvent, Entity, EntityInputHandler, ExternalPaths, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseUpEvent, ParentElement as _, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, UTF16Selection, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    radio::Radio,
    switch::Switch,
    v_flex, Icon, Sizable as _, Theme, ThemeMode, ThemeTokens,
};

use crate::config::{self, Config, EditorKind};
use crate::display::{
    list_sibling_index, ordered_marker, project, wrap_cols_for, Affinity, BlockExtra, Projection,
    CODE_LANGS, COLUMN_PX,
};
use crate::document::{
    alert_icon_name, extract_links, parse_ranges, splice, BlockKind, PaintRange,
};
use crate::images;
use crate::mode::{self, Caret, ExCommand, Mode};
use crate::motion::{
    after_caret_same_line, apply_motion, block_caret_range, delete_char_at, delete_range,
    extend_visual_line, find_char, first_non_blank_in, join_next_lines, join_range,
    last_line_start, line_start_n, logical_line_delete_range, logical_line_range, paragraph_jump,
    push_count, replace_chars, replace_selection, search_next, search_prev, take_count,
    visual_line_range, whichwrap, word_range_at, FindKind, Motion,
};
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
        QuitApp,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", Save, Some("Workspace")),
        KeyBinding::new("ctrl-s", Save, Some("Workspace")),
        KeyBinding::new("escape", LeaveInsert, Some("Workspace")),
        KeyBinding::new("escape", LeaveInsert, Some("Input")),
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
        KeyBinding::new("shift-g", LastDoc, Some("Normal")),
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
        KeyBinding::new("cmd-k", ToggleLink, Some("Workspace")),
        KeyBinding::new("ctrl-k", ToggleLink, Some("Workspace")),
        KeyBinding::new("cmd-x", CutSelection, Some("Workspace")),
        KeyBinding::new("ctrl-x", CutSelection, Some("Workspace")),
        KeyBinding::new("cmd-c", CopySelection, Some("Workspace")),
        KeyBinding::new("ctrl-c", CopySelection, Some("Workspace")),
        KeyBinding::new("cmd-v", PasteClipboard, Some("Workspace")),
        KeyBinding::new("ctrl-v", PasteClipboard, Some("Workspace")),
        KeyBinding::new("cmd-a", SelectAll, Some("Workspace")),
        KeyBinding::new("ctrl-a", SelectAll, Some("Workspace")),
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
                    if let Ok(n) = raw.trim().parse::<u32>() {
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

pub struct Workspace {
    path: PathBuf,
    doc: crate::tree::Doc,
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
    status: SharedString,
    command: Option<String>,
    search: Option<(String, bool)>,
    last_search: Option<(String, bool)>,
    affinity: Affinity,
    marked: Option<Range<usize>>,
    sticky: crate::display::Marks,
    link_open: bool,
    link_draft: String,
    hits: Vec<Hit>,
    pending_replace: Option<usize>,
    pending_find: Option<(FindKind, usize)>,
    last_find: Option<(FindKind, char)>,
    pending_bracket: Option<i8>,
    config: Config,
    undo: UndoStack,
    insert_origin: Option<Snapshot>,
    settings_open: bool,
    settings_pane: SettingsPane,
    scroll_handle: ScrollHandle,
    /// Keep the caret in view after the next paint. Clicks leave scroll alone.
    follow_caret: bool,
    surface_h: gpui::Pixels,
    titlebar_moving: bool,
    mouse_anchor: Option<usize>,
    /// Initial word/line unit from a multi-click, used while dragging.
    mouse_unit: Option<Range<usize>>,
    mouse_dragging: bool,
    mouse_granularity: SelectGranularity,
    block_dragging: Option<usize>,
    block_drag_gap: Option<usize>,
    block_menu: Option<usize>,
    loading: bool,
    fonts: FontInputs,
}

impl Workspace {
    pub fn new(
        path: PathBuf,
        source: String,
        palette: Palette,
        config: Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        apply_palette(&palette, cx);

        let empty_doc = source.trim().is_empty();
        let fonts = FontInputs::new(&config, window, cx);
        let doc = crate::tree::Doc::from_gfm(&source);
        let mut this = Self {
            path,
            doc,
            source,
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
            status: "ready".into(),
            command: None,
            search: None,
            last_search: None,
            affinity: Affinity::Inside,
            marked: None,
            sticky: crate::display::Marks::default(),
            link_open: false,
            link_draft: String::new(),
            hits: Vec::new(),
            pending_replace: None,
            pending_find: None,
            last_find: None,
            pending_bracket: None,
            config,
            undo: UndoStack::default(),
            insert_origin: None,
            settings_open: false,
            settings_pane: SettingsPane::Editor,
            scroll_handle: ScrollHandle::new(),
            follow_caret: true,
            surface_h: px(0.),
            titlebar_moving: false,
            mouse_anchor: None,
            mouse_unit: None,
            mouse_dragging: false,
            mouse_granularity: SelectGranularity::Char,
            block_dragging: None,
            block_drag_gap: None,
            block_menu: None,
            loading: false,
            fonts,
        };

        if empty_doc || !this.config.editor.is_modal() {
            this.enter_insert(Caret::End, window, cx);
        } else {
            this.mode = Mode::Normal;
            this.refresh_raw(window, cx);
        }
        this
    }

    pub fn view(
        path: PathBuf,
        source: String,
        palette: Palette,
        config: Config,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new(path, source, palette, config, window, cx))
    }

    fn key_context(&self) -> &'static str {
        if self.command.is_some() || self.search.is_some() {
            return "Workspace Command";
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
        wrap_cols_for(self.config.markdown_font.size, self.config.wrap_motions)
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
        self.dirty = true;
        self.status = "unsaved".into();
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
        let n = self.proj().display.len();
        if self.caret > n {
            self.caret = n;
        }
        if let Some(sel) = self.sel.as_mut() {
            sel.start = sel.start.min(n);
            sel.end = sel.end.min(n);
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

    fn apply_source(
        &mut self,
        source: String,
        caret: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keep_insert = self.mode.is_insert() || self.is_notion();
        self.doc = crate::tree::Doc::from_gfm(&source);
        self.source = self.doc.to_gfm();
        self.caret = caret.min(self.proj().display.len());
        self.sel = None;
        self.visual_anchor = None;
        self.dirty = true;
        self.status = "unsaved".into();
        if keep_insert {
            self.mode = Mode::Insert;
        } else {
            self.mode = Mode::Normal;
        }
        self.refresh_raw(window, cx);
        self.sync_title(window);
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
        self.doc = snap.doc;
        self.source = snap.source;
        self.caret = snap.caret.min(self.doc.project().display.len());
        self.sel = snap.sel;
        self.insert_origin = None;
        self.clear_pending();
        self.visual_anchor = self.sel.as_ref().map(|s| s.start);
        self.dirty = true;
        self.status = "unsaved".into();
        // Notion stays in insert. Vim/Helix undo/redo always land in Normal —
        // never bounce back into Insert just because the snapshot was taken
        // during an insert session.
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
    }

    fn on_press_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_newline(false, window, cx);
    }

    fn insert_newline(&mut self, hard: bool, window: &mut Window, cx: &mut Context<Self>) {
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
        self.push_doc_undo();
        self.caret = self.doc.enter(self.caret, hard);
        self.sync_gfm();
        self.caret = self.caret.min(self.proj().display.len());
        self.sel = None;
        self.dirty = true;
        self.status = "unsaved".into();
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
        if self.link_open {
            self.link_open = false;
            self.link_draft.clear();
        }
        if !shift {
            let p = self.proj();
            if let Some((range, url)) = p.link_at(d) {
                if cmd {
                    self.open_link_url(url, cx);
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
        if click_count >= 2 && !shift {
            let p = self.proj();
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
        if self.link_open {
            self.link_open = false;
            self.link_draft.clear();
        }
        if !self.mouse_dragging {
            return;
        }
        let p = self.proj();
        let d = d.min(p.display.len());
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

    fn commit_edit(
        &mut self,
        source: String,
        caret: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc = crate::tree::Doc::from_gfm(&source);
        self.source = self.doc.to_gfm();
        self.caret = caret.min(self.proj().display.len());
        self.sel = None;
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn slash_query(&self) -> Option<String> {
        let p = self.proj();
        let d = self.caret;
        let block = p.block_at_display(d)?;
        let body = &p.display[block.display.clone()];
        let local = d.saturating_sub(block.display.start).min(body.len());
        let line_start = body[..local].rfind('\n').map(|i| i + 1).unwrap_or(0);
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
        self.slash_index = 0;
        self.last_slash_query.clear();
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh_raw(window, cx);
    }

    fn close_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.proj();
        if let Some((display, _)) = {
            let d = self.caret.min(p.display.len());
            p.block_at_display(d).and_then(|block| {
                let body = p.display.get(block.display.clone())?;
                let local = d.saturating_sub(block.display.start).min(body.len());
                let line_start = body[..local].rfind('\n').map(|i| i + 1).unwrap_or(0);
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
        self.dirty = true;
        self.status = "unsaved".into();
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
        if self.command.is_some() || self.search.is_some() || self.link_open {
            return;
        }
        window.prevent_default();
        let count = take_count(&mut self.pending_count);
        self.pending_g = false;
        let p = self.proj();
        let d = self.caret;
        let wrap = self.wrap_cols();
        let mut next_d = apply_motion(&p.display, d, motion, count, wrap);
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
        if self.pending_replace.is_some()
            || self.pending_find.is_some()
            || self.pending_bracket.is_some()
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
        if self.slash_is_open() {
            self.close_slash(window, cx);
            return;
        }
        window.prevent_default();
        self.leave_insert(window, cx);
    }

    fn on_insert_caret(&mut self, _: &InsertAtCaret, window: &mut Window, cx: &mut Context<Self>) {
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
        let (next, caret) = wysiwyg::open_line(&self.source, self.caret, above);
        self.source = next;
        self.caret = caret;
        self.dirty = true;
        self.status = "unsaved".into();
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
        self.go_doc_edge(false, window, cx);
    }
    fn on_last_doc(&mut self, _: &LastDoc, window: &mut Window, cx: &mut Context<Self>) {
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
        let range = if let Some(sel) = self.sel.clone() {
            sel
        } else {
            block_caret_range(&self.source, self.caret)
        };
        self.push_doc_undo();
        let (next, caret) = if range.start == range.end {
            delete_char_at(&self.source, range.start)
        } else {
            delete_range(&self.source, range)
        };
        self.apply_source(next, caret, window, cx);
    }

    fn delete_current_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let range = logical_line_delete_range(&self.source, self.caret);
        self.push_doc_undo();
        let (next, caret) = delete_range(&self.source, range);
        self.apply_source(next, caret, window, cx);
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
        let end = logical_line_range(&self.source, start).end;
        self.push_doc_undo();
        let (next, caret) = delete_range(&self.source, start..end);
        self.apply_source(next, caret, window, cx);
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
        window.prevent_default();
        self.finish_insert_undo();
        let current = self.snapshot();
        if let Some(prev) = self.undo.undo(current) {
            self.apply_snapshot(prev, window, cx);
        }
    }
    fn on_redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
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
        if self.command.is_some() || self.search.is_some() || self.link_open {
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
        self.dirty = true;
        self.status = "unsaved".into();
        let q = self.slash_query().unwrap_or_default();
        if q != self.last_slash_query {
            self.last_slash_query = q;
            self.slash_index = 0;
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
        let (next, caret) = wysiwyg::delete_display_range(&self.source, start_d..d);
        self.commit_edit(next, caret, window, cx);
    }

    fn on_delete_line_back(
        &mut self,
        _: &DeleteLineBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let (next, caret) = wysiwyg::delete_display_range(&self.source, start_d..d);
        self.commit_edit(next, caret, window, cx);
    }

    fn on_block_backspace(
        &mut self,
        _: &BlockBackspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.search = Some((String::new(), backward));
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn cancel_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search = None;
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn submit_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((query, backward)) = self.search.take() else {
            return;
        };
        self.focus.focus(window, cx);
        let q = if query.is_empty() {
            match &self.last_search {
                Some((prev, _)) => prev.clone(),
                None => {
                    cx.notify();
                    return;
                }
            }
        } else {
            query
        };
        self.last_search = Some((q.clone(), backward));
        self.jump_search(!backward, true, window, cx);
        cx.notify();
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
        let (next, caret) = if let Some(sel) = self.sel.clone() {
            join_range(&self.source, sel)
        } else {
            join_next_lines(&self.source, self.caret, count)
        };
        self.apply_source(next, caret, window, cx);
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
        let (next, caret) = if let Some(sel) = self.sel.clone() {
            replace_selection(&self.source, sel, ch)
        } else {
            replace_chars(&self.source, self.caret, count, ch)
        };
        self.apply_source(next, caret, window, cx);
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
        self.on_find(FindKind::Till, window, cx);
    }
    fn on_find_till_back(&mut self, _: &FindTillBack, window: &mut Window, cx: &mut Context<Self>) {
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
    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        if !self.settings_open {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }
    fn set_editor(&mut self, editor: EditorKind, window: &mut Window, cx: &mut Context<Self>) {
        let was_notion = self.is_notion();
        self.config.editor = editor;
        self.persist_config(cx);
        if editor == EditorKind::Notion {
            self.enter_insert(Caret::End, window, cx);
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
    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.write_to_disk(cx) {
            self.status = "saved".into();
        }
        self.sync_title(window);
    }
    fn write_to_disk(&mut self, cx: &mut Context<Self>) -> bool {
        let ok = match std::fs::write(&self.path, self.source.as_bytes()) {
            Ok(()) => {
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
        self.command = Some(String::new());
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
        match mode::parse_ex(&input) {
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
                self.config.theme = name.to_string();
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
        self.dirty = true;
        self.status = "unsaved".into();
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
        let line = images::gfm_image(alt, filename);
        self.push_doc_undo();
        let at = self.caret_src();
        let insert = if at > 0 && !self.source[..at].ends_with('\n') {
            format!("\n{line}\n")
        } else {
            format!("{line}\n")
        };
        self.source = splice(&self.source, at..at, &insert);
        self.set_caret_src(at + insert.len());
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh(window, cx);
        cx.notify();
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
            if !images::is_image_path(path) {
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
    fn handle_capture_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = ev.keystroke.key.as_str();
        let mods = ev.keystroke.modifiers;

        // Let cmd/ctrl-c/x/v reach Copy/Cut/Paste actions (including overlays).
        if matches!(key, "c" | "x" | "v")
            && (mods.platform || mods.control)
            && !mods.alt
            && !mods.shift
        {
            return false;
        }

        if self.link_open {
            window.prevent_default();
            match key {
                "escape" => {
                    self.link_open = false;
                    self.link_draft.clear();
                    cx.notify();
                }
                "enter" => self.commit_link(window, cx),
                "backspace" => {
                    self.link_draft.pop();
                    cx.notify();
                }
                "space" => {
                    self.link_draft.push(' ');
                    cx.notify();
                }
                k if k.chars().count() == 1 && !mods.control && !mods.platform => {
                    if let Some(ch) = k.chars().next() {
                        if !ch.is_control() {
                            self.link_draft.push(ch);
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

        if self.pending_g && self.is_modal_nav() && key == "s" && !mods.control && !mods.platform {
            window.prevent_default();
            self.pending_g = false;
            self.apply_buffer_motion(Motion::LineFirstNonBlank, window, cx);
            return true;
        }

        if self.command.is_some() {
            window.prevent_default();
            if mods.control || mods.platform || mods.alt {
                return true;
            }
            match key {
                "escape" => self.cancel_command(window, cx),
                "enter" => self.submit_command(window, cx),
                "backspace" => {
                    if let Some(buf) = self.command.as_mut() {
                        buf.pop();
                    }
                    cx.notify();
                }
                "space" => {
                    if let Some(buf) = self.command.as_mut() {
                        buf.push(' ');
                    }
                    cx.notify();
                }
                k if k.chars().count() == 1 => {
                    if let Some(ch) = k.chars().next() {
                        if !ch.is_control() {
                            if let Some(buf) = self.command.as_mut() {
                                buf.push(ch);
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
            if mods.control || mods.platform || mods.alt {
                return true;
            }
            match key {
                "escape" => self.cancel_search(window, cx),
                "enter" => self.submit_search(window, cx),
                "backspace" => {
                    if let Some((buf, _)) = self.search.as_mut() {
                        buf.pop();
                    }
                    cx.notify();
                }
                "space" => {
                    if let Some((buf, _)) = self.search.as_mut() {
                        buf.push(' ');
                    }
                    cx.notify();
                }
                k if k.chars().count() == 1 => {
                    if let Some(ch) = k.chars().next() {
                        if !ch.is_control() {
                            if let Some((buf, _)) = self.search.as_mut() {
                                buf.push(ch);
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
            && !self.link_open
        {
            window.prevent_default();
            self.insert_newline(mods.shift, window, cx);
            return true;
        }

        let insert_like = self.mode.is_insert() || self.is_notion();
        let can_nav = (self.is_modal_nav() || insert_like)
            && self.command.is_none()
            && self.search.is_none()
            && !self.link_open
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

    fn render_slash_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let items = self.slash_items();
        if items.is_empty() {
            return div().into_any_element();
        }
        let selected = slash::clamp_index(self.slash_index, items.len());
        let p = &self.palette;
        v_flex()
            .w(px(340.))
            .py_1()
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.background_panel)
            .shadow_lg()
            .children(items.into_iter().enumerate().map(|(i, item)| {
                let active = i == selected;
                div()
                    .id(("slash-item", i))
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
                        cx.listener(move |this, _, _, cx| {
                            this.open_link_url(&url, cx);
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

    fn render_titlebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette.clone();
        let file_name = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string();
        let dirty = self.dirty;
        let inset = if cfg!(target_os = "macos") {
            px(80.)
        } else {
            px(12.)
        };
        h_flex()
            .id("titlebar")
            .w_full()
            .h(px(36.))
            .pl(inset)
            .pr_3()
            .items_center()
            .font_family(self.config.ui_font.family.clone())
            .text_size(self.ui_font_px())
            .bg(p.background_panel)
            .border_b_1()
            .border_color(p.border)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.titlebar_moving = true;
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.titlebar_moving = false;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, _, window, _cx| {
                if this.titlebar_moving {
                    this.titlebar_moving = false;
                    window.start_window_move();
                }
            }))
            .on_click(|ev, window, _| {
                if ev.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.markdown_text)
                            .child(file_name),
                    )
                    .when(dirty, |el| {
                        el.child(div().w(px(7.)).h(px(7.)).rounded_full().bg(p.primary))
                    }),
            )
            .into_any_element()
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette.clone();
        let pane = self.settings_pane;
        let editor = self.config.editor;
        let theme_name = self.palette.name.clone();
        let wrap = self.config.wrap_motions;

        let nav_row = |label: &'static str, which: SettingsPane, cx: &mut Context<Self>| {
            let active = pane == which;
            div()
                .id(("settings-nav", which as usize))
                .w_full()
                .px_3()
                .py_2()
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
                            .child(
                                Radio::new("editor-helix")
                                    .label("Helix")
                                    .checked(editor == EditorKind::Helix)
                                    .on_click(cx.listener(|this, checked, window, cx| {
                                        if *checked {
                                            this.set_editor(EditorKind::Helix, window, cx);
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .pl_6()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .child("x select line · d delete · v select · u undo · U redo"),
                            )
                            .child(
                                Radio::new("editor-vim")
                                    .label("Vim")
                                    .checked(editor == EditorKind::Vim)
                                    .on_click(cx.listener(|this, checked, window, cx| {
                                        if *checked {
                                            this.set_editor(EditorKind::Vim, window, cx);
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .pl_6()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .child("x delete char · dd line · v/V visual · u undo · ctrl-r redo"),
                            )
                            .child(
                                Radio::new("editor-notion")
                                    .label("Notion")
                                    .checked(editor == EditorKind::Notion)
                                    .on_click(cx.listener(|this, checked, window, cx| {
                                        if *checked {
                                            this.set_editor(EditorKind::Notion, window, cx);
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .pl_6()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .child("Rendered blocks; edit visible text. j/k move between blocks."),
                            ),
                    )
                    .child(
                        Switch::new("wrap-motions")
                            .label("Wrap lines")
                            .checked(wrap)
                            .on_click(move |checked, window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_wrap_motions(*checked, window, cx);
                                });
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.text_muted)
                            .child("When on, j/k count wrapped visual lines and the surface wraps. Off: logical lines."),
                    )
                    .into_any_element()
            }
            SettingsPane::Theme => {
                let names = theme::list_theme_names();
                v_flex()
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
                            .child("OpenCode JSON themes. Applied immediately."),
                    )
                    .child(v_flex().gap_1().children(names.into_iter().enumerate().map(
                        |(i, name)| {
                            let selected = theme_name.eq_ignore_ascii_case(name);
                            Radio::new(("theme-opt", i))
                                .label(name.to_string())
                                .checked(selected)
                                .on_click(cx.listener(move |this, checked, window, cx| {
                                    if *checked {
                                        this.set_theme(name, window, cx);
                                    }
                                }))
                        },
                    )))
                    .into_any_element()
            }
            SettingsPane::Font => {
                let row = |label: &'static str,
                           family: &Entity<InputState>,
                           size: &Entity<InputState>,
                           hint: &'static str,
                           p: &crate::theme::Palette| {
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
                                        .child(Input::new(family).cleanable(true)),
                                )
                                .child(div().w(px(72.)).child(Input::new(size).cleanable(false))),
                        )
                };
                let p = &self.palette;
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
                            .child("UI · Markdown · Buffer (code). Family + size (px)."),
                    )
                    .child(row(
                        "UI",
                        &self.fonts.ui_family,
                        &self.fonts.ui_size,
                        "Titlebar, footer, settings",
                        p,
                    ))
                    .child(row(
                        "Markdown",
                        &self.fonts.markdown_family,
                        &self.fonts.markdown_size,
                        "Paragraphs, headings, lists, quotes",
                        p,
                    ))
                    .child(row(
                        "Buffer",
                        &self.fonts.buffer_family,
                        &self.fonts.buffer_size,
                        "Fenced code blocks",
                        p,
                    ))
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.settings_open = false;
                    this.focus.focus(window, cx);
                    cx.notify();
                }),
            )
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
                            .px_5()
                            .py_4()
                            .gap_2()
                            .child(
                                h_flex().w_full().min_w_0().justify_end().child(
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
                                    .w_full()
                                    .overflow_y_scroll()
                                    .overflow_x_hidden()
                                    .child(content),
                            ),
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
            ime,
            &pal,
            placeholder,
            font,
            font_px,
            heading,
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
        let wrap = self.config.wrap_motions;
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
        let placeholder = if empty && caret_here {
            Some("Type to write, or / for blocks")
        } else {
            None
        };
        let heading = matches!(block.kind, BlockKind::Heading(_));
        let heading_level = match block.kind {
            BlockKind::Heading(l) => Some(l),
            _ => None,
        };
        let is_code = matches!(block.kind, BlockKind::Code);
        let syntax_lang = match &block.extra {
            BlockExtra::Code { lang } if !lang.is_empty() => Some(lang.as_str()),
            _ => None,
        };
        let body = self.render_edit(
            block.display.clone(),
            &text,
            heading,
            heading_level,
            placeholder,
            wrap && !matches!(block.kind, BlockKind::Code | BlockKind::Html),
            syntax_lang,
            is_code,
            cx,
        );
        let slash = self.slash_is_open()
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
            BlockExtra::Code { lang } => {
                let caret = self.caret;
                let family = self.config.buffer_font.family.clone();
                v_flex()
                    .w_full()
                    .min_w_0()
                    .rounded(px(6.))
                    .bg(pal.background_element)
                    .px_3()
                    .py_2()
                    .gap_1()
                    .font_family(family)
                    .text_size(self.buffer_font_px())
                    .text_color(pal.markdown_code_block)
                    .child(self.render_lang_chip(lang, caret, cx))
                    .child(body)
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
                self.render_image_hit(alt, src, block.display.start, cx)
            }
            BlockExtra::Heading(level) => {
                let mut el = v_flex()
                    .relative()
                    .w_full()
                    .min_w_0()
                    .when(*level <= 2, |el| el.pt_2())
                    .child(body);
                if slash {
                    el = el.child(
                        deferred(
                            div()
                                .absolute()
                                .top(px(28.))
                                .left_0()
                                .occlude()
                                .child(self.render_slash_menu(cx)),
                        )
                        .with_priority(1),
                    );
                }
                el.into_any_element()
            }
            BlockExtra::List { items, ordered } => self.render_list(ix, items, *ordered, body, cx),
            BlockExtra::Table { .. } => self.render_table_block(ix, cx),
            BlockExtra::Text | BlockExtra::Html => {
                let mut el = v_flex().relative().w_full().min_w_0().child(body);
                if slash {
                    // `deferred` paints after later sibling blocks so the menu
                    // floats above the document instead of sitting underneath.
                    el = el.child(
                        deferred(
                            div()
                                .absolute()
                                .top(px(28.))
                                .left_0()
                                .occlude()
                                .child(self.render_slash_menu(cx)),
                        )
                        .with_priority(1),
                    );
                }
                el.into_any_element()
            }
        }
    }

    fn render_image_hit(
        &self,
        alt: &str,
        src: &str,
        display_start: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pal = &self.palette;
        let path = images::resolve_beside(&self.path, src);
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
            .when(path.exists(), |el| {
                el.child(
                    img(path.clone())
                        .max_w_full()
                        .max_h(px(480.))
                        .rounded(px(6.)),
                )
            })
            .when(!path.exists(), |el| {
                el.child(
                    div()
                        .text_color(pal.text_muted)
                        .child(format!("missing image: {src}")),
                )
            })
            .when(!alt.is_empty(), |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(pal.text_muted)
                        .child(alt.to_string()),
                )
            })
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
                                    if let Some((next, caret)) =
                                        wysiwyg::set_code_lang(&this.source, this.caret, &lang)
                                    {
                                        this.commit_edit(next, caret, window, cx);
                                    }
                                });
                            }));
                    }
                    menu
                }
            })
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
        let wrap = self.config.wrap_motions;
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
        let wrap = self.config.wrap_motions;
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
        let in_table = p.table_cell_at(self.caret).is_some();
        v_flex()
            .w_full()
            .min_w_0()
            .border_1()
            .border_color(pal.border)
            .rounded(px(8.))
            .overflow_hidden()
            .children(grid.into_iter().enumerate().map(|(r, row)| {
                h_flex()
                    .w_full()
                    .children(row.into_iter().enumerate().map(|(c, cell)| {
                        let (disp, header) = if let Some(cell) = cell {
                            (cell.display, cell.header)
                        } else {
                            (0..0, r == 0)
                        };
                        let text = p
                            .display
                            .get(disp.clone())
                            .unwrap_or("")
                            .replace('\t', "")
                            .to_string();
                        let edit = self
                            .render_edit(disp, &text, header, None, None, wrap, None, false, cx);
                        div()
                            .id(("td", r * 100 + c))
                            .flex_1()
                            .min_w_0()
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
                            .into_any_element()
                    }))
                    .into_any_element()
            }))
            .when(in_table, |el| el.child(self.render_table_menu(cx)))
            .into_any_element()
    }

    fn render_table_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_1()
            .p_1()
            .child(self.table_btn("row-above", "Row ↑", false, true, cx))
            .child(self.table_btn("row-below", "Row ↓", false, false, cx))
            .child(self.table_btn("col-left", "Col ←", true, true, cx))
            .child(self.table_btn("col-right", "Col →", true, false, cx))
            .child(
                Button::new("tbl-del")
                    .ghost()
                    .xsmall()
                    .label("Delete")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.delete_current_table(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn table_btn(
        &self,
        id: &'static str,
        label: &'static str,
        col: bool,
        before: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(id)
            .ghost()
            .xsmall()
            .label(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.table_insert(col, before, window, cx);
            }))
            .into_any_element()
    }

    fn table_insert(
        &mut self,
        col: bool,
        before: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = self.proj();
        let d = self.caret;
        let Some((block, cell)) = p.table_cell_at(d) else {
            return;
        };
        let BlockExtra::Table { cells, cols, .. } = &block.extra else {
            return;
        };
        let cols = *cols;
        let (mut headers, mut rows) = {
            // rebuild from display
            let mut headers = vec![String::new(); cols.max(1)];
            let mut body: Vec<Vec<String>> = Vec::new();
            for c in cells {
                let text = p
                    .display
                    .get(c.display.clone())
                    .unwrap_or("")
                    .replace('\t', "")
                    .to_string();
                if c.header {
                    if c.col < headers.len() {
                        headers[c.col] = text;
                    }
                } else {
                    let br = c.row.saturating_sub(1);
                    while body.len() <= br {
                        body.push(vec![String::new(); cols.max(1)]);
                    }
                    if c.col < body[br].len() {
                        body[br][c.col] = text;
                    }
                }
            }
            (headers, body)
        };
        if col {
            let at = if before { cell.col } else { cell.col + 1 };
            headers.insert(at.min(headers.len()), String::new());
            for row in &mut rows {
                row.insert(at.min(row.len()), String::new());
            }
        } else {
            let body_row = if cell.header {
                0
            } else {
                cell.row.saturating_sub(1)
            };
            let at = if before { body_row } else { body_row + 1 };
            rows.insert(
                at.min(rows.len()),
                vec![String::new(); headers.len().max(1)],
            );
        }
        let gfm = crate::display::serialize_table(&headers, &rows);
        self.push_doc_undo();
        let next = splice(&self.source, block.source.clone(), &gfm);
        self.commit_edit(next, block.source.start, window, cx);
    }

    fn delete_current_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.proj();
        let d = self.caret;
        let Some(block) = p.block_at_display(d) else {
            return;
        };
        if !matches!(block.kind, BlockKind::Table) {
            return;
        }
        self.push_doc_undo();
        let next = splice(&self.source, block.source.clone(), "");
        self.commit_edit(next, block.source.start, window, cx);
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
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_link_url(&url, cx);
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
        let p = self.proj();
        let this = p.blocks.get(ix)?;
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
        if !show {
            return None;
        }
        // Float above the selected block (same deferred trick as the slash menu).
        Some(
            deferred(
                div()
                    .absolute()
                    .top(px(-40.))
                    .left(px(48.))
                    .occlude()
                    .child(self.render_bubble(cx)),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    fn mark_btn(&self, label: &'static str, mark: Mark, cx: &mut Context<Self>) -> AnyElement {
        Button::new(("mk", label.len()))
            .ghost()
            .xsmall()
            .label(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_mark_action(mark, window, cx);
            }))
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
                    .child(format!("{}▌", self.link_draft)),
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
                self.dirty = true;
                self.status = "unsaved".into();
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
            if let Some((next, caret)) = wysiwyg::tab(&self.source, self.caret, false) {
                self.push_doc_undo();
                self.commit_edit(next, caret, window, cx);
            }
        }
    }
    fn on_outdent_tab(&mut self, _: &OutdentTab, window: &mut Window, cx: &mut Context<Self>) {
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
                self.dirty = true;
                self.status = "unsaved".into();
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
            if let Some((next, caret)) = wysiwyg::tab(&self.source, self.caret, true) {
                self.push_doc_undo();
                self.commit_edit(next, caret, window, cx);
            }
        }
    }
    fn on_toggle_bold(&mut self, _: &ToggleBold, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark_action(Mark::Bold, window, cx);
    }
    fn on_toggle_italic(&mut self, _: &ToggleItalic, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark_action(Mark::Italic, window, cx);
    }
    fn on_toggle_strike(&mut self, _: &ToggleStrike, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark_action(Mark::Strike, window, cx);
    }
    fn on_toggle_code(&mut self, _: &ToggleCode, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark_action(Mark::Code, window, cx);
    }
    fn on_toggle_underline(
        &mut self,
        _: &ToggleUnderline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_mark_action(Mark::Underline, window, cx);
    }
    fn on_toggle_link(&mut self, _: &ToggleLink, window: &mut Window, cx: &mut Context<Self>) {
        if self.sel.as_ref().is_none_or(|s| s.start == s.end) && !self.link_open {
            return;
        }
        self.link_open = true;
        self.link_draft.clear();
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn on_cut_selection(&mut self, _: &CutSelection, window: &mut Window, cx: &mut Context<Self>) {
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
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn on_copy_selection(
        &mut self,
        _: &CopySelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.prevent_default();
        if self.link_open {
            if !self.link_draft.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(self.link_draft.clone()));
            }
            return;
        }
        if let Some(buf) = self.command.as_ref() {
            if !buf.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(buf.clone()));
            }
            return;
        }
        if let Some((buf, _)) = self.search.as_ref() {
            if !buf.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(buf.clone()));
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
            self.link_draft.push_str(&text.replace('\n', ""));
            cx.notify();
            return;
        }
        if let Some(buf) = self.command.as_mut() {
            buf.push_str(&text.replace('\n', ""));
            cx.notify();
            return;
        }
        if let Some((buf, _)) = self.search.as_mut() {
            buf.push_str(&text.replace('\n', ""));
            cx.notify();
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
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn on_select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        window.prevent_default();
        self.clear_pending();
        let end = self.proj().display.len();
        self.sel = Some(0..end);
        self.caret = end;
        self.visual_anchor = Some(0);
        self.mouse_anchor = Some(0);
        self.refresh(window, cx);
    }

    fn render_surface(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !cx.has_active_drag() {
            self.block_dragging = None;
            self.block_drag_gap = None;
        }
        if cx.has_active_drag() {
            self.block_menu = None;
        }
        self.hits.clear();
        let p = self.proj();
        let us = wysiwyg::units(&p);
        let n = us.len();
        let mut kids = Vec::new();
        for (ui, unit) in us.iter().copied().enumerate() {
            let view = cx.entity();
            let group = SharedString::from(format!("row-{ui}"));
            let dragging = self.block_dragging == Some(ui);
            let disp = wysiwyg::unit_display(&p, unit);
            let start = disp.start;
            let end = disp.end;
            let list_item = unit.item;
            kids.push(
                div()
                    .id(("row", ui))
                    .group(group.clone())
                    .relative()
                    .w(px(COLUMN_PX))
                    .max_w_full()
                    .min_w_0()
                    .mx_auto()
                    .px_8()
                    .when(list_item.is_some(), |el| el.py_1())
                    .when(list_item.is_none(), |el| el.py_2())
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
                    .child(if let Some(item_ix) = list_item {
                        self.render_list_item_row(unit.block, item_ix, cx)
                    } else {
                        self.render_block(unit.block, cx)
                    })
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
        let wrap = self.config.wrap_motions;
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
        if let Some((next, caret)) = wysiwyg::delete_block(&self.source, ix) {
            self.push_doc_undo();
            self.commit_edit(next, caret, window, cx);
        }
    }

    fn duplicate_block_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.block_menu = None;
        if let Some((next, caret)) = wysiwyg::duplicate_block(&self.source, ix) {
            self.push_doc_undo();
            self.commit_edit(next, caret, window, cx);
        }
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
        if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
            self.push_doc_undo();
            if let Some(range) = self.doc.toggle_mark(sel, mark) {
                self.sync_gfm();
                self.caret = range.end.min(self.proj().display.len());
                self.sel = Some(range);
                self.dirty = true;
                self.status = "unsaved".into();
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
        let url = self.link_draft.clone();
        self.link_open = false;
        self.link_draft.clear();
        let sel = self.sel.clone().unwrap_or(self.caret..self.caret);
        self.push_doc_undo();
        self.caret = self.doc.apply_link(sel, &url);
        self.sync_gfm();
        self.caret = self.caret.min(self.proj().display.len());
        self.sel = None;
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn open_link_url(&self, raw: &str, cx: &mut App) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            eprintln!("crabmd: open_link_url: empty url");
            return;
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
        cx.open_url(&url);
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
        self.dirty = true;
        self.status = "unsaved".into();
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
    Icon::default()
        .path(crate::assets::path(name))
        .text_color(color)
        .w(px(16.))
        .h(px(16.))
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
        theme.accent = palette.accent;
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
        theme.scrollbar = palette.background;
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
        if self.is_modal_nav() {
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
            self.dirty = true;
            self.status = "unsaved".into();
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
        self.dirty = true;
        self.status = "unsaved".into();
        let q = self.slash_query().unwrap_or_default();
        if q != self.last_slash_query {
            self.last_slash_query = q;
            self.slash_index = 0;
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
        for hit in &self.hits {
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
        !self.is_modal_nav() && self.command.is_none() && self.search.is_none() && !self.link_open
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
        let status = self.status.clone();
        let theme_name = p.name.clone();
        let mode_label = self.mode.status(self.config.editor);
        let insert = self.mode.is_insert() || self.is_notion();
        let settings_open = self.settings_open;
        let command = self.command.clone();
        let search = self.search.clone();

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
            .on_action(cx.listener(|_, _: &QuitApp, _, cx| cx.quit()))
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
            .child(self.render_titlebar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .w_full()
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
                    .gap_1()
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
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
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
                    .when_some(command, |el, cmd| {
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
                                        .child(format!("{cmd}▌")),
                                ),
                        )
                    })
                    .when_some(search, |el, (q, back)| {
                        let prefix = if back { "?" } else { "/" };
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
                                        .child(prefix),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(p.markdown_text)
                                        .child(format!("{q}▌")),
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
            .when(settings_open, |el| el.child(self.render_settings(cx)))
    }
}

fn open_in_browser(url: &str) {
    // GPUI's open_url (NSWorkspace) is the primary launcher. Also try the
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
