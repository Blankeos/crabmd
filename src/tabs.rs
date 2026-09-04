//! Multi-buffer tab shell: one `Workspace` entity per tab, Zed-style.
//!
//! Wrapping instead of splitting keeps every per-file state (undo, caret,
//! scroll, mode) intact per tab, and GPUI swaps the visible entity — fast.

use std::path::PathBuf;

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
    [NextTab, PrevTab, CloseTab, NewTab, NewWindow, CloseWindow]
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
    ]);
}

pub struct WorkspaceShell {
    tabs: Vec<Entity<Workspace>>,
    active: usize,
    palette: Palette,
    config: Config,
    titlebar_moving: bool,
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
                    .px_3()
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
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_new_window))
            .on_action(cx.listener(Self::on_close_window))
            .child(self.render_tab_bar(cx))
            .child(active)
    }
}
