//! WYSIWYG text paint: StyledText + caret overlay. Click maps through TextLayout.

use std::ops::Range;

use gpui::{
    fill, px, size, App, Bounds, Element, ElementId, Entity, EntityInputHandler, FocusHandle,
    GlobalElementId, HighlightStyle, Hsla, InspectorElementId, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, Point,
    Styled, StyledText, TextLayout, Window, div,
};
use gpui::prelude::FluentBuilder as _;

use crate::display::{Marks, Projection};
use crate::theme::Palette;

#[derive(Clone)]
pub struct Hit {
    pub display_start: usize,
    pub layout: TextLayout,
}

pub fn highlights(
    text_len: usize,
    marks: &[(Range<usize>, Marks)],
    sel: Option<Range<usize>>,
    marked: Option<Range<usize>>,
    p: &Palette,
    heading: bool,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut out = Vec::new();
    if heading && text_len > 0 {
        out.push((
            0..text_len,
            HighlightStyle {
                font_weight: Some(gpui::FontWeight::SEMIBOLD),
                color: Some(p.markdown_heading),
                ..Default::default()
            },
        ));
    }
    for (range, m) in marks {
        let mut h = HighlightStyle::default();
        if m.bold {
            h.font_weight = Some(gpui::FontWeight::BOLD);
        }
        if m.italic {
            h.font_style = Some(gpui::FontStyle::Italic);
        }
        if m.strike {
            h.strikethrough = Some(gpui::StrikethroughStyle {
                thickness: px(1.),
                color: Some(p.markdown_text),
            });
        }
        if m.code {
            h.color = Some(p.markdown_code);
            h.background_color = Some(p.background_element);
        }
        if m.link.is_some() {
            h.color = Some(p.markdown_link);
            h.underline = Some(gpui::UnderlineStyle {
                thickness: px(1.),
                color: Some(p.markdown_link),
                wavy: false,
            });
        }
        if range.start < range.end {
            out.push((range.clone(), h));
        }
    }
    if let Some(sel) = sel.filter(|s| s.start != s.end) {
        let a = sel.start.min(sel.end).min(text_len);
        let b = sel.start.max(sel.end).min(text_len);
        if a < b {
            out.push((
                a..b,
                HighlightStyle {
                    background_color: Some(p.primary.opacity(0.28)),
                    ..Default::default()
                },
            ));
        }
    }
    if let Some(m) = marked.filter(|s| s.start != s.end) {
        let a = m.start.min(text_len);
        let b = m.end.min(text_len);
        if a < b {
            out.push((
                a..b,
                HighlightStyle {
                    underline: Some(gpui::UnderlineStyle {
                        thickness: px(1.),
                        color: Some(p.markdown_text),
                        wavy: false,
                    }),
                    ..Default::default()
                },
            ));
        }
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}

pub fn mark_runs(proj: &Projection, display: Range<usize>) -> Vec<(Range<usize>, Marks)> {
    let mut out = Vec::new();
    for seg in &proj.segments {
        if seg.display.end <= display.start || seg.display.start >= display.end {
            continue;
        }
        let start = seg.display.start.max(display.start) - display.start;
        let end = seg.display.end.min(display.end) - display.start;
        if start < end && seg.marks.any() {
            out.push((start..end, seg.marks));
        }
    }
    out
}

pub fn clip_range(global: Option<Range<usize>>, local: Range<usize>) -> Option<Range<usize>> {
    let g = global?;
    let a = g.start.max(local.start);
    let b = g.end.min(local.end);
    if a >= b {
        return None;
    }
    Some(a - local.start..b - local.start)
}

pub struct CaretLayer<V: EntityInputHandler> {
    pub layout: TextLayout,
    pub local_caret: Option<usize>,
    pub block: bool,
    pub color: Hsla,
    pub view: Entity<V>,
    pub focus: FocusHandle,
    pub ime: bool,
}

impl<V: EntityInputHandler> IntoElement for CaretLayer<V> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl<V: EntityInputHandler> Element for CaretLayer<V> {
    type RequestLayoutState = ();
    type PrepaintState = Option<gpui::PaintQuad>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        style.position = gpui::Position::Absolute;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let Some(local) = self.local_caret else {
            return None;
        };
        if self.layout.len() == 0 && local > 0 {
            return None;
        }
        let pos = self.layout.position_for_index(local.min(self.layout.len()))?;
        let h = self.layout.line_height();
        let w = if self.block {
            self.layout
                .position_for_index((local + 1).min(self.layout.len()))
                .map(|p| (p.x - pos.x).abs())
                .unwrap_or(px(8.))
                .max(px(6.))
        } else {
            px(2.)
        };
        Some(fill(Bounds::new(pos, size(w, h)), self.color))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        caret: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.ime {
            window.handle_input(
                &self.focus,
                gpui::ElementInputHandler::new(bounds, self.view.clone()),
                cx,
            );
        }
        if let Some(quad) = caret.take() {
            window.paint_quad(quad);
        }
    }
}

pub fn edit_text<V: EntityInputHandler>(
    view: Entity<V>,
    focus: FocusHandle,
    hits: &mut Vec<Hit>,
    display_start: usize,
    text: impl Into<gpui::SharedString>,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    caret_local: Option<usize>,
    block_caret: bool,
    wrap: bool,
    ime: bool,
    p: &Palette,
    placeholder: Option<&str>,
    on_click: impl Fn(usize, bool, &mut Window, &mut App) + 'static,
    on_drag: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let text = text.into();
    let empty = text.is_empty();
    let shown: gpui::SharedString = if empty {
        placeholder.unwrap_or("").to_string().into()
    } else {
        text.clone()
    };
    let mut hs = highlights;
    if empty {
        hs.clear();
        if !shown.is_empty() {
            hs.push((
                0..shown.len(),
                HighlightStyle {
                    color: Some(p.text_muted),
                    ..Default::default()
                },
            ));
        }
    }
    let styled = StyledText::new(shown).with_highlights(hs);
    let layout = styled.layout().clone();
    hits.push(Hit {
        display_start,
        layout: layout.clone(),
    });
    let color = p.primary;
    div()
        .id(("edit", display_start))
        .relative()
        .w_full()
        .min_w_0()
        .min_h(px(8.))
        .when(wrap, |el| el.whitespace_normal())
        .when(!wrap, |el| el.whitespace_nowrap())
        .child(styled)
        .child(CaretLayer {
            layout: layout.clone(),
            local_caret: if empty { None } else { caret_local },
            block: block_caret,
            color,
            view,
            focus,
            ime,
        })
        .on_mouse_down(MouseButton::Left, {
            let layout = layout.clone();
            move |ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let idx = layout.index_for_position(ev.position).unwrap_or_else(|e| e);
                on_click(display_start + idx, ev.modifiers.shift, window, cx);
            }
        })
        .on_mouse_move({
            let layout = layout.clone();
            move |ev: &MouseMoveEvent, window, cx| {
                if ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let idx = layout.index_for_position(ev.position).unwrap_or_else(|e| e);
                on_drag(display_start + idx, window, cx);
            }
        })
        .into_any_element()
}

pub fn index_for_point(hits: &[Hit], point: Point<Pixels>) -> Option<usize> {
    for hit in hits {
        let bounds = hit.layout.bounds();
        if point.y < bounds.top() || point.y > bounds.bottom() {
            continue;
        }
        let idx = hit.layout.index_for_position(point).unwrap_or_else(|e| e);
        return Some(hit.display_start + idx);
    }
    hits.last().map(|h| h.display_start + h.layout.len())
}
