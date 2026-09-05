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

/// Inline `code` renders at 85% of the surrounding text (GitHub-like).
/// GPUI sizes one `StyledText` uniformly, so code-bearing blocks split into
/// word-sized flow items in a wrapping flex row instead of one layout.
pub const INLINE_CODE_SCALE: f32 = 0.85;

/// One wrappable unit of a segmented block: a word slice (with its code flag)
/// or a hard break at a byte offset.
pub enum FlowBit {
    Word(Range<usize>, bool),
    Brk(usize),
}

fn is_ri(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// No line-break opportunity before these (combining marks, ZWJ glue,
/// variation selectors, skin tones) — chunking must not split them.
fn no_cut_before(c: char) -> bool {
    matches!(c, '\u{200D}' | '\u{FE0E}' | '\u{FE0F}')
        || ('\u{0300}'..='\u{036F}').contains(&c)
        || ('\u{1AB0}'..='\u{1AFF}').contains(&c)
        || ('\u{1DC0}'..='\u{1DFF}').contains(&c)
        || ('\u{20D0}'..='\u{20FF}').contains(&c)
        || ('\u{FE20}'..='\u{FE2F}').contains(&c)
        || ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

/// Split an over-long word core so nothing overflows the viewport. Leading
/// spaces stay on the first chunk, trailing on the last.
fn chunk_word(text: &str, r: Range<usize>) -> Vec<Range<usize>> {
    const MAX: usize = 32;
    let s = text.get(r.clone()).unwrap_or("");
    let lead = s.len() - s.trim_start_matches([' ', '\t']).len();
    let trail = s.len() - s.trim_end_matches([' ', '\t']).len();
    let core_s = r.start + lead;
    let core_e = r.end.saturating_sub(trail);
    let core = text.get(core_s..core_e).unwrap_or("");
    if core.chars().count() <= MAX || core.is_empty() {
        return vec![r];
    }
    let chars: Vec<(usize, char)> = core.char_indices().collect();
    let mut cuts: Vec<usize> = vec![core_s];
    let mut since = 0usize;
    let mut ris = 0usize;
    for (idx, (off, c)) in chars.iter().enumerate() {
        if is_ri(*c) {
            ris += 1;
        }
        since += 1;
        if since >= MAX {
            let next = chars.get(idx + 1).map(|(_, c)| *c);
            let safe = match next {
                None => true,
                Some(nc) => {
                    if no_cut_before(nc) {
                        false
                    } else if ris % 2 == 1 && is_ri(nc) {
                        // Flag pairs: don't strand an odd RI count.
                        false
                    } else {
                        true
                    }
                }
            };
            if safe {
                cuts.push(core_s + off + c.len_utf8());
                since = 0;
                ris = 0;
            }
        }
    }
    cuts.push(core_e);
    cuts.sort_unstable();
    cuts.dedup();
    let mut pieces = Vec::new();
    for (i, w) in cuts.windows(2).enumerate() {
        let a = if i == 0 { r.start } else { w[0] };
        let b = if i + 2 == cuts.len() { r.end } else { w[1] };
        if a < b {
            pieces.push(a..b);
        }
    }
    if pieces.is_empty() { vec![r] } else { pieces }
}

fn push_word(out: &mut Vec<FlowBit>, text: &str, r: Range<usize>, code: bool) {
    if r.start >= r.end {
        return;
    }
    for piece in chunk_word(text, r) {
        out.push(FlowBit::Word(piece, code));
    }
}

/// Split block text into wrappable flow items for the segmented (mixed-size)
/// path. Words keep surrounding spaces (trailing preferred, so wrapped lines
/// never start with a gap); `\n`/`\r\n` become hard breaks. Ranges partition
/// the text — every byte belongs to exactly one word — so display offsets
/// map 1:1 with the single-layout path.
fn flow_bits(text: &str, code_ranges: &[Range<usize>]) -> Vec<FlowBit> {
    // Sorted, merged, clamped code spans.
    let mut codes: Vec<Range<usize>> = code_ranges
        .iter()
        .filter(|r| {
            r.start < r.end
                && r.start <= text.len()
                && text.is_char_boundary(r.start)
                && (r.end > text.len() || text.is_char_boundary(r.end))
        })
        .map(|r| r.start..r.end.min(text.len()))
        .collect();
    codes.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in codes {
        if let Some(last) = merged.last_mut() {
            if r.start <= last.end {
                last.end = last.end.max(r.end);
                continue;
            }
        }
        merged.push(r);
    }
    // Atoms ignoring code: words (leading spaces + core + trailing spaces).
    enum Atom {
        W(Range<usize>),
        B(usize),
    }
    let mut atoms: Vec<Atom> = Vec::new();
    let mut i = 0usize;
    let mut lead: Option<usize> = None;
    while i < text.len() {
        let ch = text[i..].chars().next().unwrap_or('\0');
        match ch {
            '\n' | '\r' => {
                // Trailing spaces stay with the previous word — never lead
                // the next line.
                if let Some(s) = lead.take() {
                    if let Some(Atom::W(r)) = atoms.last_mut() {
                        r.end = r.end.max(i);
                        let _ = s;
                    } else {
                        lead = Some(s);
                    }
                }
                atoms.push(Atom::B(i));
                i += if ch == '\r' && text[i + 1..].starts_with('\n') {
                    2
                } else {
                    1
                };
            }
            ' ' | '\t' => {
                if lead.is_none() {
                    lead = Some(i);
                }
                i += 1;
            }
            _ => {
                let start = lead.take().unwrap_or(i);
                let mut j = i;
                while j < text.len() {
                    let c = text[j..].chars().next().unwrap_or('\0');
                    if matches!(c, ' ' | '\t' | '\n' | '\r') {
                        break;
                    }
                    j += c.len_utf8();
                }
                let mut k = j;
                while k < text.len() {
                    let c = text[k..].chars().next().unwrap_or('\0');
                    if !matches!(c, ' ' | '\t') {
                        break;
                    }
                    k += 1;
                }
                atoms.push(Atom::W(start..k));
                i = k;
            }
        }
    }
    if let Some(_s) = lead.take() {
        if let Some(Atom::W(r)) = atoms.last_mut() {
            r.end = r.end.max(text.len());
        } else {
            // All-space text — one body word so offsets stay covered.
            atoms.push(Atom::W(0..text.len()));
        }
    }
    // Subdivide atoms at code edges (kissing runs can share one word atom).
    let mut out: Vec<FlowBit> = Vec::new();
    for atom in atoms {
        match atom {
            Atom::B(o) => out.push(FlowBit::Brk(o)),
            Atom::W(r) => {
                let mut cur = r.start;
                for c in merged.iter() {
                    if c.end <= cur || c.start >= r.end {
                        continue;
                    }
                    let (cs, ce) = (c.start.max(cur), c.end.min(r.end));
                    if cs > cur {
                        push_word(&mut out, text, cur..cs, false);
                    }
                    push_word(&mut out, text, cs..ce, true);
                    cur = ce;
                }
                if cur < r.end {
                    push_word(&mut out, text, cur..r.end, false);
                }
            }
        }
    }
    out
}

/// Clip block-local highlights to a word piece, rebased to piece origin.
fn clip_hs(
    hs: &[(Range<usize>, HighlightStyle)],
    a: usize,
    b: usize,
) -> Vec<(Range<usize>, HighlightStyle)> {
    hs.iter()
        .filter_map(|(r, h)| {
            let s = r.start.max(a).min(b);
            let e = r.end.max(a).min(b);
            if s < e {
                Some((s - a..e - a, h.clone()))
            } else {
                None
            }
        })
        .collect()
}

/// First hit containing a display offset (upstream at piece edges), else the
/// nearest preceding hit. Hits must be sorted by `display_start`.
pub fn piece_for_offset(hits: &[Hit], display: usize) -> Option<&Hit> {
    let mut fallback = None;
    for hit in hits {
        if display < hit.display_start {
            break;
        }
        fallback = Some(hit);
        if display <= hit.display_start + hit.doc_len {
            return Some(hit);
        }
    }
    fallback
}

/// Rounded, padded backgrounds for inline `code` spans. Each span holds one
/// nowrap word layout per fragment; rows merge at paint time so wrapped
/// spans read as one broken pill.
pub struct CodePillLayer {
    pub layouts: Vec<TextLayout>,
    pub color: Hsla,
    pub font_size: Pixels,
    pub pad_l: Pixels,
    pub pad_r: Pixels,
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
        // prepaints *before* its sibling `StyledText`s, and
        // `position_for_index` panics until a sibling's prepaint stores its
        // bounds. By paint time every prepaint has run, so positions are
        // final for this frame.
        // NOTE: positions are already window coordinates, no origin offset.
        if self.layouts.is_empty() {
            return;
        }
        let line_h = self
            .layouts
            .iter()
            .find_map(|l| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l.line_height())).ok()
            })
            .unwrap_or(px(16.));
        // GFM-like pill: height hugs the glyphs (font size + a whisper of
        // vertical padding) instead of filling the full line height, then
        // centered on the line. Each word frag is nowrap so it covers at
        // most one visual row; adjacent words sharing a row merge (the
        // inter-word gap stays washed so the span reads as one pill).
        // Outer pads come from the span edges (neighbor-aware at
        // construction); interior wrap edges use the tight scheme below.
        // 6px radius.
        let pad_y = px(3.);
        let radius = px(6.);
        let tight_pad_x = px(1.);
        let pill_h = self.font_size + pad_y * 2.;
        let y_off = (line_h - pill_h).max(px(0.)) / 2.;
        let mut rows: Vec<(Pixels, Pixels, Pixels)> = Vec::new();
        for layout in &self.layouts {
            let Some(len) = layout_len(layout) else {
                continue;
            };
            let (Some(a), Some(b)) = (
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    layout.position_for_index(0)
                }))
                .ok()
                .flatten(),
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    layout.position_for_index(len)
                }))
                .ok()
                .flatten(),
            ) else {
                continue;
            };
            let y = if (a.y - b.y).abs() < px(1.) {
                a.y
            } else {
                a.y.min(b.y)
            };
            let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
            if x1 - x0 < px(1.) {
                continue;
            }
            let mut placed = false;
            for (gy, gx0, gx1) in rows.iter_mut() {
                // Same visual row, adjacent words (the inter-word gap stays
                // washed so the span reads as one pill). 14px covers a space
                // advance plus pixel-snapping slack; frags belong to one
                // span by construction so nothing foreign can merge.
                if (*gy - y).abs() < px(1.) && x0 - *gx1 <= px(14.) && *gx0 - x1 <= px(14.) {
                    *gx0 = (*gx0).min(x0);
                    *gx1 = (*gx1).max(x1);
                    placed = true;
                    break;
                }
            }
            if !placed {
                rows.push((y, x0, x1));
            }
        }
        if rows.is_empty() {
            return;
        }
        rows.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        // Coalesce pass after sorting (out-of-order frags can share rows).
        let mut merged: Vec<(Pixels, Pixels, Pixels)> = Vec::new();
        for (y, x0, x1) in rows {
            if let Some((gy, _, gx1)) = merged.last_mut() {
                if (*gy - y).abs() < px(1.) && x0 - *gx1 <= px(14.) {
                    *gx1 = (*gx1).max(x1);
                    continue;
                }
            }
            merged.push((y, x0, x1));
        }
        if merged.is_empty() {
            return;
        }
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
        let m = merged.len();
        for (i, (y, x0, x1)) in merged.iter().enumerate() {
            let first = i == 0;
            let last = i + 1 == m;
            // Interior edges sit at row ends: the continuation start takes
            // the full wash (margin is free at a line start), the row end
            // stays tight (the row is full; avoids viewport overflow).
            let pad_l = if first { self.pad_l } else { px(3.) };
            let pad_r = if last { self.pad_r } else { tight_pad_x };
            let corners = if first && last {
                all_round
            } else if first {
                left_round
            } else if last {
                right_round
            } else {
                square
            };
            let bounds = Bounds::new(
                Point::new(*x0 - pad_l, *y + y_off),
                size((*x1 - *x0) + pad_l + pad_r, pill_h),
            );
            window.paint_quad(fill(bounds, self.color).corner_radii(corners));
        }
    }
}

/// Mixed-size block rendering: body words at body size, code words at 85%,
/// laid out as baseline-aligned flow items in a wrapping flex row. Pieces
/// partition the block text 1:1, so every display offset maps exactly like
/// the single-layout path — only rendering is split.
#[allow(clippy::too_many_arguments)]
fn edit_segmented<V: EntityInputHandler>(
    view: Entity<V>,
    focus: FocusHandle,
    hits: &mut Vec<Hit>,
    display_start: usize,
    shown: gpui::SharedString,
    hs: Vec<(Range<usize>, HighlightStyle)>,
    caret_local: Option<usize>,
    block_caret: bool,
    inverted: bool,
    ime: bool,
    _p: &Palette,
    body_family: Option<gpui::SharedString>,
    body_px: Option<Pixels>,
    code_family: Option<gpui::SharedString>,
    code_px: Pixels,
    heading: bool,
    code_ranges: Vec<Range<usize>>,
    pill_color: Hsla,
    color: Hsla,
    on_click: impl Fn(usize, bool, bool, usize, &mut Window, &mut App) + 'static,
    on_drag: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    // Merged code runs (text order) — one pill span each.
    let mut sorted = code_ranges;
    sorted.sort_by_key(|r| r.start);
    let mut runs: Vec<Range<usize>> = Vec::new();
    for r in sorted {
        if let Some(last) = runs.last_mut() {
            if r.start <= last.end {
                last.end = last.end.max(r.end);
                continue;
            }
        }
        runs.push(r);
    }
    let on_click = std::rc::Rc::new(on_click);
    let on_drag = std::rc::Rc::new(on_drag);
    let mut flow: Vec<gpui::AnyElement> = Vec::new();
    let mut run_frags: Vec<Vec<TextLayout>> = runs.iter().map(|_| Vec::new()).collect();
    let mut ri = 0usize;
    let mut caret_piece: Option<(TextLayout, usize)> = None;
    let mut last_piece: Option<(TextLayout, usize)> = None;
    let hits_base = hits.len();
    for bit in flow_bits(&shown, &runs) {
        match bit {
            FlowBit::Brk(_) => {
                // Full-width zero-height item forces the flex wrap.
                flow.push(
                    div()
                        .flex_basis(gpui::relative(1.))
                        .h(px(0.))
                        .into_any_element(),
                );
            }
            FlowBit::Word(r, is_code) => {
                let word: gpui::SharedString =
                    gpui::SharedString::from(shown.get(r.clone()).unwrap_or(""));
                let styled =
                    StyledText::new(word).with_highlights(clip_hs(&hs, r.start, r.end));
                let layout = styled.layout().clone();
                hits.push(Hit {
                    display_start: display_start + r.start,
                    doc_len: r.len(),
                    layout: layout.clone(),
                });
                // Every run edge gets a 3px in-flow margin matching the 3px
                // overlay wash. Kissing runs (`a`b`c`) need it for any
                // breathing room; spaced runs (`run `bun dev``) need it
                // because the wash would otherwise eat the word space —
                // space advance minus 3px overlay left a ~1px sliver.
                // Only run edges — interior frags stay tight so wrapped
                // multi-word spans don't double-pad.
                let mut need_ml = false;
                let mut need_mr = false;
                if is_code {
                    while ri < runs.len() && r.start >= runs[ri].end {
                        ri += 1;
                    }
                    if ri < runs.len()
                        && r.start >= runs[ri].start
                        && r.start < runs[ri].end
                    {
                        run_frags[ri].push(layout.clone());
                        if r.start == runs[ri].start {
                            need_ml = true;
                        }
                        if r.end == runs[ri].end {
                            need_mr = true;
                        }
                    }
                }
                // Upstream at piece edges (matches `piece_for_offset`).
                if caret_piece.is_none() {
                    if let Some(c) = caret_local {
                        if c >= r.start && c <= r.end {
                            caret_piece = Some((layout.clone(), c - r.start));
                        }
                    }
                }
                last_piece = Some((layout.clone(), r.start));
                let sz = if is_code {
                    code_px
                } else {
                    body_px.unwrap_or(code_px)
                };
                let fam = if is_code { code_family.clone() } else { None };
                flow.push(
                    div()
                        .text_size(sz)
                        .when_some(fam, |el, f| el.font_family(f))
                        .whitespace_nowrap()
                        .when(need_ml, |el| el.ml(px(3.)))
                        .when(need_mr, |el| el.mr(px(3.)))
                        .child(styled)
                        .into_any_element(),
                );
            }
        }
    }
    // Caret past the last word (or inside a break) docks to the final piece;
    // `CaretLayer` clamps the local offset to the layout length.
    let caret_layer = match caret_piece {
        Some((layout, local)) => Some((layout, local)),
        None => caret_local.and_then(|c| {
            last_piece.map(|(layout, start)| (layout, c.saturating_sub(start)))
        }),
    };
    // Parent fallback resolve covers gaps between flow items (line-remainder
    // space has no word element under the cursor).
    let seg_hits: Vec<Hit> = hits[hits_base..].to_vec();
    let mut root = div()
        .id(("edit", display_start))
        .relative()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_baseline()
        .when_some(body_family, |el, fam| el.font_family(fam))
        .when_some(body_px, |el, sz| el.text_size(sz))
        .when(heading, |el| el.font_weight(gpui::FontWeight::SEMIBOLD));
    for (run, frags) in runs.iter().zip(run_frags) {
        if frags.is_empty() {
            continue;
        }
        // Overlay wash always 3px with a matching 3px in-flow margin above,
        // so the wash lands on free gap: full word-space preserved outside,
        // 3px breathing room inside. No overhang onto neighbors.
        root = root.child(CodePillLayer {
            layouts: frags,
            color: pill_color,
            font_size: code_px,
            pad_l: px(3.),
            pad_r: px(3.),
        });
    }
    for item in flow {
        root = root.child(item);
    }
    if let Some((layout, local)) = caret_layer {
        root = root.child(CaretLayer {
            layout,
            local_caret: Some(local),
            block: block_caret,
            skip_block: inverted,
            color,
            view,
            focus,
            ime,
        });
    }
    let cb = on_click.clone();
    let seg_hits_down = seg_hits.clone();
    root = root.on_mouse_down(
        MouseButton::Left,
        move |ev: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            if let Some(d) = index_for_point(&seg_hits_down, ev.position) {
                let cmd_or_ctrl = ev.modifiers.platform || ev.modifiers.control;
                cb(d, ev.modifiers.shift, cmd_or_ctrl, ev.click_count, window, cx);
            }
        },
    );
    let db = on_drag.clone();
    root = root.on_mouse_move(move |ev: &MouseMoveEvent, window, cx| {
        if ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some(d) = index_for_point(&seg_hits, ev.position) {
            db(d, window, cx);
        }
    });
    root.into_any_element()
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
    code_px: Option<gpui::Pixels>,
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
    let mut code_ranges: Vec<Range<usize>> = Vec::new();
    if let Some((_, ranges)) = &code_font {
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
    }
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
    // Segmented (mixed-size) path: inline code at 85% needs word-sized flow
    // items — one uniform layout cannot carry two sizes.
    if !empty {
        if let Some(cpx) = code_px {
            if !code_ranges.is_empty() {
                let code_fam = code_font.map(|(f, _)| f);
                return edit_segmented(
                    view,
                    focus,
                    hits,
                    display_start,
                    shown.clone(),
                    hs,
                    caret_local,
                    block_caret,
                    inverted,
                    ime,
                    p,
                    font_family,
                    font_px,
                    code_fam,
                    cpx,
                    heading,
                    code_ranges,
                    pill_color,
                    color,
                    on_click,
                    on_drag,
                );
            }
        }
    }
    let mut styled = StyledText::new(shown.clone()).with_highlights(hs);
    if let Some((fam, _)) = &code_font {
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
        // NOTE: inline-code pills live on the segmented path (`code_px` set);
        // the single layout never carries code ranges (fenced blocks emit
        // default marks), so no pill layer here.
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
    // Nearest-glyph resolve: segmented blocks hold one hit per word piece
    // and pieces share visual rows, so y-bands alone can't disambiguate —
    // score every painted layout by distance to its closest caret.
    let mut best: Option<(f32, usize)> = None;
    for hit in hits {
        let Some(len) = layout_len(&hit.layout) else {
            continue;
        };
        let idx = index_for_click(&hit.layout, point)
            .min(hit.doc_len)
            .min(len);
        let Some(pos) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hit.layout.position_for_index(idx)
        }))
        .ok()
        .flatten() else {
            continue;
        };
        let dx: f32 = (pos.x - point.x).into();
        let dy: f32 = (pos.y - point.y).into();
        let score = dx * dx + dy * dy;
        // Strict `<` keeps the earliest hit on ties (upstream at edges,
        // matching `piece_for_offset`).
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, hit.display_start + idx));
        }
    }
    if best.is_some() {
        return best.map(|(_, d)| d);
    }
    // Nothing painted yet — fall back to nearest block edge by y.
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
    piece_for_offset(hits, display).and_then(|hit| caret_y(hit, display))
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
    piece_for_offset(hits, display).and_then(|hit| caret_x(hit, display))
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

    fn word_ranges(bits: &[FlowBit]) -> Vec<(Range<usize>, bool)> {
        bits.iter()
            .filter_map(|b| match b {
                FlowBit::Word(r, c) => Some((r.clone(), *c)),
                FlowBit::Brk(_) => None,
            })
            .collect()
    }

    #[test]
    fn flow_bits_partition_text() {
        // "Use `ZedMono NF` words" displays as "Use ZedMono NF words"
        // with the code span at 4..14.
        let text = "Use ZedMono NF words";
        let bits = flow_bits(text, &[4..14]);
        let words = word_ranges(&bits);
        // Full coverage, no gaps or overlaps, in order.
        let mut cursor = 0;
        for (r, _) in &words {
            assert_eq!(r.start, cursor, "{words:?}");
            cursor = r.end;
        }
        assert_eq!(cursor, text.len());
        // Code flags land on the span (split at the space inside it).
        assert!(words.iter().any(|(r, c)| *c && text.get(r.clone()) == Some("ZedMono ")));
        assert!(words.iter().any(|(r, c)| *c && text.get(r.clone()) == Some("NF")));
        assert!(words.iter().all(|(r, c)| {
            *c == (r.start >= 4 && r.end <= 14) || text.get(r.clone()).unwrap_or("").trim().is_empty()
        }));
        // Body words keep their trailing space.
        assert_eq!(text.get(words[0].0.clone()), Some("Use "));
    }

    #[test]
    fn flow_bits_kissing_runs_split_mid_word() {
        // `a`b`c` displays as "abc" with code at 1..2.
        let bits = flow_bits("abc", &[1..2]);
        let words = word_ranges(&bits);
        assert_eq!(words.len(), 3);
        assert_eq!(words[0], (0..1, false));
        assert_eq!(words[1], (1..2, true));
        assert_eq!(words[2], (2..3, false));
    }

    #[test]
    fn flow_bits_hard_breaks_and_long_words() {
        let bits = flow_bits("a\nb", &[]);
        assert!(matches!(bits[0], FlowBit::Word(_, _)));
        assert!(matches!(bits[1], FlowBit::Brk(1)));
        assert!(matches!(bits[2], FlowBit::Word(_, _)));
        // 40 ASCII chars chunk; CJK stays atomic per char run.
        let long: String = "x".repeat(40);
        let bits = flow_bits(&long, &[]);
        assert_eq!(word_ranges(&bits).len(), 2);
        let cjk = "日本語テスト block";
        let bits = flow_bits(cjk, &[]);
        let mut cursor = 0;
        for (r, _) in word_ranges(&bits) {
            assert_eq!(r.start, cursor);
            assert!(cjk.is_char_boundary(r.start) && cjk.is_char_boundary(r.end));
            cursor = r.end;
        }
        assert_eq!(cursor, cjk.len());
    }
}
