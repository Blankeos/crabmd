//! Little toast library: short-lived bottom-right notices (e.g.
//! "File not found: notes/missing.md").
//!
//! Usage: keep a [`Toasts`] in your view state, call
//! [`Toasts::push_info`] / [`Toasts::push_error`] (or `push` with an
//! explicit [`ToastKind`]), then `.child(...)` the result of
//! [`Toasts::render`] in a relative/deferred overlay. Call [`Toasts::prune`]
//! on a timer to drop expired toasts — the helper [`spawn_expiry`] does the
//! `background_executor().timer(...)` + `notify` dance for a GPUI entity.
//!
//! At most [`Toasts::MAX`] toasts are kept; the oldest is evicted first.
//! Each toast lives [`Toasts::TTL`] before it expires.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::FluentBuilder as _, px, AnyElement, Context, Entity, IntoElement,
    ParentElement as _, Styled as _, Window,
};

use crate::theme::Palette;

/// Toast severity — controls the accent (border/dot) color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

/// One toast notice.
#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub kind: ToastKind,
    pub created: Instant,
}

/// Short-lived notice queue. See the module docs for usage.
#[derive(Clone, Debug, Default)]
pub struct Toasts {
    items: VecDeque<Toast>,
    next_id: u64,
}

impl Toasts {
    /// Max visible toasts; older ones are evicted first.
    pub const MAX: usize = 3;
    /// How long a toast stays visible.
    pub const TTL: Duration = Duration::from_millis(3500);

    pub fn push(&mut self, kind: ToastKind, message: impl Into<String>) {
        let message = message.into();
        if message.is_empty() {
            return;
        }
        // Refresh instead of stacking identical messages.
        if let Some(existing) = self.items.iter_mut().find(|t| t.message == message) {
            existing.created = Instant::now();
            existing.kind = kind;
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push_back(Toast {
            id,
            message,
            kind,
            created: Instant::now(),
        });
        while self.items.len() > Self::MAX {
            self.items.pop_front();
        }
    }

    pub fn push_info(&mut self, message: impl Into<String>) {
        self.push(ToastKind::Info, message);
    }

    pub fn push_error(&mut self, message: impl Into<String>) {
        self.push(ToastKind::Error, message);
    }

    /// Drop toasts older than [`Toasts::TTL`]. Returns true when anything
    /// was removed (caller should `cx.notify()` then).
    pub fn prune(&mut self) -> bool {
        let before = self.items.len();
        while let Some(front) = self.items.front() {
            if front.created.elapsed() < Self::TTL {
                break;
            }
            self.items.pop_front();
        }
        self.items.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Render the stack. Empty queue renders an empty (zero-cost) element.
    /// The stack is bottom-right aligned; embed it in a `relative` parent —
    /// or wrap in `deferred(...).with_priority(...)` to float above
    /// same-parent overlays.
    pub fn render(&self, p: &Palette) -> AnyElement {
        if self.items.is_empty() {
            return div().into_any_element();
        }
        div()
            .absolute()
            .bottom(px(44.))
            .right(px(16.))
            .flex()
            .flex_col()
            .items_end()
            .gap_2()
            .children(self.items.iter().map(|t| {
                let accent = match t.kind {
                    ToastKind::Info => p.primary,
                    ToastKind::Error => p.error,
                };
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .bg(p.background_panel)
                    .border_1()
                    .border_color(accent.opacity(0.55))
                    .text_size(px(12.))
                    .text_color(p.text)
                    .child(
                        div()
                            .w(px(7.))
                            .h(px(7.))
                            .rounded_full()
                            .bg(accent),
                    )
                    .child(t.message.clone())
            }))
            .into_any_element()
    }
}

/// Spawn a background timer that prunes expired toasts on `view` after
/// [`Toasts::TTL`] plus a small grace. Call right after pushing a toast —
/// the view re-renders without the toast once it expires, no polling.
pub fn spawn_expiry<T: 'static>(
    view: Entity<T>,
    cx: &mut Context<T>,
    update: impl FnOnce(&mut T) -> bool + 'static,
) {
    let ttl = Toasts::TTL + Duration::from_millis(150);
    cx.spawn(async move |_, cx| {
        cx.background_executor().timer(ttl).await;
        let _ = cx.update(|cx| {
            view.update(cx, |this, cx| {
                if update(this) {
                    cx.notify();
                }
            });
            let _ = cx;
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_at_max_evicting_oldest() {
        let mut q = Toasts::default();
        for i in 0..(Toasts::MAX + 2) {
            q.push_info(format!("toast {i}"));
        }
        assert_eq!(q.len(), Toasts::MAX);
        let msgs: Vec<_> = q.items.iter().map(|t| t.message.clone()).collect();
        assert!(!msgs.iter().any(|m| m == "toast 0"));
        assert!(msgs.iter().any(|m| *m == format!("toast {}", Toasts::MAX + 1)));
    }

    #[test]
    fn duplicate_message_refreshes_instead_of_stacking() {
        let mut q = Toasts::default();
        q.push_error("missing.md");
        q.push_error("missing.md");
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn prune_drops_expired() {
        let mut q = Toasts::default();
        q.push_info("old");
        q.items[0].created = Instant::now() - Toasts::TTL - Duration::from_secs(1);
        q.push_info("fresh");
        assert!(q.prune());
        assert_eq!(q.len(), 1);
        assert_eq!(q.items[0].message, "fresh");
        assert!(!q.prune());
    }

    #[test]
    fn empty_message_ignored() {
        let mut q = Toasts::default();
        q.push_info("");
        assert!(q.is_empty());
    }
}
