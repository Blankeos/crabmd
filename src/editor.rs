//! One markdown buffer. GFM `source` is truth; paint and the caret walk a
//! visible projection. No textarea — WYSIWYG overlay on every block.

use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::{
    actions, div, img, point, prelude::FluentBuilder as _, px, rgb, AnyElement, App,
    AppContext as _, ClipboardEntry, Context, Entity, EntityInputHandler,
    ExternalPaths, FocusHandle, Focusable, FontWeight, InteractiveElement as _, IntoElement,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, MouseUpEvent,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled, UTF16Selection, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    radio::Radio,
    switch::Switch,
    v_flex, Icon, Sizable as _, Theme, ThemeMode, ThemeTokens,
};

use crate::config::{self, Config, EditorKind, FONT_FAMILIES, FONT_SIZES};
use crate::display::{
    project, wrap_cols_for, Affinity, BlockExtra, CODE_LANGS, COLUMN_PX, Projection,
};
use crate::document::{
    alert_icon_name, extract_links, parse_ranges, splice, toggle_task_line, BlockKind, PaintRange,
};
use crate::surface::{self, Hit};
use crate::wysiwyg::{self, Mark};
use crate::images;
use crate::mode::{self, Caret, ExCommand, Mode};
use crate::motion::{
    after_caret, apply_motion, block_caret_range, delete_char_at, delete_range, extend_visual_line,
    find_char, first_non_blank_in, join_next_lines, join_range, last_line_start,
    line_start_n, logical_line_delete_range, logical_line_range, open_line_above, open_line_below,
    paragraph_jump, push_count, replace_chars, replace_selection, search_next, search_prev,
    take_count, visual_line_range, FindKind, Motion,
};
use crate::slash::{self, SlashItem};
use crate::theme::{self, Appearance, Palette};
use crate::undo::{Snapshot, UndoStack};

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
        ToggleLink,
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
        KeyBinding::new("j", LineDown, Some("Notion")),
        KeyBinding::new("k", LineUp, Some("Notion")),
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
        KeyBinding::new("/", OpenSearch, Some("Normal")),
        KeyBinding::new("shift-/", SearchBack, Some("Normal")),
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
        KeyBinding::new("cmd-e", ToggleCode, Some("Workspace")),
        KeyBinding::new("ctrl-e", ToggleCode, Some("Workspace")),
        KeyBinding::new("cmd-shift-s", ToggleStrike, Some("Workspace")),
        KeyBinding::new("cmd-k", ToggleLink, Some("Workspace")),
        KeyBinding::new("ctrl-k", ToggleLink, Some("Workspace")),
        KeyBinding::new("backspace", BlockBackspace, Some("Workspace")),
        KeyBinding::new("shift-backspace", BlockBackspace, Some("Workspace")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
enum SettingsPane {
    Editor,
    Theme,
    Font,
}

pub struct Workspace {
    path: PathBuf,
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
    titlebar_moving: bool,
    mouse_anchor: Option<usize>,
    mouse_dragging: bool,
    loading: bool,
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
        let mut this = Self {
            path,
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
            titlebar_moving: false,
            mouse_anchor: None,
            mouse_dragging: false,
            loading: false,
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
        wrap_cols_for(self.config.font_size, self.config.wrap_motions)
    }

    fn proj(&self) -> Projection {
        project(&self.source)
    }

    fn font_px(&self) -> gpui::Pixels {
        px(self.config.font_size.clamp(13, 20) as f32)
    }

    fn paint_ranges(&self) -> Vec<PaintRange> {
        parse_ranges(&self.source)
    }

    fn compute_raw(&self) -> Range<usize> {
        0..0
    }

    fn clamp_caret(&mut self) {
        if self.caret > self.source.len() {
            self.caret = self.source.len();
        }
        if let Some(sel) = self.sel.as_mut() {
            sel.start = sel.start.min(self.source.len());
            sel.end = sel.end.min(self.source.len());
            if sel.start == sel.end {
                self.sel = None;
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot::new(&self.source, self.caret, self.sel.clone(), self.mode)
    }

    fn push_doc_undo(&mut self) {
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
        self.scroll_caret_into_view();
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn snap_visual_sel(&mut self) {
        let Some(anchor) = self.visual_anchor else {
            return;
        };
        let p = self.proj();
        let da = p.to_display(anchor);
        let dc = p.to_display(self.caret);
        if self.mode == Mode::VisualLine {
            let a = da.min(dc);
            let b = da.max(dc);
            let start = logical_line_range(&p.display, a).start;
            let mut end = logical_line_range(&p.display, b).end;
            if end < p.display.len() && p.display.as_bytes()[end] == b'\n' {
                end += 1;
            }
            let s = p.display_range_to_source(start..end, Affinity::Inside);
            self.sel = Some(s);
        } else {
            let a = da.min(dc);
            let b = da.max(dc);
            if a == b {
                self.sel = None;
            } else {
                self.sel = Some(p.display_range_to_source(a..b, Affinity::Inside));
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
        self.source = source;
        self.caret = caret.min(self.source.len());
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
        self.caret = next.min(self.source.len());
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

    fn scroll_caret_into_view(&self) {
        let ranges = self.paint_ranges();
        if let Some(ix) = ranges.iter().position(|r| {
            r.range.start <= self.caret && (self.caret < r.range.end || r.range.end == self.source.len())
        }) {
            self.scroll_handle.scroll_to_item(ix);
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
        let Some(origin) = self.insert_origin.take() else {
            return;
        };
        let current = self.snapshot();
        if origin.source != current.source {
            self.undo.push(origin);
        }
    }

    fn apply_snapshot(&mut self, snap: Snapshot, window: &mut Window, cx: &mut Context<Self>) {
        self.source = snap.source;
        self.caret = snap.caret.min(self.source.len());
        self.sel = snap.sel;
        self.insert_origin = None;
        self.clear_pending();
        self.visual_anchor = self.sel.as_ref().map(|s| s.start);
        self.dirty = true;
        self.status = "unsaved".into();
        if !self.config.editor.is_modal() || snap.mode == Mode::Insert {
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
        let (next, caret) = wysiwyg::enter(&self.source, self.caret, self.affinity, hard);
        self.source = next;
        self.caret = caret.min(self.source.len());
        self.sel = None;
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    pub fn click_display(&mut self, d: usize, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.proj();
        let src = p.to_source(d, Affinity::Inside);
        if shift {
            if self.visual_anchor.is_none() {
                self.visual_anchor = Some(self.caret);
            }
            self.caret = src;
            self.mode = if self.config.editor == EditorKind::Helix {
                Mode::Select
            } else if self.config.editor.is_modal() {
                Mode::Visual
            } else {
                self.mode
            };
            self.snap_visual_sel();
        } else {
            self.caret = src;
            self.mouse_anchor = Some(src);
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
        self.refresh(window, cx);
    }

    pub fn drag_display(&mut self, d: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mouse_dragging {
            return;
        }
        let p = self.proj();
        let src = p.to_source(d, Affinity::Inside);
        let anchor = self.mouse_anchor.unwrap_or(self.caret);
        self.visual_anchor = Some(anchor);
        self.caret = src;
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            self.mode = if self.config.editor == EditorKind::Helix {
                Mode::Select
            } else {
                Mode::Visual
            };
        }
        self.snap_visual_sel();
        self.refresh(window, cx);
    }

    fn commit_edit(&mut self, source: String, caret: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.source = source;
        self.caret = caret.min(self.source.len());
        self.sel = None;
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh(window, cx);
        self.sync_title(window);
    }

    fn slash_query(&self) -> Option<String> {
        let p = self.proj();
        let d = p.to_display(self.caret);
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
        let line = logical_line_range(&self.source, self.caret);
        self.source = splice(&self.source, line.clone(), item.template);
        self.caret = line.start + item.template.len();
        self.slash_index = 0;
        self.last_slash_query.clear();
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh_raw(window, cx);
    }

    fn close_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = logical_line_range(&self.source, self.caret);
        self.source = splice(&self.source, line.clone(), "");
        self.caret = line.start;
        self.slash_index = 0;
        self.last_slash_query.clear();
        self.dirty = true;
        self.status = "unsaved".into();
        self.refresh_raw(window, cx);
    }

    fn leave_insert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_notion() {
            self.scroll_caret_into_view();
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
            Caret::End => self.caret = self.source.len(),
            Caret::Offset(n) => self.caret = n.min(self.source.len()),
        }
        self.refresh(window, cx);
    }

    fn apply_buffer_motion(
        &mut self,
        motion: Motion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_modal_nav() && !(self.is_notion() && matches!(motion, Motion::Down | Motion::Up)) {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let count = take_count(&mut self.pending_count);
        self.pending_g = false;
        let p = self.proj();
        let d = p.to_display(self.caret);
        let wrap = self.wrap_cols();
        let next_d = apply_motion(&p.display, d, motion, count, wrap);
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
        self.caret = p.to_source(next_d, self.affinity);
        self.land(self.caret, window, cx);
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
        self.land(pos, window, cx);
    }

    fn click_range(&mut self, range: Range<usize>, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
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
        let at = if range.start >= anchor { range.end } else { range.start };
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
        if self.pending_replace.is_some() || self.pending_find.is_some() || self.pending_bracket.is_some() {
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

    fn on_insert_after(&mut self, _: &InsertAfterCaret, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let off = {
            let p = self.proj();
            let d = p.to_display(self.caret);
            let d2 = after_caret(&p.display, d);
            p.to_source(d2, Affinity::Inside)
        };
        self.enter_insert(Caret::Offset(off), window, cx);
    }

    fn on_insert_line_start(&mut self, _: &InsertLineStart, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            return;
        }
        let p = self.proj();
        let d = p.to_display(self.caret);
        let range = logical_line_range(&p.display, d);
        let off = first_non_blank_in(&p.display, range);
        let off = p.to_source(off, Affinity::Inside);
        self.enter_insert(Caret::Offset(off), window, cx);
    }

    fn on_insert_line_end(&mut self, _: &InsertLineEnd, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            return;
        }
        let p = self.proj();
        let d = p.to_display(self.caret);
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
        let (next, caret) = if above {
            open_line_above(&self.source, self.caret)
        } else {
            open_line_below(&self.source, self.caret)
        };
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
            let sel = self.sel.clone().unwrap_or_else(|| visual_line_range(&self.source, self.caret));
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
            let sel = self.sel.clone().unwrap_or_else(|| visual_line_range(&self.source, self.caret));
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
    fn on_word_forward_ws(&mut self, _: &WordForwardWs, window: &mut Window, cx: &mut Context<Self>) {
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
    fn on_line_first_non_blank(&mut self, _: &LineFirstNonBlank, window: &mut Window, cx: &mut Context<Self>) {
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
    fn on_digit0(&mut self, _: &Digit0, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(0, window, cx); }
    fn on_digit1(&mut self, _: &Digit1, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(1, window, cx); }
    fn on_digit2(&mut self, _: &Digit2, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(2, window, cx); }
    fn on_digit3(&mut self, _: &Digit3, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(3, window, cx); }
    fn on_digit4(&mut self, _: &Digit4, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(4, window, cx); }
    fn on_digit5(&mut self, _: &Digit5, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(5, window, cx); }
    fn on_digit6(&mut self, _: &Digit6, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(6, window, cx); }
    fn on_digit7(&mut self, _: &Digit7, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(7, window, cx); }
    fn on_digit8(&mut self, _: &Digit8, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(8, window, cx); }
    fn on_digit9(&mut self, _: &Digit9, window: &mut Window, cx: &mut Context<Self>) { self.on_digit(9, window, cx); }

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
            let sel = self.sel.clone().unwrap_or_else(|| visual_line_range(&self.source, self.caret));
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
            let sel = self.sel.clone().unwrap_or_else(|| visual_line_range(&self.source, self.caret));
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
        if !self.is_modal_nav() {
            return;
        }
        let current = self.snapshot();
        if let Some(prev) = self.undo.undo(current) {
            self.apply_snapshot(prev, window, cx);
        }
    }
    fn on_redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            return;
        }
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
            let p = self.proj();
            let d0 = p.to_display(sel.start);
            let d1 = p.to_display(sel.end);
            let (next, caret) = wysiwyg::delete_display_range(&self.source, d0..d1);
            self.commit_edit(next, caret, window, cx);
            return;
        }
        if let Some((next, caret)) = wysiwyg::backspace(&self.source, self.caret, self.affinity) {
            self.push_doc_undo();
            self.commit_edit(next, caret, window, cx);
            return;
        }
        let p = self.proj();
        let d = p.to_display(self.caret);
        if d == 0 {
            return;
        }
        self.push_doc_undo();
        let prev = p.display[..d].chars().next_back().map(|c| d - c.len_utf8()).unwrap_or(0);
        let (next, caret) = wysiwyg::delete_display_range(&self.source, prev..d);
        self.commit_edit(next, caret, window, cx);
    }

    fn on_open_search(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search(false, window, cx);
    }
    fn on_search_back(&mut self, _: &SearchBack, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search(true, window, cx);
    }
    fn open_search(&mut self, backward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
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
        let Some((query, backward)) = self.search.take() else { return };
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
        let Some((query, _)) = self.last_search.clone() else { return };
        let p = self.proj();
        let from = p.to_display(self.caret);
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
            self.caret = p.to_source(range.start, Affinity::Inside);
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
        let Some((kind, count)) = self.pending_find.take() else { return };
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
        let Some(dir) = self.pending_bracket.take() else { return };
        let count = take_count(&mut self.pending_count);
        let p = self.proj();
        let d = p.to_display(self.caret);
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

    fn set_font_family(&mut self, family: String, cx: &mut Context<Self>) {
        self.config.font_family = family;
        self.persist_config(cx);
        cx.notify();
    }
    fn set_font_size(&mut self, size: u32, cx: &mut Context<Self>) {
        self.config.font_size = size.clamp(13, 20);
        self.persist_config(cx);
        cx.notify();
    }
    fn on_toggle_settings(&mut self, _: &ToggleSettings, window: &mut Window, cx: &mut Context<Self>) {
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
        let Some(input) = self.command.take() else { return };
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
        range: Range<usize>,
        line_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = range.start.min(self.source.len());
        let end = range.end.min(self.source.len()).max(start);
        let slice = &self.source[start..end];
        let Some(new_src) = toggle_task_line(slice, line_ix) else {
            return;
        };
        if !self.mode.is_insert() {
            self.push_doc_undo();
        }
        self.source = splice(&self.source, start..end, &new_src);
        self.dirty = true;
        self.status = "unsaved".into();
        self.sync_title(window);
        self.refresh_raw(window, cx);
    }

    fn insert_image_line(&mut self, filename: &str, window: &mut Window, cx: &mut Context<Self>) {
        let alt = Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image");
        let line = images::gfm_image(alt, filename);
        self.push_doc_undo();
        let at = self.caret.min(self.source.len());
        let insert = if at > 0 && !self.source[..at].ends_with('\n') {
            format!("\n{line}\n")
        } else {
            format!("{line}\n")
        };
        self.source = splice(&self.source, at..at, &insert);
        self.caret = at + insert.len();
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
            let ClipboardEntry::Image(image) = entry else { continue };
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

        if key == "v" && (mods.platform || mods.control) && !mods.alt && !mods.shift {
            if self.try_paste_image(window, cx) {
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
            .mt_1()
            .py_1()
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.background_panel)
            .shadow_sm()
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
                        cx.listener(move |_, _, _, cx| {
                            cx.open_url(&url);
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
                        el.child(
                            div()
                                .w(px(7.))
                                .h(px(7.))
                                .rounded_full()
                                .bg(p.primary),
                        )
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
                let family = self.config.font_family.clone();
                let size = self.config.font_size;
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Font"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(p.text_muted)
                            .child("Monospace family and size for the raw GFM editor. Rendered blocks keep the UI font."),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .children(FONT_FAMILIES.iter().enumerate().map(|(i, name)| {
                                let selected = family == *name;
                                let name_owned = (*name).to_string();
                                Radio::new(("font-fam", i))
                                    .label((*name).to_string())
                                    .checked(selected)
                                    .on_click(cx.listener(move |this, checked, _, cx| {
                                        if *checked {
                                            this.set_font_family(name_owned.clone(), cx);
                                        }
                                    }))
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .children(FONT_SIZES.iter().enumerate().map(|(i, sz)| {
                                let selected = size == *sz;
                                let sz = *sz;
                                Radio::new(("font-sz", i))
                                    .label(sz.to_string())
                                    .checked(selected)
                                    .on_click(cx.listener(move |this, checked, _, cx| {
                                        if *checked {
                                            this.set_font_size(sz, cx);
                                        }
                                    }))
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
                    .w(px(640.))
                    .h(px(520.))
                    .max_w_full()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.background_panel)
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        v_flex()
                            .w(px(168.))
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
                            .h_full()
                            .px_5()
                            .py_4()
                            .child(
                                h_flex().w_full().justify_end().child(
                                    Button::new("settings-done")
                                        .ghost()
                                        .xsmall()
                                        .label("Done")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_settings(window, cx);
                                        })),
                                ),
                            )
                            .child(content),
                    ),
            )
            .into_any_element()
    }

    fn render_edit(
        &mut self,
        display: std::ops::Range<usize>,
        text: &str,
        heading: bool,
        placeholder: Option<&str>,
        wrap: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.proj();
        let pal = self.palette.clone();
        let d_caret = p.to_display(self.caret);
        let d_sel = self.sel.as_ref().map(|s| {
            p.to_display(s.start.min(s.end))..p.to_display(s.end.max(s.start))
        });
        let d_marked = self.marked.clone();
        let local_caret = if d_caret >= display.start && d_caret <= display.end {
            Some(d_caret - display.start)
        } else {
            None
        };
        let local_sel = surface::clip_range(d_sel, display.clone());
        let local_marked = surface::clip_range(d_marked, display.clone());
        let runs = surface::mark_runs(&p, display.clone());
        let hs = surface::highlights(
            text.len(),
            &runs,
            local_sel,
            local_marked,
            &pal,
            heading,
        );
        let ime = local_caret.is_some() && (self.mode.is_insert() || self.is_notion());
        let block_caret = self.uses_block_caret();
        let view = cx.entity();
        let focus = self.focus.clone();
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
            {
                let view = view.clone();
                move |d, shift, window, cx| {
                    view.update(cx, |this, cx| this.click_display(d, shift, window, cx));
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
        let text = p.display.get(block.display.clone()).unwrap_or("").to_string();
        let empty = text.trim().is_empty() && !block.extra.is_atomic();
        let placeholder = if empty {
            Some("Type to write, or / for blocks")
        } else {
            None
        };
        let heading = matches!(block.kind, BlockKind::Heading(_));
        let body = self.render_edit(block.display.clone(), &text, heading, placeholder, wrap && !matches!(block.kind, BlockKind::Code | BlockKind::Html), cx);
        let slash = self.slash_is_open() && p.block_at_display(p.to_display(self.caret)).map(|b| b.source == block.source).unwrap_or(false);
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
                v_flex()
                    .w_full()
                    .min_w_0()
                    .rounded(px(6.))
                    .bg(pal.background_element)
                    .px_3()
                    .py_2()
                    .gap_1()
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
                        view.update(cx, |this, cx| this.click_display(start, ev.modifiers.shift, window, cx));
                    }
                })
                .into_any_element(),
            BlockExtra::Image { alt, src } => self.render_image_hit(alt, src, block.display.start, cx),
            BlockExtra::List { items, ordered } => self.render_list(ix, items, *ordered, body, cx),
            BlockExtra::Table { .. } => self.render_table_block(ix, cx),
            BlockExtra::Heading(_) | BlockExtra::Text | BlockExtra::Html => {
                let mut el = v_flex().w_full().min_w_0().child(body);
                if slash {
                    el = el.child(self.render_slash_menu(cx));
                }
                el.into_any_element()
            }
        }
    }

    fn render_image_hit(&self, alt: &str, src: &str, display_start: usize, cx: &mut Context<Self>) -> AnyElement {
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
                    view.update(cx, |this, cx| this.click_display(display_start, ev.modifiers.shift, window, cx));
                }
            })
            .when(path.exists(), |el| {
                el.child(img(path.clone()).max_w_full().max_h(px(480.)).rounded(px(6.)))
            })
            .when(!path.exists(), |el| {
                el.child(div().text_color(pal.text_muted).child(format!("missing image: {src}")))
            })
            .when(!alt.is_empty(), |el| {
                el.child(div().text_xs().text_color(pal.text_muted).child(alt.to_string()))
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
                        menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                            entity.update(cx, |this, cx| {
                                if let Some((next, caret)) = wysiwyg::set_code_lang(&this.source, this.caret, &lang) {
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
        let block = p.blocks[ix].clone();
        let wrap = self.config.wrap_motions;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .children(items.iter().enumerate().map(|(i, item)| {
                let text = p.display.get(item.display.clone()).unwrap_or("").to_string();
                let bullet = if let Some(checked) = item.checked {
                    if checked { "☑".to_string() } else { "☐".to_string() }
                } else if ordered {
                    format!("{}.", i + 1)
                } else {
                    "•".to_string()
                };
                let indent = px((item.indent as f32) * 16.);
                let edit = self.render_edit(item.display.clone(), &text, false, None, wrap, cx);
                let src_range = block.source.clone();
                h_flex()
                    .id(("li", item.display.start))
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap_2()
                    .pl(indent)
                    .child(
                        div()
                            .w(px(18.))
                            .pt(px(2.))
                            .text_color(pal.markdown_list_item)
                            .when(item.checked.is_some(), |el| {
                                el.cursor_pointer().on_mouse_down(MouseButton::Left, {
                                    let view = cx.entity();
                                    let src_range = src_range.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.toggle_task(src_range.clone(), i, window, cx);
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
        let in_table = p.table_cell_at(p.to_display(self.caret)).is_some();
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
                        let text = p.display.get(disp.clone()).unwrap_or("").replace('\t', "").to_string();
                        let edit = self.render_edit(disp, &text, header, None, wrap, cx);
                        div()
                            .id(("td", r * 100 + c))
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .border_r_1()
                            .border_b_1()
                            .border_color(pal.border)
                            .when(header, |el| el.bg(pal.background_element).font_weight(FontWeight::SEMIBOLD))
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

    fn table_btn(&self, id: &'static str, label: &'static str, col: bool, before: bool, cx: &mut Context<Self>) -> AnyElement {
        Button::new(id)
            .ghost()
            .xsmall()
            .label(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.table_insert(col, before, window, cx);
            }))
            .into_any_element()
    }

    fn table_insert(&mut self, col: bool, before: bool, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.proj();
        let d = p.to_display(self.caret);
        let Some((block, cell)) = p.table_cell_at(d) else { return };
        let BlockExtra::Table { cells, cols, .. } = &block.extra else { return };
        let cols = *cols;
        let (mut headers, mut rows) = {
            // rebuild from display
            let mut headers = vec![String::new(); cols.max(1)];
            let mut body: Vec<Vec<String>> = Vec::new();
            for c in cells {
                let text = p.display.get(c.display.clone()).unwrap_or("").replace('\t', "").to_string();
                if c.header {
                    if c.col < headers.len() { headers[c.col] = text; }
                } else {
                    let br = c.row.saturating_sub(1);
                    while body.len() <= br { body.push(vec![String::new(); cols.max(1)]); }
                    if c.col < body[br].len() { body[br][c.col] = text; }
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
            let body_row = if cell.header { 0 } else { cell.row.saturating_sub(1) };
            let at = if before { body_row } else { body_row + 1 };
            rows.insert(at.min(rows.len()), vec![String::new(); headers.len().max(1)]);
        }
        let gfm = crate::display::serialize_table(&headers, &rows);
        self.push_doc_undo();
        let next = splice(&self.source, block.source.clone(), &gfm);
        self.commit_edit(next, block.source.start, window, cx);
    }

    fn delete_current_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.proj();
        let d = p.to_display(self.caret);
        let Some(block) = p.block_at_display(d) else { return };
        if !matches!(block.kind, BlockKind::Table) { return; }
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
        h_flex()
            .absolute()
            .top(px(8.))
            .left(px(80.))
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
            .when(self.link_open, |el| el.child(self.render_link_field(cx)))
            .into_any_element()
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
            .child(div().text_sm().text_color(p.markdown_text).child(format!("{}▌", self.link_draft)))
            .into_any_element()
    }

    fn on_insert_newline(&mut self, _: &InsertNewline, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_newline(false, window, cx);
    }
    fn on_insert_hard_break(&mut self, _: &InsertHardBreak, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_newline(true, window, cx);
    }
    fn on_indent_tab(&mut self, _: &IndentTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if let Some((next, caret)) = wysiwyg::tab(&self.source, self.caret, false) {
            self.push_doc_undo();
            self.commit_edit(next, caret, window, cx);
        }
    }
    fn on_outdent_tab(&mut self, _: &OutdentTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        if let Some((next, caret)) = wysiwyg::tab(&self.source, self.caret, true) {
            self.push_doc_undo();
            self.commit_edit(next, caret, window, cx);
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
    fn on_toggle_link(&mut self, _: &ToggleLink, window: &mut Window, cx: &mut Context<Self>) {
        if self.sel.as_ref().is_none_or(|s| s.start == s.end) && !self.link_open {
            return;
        }
        self.link_open = true;
        self.link_draft.clear();
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn render_surface(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.hits.clear();
        let p = self.proj();
        let n = p.blocks.len();
        let mut kids = Vec::new();
        for i in 0..n {
            kids.push(
                div()
                    .id(("blk", i))
                    .w(px(COLUMN_PX))
                    .max_w_full()
                    .min_w_0()
                    .mx_auto()
                    .px_8()
                    .py_1()
                    .child(self.render_block(i, cx))
                    .into_any_element(),
            );
        }
        kids.push(self.render_bubble(cx));
        kids
    }

    fn toggle_mark_action(&mut self, mark: Mark, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(sel) = self.sel.clone().filter(|s| s.start != s.end) {
            if let Some((next, range)) = wysiwyg::toggle_mark(&self.source, sel, mark) {
                self.push_doc_undo();
                self.source = next;
                self.caret = range.end;
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
        let (next, caret) = wysiwyg::apply_link(&self.source, sel, &url);
        self.commit_edit(next, caret, window, cx);
    }

    fn offset_from_utf16(text: &str, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in text.chars() {
            if utf16 >= offset { break; }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(text: &str, offset: usize) -> usize {
        let mut utf16 = 0;
        let mut utf8 = 0;
        for ch in text.chars() {
            if utf8 >= offset { break; }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
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
        *adjusted = Some(
            Self::offset_to_utf16(&p.display, start)..Self::offset_to_utf16(&p.display, end),
        );
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
        let d = p.to_display(self.caret);
        let range = if let Some(sel) = &self.sel {
            let a = p.to_display(sel.start.min(sel.end));
            let b = p.to_display(sel.end.max(sel.start));
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

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
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
            p.to_display(sel.start.min(sel.end))..p.to_display(sel.end.max(sel.start))
        } else if let Some(m) = &self.marked {
            m.clone()
        } else {
            let d = p.to_display(self.caret);
            d..d
        };
        let mut insert = text.to_string();
        if self.sticky.bold && !insert.contains("**") {
            insert = format!("**{insert}**");
        } else if self.sticky.italic && !insert.contains('*') {
            insert = format!("*{insert}*");
        } else if self.sticky.strike {
            insert = format!("~~{insert}~~");
        } else if self.sticky.code {
            insert = format!("`{insert}`");
        }
        let src_range = p.display_range_to_source(display_range, self.affinity);
        if self.insert_origin.is_none() {
            self.insert_origin = Some(self.snapshot());
        }
        let next = splice(&self.source, src_range.clone(), &insert);
        let caret = src_range.start + insert.len();
        self.source = next;
        self.caret = caret.min(self.source.len());
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
        let d = p.to_display(self.caret);
        let start = d.saturating_sub(new_text.len());
        self.marked = if new_text.is_empty() {
            None
        } else {
            Some(start..d)
        };
        if let Some(sel) = new_selected_range {
            let a = start + Self::offset_from_utf16(new_text, sel.start);
            let b = start + Self::offset_from_utf16(new_text, sel.end);
            self.caret = p.to_source(b.min(p.display.len()), self.affinity);
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
            let local = start.saturating_sub(hit.display_start);
            if start >= hit.display_start && start <= hit.display_start + hit.layout.len() {
                let pos = hit.layout.position_for_index(local)?;
                let h = hit.layout.line_height();
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
            .on_action(cx.listener(Self::on_toggle_link))
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
            .can_drop(|v, _, _| v.downcast_ref::<ExternalPaths>().is_some())
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.import_paths(paths.paths(), window, cx);
            }))
            .size_full()
            .bg(p.background)
            .text_color(p.markdown_text)
            .child(self.render_titlebar(cx))
            .child(
                v_flex()
                    .id("surface")
                    .flex_1()
                    .w_full()
                    .min_w_0()
                    .overflow_y_scroll()
                    .overflow_x_hidden()
                    .track_scroll(&self.scroll_handle)
                    .py_10()
                    .gap_1()
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
                    .when(command.is_none() && search.is_none(), |el| {
                        el.child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if insert { p.primary } else { p.text_muted })
                                .child(mode_label),
                        )
                        .child(div().text_xs().text_color(p.text_muted).child(format!(
                            "{}{}",
                            file_name,
                            if dirty { " · unsaved" } else { "" }
                        )))
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

#[allow(dead_code)]
fn _rgb_anchor() -> gpui::Rgba {
    let _ = point(px(0.), px(0.));
    rgb(0x000000)
}
