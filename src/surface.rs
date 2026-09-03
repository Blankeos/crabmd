//! WYSIWYG text paint: StyledText + caret overlay. Click maps through TextLayout.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, fill, px, size, App, Bounds, Corners, Element, ElementId, Entity, EntityInputHandler,
    FocusHandle, GlobalElementId, HighlightStyle, Hsla, InspectorElementId,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement as _, Pixels, Point, Styled, StyledText, TextLayout, Window,
};

use crate::display::{Marks, Projection};
use crate::theme::Palette;

#[derive(Clone)]
pub struct Hit {
    pub display_start: usize,
    /// Document display length for this hit. May be 0 for empty blocks even
    /// when `layout` contains a line-height probe character.
    pub doc_len: usize,
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
            // Color only — rounded padded pill is painted by CodePillLayer.
            h.color = Some(p.markdown_code);
        }
        if m.underline {
            h.underline = Some(gpui::UnderlineStyle {
                thickness: px(1.),
                color: Some(p.markdown_text),
                wavy: false,
            });
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
    flatten(text_len, out)
}

/// GPUI `StyledText` runs cannot overlap. Merge stacked styles (heading +
/// selection, syntax + selection, …) into sorted, disjoint ranges.
pub fn flatten(
    text_len: usize,
    items: Vec<(Range<usize>, HighlightStyle)>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut bounds = vec![0usize, text_len];
    for (r, _) in &items {
        bounds.push(r.start.min(text_len));
        bounds.push(r.end.min(text_len));
    }
    bounds.sort_unstable();
    bounds.dedup();
    let mut out: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for w in bounds.windows(2) {
        let a = w[0];
        let b = w[1];
        if a >= b {
            continue;
        }
        let mut style = HighlightStyle::default();
        let mut any = false;
        for (r, h) in &items {
            if r.start < b && r.end > a {
                let prev_bg = style.background_color;
                style = style.highlight(*h);
                // Blend stacked backgrounds (e.g. inline code + selection)
                // so the selection tint still covers code pills.
                if let (Some(base), Some(over)) = (prev_bg, h.background_color) {
                    style.background_color = Some(base.blend(over));
                }
                any = true;
            }
        }
        if any {
            if let Some(last) = out.last_mut() {
                if last.1 == style && last.0.end == a {
                    last.0.end = b;
                    continue;
                }
            }
            out.push((a..b, style));
        }
    }
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
    /// Skip the block quad (the character is already inverted via highlight).
    pub skip_block: bool,
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
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let Some(local) = self.local_caret else {
            return None;
        };
        if self.block && self.skip_block {
            return None;
        }
        let Some(len) = layout_len(&self.layout) else {
            return None;
        };
        if len == 0 && local > 0 {
            return None;
        }
        let pos = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.layout.position_for_index(local.min(len))
        }))
        .ok()
        .flatten()
        .unwrap_or(Point::new(bounds.origin.x, bounds.origin.y));
        let h =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.layout.line_height()))
                .unwrap_or(px(16.));
        let w = if self.block {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.layout.position_for_index((local + 1).min(len))
            }))
            .ok()
            .flatten()
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

/// Rounded, padded backgrounds for inline `code` spans.
pub struct CodePillLayer {
    pub layout: TextLayout,
    pub ranges: Vec<Range<usize>>,
    pub color: Hsla,
}

impl IntoElement for CodePillLayer {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for CodePillLayer {
    type RequestLayoutState = ();
    type PrepaintState = Vec<gpui::PaintQuad>;

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
        let Some(len) = layout_len(&self.layout) else {
            return Vec::new();
        };
        let line_h =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.layout.line_height()))
                .unwrap_or(px(16.));
        let pad_x = px(4.);
        let pad_y = px(1.5);
        let radius = px(4.);
        let mut quads = Vec::new();
        for range in &self.ranges {
            if range.start >= range.end || range.end > len {
                continue;
            }
            let Some(start) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.layout.position_for_index(range.start)
            }))
            .ok()
            .flatten() else {
                continue;
            };
            let end = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.layout.position_for_index(range.end.min(len))
            }))
            .ok()
            .flatten()
            .unwrap_or(start);
            // Same visual line — single padded pill. Wrapped code is rare;
            // fall back to one pill spanning the start line width.
            let (x0, x1, y) = if (start.y - end.y).abs() < px(0.5) {
                (start.x.min(end.x), start.x.max(end.x), start.y)
            } else {
                (start.x, start.x + px(48.), start.y)
            };
            let bounds = Bounds::new(
                Point::new(x0 - pad_x, y - pad_y),
                size((x1 - x0) + pad_x * 2., line_h + pad_y * 2.),
            );
            quads.push(fill(bounds, self.color).corner_radii(Corners::all(radius)));
        }
        quads
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        quads: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        for quad in quads.drain(..) {
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
    font_family: Option<gpui::SharedString>,
    font_px: Option<gpui::Pixels>,
    heading: bool,
    code_font: Option<(gpui::SharedString, Vec<Range<usize>>)>,
    on_click: impl Fn(usize, bool, bool, usize, &mut Window, &mut App) + 'static,
    on_drag: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let text = text.into();
    let empty = text.is_empty();
    let ph = if empty {
        placeholder.unwrap_or("").to_string()
    } else {
        String::new()
    };
    // Empty `StyledText` is 0-tall and an absolutely positioned placeholder
    // overflows neighbors (the heading-sized hint the user sees). Keep the
    // strut *in-flow* in the same layout the caret uses:
    // - placeholder text when focused (muted, nowrap → one line)
    // - "Ag" font strut otherwise (ascent+descent, painted transparent)
    // Document/IME stay empty (`doc_len == 0`).
    let shown: gpui::SharedString = if empty {
        if ph.is_empty() {
            gpui::SharedString::from("\u{200B}")
        } else {
            gpui::SharedString::from(ph.clone())
        }
    } else {
        text.clone()
    };
    let mut hs = highlights;
    if empty {
        hs.clear();
        hs.push((
            0..shown.len(),
            HighlightStyle {
                font_weight: heading.then_some(gpui::FontWeight::SEMIBOLD),
                color: Some(if ph.is_empty() {
                    p.primary.opacity(0.)
                } else {
                    p.text_muted
                }),
                ..Default::default()
            },
        ));
    }
    hs.retain(|(r, _)| {
        r.start < r.end
            && r.end <= shown.len()
            && shown.is_char_boundary(r.start)
            && shown.is_char_boundary(r.end)
    });
    // Vim/Helix block caret: invert the covered character (block-colored
    // background, editor-background glyph) so it reads as a true block
    // cursor instead of a solid quad hiding the char. The painted quad is
    // skipped when inversion applies; empty blocks keep the quad fallback.
    // NOTE: re-flatten after pushing — `hs` is already disjoint (overlapping
    // heading/bold/italic/code/link/syntax ranges would otherwise swallow
    // the inversion in StyledText, which is why the caret vanished on
    // formatted text). Last writer wins, so the caret colors take over.
    let mut inverted = false;
    if block_caret && !text.is_empty() {
        if let Some(local) = caret_local {
            let range = crate::motion::block_caret_range(&text, local);
            if range.start < range.end {
                hs.push((
                    range,
                    HighlightStyle {
                        color: Some(p.background),
                        background_color: Some(p.primary),
                        ..Default::default()
                    },
                ));
                hs = flatten(shown.len(), hs);
                inverted = true;
            }
        }
    }
    let mut styled = StyledText::new(shown.clone()).with_highlights(hs);
    let mut code_ranges: Vec<Range<usize>> = Vec::new();
    if let Some((fam, ranges)) = code_font {
        code_ranges = ranges
            .iter()
            .filter(|r| {
                r.start < r.end
                    && r.end <= shown.len()
                    && shown.is_char_boundary(r.start)
                    && shown.is_char_boundary(r.end)
            })
            .cloned()
            .collect();
        let overrides: Vec<_> = code_ranges
            .iter()
            .map(|r| (r.clone(), fam.clone()))
            .collect();
        if !overrides.is_empty() {
            styled = styled.with_font_family_overrides(overrides);
        }
    }
    let layout = styled.layout().clone();
    hits.push(Hit {
        display_start,
        doc_len: if empty { 0 } else { text.len() },
        layout: layout.clone(),
    });
    let color = p.primary;
    let pill_color = p.background_element;
    let click_empty = empty;
    div()
        .id(("edit", display_start))
        .relative()
        .w_full()
        .min_w_0()
        .when_some(font_family, |el, fam| el.font_family(fam))
        .when_some(font_px, |el, sz| el.text_size(sz))
        .when(heading, |el| el.font_weight(gpui::FontWeight::SEMIBOLD))
        .when(wrap, |el| el.whitespace_normal())
        .when(!wrap, |el| el.whitespace_nowrap())
        .when(!code_ranges.is_empty(), |el| {
            el.child(CodePillLayer {
                layout: layout.clone(),
                ranges: code_ranges,
                color: pill_color,
            })
        })
        .child(styled)
        .child(CaretLayer {
            layout: layout.clone(),
            local_caret: if empty {
                caret_local.map(|_| 0)
            } else {
                caret_local
            },
            block: block_caret,
            skip_block: inverted,
            color,
            view,
            focus,
            ime,
        })
        .on_mouse_down(MouseButton::Left, {
            let layout = layout.clone();
            move |ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let idx = if click_empty {
                    0
                } else {
                    index_for_click(&layout, ev.position)
                };
                let cmd_or_ctrl = ev.modifiers.platform || ev.modifiers.control;
                on_click(
                    display_start + idx,
                    ev.modifiers.shift,
                    cmd_or_ctrl,
                    ev.click_count,
                    window,
                    cx,
                );
            }
        })
        .on_mouse_move({
            let layout = layout.clone();
            move |ev: &MouseMoveEvent, window, cx| {
                if ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                if click_empty {
                    on_drag(display_start, window, cx);
                    return;
                }
                let idx = index_for_click(&layout, ev.position);
                on_drag(display_start + idx, window, cx);
            }
        })
        .into_any_element()
}

/// `TextLayout::{bounds,len,index_for_position,…}` panic if measure/prepaint
/// has not run. Swallow that for hit-testing during events / IME.
fn layout_bounds(layout: &TextLayout) -> Option<Bounds<Pixels>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| layout.bounds())).ok()
}

fn layout_len(layout: &TextLayout) -> Option<usize> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| layout.len())).ok()
}

pub fn index_for_click(layout: &TextLayout, position: Point<Pixels>) -> usize {
    let Some(len) = layout_len(layout) else {
        return 0;
    };
    let idx = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        layout.index_for_position(position).unwrap_or_else(|e| e)
    }))
    .unwrap_or(0)
    .min(len);
    let Some(pos) = (std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        layout.position_for_index(idx)
    }))
    .ok()
    .flatten()) else {
        return idx;
    };
    let h = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| layout.line_height()))
        .unwrap_or(px(16.));
    if pos.y > position.y + h * 0.4 && idx > 0 {
        let mut i = idx;
        while i > 0 {
            i -= 1;
            if let Some(p2) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                layout.position_for_index(i)
            }))
            .ok()
            .flatten()
            {
                if p2.y <= position.y + h * 0.4 {
                    return i;
                }
            }
        }
    }
    idx
}

pub fn index_for_point(hits: &[Hit], point: Point<Pixels>) -> Option<usize> {
    if hits.is_empty() {
        return None;
    }
    for hit in hits {
        let Some(bounds) = layout_bounds(&hit.layout) else {
            continue;
        };
        if point.y < bounds.top() || point.y > bounds.bottom() {
            continue;
        }
        let idx = index_for_click(&hit.layout, point).min(hit.doc_len);
        return Some(hit.display_start + idx);
    }
    let mut best: Option<(Pixels, usize)> = None;
    for hit in hits {
        let Some(bounds) = layout_bounds(&hit.layout) else {
            continue;
        };
        let dist = if point.y < bounds.top() {
            bounds.top() - point.y
        } else {
            point.y - bounds.bottom()
        };
        let d = if point.y > bounds.bottom() {
            hit.display_start + hit.doc_len
        } else {
            hit.display_start
        };
        match best {
            Some((b, _)) if dist >= b => {}
            _ => best = Some((dist, d)),
        }
    }
    best.map(|(_, d)| d)
}

/// Window-space Y of the caret and its line height, after text layouts have
/// been prepainted into `hits`.
pub fn caret_screen_y(hits: &[Hit], display: usize) -> Option<(Pixels, Pixels)> {
    let mut fallback: Option<&Hit> = None;
    for hit in hits {
        if display < hit.display_start {
            break;
        }
        fallback = Some(hit);
        if display <= hit.display_start + hit.doc_len {
            return caret_y(hit, display);
        }
    }
    fallback.and_then(|hit| caret_y(hit, display))
}

fn caret_y(hit: &Hit, display: usize) -> Option<(Pixels, Pixels)> {
    let len = layout_len(&hit.layout)?;
    let local = display.saturating_sub(hit.display_start).min(len);
    let pos = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hit.layout.position_for_index(local)
    }))
    .ok()
    .flatten()?;
    let h = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hit.layout.line_height()))
        .unwrap_or(px(16.));
    Some((pos.y, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgb;

    #[test]
    fn flatten_merges_heading_and_selection() {
        let heading = HighlightStyle {
            font_weight: Some(gpui::FontWeight::SEMIBOLD),
            color: Some(rgb(0x111111).into()),
            ..Default::default()
        };
        let sel = HighlightStyle {
            background_color: Some(rgb(0x3366ff).into()),
            ..Default::default()
        };
        let out = flatten(8, vec![(0..8, heading), (2..5, sel)]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].0, 0..2);
        assert_eq!(out[1].0, 2..5);
        assert_eq!(out[1].1.background_color, sel.background_color);
        assert_eq!(out[1].1.font_weight, heading.font_weight);
        assert_eq!(out[2].0, 5..8);
    }
}
