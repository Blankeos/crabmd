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
    pub font_size: Pixels,
    pub text: gpui::SharedString,
}

impl IntoElement for CodePillLayer {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for CodePillLayer {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        // Geometry is computed here — not in prepaint — because this layer
        // prepaints *before* its sibling `StyledText`, and `position_for_index`
        // panics until that sibling's prepaint stores its bounds. By paint
        // time every prepaint has run, so positions are final for this frame.
        // NOTE: positions are already window coordinates, no origin offset.
        let Some(len) = layout_len(&self.layout) else {
            return;
        };
        let line_h =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.layout.line_height()))
                .unwrap_or(px(16.));
        // GFM-like pill: height hugs the glyphs (font size + a whisper of
        // vertical padding) instead of filling the full line height, then
        // centered on the line. Horizontal padding is adaptive on purpose:
        // the pill is a background overlay — it can't push neighbors away
        // like GitHub's in-flow `<code>` padding, so the pad budget comes
        // out of the existing gap instead:
        // - block/line edge: full 3px wash (margin is free there)
        // - facing a space: 1px hair, leaving the word gap visible
        // - kissing punctuation/letters: 0, no overhang onto ink
        // - wrapped line end: 1px (the row is full; avoids viewport clip)
        // 6px radius.
        let full_pad_x = px(3.);
        let tight_pad_x = px(1.);
        let zero_pad_x = px(0.);
        let pad_y = px(3.);
        let radius = px(6.);
        let pill_h = self.font_size + pad_y * 2.;
        let y_off = (line_h - pill_h).max(px(0.)) / 2.;
        for range in &self.ranges {
            if range.start >= range.end || range.end > len {
                continue;
            }
            let t = self.text.as_ref();
            let tlen = t.len();
            let r_end = range.end.min(len).min(tlen);
            let r_start = range.start.min(r_end);
            if r_start >= r_end
                || !t.is_char_boundary(r_start)
                || (r_end < tlen && !t.is_char_boundary(r_end))
            {
                continue;
            }
            let pos = |ix: usize| -> Option<Point<Pixels>> {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.layout.position_for_index(ix.min(len))
                }))
                .ok()
                .flatten()
            };
            let Some(start) = pos(r_start) else {
                continue;
            };
            let end = pos(r_end).unwrap_or(start);
            let left_prev = t.get(..r_start).and_then(|s| s.chars().next_back());
            let pad_l_first = match left_prev {
                None => full_pad_x, // block start
                Some('\n') | Some('\r') => full_pad_x, // line start
                Some(c) if c.is_whitespace() => tight_pad_x, // keep word gap
                _ => zero_pad_x,     // kiss ink: no overhang
            };
            let right_next = t.get(r_end..).and_then(|s| s.chars().next());
            let pad_r_last = match right_next {
                None => full_pad_x, // block end
                Some('\n') | Some('\r') => full_pad_x, // line end
                Some(c) if c.is_whitespace() => tight_pad_x, // keep word gap
                _ => zero_pad_x,     // kiss ink: no overhang
            };
            let paint_seg = |x0: Pixels,
                             x1: Pixels,
                             y: Pixels,
                             pad_l: Pixels,
                             pad_r: Pixels,
                             corners: Corners<Pixels>,
                             window: &mut Window| {
                if x1 - x0 < px(0.5) {
                    return;
                }
                let bounds = Bounds::new(
                    Point::new(x0 - pad_l, y + y_off),
                    size((x1 - x0) + pad_l + pad_r, pill_h),
                );
                window.paint_quad(fill(bounds, self.color).corner_radii(corners));
            };
            let all_round = Corners::all(radius);
            let zero = px(0.);
            // Wrapped fragments read as one broken pill: outer edges round,
            // interior (continuation) edges square — GitHub slice style.
            let left_round = Corners {
                top_left: radius,
                bottom_left: radius,
                top_right: zero,
                bottom_right: zero,
            };
            let right_round = Corners {
                top_left: zero,
                bottom_left: zero,
                top_right: radius,
                bottom_right: radius,
            };
            let square = Corners::all(zero);
            // Fast path: whole range on one visual line.
            if (start.y - end.y).abs() < px(0.5)
                && t.get(r_start..r_end).is_some_and(|s| !s.contains('\n'))
            {
                paint_seg(
                    start.x.min(end.x),
                    start.x.max(end.x),
                    start.y,
                    pad_l_first,
                    pad_r_last,
                    all_round,
                    window,
                );
                continue;
            }
            // Slow path: soft-wrapped (or hard-broken block) code. Walk the
            // caret positions of every char boundary, grouping by visual
            // line so each wrapped row gets its own pill. Hard lines are
            // fast-pathed first so big fenced blocks stay at 2 queries
            // per line; only soft-wrapped rows pay per-glyph.
            let slice = t.get(r_start..r_end).unwrap_or("");
            let mut hard: Vec<(usize, usize)> = Vec::new();
            if slice.contains('\n') {
                let mut off = r_start;
                for part in slice.split_inclusive('\n') {
                    let part_end = off + part.len();
                    let mut seg_end = part_end;
                    if part.ends_with('\n') {
                        seg_end -= 1;
                        if part.ends_with("\r\n") {
                            seg_end -= 1;
                        }
                    }
                    if seg_end > off {
                        hard.push((off, seg_end.min(r_end)));
                    }
                    off = part_end;
                }
            } else {
                hard.push((r_start, r_end));
            }
            // Per hard segment, per visual line — (y, x0, x1) in y order.
            // Glyphs (not carets) are grouped by the row of their
            // *trailing* caret, which GPUI reports on the glyph's own
            // row. The leading caret of a soft-wrapped row's first glyph
            // instead reports at the previous row's end (wrap-boundary
            // affinity) — using it as the row start is what used to clip
            // that glyph (e.g. the `n` in `a↵ntonio`). The fix: a glyph
            // whose carets straddle rows opens the new row one mono
            // advance left of its trailing caret. Wrap-collapsed spaces
            // (the break point) are trimmed off the previous row's end.
            let mut frags: Vec<(Pixels, Pixels, Pixels)> = Vec::new();
            for (s0, s1) in hard {
                // Boundaries s0..=s1 (s1 inclusive), then pair into glyphs.
                let mut bnd: Vec<usize> = Vec::new();
                let mut j = s0;
                loop {
                    bnd.push(j);
                    if j >= s1 {
                        break;
                    }
                    match t.get(j..s1).and_then(|s| s.chars().next()) {
                        Some(c) => j += c.len_utf8(),
                        None => j += 1,
                    }
                }
                if bnd.len() < 2 {
                    continue;
                }
                let mut pts: Vec<Option<Point<Pixels>>> = Vec::with_capacity(bnd.len());
                for b in &bnd {
                    pts.push(pos(*b));
                }
                // Mono advance: median same-row caret step, else ~0.6em.
                let mut steps: Vec<f32> = Vec::new();
                for w in pts.windows(2) {
                    if let [Some(a), Some(b)] = w {
                        if (a.y - b.y).abs() < px(0.5) {
                            let dx: f32 = (b.x - a.x).into();
                            let cap: f32 = self.font_size.into();
                            if dx > 0.5 && dx <= cap * 3.0 {
                                steps.push(dx);
                            }
                        }
                    }
                }
                steps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let median_step: f32 = if steps.is_empty() {
                    self.font_size.into()
                } else {
                    steps[steps.len() / 2]
                };
                let em: f32 = self.font_size.into();
                let advance = px(median_step.clamp(em * 0.4, em * 3.0));
                // Rows in y order; each glyph joins its trailing caret's row.
                let mut rows: Vec<(Pixels, Pixels, Pixels)> = Vec::new();
                for (i, w) in bnd.windows(2).enumerate() {
                    let (a, b) = (w[0], w[1]);
                    let (Some(pa), Some(pb)) = (pts[i], pts[i + 1]) else {
                        continue;
                    };
                    let ch = t.get(a..b).and_then(|s| s.chars().next());
                    if (pa.y - pb.y).abs() < px(0.5) {
                        // Whole glyph on one row.
                        // Wrap-collapsed break space: don't stretch the row.
                        let next_starts_row = pts
                            .get(i + 2)
                            .and_then(|np| *np)
                            .is_some_and(|np| (np.y - pb.y).abs() >= px(0.5));
                        let collapsed = ch.is_some_and(|c| c.is_whitespace()) && next_starts_row;
                        let row = row_for(&mut rows, pb.y);
                        row.1 = row.1.min(pa.x.min(pb.x));
                        row.2 = row.2.max(pa.x.min(pb.x));
                        if !collapsed {
                            row.2 = row.2.max(pa.x.max(pb.x));
                        }
                    } else {
                        // Straddles rows: first glyph of a soft-wrapped row.
                        // Its extent runs one advance left of its trailing
                        // caret (the line start the affinity hides).
                        let row = row_for(&mut rows, pb.y);
                        row.1 = row.1.min(pb.x - advance);
                        row.2 = row.2.max(pb.x);
                    }
                }
                for (y, x0, x1) in rows {
                    if x1 > x0 {
                        frags.push((y, x0, x1));
                    }
                }
            }
            if frags.is_empty() {
                continue;
            }
            // Drop zero-width wrap artifacts (a boundary belonging to both
            // rows carries no glyphs) *before* assigning edge styles, so
            // first/last corners land on real fragments.
            let mut kept: Vec<(Pixels, Pixels, Pixels)> = frags
                .iter()
                .filter(|(_, x0, x1)| *x1 - *x0 >= px(1.))
                .cloned()
                .collect();
            if kept.is_empty() {
                // Lone sub-pixel fragment still deserves its pill.
                if let Some(widest) = frags
                    .iter()
                    .max_by(|(_, a0, a1), (_, b0, b1)| {
                        (*a1 - *a0)
                            .partial_cmp(&(*b1 - *b0))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .cloned()
                {
                    kept.push(widest);
                }
            }
            let m = kept.len();
            for (i, (y, x0, x1)) in kept.iter().enumerate() {
                let first = i == 0;
                let last = i + 1 == m;
                // Interior edges sit at row ends: line-start side takes the
                // full wash (margin is free), line-end side stays tight
                // (the row is full; avoids viewport overflow).
                let pad_l = if first { pad_l_first } else { full_pad_x };
                let pad_r = if last { pad_r_last } else { tight_pad_x };
                let corners = if first && last {
                    all_round
                } else if first {
                    left_round
                } else if last {
                    right_round
                } else {
                    square
                };
                paint_seg(*x0, *x1, *y, pad_l, pad_r, corners, window);
            }
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
    fit_content: bool,
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
    // GitHub-like pill: neutral wash behind the mono text. Dark needs a
    // stronger lift to read on near-black; light needs only a whisper.
    let wash_opacity = match p.appearance {
        crate::theme::Appearance::Dark => 0.20,
        crate::theme::Appearance::Light => 0.09,
    };
    let pill_color = p
        .background_element
        .blend(p.markdown_text.opacity(wash_opacity));
    let click_empty = empty;
    div()
        .id(("edit", display_start))
        .relative()
        .when(!fit_content, |el| el.w_full().min_w_0())
        .when(fit_content, |el| {
            el.flex_none()
                .w_auto()
                .min_w_full()
                // Trailing clearance so the last columns scroll past the
                // floating lang/copy pill instead of hiding under it.
                .pr(px(112.))
        })
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
                font_size: font_px.unwrap_or(px(14.)),
                text: shown.clone(),
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

/// Find (or open) the pill fragment for visual row `y`. Rows arrive in
/// ascending y, so this is usually the tail — linear scan as fallback.
fn row_for(
    rows: &mut Vec<(Pixels, Pixels, Pixels)>,
    y: Pixels,
) -> &mut (Pixels, Pixels, Pixels) {
    if let Some(i) = rows
        .iter()
        .position(|(gy, _, _)| (*gy - y).abs() < px(0.5))
    {
        return &mut rows[i];
    }
    rows.push((y, px(f32::MAX), px(f32::MIN)));
    let n = rows.len();
    &mut rows[n - 1]
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

fn caret_y(hit: &Hit, display: usize) -> Option<(Pixels, Pixels)> {    let len = layout_len(&hit.layout)?;
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

/// Window-space X of the caret, after text layouts have been prepainted into
/// `hits`. Used to reveal the caret inside horizontal code scrollers.
pub fn caret_screen_x(hits: &[Hit], display: usize) -> Option<Pixels> {
    let mut fallback: Option<&Hit> = None;
    for hit in hits {
        if display < hit.display_start {
            break;
        }
        fallback = Some(hit);
        if display <= hit.display_start + hit.doc_len {
            return caret_x(hit, display);
        }
    }
    fallback.and_then(|hit| caret_x(hit, display))
}

fn caret_x(hit: &Hit, display: usize) -> Option<Pixels> {
    let len = layout_len(&hit.layout)?;
    let local = display.saturating_sub(hit.display_start).min(len);
    let pos = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hit.layout.position_for_index(local)
    }))
    .ok()
    .flatten()?;
    Some(pos.x)
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
