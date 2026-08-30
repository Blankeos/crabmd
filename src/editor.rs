//! One markdown buffer. Paint ranges are a reaction to `source`.
//!
//! Unfocused ranges render as GFM. The contiguous span that intersects the
//! caret or selection is one raw Editor. Motions, search, join, replace,
//! backspace, and undo operate on `source` + caret/sel only.

use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::{
    actions, div, img, point, prelude::FluentBuilder as _, px, rems, rgb, AnyElement, App,
    AppContext as _, ClipboardEntry, Context, Entity, ExternalPaths, FocusHandle,
    Focusable, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement, KeyBinding,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, InputEvent},
    menu::{DropdownMenu as _, PopupMenuItem},
    radio::Radio,
    switch::Switch,
    text::{TextView, TextViewStyle},
    v_flex, Icon, Sizable as _, Theme, ThemeMode, ThemeTokens,
};

use crate::config::{self, Config, EditorKind, FONT_FAMILIES, FONT_SIZES};
use crate::coords::backspace_join_doc;
use crate::document::{
    alert_icon_name, extract_links, has_task_line, parse_ranges, raw_span, slash_query_at,
    sole_image, split_task_line, splice, toggle_task_line, Block, BlockKind, PaintRange,
};
use crate::images;
use crate::mode::{self, Caret, ExCommand, Mode};
use crate::motion::{
    after_caret, apply_motion, block_caret_range, delete_char_at, delete_range, extend_visual_line,
    find_char, first_non_blank_in, heading_jump, join_next_lines, join_range, last_line_start,
    line_start_n, logical_line_delete_range, logical_line_range, open_line_above, open_line_below,
    paragraph_jump, push_count, replace_chars, replace_selection, search_next, search_prev,
    take_count, visual_line_range, visual_rows, FindKind, Motion,
};
use crate::notion;
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
    textarea: Entity<EditorState>,
    focus: FocusHandle,
    palette: Palette,
    dirty: bool,
    status: SharedString,
    _subscriptions: Vec<Subscription>,
    command: Option<String>,
    search: Option<(String, bool)>,
    last_search: Option<(String, bool)>,
    raw: Range<usize>,
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
        let wrap = config.wrap_motions;
        let textarea = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .line_number(false)
                .folding(false)
                .searchable(false)
                .soft_wrap(wrap)
                .submit_on_enter(true)
                .placeholder("Write, or type / for blocks")
        });

        let _subscriptions = vec![cx.subscribe_in(
            &textarea,
            window,
            |this: &mut Self, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.on_input_change(input, window, cx),
                InputEvent::PressEnter { shift, .. } if !shift => {
                    this.on_press_enter(window, cx);
                }
                _ => {}
            },
        )];

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
            textarea,
            focus: cx.focus_handle(),
            palette,
            dirty: false,
            status: "ready".into(),
            _subscriptions,
            command: None,
            search: None,
            last_search: None,
            raw: 0..0,
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
        if self.config.wrap_motions {
            Some(80)
        } else {
            None
        }
    }

    fn font_px(&self) -> gpui::Pixels {
        px(self.config.font_size.clamp(13, 20) as f32)
    }

    fn paint_ranges(&self) -> Vec<PaintRange> {
        parse_ranges(&self.source)
    }

    fn compute_raw(&self) -> Range<usize> {
        raw_span(
            &self.paint_ranges(),
            self.source.len(),
            self.caret,
            self.sel.clone(),
        )
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

    fn notion_range(&self) -> Option<PaintRange> {
        if !self.is_notion() {
            return None;
        }
        let ranges = self.paint_ranges();
        let raw = raw_span(&ranges, self.source.len(), self.caret, self.sel.clone());
        let hits: Vec<_> = ranges
            .into_iter()
            .filter(|r| r.range.start >= raw.start && r.range.end <= raw.end && !r.range.is_empty() || r.range == raw)
            .collect();
        if hits.len() == 1 && !notion::uses_raw_exception(hits[0].kind) {
            Some(hits[0].clone())
        } else {
            None
        }
    }

    fn display_for_raw(&self) -> String {
        if let Some(range) = self.notion_range() {
            let block = Block::with_kind(range.kind, range.slice(&self.source));
            return notion::edit_text(&block);
        }
        let start = self.raw.start.min(self.source.len());
        let end = self.raw.end.min(self.source.len()).max(start);
        self.source[start..end].to_string()
    }

    fn refresh_raw(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clamp_caret();
        if self.mode.extends_selection() {
            self.snap_visual_sel();
        }
        self.raw = self.compute_raw();
        self.load_textarea(window, cx);
        self.scroll_caret_into_view();
        cx.notify();
    }

    fn snap_visual_sel(&mut self) {
        let Some(anchor) = self.visual_anchor else {
            return;
        };
        if self.mode == Mode::VisualLine {
            let a = anchor.min(self.caret);
            let b = anchor.max(self.caret);
            let start = logical_line_range(&self.source, a).start;
            let mut end = logical_line_range(&self.source, b).end;
            if end < self.source.len() && self.source.as_bytes()[end] == b'\n' {
                end += 1;
            }
            self.sel = Some(start..end);
        } else {
            let a = anchor.min(self.caret);
            let b = anchor.max(self.caret);
            self.sel = if a == b { None } else { Some(a..b) };
        }
    }

    fn load_textarea(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        let source = self.display_for_raw();
        let wrap = self.config.wrap_motions;
        let block_caret = self.uses_block_caret();
        let origin = self.raw.start;
        let local = self.caret.saturating_sub(origin).min(source.len());
        let sel_local = self.sel.as_ref().map(|s| {
            s.start.saturating_sub(origin).min(source.len())
                ..s.end.saturating_sub(origin).min(source.len())
        });
        self.textarea.update(cx, |state, cx| {
            state.set_soft_wrap(wrap, window, cx);
            state.set_value(source.clone(), window, cx);
            let len = state.value().len();
            let range = if let Some(sel) = sel_local {
                sel.start.min(len)..sel.end.min(len)
            } else if block_caret {
                block_caret_range(&source, local.min(len))
            } else {
                let pos = local.min(len);
                pos..pos
            };
            state.set_selected_range(range, cx);
            state.focus(window, cx);
        });
        self.loading = false;
    }

    fn sync_from_textarea(&mut self, cx: &App) {
        if self.loading {
            return;
        }
        let value = self.textarea.read(cx).value().to_string();
        let local = self.textarea.read(cx).selected_range();
        if let Some(range) = self.notion_range() {
            let block = Block::with_kind(range.kind, range.slice(&self.source));
            let gfm = notion::commit(&block, &value);
            let start = range.range.start;
            self.source = splice(&self.source, range.range.clone(), &gfm);
            self.raw = start..start + gfm.len();
            self.caret = (start + local.start).min(self.source.len());
            return;
        }
        let start = self.raw.start.min(self.source.len());
        let end = self.raw.end.min(self.source.len()).max(start);
        self.source = splice(&self.source, start..end, &value);
        self.raw = start..start + value.len();
        self.caret = (start + local.start).min(self.source.len());
        if local.start != local.end {
            self.sel = Some(start + local.start..start + local.end);
        } else if !self.mode.extends_selection() {
            self.sel = None;
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

    fn textarea_source(&self, cx: &App) -> String {
        self.textarea.read(cx).value().to_string()
    }

    fn caret_offset_local(&self, cx: &App) -> usize {
        self.textarea.read(cx).selected_range().start
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

    fn on_input_change(
        &mut self,
        _input: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loading {
            return;
        }
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            self.sync_editor_sel(cx);
            return;
        }
        self.sync_from_textarea(cx);
        let q = slash_query_at(&self.source, self.caret)
            .unwrap_or("")
            .to_string();
        if q != self.last_slash_query {
            self.last_slash_query = q;
            self.slash_index = 0;
        }
        self.dirty = true;
        self.status = "unsaved".into();
        self.sync_title(window);
        cx.notify();
    }

    fn sync_editor_sel(&mut self, cx: &App) {
        let local = self.textarea.read(cx).selected_range();
        let start = self.raw.start + local.start;
        let end = self.raw.start + local.end;
        self.caret = end.min(self.source.len());
        if local.start != local.end {
            self.sel = Some(start.min(self.source.len())..end.min(self.source.len()));
        }
    }

    fn on_press_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.editor.is_modal() && !self.mode.is_insert() {
            return;
        }
        if let Some(item) = self.current_slash_pick() {
            self.apply_slash(item, window, cx);
            return;
        }
        self.textarea.update(cx, |state, cx| {
            state.insert("\n", window, cx);
        });
    }

    fn slash_items(&self) -> Vec<&'static SlashItem> {
        let q = slash_query_at(&self.source, self.caret).unwrap_or("");
        slash::filter(q)
    }

    fn slash_is_open(&self) -> bool {
        self.mode.is_insert()
            && slash_query_at(&self.source, self.caret).is_some()
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
            self.sync_from_textarea(cx);
            self.scroll_caret_into_view();
            cx.notify();
            return;
        }
        if self.mode.is_visual() {
            self.mode = Mode::Normal;
            self.sel = None;
            self.visual_anchor = None;
            self.refresh_raw(window, cx);
            return;
        }
        if !self.mode.is_insert() {
            return;
        }
        self.sync_from_textarea(cx);
        self.finish_insert_undo();
        self.mode = Mode::Normal;
        self.clear_pending();
        self.visual_anchor = None;
        self.sel = None;
        self.refresh_raw(window, cx);
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
            Caret::Start => self.caret = self.raw.start,
            Caret::End => self.caret = self.raw.end.min(self.source.len()),
            Caret::Offset(n) => {
                if n <= self.source.len() {
                    self.caret = n;
                } else {
                    self.caret = (self.raw.start + n).min(self.source.len());
                }
            }
        }
        self.refresh_raw(window, cx);
    }

    fn apply_buffer_motion(
        &mut self,
        motion: Motion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_notion() && matches!(motion, Motion::Down | Motion::Up) {
            self.pending_count = None;
            let next = apply_motion(&self.source, self.caret, motion, 1, None);
            self.land(next, window, cx);
            return;
        }
        if !self.is_modal_nav() {
            cx.propagate();
            return;
        }
        window.prevent_default();
        let count = take_count(&mut self.pending_count);
        self.pending_g = false;
        let wrap = self.wrap_cols();
        let next = apply_motion(&self.source, self.caret, motion, count, wrap);
        self.land(next, window, cx);
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
        let off = after_caret(&self.source, self.caret);
        self.enter_insert(Caret::Offset(off), window, cx);
    }

    fn on_insert_line_start(&mut self, _: &InsertLineStart, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            return;
        }
        let range = logical_line_range(&self.source, self.caret);
        let off = first_non_blank_in(&self.source, range);
        self.enter_insert(Caret::Offset(off), window, cx);
    }

    fn on_insert_line_end(&mut self, _: &InsertLineEnd, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_modal_nav() {
            return;
        }
        let end = logical_line_range(&self.source, self.caret).end;
        self.enter_insert(Caret::Offset(end), window, cx);
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
        let range = self.textarea.read(cx).selected_range();
        if range.start != range.end {
            cx.propagate();
            return;
        }
        self.sync_from_textarea(cx);
        if let Some((next, caret)) = backspace_join_doc(&self.source, self.caret) {
            window.prevent_default();
            self.apply_source(next, caret, window, cx);
            return;
        }
        cx.propagate();
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
        self.textarea.focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    fn submit_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((query, backward)) = self.search.take() else { return };
        self.textarea.focus_handle(cx).focus(window, cx);
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
        let from = self.caret;
        let found = if forward {
            let start = if from < self.source.len() { from + 1 } else { 0 };
            search_next(&self.source, start, &query, wrap)
                .or_else(|| search_next(&self.source, 0, &query, false))
        } else {
            search_prev(&self.source, from, &query, wrap)
        };
        if let Some(range) = found {
            self.mode = Mode::Normal;
            self.visual_anchor = None;
            self.sel = None;
            self.caret = range.start;
            self.refresh_raw(window, cx);
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
        let next = match key {
            "p" => paragraph_jump(&self.source, self.caret, dir, count),
            "h" => heading_jump(&self.source, self.caret, dir, count),
            _ => {
                self.pending_count = None;
                cx.notify();
                return;
            }
        };
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
            self.textarea.focus_handle(cx).focus(window, cx);
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
    fn set_wrap_motions(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.config.wrap_motions = wrap;
        self.persist_config(cx);
        self.textarea.update(cx, |state, cx| {
            state.set_soft_wrap(wrap, window, cx);
        });
        cx.notify();
    }
    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.write_to_disk(cx) {
            self.status = "saved".into();
        }
        self.sync_title(window);
    }
    fn write_to_disk(&mut self, cx: &mut Context<Self>) -> bool {
        self.sync_from_textarea(cx);
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
        self.textarea.focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    fn submit_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.command.take() else { return };
        self.textarea.focus_handle(cx).focus(window, cx);
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
        if self.mode.is_insert() || self.is_notion() {
            self.textarea.update(cx, |state, cx| {
                let current = state.value().to_string();
                if !current.is_empty() && !current.ends_with('\n') {
                    state.insert("\n", window, cx);
                }
                state.insert(&line, window, cx);
            });
        } else {
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
            self.refresh_raw(window, cx);
        }
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

    fn text_view_style(&self) -> TextViewStyle {
        let mut style = TextViewStyle::default()
            .paragraph_gap(rems(0.2))
            .heading_font_size(|level, base| match level {
                1 => base * 2.05,
                2 => base * 1.6,
                3 => base * 1.3,
                4 => base * 1.12,
                _ => base,
            })
            .inline_code(HighlightStyle {
                color: Some(self.palette.markdown_code),
                background_color: Some(self.palette.background_element),
                ..Default::default()
            });
        style.heading_base_font_size = px(16.);
        style.is_dark = self.palette.appearance == Appearance::Dark;
        style
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
                    this.textarea.focus_handle(cx).focus(window, cx);
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

    fn raw_editor_height(&self, cx: &App) -> gpui::Pixels {
        let value = self.textarea.read(cx).value().to_string();
        let n = visual_rows(&value, self.wrap_cols()).len().max(1);
        let line = self.config.font_size.clamp(13, 20) as f32 * 1.5;
        px(line * n as f32)
    }

    fn render_textarea(&self, kind: BlockKind, readonly: bool, cx: &App) -> AnyElement {
        let p = &self.palette;
        let family = self.config.font_family.clone();
        let size = self.font_px();
        let height = self.raw_editor_height(cx);
        Editor::new(&self.textarea)
            .appearance(false)
            .bordered(false)
            .readonly(readonly)
            .w_full()
            .min_w_0()
            .max_w_full()
            .h(height)
            .overflow_hidden()
            .font_family(family)
            .text_size(size)
            .when(self.is_notion() && matches!(kind, BlockKind::Heading(_)), |el| {
                el.font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.markdown_heading)
            })
            .into_any_element()
    }

    fn focused_kind(&self) -> BlockKind {
        self.notion_range()
            .map(|r| r.kind)
            .or_else(|| {
                self.paint_ranges()
                    .into_iter()
                    .find(|r| r.range.start == self.raw.start)
                    .map(|r| r.kind)
            })
            .unwrap_or(BlockKind::Paragraph)
    }

    fn render_focused(&self, cx: &mut Context<Self>) -> AnyElement {
        let kind = self.focused_kind();
        let readonly = self.config.editor.is_modal() && !self.mode.is_insert();
        if self.is_notion() && !notion::uses_raw_exception(kind) {
            return self.render_notion_focused(kind, cx);
        }
        let source_slice = if self.raw.end <= self.source.len() {
            &self.source[self.raw.start.min(self.source.len())..self.raw.end.min(self.source.len())]
        } else {
            ""
        };
        v_flex()
            .w_full()
            .min_w_0()
            .max_w_full()
            .child(self.render_textarea(kind, readonly, cx))
            .when(slash_query_at(&self.source, self.caret).is_some(), |el| {
                el.child(self.render_slash_menu(cx))
            })
            .child(self.render_link_chips(source_slice, cx))
            .into_any_element()
    }

    fn render_notion_focused(&self, kind: BlockKind, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let ta = self.render_textarea(kind, false, cx);
        match kind {
            BlockKind::Alert(akind) => {
                let color = p.alert_color(akind);
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .border_l_4()
                    .border_color(color)
                    .bg(p.background_element)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(icon_el(alert_icon_name(akind), color))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(color)
                                    .child(akind.as_str().to_string()),
                            ),
                    )
                    .child(ta)
                    .into_any_element()
            }
            BlockKind::Quote => div()
                .w_full()
                .min_w_0()
                .border_l_2()
                .border_color(p.markdown_block_quote)
                .px_3()
                .child(ta)
                .into_any_element(),
            BlockKind::Code => div()
                .w_full()
                .min_w_0()
                .rounded(px(6.))
                .bg(p.background_element)
                .px_3()
                .py_2()
                .child(ta)
                .into_any_element(),
            BlockKind::Rule => div()
                .w_full()
                .h(px(1.))
                .my_3()
                .bg(p.markdown_horizontal_rule)
                .into_any_element(),
            _ => v_flex()
                .w_full()
                .min_w_0()
                .child(ta)
                .when(slash_query_at(&self.source, self.caret).is_some(), |el| {
                    el.child(self.render_slash_menu(cx))
                })
                .into_any_element(),
        }
    }

    fn range_in_raw(&self, range: &Range<usize>) -> bool {
        if self.raw.start == self.raw.end && self.source.is_empty() {
            return true;
        }
        range.start >= self.raw.start && range.end <= self.raw.end && self.raw.start < self.raw.end
            || (range.start == self.raw.start && range.end == self.raw.end)
    }

    fn render_unfocused(&self, range: &PaintRange, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let style = self.text_view_style();
        let src = range.slice(&self.source);
        let wrap = self.config.wrap_motions;
        let id = range.range.start;

        if range.is_blank(&self.source) {
            return div()
                .w_full()
                .min_w_0()
                .h(px(8.))
                .into_any_element();
        }

        if let Some((alt, img_src)) = sole_image(src) {
            return self.render_image_block(&alt, &img_src);
        }

        if matches!(range.kind, BlockKind::List { .. }) && has_task_line(src) {
            return self.render_task_list(range, cx);
        }

        match range.kind {
            BlockKind::Alert(kind) => {
                let color = p.alert_color(kind);
                v_flex()
                    .w_full()
                    .min_w_0()
                    .max_w_full()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .border_l_4()
                    .border_color(color)
                    .bg(p.background_element)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(icon_el(alert_icon_name(kind), color))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(color)
                                    .child(kind.as_str().to_string()),
                            ),
                    )
                    .child(markdown_view(
                        ("alert-body", id),
                        alert_body(src),
                        style,
                        p,
                        wrap,
                        false,
                    ))
                    .into_any_element()
            }
            BlockKind::Rule => div()
                .w_full()
                .h(px(1.))
                .my_3()
                .bg(p.markdown_horizontal_rule)
                .into_any_element(),
            BlockKind::Heading(_) => div()
                .w_full()
                .min_w_0()
                .max_w_full()
                .text_color(p.markdown_heading)
                .font_weight(FontWeight::SEMIBOLD)
                .child(markdown_view(("md", id), src.to_string(), style, p, wrap, false))
                .into_any_element(),
            BlockKind::Quote => div()
                .w_full()
                .min_w_0()
                .max_w_full()
                .border_l_2()
                .border_color(p.markdown_block_quote)
                .px_3()
                .text_color(p.markdown_block_quote)
                .child(markdown_view(("md", id), src.to_string(), style, p, wrap, false))
                .into_any_element(),
            BlockKind::Code => div()
                .w_full()
                .min_w_0()
                .max_w_full()
                .rounded(px(6.))
                .bg(p.background_element)
                .px_3()
                .py_2()
                .text_color(p.markdown_code_block)
                .child(markdown_view(("md", id), src.to_string(), style, p, wrap, true))
                .into_any_element(),
            _ => markdown_view(("md", id), src.to_string(), style, p, wrap, false),
        }
    }

    fn render_image_block(&self, alt: &str, src: &str) -> AnyElement {
        let p = &self.palette;
        let path = images::resolve_beside(&self.path, src);
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
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
                        .text_color(p.text_muted)
                        .child(format!("missing image: {src}")),
                )
            })
            .when(!alt.is_empty(), |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(p.text_muted)
                        .child(alt.to_string()),
                )
            })
            .into_any_element()
    }

    fn render_task_list(&self, range: &PaintRange, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let src = range.slice(&self.source).to_string();
        let range_bytes = range.range.clone();
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .children(src.lines().enumerate().map(|(line_ix, line)| {
                let range_bytes = range_bytes.clone();
                if let Some((_, checked, rest)) = split_task_line(line) {
                    let icon_name = if checked { "square-check" } else { "square" };
                    let color = if checked { p.success } else { p.text_muted };
                    h_flex()
                        .id(("task", range_bytes.start * 1000 + line_ix))
                        .w_full()
                        .min_w_0()
                        .items_start()
                        .gap_2()
                        .child(
                            h_flex()
                                .id(("task-box", range_bytes.start * 1000 + line_ix))
                                .w(px(16.))
                                .h(px(20.))
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.toggle_task(range_bytes.clone(), line_ix, window, cx);
                                    }),
                                )
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(icon_el(icon_name, color)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_color(p.markdown_text)
                                .when(checked, |el| el.text_color(p.text_muted))
                                .child(rest.trim().to_string()),
                        )
                        .into_any_element()
                } else {
                    div()
                        .w_full()
                        .min_w_0()
                        .text_color(p.markdown_text)
                        .child(line.to_string())
                        .into_any_element()
                }
            }))
            .into_any_element()
    }

    fn render_surface(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let ranges = self.paint_ranges();
        let p = &self.palette;
        let editing = self.mode.is_insert() || self.is_notion();
        let mut kids = Vec::new();
        let mut i = 0usize;
        let mut painted_raw = false;
        while i < ranges.len() {
            let r = &ranges[i];
            let in_raw = self.range_in_raw(&r.range) || (self.source.is_empty() && !painted_raw);
            if in_raw {
                if !painted_raw {
                    painted_raw = true;
                    let body = self.render_focused(cx);
                    kids.push(
                        div()
                            .id("raw-span")
                            .w(px(740.))
                            .max_w_full()
                            .min_w_0()
                            .mx_auto()
                            .px_8()
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .py_1()
                                    .pl_2()
                                    .rounded(px(6.))
                                    .border_l_4()
                                    .border_color(p.primary)
                                    .when(editing, |el| el.bg(p.background_element))
                                    .when(!editing, |el| el.bg(p.background_element.opacity(0.45)))
                                    .child(body),
                            )
                            .into_any_element(),
                    );
                }
                i += 1;
                continue;
            }
            let range = r.range.clone();
            let empty = r.is_blank(&self.source);
            let body = if empty {
                div()
                    .text_color(p.text_muted)
                    .child("Type to write, or / for blocks")
                    .into_any_element()
            } else {
                self.render_unfocused(r, cx)
            };
            kids.push(
                div()
                    .id(("range", range.start))
                    .w(px(740.))
                    .max_w_full()
                    .min_w_0()
                    .mx_auto()
                    .px_8()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .max_w_full()
                            .py_1()
                            .pl_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                {
                                    let range = range.clone();
                                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.click_range(range.clone(), ev.modifiers.shift, window, cx);
                                    })
                                },
                            )
                            .on_mouse_move({
                                let range = range.clone();
                                cx.listener(move |this, ev: &MouseMoveEvent, window, cx| {
                                    if this.mouse_dragging && ev.dragging() {
                                        this.drag_to_range(range.clone(), window, cx);
                                    }
                                })
                            })
                            .child(body),
                    )
                    .into_any_element(),
            );
            i += 1;
        }
        if kids.is_empty() {
            kids.push(
                div()
                    .id("raw-span")
                    .w(px(740.))
                    .max_w_full()
                    .min_w_0()
                    .mx_auto()
                    .px_8()
                    .child(self.render_focused(cx))
                    .into_any_element(),
            );
        }
        kids
    }
}

fn icon_el(name: &str, color: gpui::Hsla) -> Icon {
    Icon::default()
        .path(crate::assets::path(name))
        .text_color(color)
        .w(px(16.))
        .h(px(16.))
}


fn markdown_view(
    id: (&'static str, usize),
    source: impl Into<SharedString>,
    style: TextViewStyle,
    p: &Palette,
    wrap: bool,
    code: bool,
) -> AnyElement {
    let view = TextView::markdown(id, source)
        .selectable(false)
        .style(style)
        .on_link_click(move |url, _ev, _window, cx| {
            cx.open_url(url);
            cx.stop_propagation();
        })
        .text_color(p.markdown_text)
        .w_full()
        .min_w_0()
        .max_w_full()
        .when(wrap && !code, |el| el.whitespace_normal())
        .when(!wrap, |el| el.whitespace_nowrap());
    let wrap_box = div()
        .id(("md-wrap", id.1 + if code { 1_000_000 } else { 0 }))
        .w_full()
        .min_w_0()
        .max_w_full();
    if code {
        wrap_box.overflow_x_scroll().child(view).into_any_element()
    } else if wrap {
        wrap_box.overflow_x_hidden().child(view).into_any_element()
    } else {
        wrap_box.child(view).into_any_element()
    }
}

fn alert_body(source: &str) -> String {
    crate::notion::strip_alert_body(source)
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
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.textarea.focus_handle(cx)
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
                        this.sync_editor_sel(cx);
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
