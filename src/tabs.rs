//! Multi-buffer tab shell: one `Workspace` entity per tab, Zed-style.
//!
//! Wrapping instead of splitting keeps every per-file state (undo, caret,
//! scroll, mode) intact per tab, and GPUI swaps the visible entity — fast.

use std::path::{Path, PathBuf};

use gpui::{
    actions, div, prelude::FluentBuilder as _, px, AnyElement, App, AppContext as _,
    BorrowAppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    KeyBinding, MouseButton, MouseDownEvent, ParentElement as _, PromptLevel, Render,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::{h_flex, v_flex};

use crate::config::Config;
use crate::editor::Workspace;
use crate::theme::Palette;

actions!(
    crabmd,
    [
        NextTab,
        PrevTab,
        CloseTab,
        ForceCloseTab,
        NewTab,
        NewWindow,
        CloseWindow,
        JumpBack,
        JumpForward
    ]
);

pub fn bind_tab_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-alt-left", PrevTab, Some("Workspace")),
        KeyBinding::new("cmd-alt-right", NextTab, Some("Workspace")),
        KeyBinding::new("ctrl-alt-left", PrevTab, Some("Workspace")),
        KeyBinding::new("ctrl-alt-right", NextTab, Some("Workspace")),
        KeyBinding::new("cmd-t", NewTab, Some("Workspace")),
        KeyBinding::new("cmd-w", CloseTab, Some("Workspace")),
        KeyBinding::new("cmd-shift-n", NewWindow, Some("Workspace")),
        KeyBinding::new("cmd-shift-w", CloseWindow, Some("Workspace")),
        KeyBinding::new("ctrl-shift-w", CloseWindow, Some("Workspace")),
        // Notion / Zed-style history: ctrl-minus back, ctrl-shift-minus forward.
        KeyBinding::new("ctrl--", JumpBack, Some("Workspace")),
        KeyBinding::new("ctrl-_", JumpForward, Some("Workspace")),
        // Helix + Vim jumplist (ctrl-i is italic only in Notion).
        KeyBinding::new("ctrl-o", JumpBack, Some("Helix")),
        KeyBinding::new("ctrl-o", JumpBack, Some("Vim")),
        KeyBinding::new("ctrl-i", JumpForward, Some("Helix")),
        KeyBinding::new("ctrl-i", JumpForward, Some("Vim")),
    ]);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JumpLoc {
    pub path: PathBuf,
    pub caret: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct JumpList {
    entries: Vec<JumpLoc>,
    index: usize,
    applying: bool,
}

impl JumpList {
    const MAX: usize = 80;

    pub(crate) fn record(&mut self, from: JumpLoc, to: JumpLoc) {
        if self.applying || from == to {
            return;
        }
        self.push(from);
        self.push(to);
    }

    fn push(&mut self, loc: JumpLoc) {
        if self.applying {
            return;
        }
        if self.entries.get(self.index) == Some(&loc) {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.index.saturating_add(1));
        }
        if self.entries.last() == Some(&loc) {
            self.index = self.entries.len() - 1;
            return;
        }
        self.entries.push(loc);
        self.index = self.entries.len() - 1;
        if self.entries.len() > Self::MAX {
            let drop = self.entries.len() - Self::MAX;
            self.entries.drain(..drop);
            self.index = self.index.saturating_sub(drop);
        }
    }

    pub(crate) fn back(&mut self) -> Option<JumpLoc> {
        if self.index == 0 || self.entries.is_empty() {
            return None;
        }
        self.index -= 1;
        self.entries.get(self.index).cloned()
    }

    pub(crate) fn forward(&mut self) -> Option<JumpLoc> {
        if self.index + 1 >= self.entries.len() {
            return None;
        }
        self.index += 1;
        self.entries.get(self.index).cloned()
    }
}

pub struct WorkspaceShell {
    tabs: Vec<Entity<Workspace>>,
    active: usize,
    palette: Palette,
    config: Config,
    titlebar_moving: bool,
    jumps: JumpList,
}

impl WorkspaceShell {
    pub fn view(
        path: PathBuf,
        source: String,
        palette: Palette,
        config: Config,
        initial: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let first = Workspace::view(
            path,
            source,
            palette.clone(),
            config.clone(),
            initial,
            &mut *window,
            cx,
        );
        let shell = cx.new(|_| Self {
            tabs: vec![first],
            active: 0,
            palette,
            config,
            titlebar_moving: false,
            jumps: JumpList::default(),
        });
        // Register for single-instance routing (`crabmd -r file` finds us).
        cx.update_global::<crate::ShellRegistry, _>(|reg, _| {
            reg.shells
                .push((shell.downgrade(), window.window_handle()));
        });
        // Single close guard per window (covers all tabs). Workspaces no
        // longer register their own hook, so exactly one prompt fires.
        let closer = shell.clone();
        window.on_window_should_close(cx, move |window, cx| {
            closer.update(cx, |this, cx| this.request_window_close(window, cx))
        });
        shell
    }

    fn untitled_path() -> PathBuf {
        let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let first = base.join("untitled.md");
        if !first.exists() {
            return first;
        }
        for i in 1..100 {
            let cand = base.join(format!("untitled-{i}.md"));
            if !cand.exists() {
                return cand;
            }
        }
        first
    }

    /// Open `path` in a tab, or focus it (+ jump) if already open.
    pub fn open_tab(
        &mut self,
        path: PathBuf,
        source: String,
        initial: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|t| t.read(cx).file_path() == &path)
        {
            self.active = ix;
            if let Some((line, col)) = initial {
                self.tabs[ix].update(cx, |ws, cx| ws.jump_to(line, col, window, cx));
            }
            self.focus_active(window, cx);
            cx.notify();
            return;
        }
        let palette = self.palette.clone();
        let config = self.config.clone();
        let win = &mut *window;
        let ws = cx.new(|cx| Workspace::new(path, source, palette, config, initial, win, cx));
        self.tabs.push(ws);
        self.active = self.tabs.len() - 1;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Open a local markdown path in a new tab (or focus it if already open).
    /// Missing files toast on the active tab and never open (no "new empty
    /// buffer" for a link that points nowhere). `heading` jumps to the
    /// first matching heading after open. Records `from` on the jump list.
    pub fn open_markdown_link(
        &mut self,
        from: JumpLoc,
        path: PathBuf,
        source: String,
        heading: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !path.is_file() {
            let label = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&path.display().to_string())
                .to_string();
            if let Some(tab) = self.tabs.get(self.active).cloned() {
                tab.update(cx, |ws, cx| {
                    ws.show_toast(crate::toast::ToastKind::Error, format!("File not found: {label}"), window, cx)
                });
            }
            return;
        }
        self.open_tab(path, source, None, window, cx);
        if let Some(slug) = heading.as_deref().filter(|s| !s.is_empty()) {
            if let Some(tab) = self.tabs.get(self.active).cloned() {
                tab.update(cx, |ws, cx| ws.jump_to_heading(slug, window, cx));
            }
        }
        let to = self.tabs.get(self.active).map(|t| {
            let ws = t.read(cx);
            JumpLoc {
                path: ws.file_path().clone(),
                caret: ws.caret_offset(),
            }
        });
        if let Some(to) = to {
            self.jumps.record(from, to);
        }
    }

    pub fn record_jump(&mut self, from: JumpLoc, to: JumpLoc) {
        self.jumps.record(from, to);
    }

    fn goto_loc(&mut self, loc: JumpLoc, window: &mut Window, cx: &mut Context<Self>) {
        self.jumps.applying = true;
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|t| t.read(cx).file_path() == &loc.path)
        {
            self.active = ix;
            self.tabs[ix].update(cx, |ws, cx| ws.restore_caret(loc.caret, window, cx));
            self.focus_active(window, cx);
            cx.notify();
            self.jumps.applying = false;
            return;
        }
        let source = std::fs::read_to_string(&loc.path).unwrap_or_default();
        self.open_tab(loc.path, source, None, window, cx);
        if let Some(tab) = self.tabs.get(self.active).cloned() {
            tab.update(cx, |ws, cx| ws.restore_caret(loc.caret, window, cx));
        }
        self.jumps.applying = false;
    }

    fn jump_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(loc) = self.jumps.back() {
            self.goto_loc(loc, window, cx);
        }
    }

    fn jump_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(loc) = self.jumps.forward() {
            self.goto_loc(loc, window, cx);
        }
    }

    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.tabs[self.active].read(cx).focus_handle(cx);
        handle.focus(window, cx);
        self.tabs[self.active].read(cx).sync_title_now(window);
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.active = ix;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn step(&mut self, dir: i8, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let n = self.tabs.len();
        let next = (self.active as isize + dir as isize).rem_euclid(n as isize) as usize;
        self.activate(next, window, cx);
    }

    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab(Self::untitled_path(), String::new(), None, window, cx);
    }

    fn new_window(&mut self, cx: &mut Context<Self>) {
        let palette = self.palette.clone();
        let config = self.config.clone();
        crate::open_editor_window(Self::untitled_path(), String::new(), palette, config, None, cx);
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        if !self.tabs[ix].read(cx).is_dirty() {
            self.remove_tab(ix, window, cx);
            return;
        }
        let prompt = window.prompt(
            PromptLevel::Warning,
            "Unsaved changes",
            Some("Save this tab before closing?"),
            &["Save", "Don't Save", "Cancel"],
            cx,
        );
        let shell = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let Ok(answer) = prompt.await else {
                return;
            };
            shell
                .update_in(cx, |shell, window, cx| match answer {
                    0 => {
                        let saved = ix < shell.tabs.len()
                            && shell.tabs[ix].update(cx, |ws, cx| ws.save_now(cx));
                        if saved {
                            shell.remove_tab(ix, window, cx);
                        }
                    }
                    1 => shell.remove_tab(ix, window, cx),
                    _ => {}
                })
                .ok();
        })
        .detach();
    }

    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            window.remove_window();
            return;
        }
        self.tabs.remove(ix);
        self.active = self.active.min(self.tabs.len() - 1);
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Window × / quit guard across every dirty tab (save-all, once).
    fn request_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let dirty = self.tabs.iter().filter(|t| t.read(cx).is_dirty()).count();
        if dirty == 0 {
            return true;
        }
        let prompt = window.prompt(
            PromptLevel::Warning,
            "Unsaved changes",
            Some("Save open tabs before closing?"),
            &["Save All", "Don't Save", "Cancel"],
            cx,
        );
        let shell = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            let Ok(answer) = prompt.await else {
                return;
            };
            shell
                .update_in(cx, |shell, window, cx| match answer {
                    0 => {
                        for t in &shell.tabs {
                            t.update(cx, |ws, cx| {
                                ws.save_now(cx);
                            });
                        }
                        window.remove_window();
                    }
                    1 => window.remove_window(),
                    _ => {}
                })
                .ok();
        })
        .detach();
        false
    }

    fn on_next(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step(1, window, cx);
    }

    fn on_prev(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, window, cx);
    }

    fn on_close(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active;
        self.close_tab_at(ix, window, cx);
    }

    /// `:q!` — drop the active tab immediately, no save prompt.
    /// A lone tab closes the window directly (bypassing the
    /// should-close guard, which would otherwise re-prompt).
    fn on_force_close(&mut self, _: &ForceCloseTab, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active;
        if ix >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            // Bypass the should-close guard: this tab's changes are
            // intentionally discarded by `:q!`.
            self.tabs[ix].update(cx, |ws, cx| ws.discard_unsaved(cx));
            window.remove_window();
            return;
        }
        self.tabs.remove(ix);
        self.active = self.active.min(self.tabs.len() - 1);
        self.focus_active(window, cx);
        cx.notify();
    }

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        // `cmd-k cmd-t` must open Themes, not a new tab. `cmd-t` keymap
        // dispatch beats the editor capture handler, so consume a pending
        // `cmd-k` chord on the active tab first.
        if let Some(tab) = self.tabs.get(self.active).cloned() {
            if tab.update(cx, |ws, cx| ws.consume_chord_for_new_tab(window, cx)) {
                return;
            }
        }
        self.new_tab(window, cx);
    }

    fn on_new_window(&mut self, _: &NewWindow, _window: &mut Window, cx: &mut Context<Self>) {
        self.new_window(cx);
    }

    /// cmd-shift-w: close the current OS window. The should-close guard
    /// prompts first when any tab is dirty.
    fn on_close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.remove_window();
    }

    fn on_jump_back(&mut self, _: &JumpBack, window: &mut Window, cx: &mut Context<Self>) {
        self.jump_back(window, cx);
    }

    fn on_jump_forward(&mut self, _: &JumpForward, window: &mut Window, cx: &mut Context<Self>) {
        self.jump_forward(window, cx);
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette.clone();
        let inset = if cfg!(target_os = "macos") {
            px(72.)
        } else {
            px(8.)
        };
        h_flex()
            .id("tabbar")
            .w_full()
            .h(px(32.))
            .pl(inset)
            .pr_2()
            .gap_1()
            .items_stretch()
            .flex_shrink_0()
            .overflow_hidden()
            .font_family(self.config.ui_font.family.clone())
            .text_size(px(self.config.ui_font.size.clamp(8, 48) as f32))
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
            .on_mouse_move(cx.listener(|this, _, window, _| {
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
            .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                let title = tab.read(cx).tab_title();
                let dirty = tab.read(cx).is_dirty();
                let active = ix == self.active;
                h_flex()
                    .id(("tab", ix))
                    .h_full()
                    .pl_3()
                    .pr_2()
                    .gap_1()
                    .items_center()
                    .cursor_pointer()
                    .when(active, |el| el.bg(p.background_element))
                    .hover(|el| el.bg(p.background_element.opacity(0.6)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.activate(ix, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if active { p.markdown_text } else { p.text_muted })
                            .child(title),
                    )
                    .when(dirty, |el| {
                        el.child(div().w(px(7.)).h(px(7.)).rounded_full().bg(p.primary))
                    })
                    .child(
                        div()
                            .id(("tab-close", ix))
                            .px_1()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(p.text_muted)
                            .hover(|el| el.text_color(p.primary))
                            .child("×")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.close_tab_at(ix, window, cx);
                                }),
                            ),
                    )
            }))
            .child(
                div()
                    .id("new-tab")
                    .h_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(p.text_muted)
                    .hover(|el| el.text_color(p.primary))
                    .child("+")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.new_tab(window, cx);
                        }),
                    ),
            )
            .into_any_element()
    }
}

impl Render for WorkspaceShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let active = self.tabs[self.active].clone();
        v_flex()
            .id("shell")
            .size_full()
            .bg(p.background)
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_prev))
            .on_action(cx.listener(Self::on_close))
            .on_action(cx.listener(Self::on_force_close))
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_new_window))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_jump_back))
            .on_action(cx.listener(Self::on_jump_forward))
            .child(self.render_tab_bar(cx))
            // Flex-1 wrapper (not `size_full` on the workspace itself) so the
            // tab bar never pushes the workspace footer off the window bottom.
            .child(v_flex().flex_1().w_full().min_h_0().child(active))
    }
}

pub(crate) fn shell_for_window(window: &Window, cx: &App) -> Option<Entity<WorkspaceShell>> {
    let id = window.window_handle().window_id();
    cx.try_global::<crate::ShellRegistry>().and_then(|reg| {
        reg.shells
            .iter()
            .find(|(_, h)| h.window_id() == id)
            .and_then(|(w, _)| w.upgrade())
    })
}

pub(crate) fn resolve_local_path(base: &Path, target: &str) -> PathBuf {
    let p = Path::new(target);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    };
    canonical_or_normalized(&joined)
}

/// Canonicalize, falling back to lexical `..`/`.` cleanup when the file
/// doesn't exist yet. Shared by link opens, `same_path`, and restores.
pub(crate) fn canonical_or_normalized(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| normalize_path(path))
}

/// Same-file check that survives symlinks / `..` segments.
pub(crate) fn paths_equal(a: &Path, b: &Path) -> bool {
    match (
        std::fs::canonicalize(a),
        std::fs::canonicalize(b),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Split a link target into `(file, anchor)` when it points at a local
/// markdown file. Returns None for URLs / mailto / bare domains.
/// `#anchor` alone reports as `(empty, anchor)` so callers can do a
/// same-file heading jump.
pub(crate) fn split_link_target(raw: &str) -> Option<(String, Option<String>)> {
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
        return Some((String::new(), anchor));
    }
    let lower = file.to_ascii_lowercase();
    if !(lower.ends_with(".md")
        || lower.ends_with(".mdx")
        || lower.ends_with(".markdown")
        || lower.ends_with(".mdown"))
    {
        return None;
    }
    Some((file.to_string(), anchor))
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(path: &str, caret: usize) -> JumpLoc {
        JumpLoc {
            path: PathBuf::from(path),
            caret,
        }
    }

    #[test]
    fn jump_list_back_and_forward() {
        let mut j = JumpList::default();
        j.record(loc("a.md", 0), loc("b.md", 10));
        assert_eq!(j.back(), Some(loc("a.md", 0)));
        assert_eq!(j.forward(), Some(loc("b.md", 10)));
        assert_eq!(j.forward(), None);
    }

    #[test]
    fn jump_list_new_branch_truncates_forward() {
        let mut j = JumpList::default();
        j.record(loc("a.md", 0), loc("b.md", 1));
        j.back();
        j.record(loc("a.md", 0), loc("c.md", 2));
        assert_eq!(j.back(), Some(loc("a.md", 0)));
        assert_eq!(j.forward(), Some(loc("c.md", 2)));
        assert_eq!(j.forward(), None);
    }

    #[test]
    fn normalize_dot_segments() {
        let p = normalize_path(Path::new("/notes/./sub/../other.md"));
        assert_eq!(p, PathBuf::from("/notes/other.md"));
    }
}
