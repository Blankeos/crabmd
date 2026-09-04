//! GFM `source` → visible projection. Syntax stays in the file; the caret walks
//! `display`. Re-parse after every edit — markdown-markdown is just the parser.

use std::borrow::Cow;
use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::document::{gfm_options, parse_ranges, sole_image, AlertKind, BlockKind, PaintRange};
use crate::notion;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub underline: bool,
    pub link: Option<u32>,
}

impl Marks {
    pub fn any(self) -> bool {
        self.bold
            || self.italic
            || self.strike
            || self.code
            || self.underline
            || self.link.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    /// Insert stays inside the mark at a boundary (extend bold).
    Inside,
    /// Insert leaves the mark (Right at end of a run).
    Outside,
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub display: Range<usize>,
    pub source: Range<usize>,
    pub marks: Marks,
}

#[derive(Clone, Debug)]
pub struct ListItem {
    pub display: Range<usize>,
    pub source: Range<usize>,
    pub indent: usize,
    pub checked: Option<bool>,
}

/// Sibling index of `ix` among items that share its indent under the same parent.
pub fn list_sibling_index(items: &[ListItem], ix: usize) -> usize {
    let Some(it) = items.get(ix) else {
        return 0;
    };
    let indent = it.indent;
    let mut n = 0usize;
    for j in (0..ix).rev() {
        if items[j].indent < indent {
            break;
        }
        if items[j].indent == indent {
            n += 1;
        }
    }
    n
}

/// Notion-style ordered marker: `1.` / `a.` / `i.` cycling by indent depth.
pub fn ordered_marker(indent: usize, sibling_ix: usize) -> String {
    match indent % 3 {
        0 => format!("{}.", sibling_ix + 1),
        1 => format!("{}.", to_alpha(sibling_ix)),
        _ => format!("{}.", to_roman(sibling_ix + 1)),
    }
}

fn to_alpha(mut ix: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'a' + (ix % 26) as u8) as char);
        if ix < 26 {
            break;
        }
        ix = ix / 26 - 1;
    }
    s
}

fn to_roman(n: usize) -> String {
    if n == 0 {
        return "i".into();
    }
    let table = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut n = n;
    let mut out = String::new();
    for &(val, sym) in &table {
        while n >= val {
            out.push_str(sym);
            n -= val;
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct TableCell {
    pub display: Range<usize>,
    pub source: Range<usize>,
    pub header: bool,
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Debug)]
pub enum BlockExtra {
    Text,
    Heading(u8),
    List {
        ordered: bool,
        items: Vec<ListItem>,
    },
    Code {
        lang: String,
        /// Leading spaces of the fence in source (0 for top-level).
        /// Split-out nested code keeps its visual nesting; the tree
        /// round-trips it so the file preserves the indent.
        indent: usize,
    },
    Quote,
    Alert(AlertKind),
    Table {
        cells: Vec<TableCell>,
        rows: usize,
        cols: usize,
    },
    Rule,
    Image {
        alt: String,
        src: String,
    },
    Html,
    /// Single-line `<h1>…</h1>` … `<h6>…</h6>`: renders as a heading,
    /// edits map back into the inner text.
    HtmlHeading(u8),
    /// `<details>` open tag: renders as a disclosure row showing `summary`.
    /// `open` mirrors the GFM `<details open>` attribute (open by default).
    /// Without it the section is collapsed by default.
    Details { summary: String, open: bool },
    /// `</details>` close tag: renders as zero-height chrome.
    DetailsClose,
}

impl BlockExtra {
    pub fn is_atomic(&self) -> bool {
        matches!(self, Self::Rule | Self::Image { .. })
    }
}

#[derive(Clone, Debug)]
pub struct ProjBlock {
    pub kind: BlockKind,
    pub source: Range<usize>,
    pub display: Range<usize>,
    pub extra: BlockExtra,
}

#[derive(Clone, Debug)]
pub struct Projection {
    pub display: String,
    pub segments: Vec<Segment>,
    pub blocks: Vec<ProjBlock>,
    pub links: Vec<String>,
    pub source_len: usize,
}

impl Projection {
    pub fn block_at_display(&self, d: usize) -> Option<&ProjBlock> {
        let d = d.min(self.display.len());
        self.blocks
            .iter()
            .find(|b| d >= b.display.start && d < b.display.end)
            .or_else(|| {
                self.blocks
                    .iter()
                    .rev()
                    .find(|b| d >= b.display.start && d <= b.display.end)
            })
    }

    pub fn block_at_source(&self, s: usize) -> Option<&ProjBlock> {
        let s = s.min(self.source_len);
        self.blocks
            .iter()
            .find(|b| s >= b.source.start && s < b.source.end)
            .or_else(|| {
                self.blocks
                    .iter()
                    .rev()
                    .find(|b| s >= b.source.start && s <= b.source.end)
            })
    }

    pub fn link_at(&self, d: usize) -> Option<(Range<usize>, &str)> {
        let d = d.min(self.display.len());
        let seg = self
            .segments
            .iter()
            .find(|s| d >= s.display.start && d <= s.display.end && s.marks.link.is_some())?;
        let id = seg.marks.link?;
        let url = self.links.get(id as usize)?;
        // Expand contiguous segments sharing this link id
        let mut start = seg.display.start;
        let mut end = seg.display.end;
        for s in &self.segments {
            if s.marks.link == Some(id) {
                start = start.min(s.display.start);
                end = end.max(s.display.end);
            }
        }
        Some((start..end, url.as_str()))
    }

    pub fn marks_at(&self, d: usize, affinity: Affinity) -> Marks {
        let d = d.min(self.display.len());
        match affinity {
            Affinity::Inside => self
                .segments
                .iter()
                .rev()
                .find(|s| d >= s.display.start && d <= s.display.end)
                .map(|s| s.marks)
                .unwrap_or_default(),
            Affinity::Outside => self
                .segments
                .iter()
                .find(|s| d >= s.display.start && d < s.display.end)
                .or_else(|| self.segments.iter().find(|s| d == s.display.start))
                .map(|s| s.marks)
                .unwrap_or_default(),
        }
    }

    pub fn to_display(&self, src: usize) -> usize {
        let src = src.min(self.source_len);
        if self.segments.is_empty() {
            return 0;
        }
        for (i, seg) in self.segments.iter().enumerate() {
            if src < seg.source.start {
                return seg.display.start;
            }
            if src <= seg.source.end {
                if seg.source.len() == seg.display.len() && !seg.source.is_empty() {
                    let delta = src - seg.source.start;
                    return (seg.display.start + delta).min(seg.display.end);
                }
                if src == seg.source.end {
                    return seg.display.end;
                }
                return seg.display.start;
            }
            let next_start = self
                .segments
                .get(i + 1)
                .map(|s| s.source.start)
                .unwrap_or(self.source_len);
            if src < next_start {
                return seg.display.end;
            }
        }
        self.display.len()
    }

    fn seg_is_break(seg: &Segment, display: &str) -> bool {
        display
            .get(seg.display.clone())
            .map(|s| s.starts_with('\n'))
            .unwrap_or(false)
    }

    pub fn to_source(&self, d: usize, affinity: Affinity) -> usize {
        let d = d.min(self.display.len());
        if self.segments.is_empty() {
            return 0;
        }
        for (i, seg) in self.segments.iter().enumerate() {
            if d < seg.display.start {
                return seg.source.start;
            }
            if d <= seg.display.end {
                let at_end = d == seg.display.end;
                if at_end {
                    if let Some(next) = self.segments.get(i + 1) {
                        if next.display.start == d {
                            if Self::seg_is_break(next, &self.display) {
                                return if seg.display.is_empty() {
                                    seg.source.start
                                } else {
                                    seg.source.end
                                };
                            }
                            if affinity == Affinity::Inside && seg.marks.any() && !next.marks.any()
                            {
                                return seg.source.end;
                            }
                            if affinity == Affinity::Outside && seg.marks.any() {
                                return next.source.start;
                            }
                            continue;
                        }
                    }
                    if affinity == Affinity::Outside && seg.marks.any() {
                        if let Some(next) = self.segments.get(i + 1) {
                            if !Self::seg_is_break(next, &self.display) {
                                return next.source.start;
                            }
                        }
                        if let Some(block) = self.block_at_display(seg.display.start) {
                            return block.source.end.min(self.source_len);
                        }
                        return seg.source.end;
                    }
                    if seg.display.is_empty() {
                        return seg.source.start;
                    }
                    return seg.source.end;
                }
                if seg.display.len() == seg.source.len() && !seg.display.is_empty() {
                    let delta = d - seg.display.start;
                    return (seg.source.start + delta).min(seg.source.end);
                }
                if d == seg.display.start {
                    return seg.source.start;
                }
                return seg.source.end;
            }
        }
        self.source_len
    }

    /// Source range covering a display range. Fully-selected marked runs expand
    /// to include their delimiters so deleting bold text also drops `**`.
    pub fn display_range_to_source(&self, range: Range<usize>, affinity: Affinity) -> Range<usize> {
        let start_d = range.start.min(self.display.len());
        let end_d = range.end.min(self.display.len()).max(start_d);
        if start_d == end_d {
            let s = self.to_source(start_d, affinity);
            return s..s;
        }
        let mut src_start = self.to_source(start_d, Affinity::Outside);
        let mut src_end = self.to_source(end_d, Affinity::Inside);
        for seg in &self.segments {
            if seg.display.start >= start_d && seg.display.end <= end_d && seg.marks.any() {
                src_start = src_start.min(self.opener_start(seg));
                src_end = src_end.max(self.closer_end(seg));
            }
        }
        if src_end < src_start {
            src_end = src_start;
        }
        src_start..src_end
    }

    fn opener_start(&self, seg: &Segment) -> usize {
        let prev_end = self
            .segments
            .iter()
            .rev()
            .find(|s| s.display.end <= seg.display.start)
            .map(|s| s.source.end)
            .unwrap_or(seg.source.start);
        prev_end.min(seg.source.start)
    }

    fn closer_end(&self, seg: &Segment) -> usize {
        let next_start = self
            .segments
            .iter()
            .find(|s| s.display.start >= seg.display.end)
            .map(|s| s.source.start)
            .unwrap_or(seg.source.end);
        next_start.max(seg.source.end)
    }

    pub fn list_item_at(&self, d: usize) -> Option<(&ProjBlock, &ListItem)> {
        let block = self.block_at_display(d)?;
        let BlockExtra::List { items, .. } = &block.extra else {
            return None;
        };
        let item = items
            .iter()
            .find(|it| d >= it.display.start && d <= it.display.end)?;
        Some((block, item))
    }

    pub fn table_cell_at(&self, d: usize) -> Option<(&ProjBlock, &TableCell)> {
        let block = self.block_at_display(d)?;
        let BlockExtra::Table { cells, .. } = &block.extra else {
            return None;
        };
        let cell = cells
            .iter()
            .find(|c| d >= c.display.start && d <= c.display.end)?;
        Some((block, cell))
    }
}

pub fn project(src: &str) -> Projection {
    let ranges = parse_ranges(src);
    let mut display = String::new();
    let mut segments = Vec::new();
    let mut blocks = Vec::new();
    let mut links = Vec::new();
    let mut prev_end = 0usize;

    for (i, r) in ranges.iter().enumerate() {
        if r.is_blank(src) {
            let last = i + 1 == ranges.len();
            let nls = src[r.range.clone()].bytes().filter(|&b| b == b'\n').count();
            // Skip only Raw soft gaps between blocks (a single newline separator).
            // Trailing blanks, multi-newline blanks, and explicit blank Paragraph
            // placeholders (e.g. NBSP used to break lists) become empty slots.
            if matches!(r.kind, BlockKind::Raw) && !(last || nls >= 2) {
                prev_end = r.range.end;
                continue;
            }
            // One visible empty paragraph per blank-line slot so Enter/o can
            // grow a stack of empties instead of collapsing into one.
            // trailing: nls empties; internal: nls-1 empties (nls>=2).
            let count = if last {
                nls.max(1)
            } else {
                nls.saturating_sub(1).max(1)
            };
            for j in 0..count {
                if !blocks.is_empty() {
                    let d0 = display.len();
                    display.push('\n');
                    let gap_start = if j == 0 {
                        prev_end.min(src.len())
                    } else {
                        (r.range.start + j).min(r.range.end)
                    };
                    let gap_end = (r.range.start + j).min(r.range.end);
                    segments.push(Segment {
                        display: d0..display.len(),
                        source: gap_start..gap_end.max(gap_start),
                        marks: Marks::default(),
                    });
                }
                let slot_start = (r.range.start + j).min(r.range.end);
                let slot_end = if j + 1 == count {
                    r.range.end
                } else {
                    (r.range.start + j + 1).min(r.range.end)
                };
                let src_range = slot_start..slot_end.max(slot_start);
                let d0 = display.len();
                let at = empty_insert_point(src, src_range.clone());
                segments.push(Segment {
                    display: d0..d0,
                    source: at..at,
                    marks: Marks::default(),
                });
                blocks.push(ProjBlock {
                    kind: BlockKind::Paragraph,
                    source: src_range,
                    display: d0..d0,
                    extra: BlockExtra::Text,
                });
            }
            prev_end = r.range.end;
            continue;
        }
        if !blocks.is_empty() {
            let d0 = display.len();
            display.push('\n');
            segments.push(Segment {
                display: d0..display.len(),
                source: prev_end.min(src.len())..r.range.start.min(src.len()),
                marks: Marks::default(),
            });
        }
        let _ = i;
        let d0 = display.len();
        let extra = project_block(src, r, &mut display, &mut segments, &mut links);
        if display.len() == d0 && extra.is_atomic() {
            let s0 = display.len();
            display.push('\u{00A0}');
            segments.push(Segment {
                display: s0..display.len(),
                source: r.range.clone(),
                marks: Marks::default(),
            });
        }
        if display.len() == d0 {
            segments.push(Segment {
                display: d0..d0,
                source: content_point(src, r),
                marks: Marks::default(),
            });
        }
        blocks.push(ProjBlock {
            kind: r.kind,
            source: r.range.clone(),
            display: d0..display.len(),
            extra,
        });
        prev_end = r.range.end;
    }

    if blocks.is_empty() {
        blocks.push(ProjBlock {
            kind: BlockKind::Paragraph,
            source: 0..src.len(),
            display: 0..0,
            extra: BlockExtra::Text,
        });
        segments.push(Segment {
            display: 0..0,
            source: 0..src.len(),
            marks: Marks::default(),
        });
    }

    Projection {
        display,
        segments,
        blocks,
        links,
        source_len: src.len(),
    }
}

fn empty_insert_point(src: &str, range: Range<usize>) -> usize {
    let slice = src.get(range.clone()).unwrap_or("");
    if let Some(i) = slice.find('\n') {
        (range.start + i + 1).min(range.end)
    } else {
        range.start
    }
}

fn content_point(src: &str, r: &PaintRange) -> Range<usize> {
    let slice = r.slice(src);
    match r.kind {
        BlockKind::Heading(_) => {
            let (_, text) = notion::strip_heading(slice);
            let start = r.range.end.saturating_sub(text.len()).min(src.len());
            start..start
        }
        BlockKind::Quote | BlockKind::Alert(_) => {
            let at = quote_body_point(slice, r.range.start, matches!(r.kind, BlockKind::Alert(_)));
            at..at
        }
        _ => {
            let p = r.range.start.min(src.len());
            p..p
        }
    }
}

fn quote_prefix_len(line: &str) -> usize {
    if line.starts_with("> ") {
        2
    } else if line.starts_with('>') {
        1
    } else {
        0
    }
}

/// Byte offset in `src` where quote/alert body text should be inserted.
fn quote_body_point(slice: &str, abs_start: usize, skip_alert_label: bool) -> usize {
    let mut offset = 0usize;
    if skip_alert_label {
        if let Some(first) = slice.lines().next() {
            if first.contains("[!") && first.contains(']') {
                offset += first.len();
                if slice.as_bytes().get(first.len()) == Some(&b'\n') {
                    offset += 1;
                }
            }
        }
    }
    let rest = slice.get(offset..).unwrap_or("");
    let line = rest.split('\n').next().unwrap_or("");
    (abs_start + offset + quote_prefix_len(line)).min(abs_start + slice.len())
}

/// Semantic HTML: single-line `<h1>`–`<h6>` render as headings,
/// `<details>` opens a disclosure row, `</details>` is invisible chrome.
/// Returns None for anything else (generic HTML card).
fn project_html_block(
    src: &str,
    r: &PaintRange,
    display: &mut String,
    segments: &mut Vec<Segment>,
) -> Option<BlockExtra> {
    let slice = r.slice(src);
    let lead = slice.len() - slice.trim_start().len();
    let t = slice[lead..].trim_end();
    if !t.starts_with('<') {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    // `<h1>text</h1>` (attributes allowed on the open tag).
    if let Some(level) = html_heading_level(&lower) {
        let gt = t.find('>')?;
        let close = format!("</h{level}>");
        if !lower.ends_with(&close) {
            return None;
        }
        let inner = t[gt + 1..t.len() - close.len()].trim();
        let start = r.range.start + lead + gt + 1 + (t[gt + 1..].len() - t[gt + 1..].trim_start().len());
        let end = start + inner.len();
        emit_plain(display, segments, start..end.min(src.len()), inner, Marks::default());
        restore_trailing_spaces(src, r.range.clone(), display, segments);
        return Some(BlockExtra::HtmlHeading(level));
    }
    // `<details …>` open tag: disclosure row showing the `<summary>`.
    // GFM: `<details open>` renders expanded by default, otherwise collapsed.
    // An empty `<summary></summary>` projects as empty text so the editor
    // can show a "Summary" placeholder instead of baked-in content.
    if lower.starts_with("<details") && lower.as_bytes().get(8).is_some_and(|b| matches!(b, b'>' | b' ' | b'\t' | b'\n')) {
        let summary = html_summary_text(t, &lower).unwrap_or_default();
        let range = html_summary_range(t, &lower)
            .map(|(s, e)| r.range.start + lead + s..r.range.start + lead + e);
        // Empty summary: emit nothing (placeholder shows); editing maps
        // into the `<summary>…</summary>` inner range when present.
        if summary.is_empty() {
            if let Some(rg) = range {
                // Zero-width anchor inside the summary tags so the caret
                // lands there and typing fills the summary.
                let pos = rg.start.min(src.len());
                segments.push(Segment {
                    display: display.len()..display.len(),
                    source: pos..pos,
                    marks: Marks::default(),
                });
            }
        } else {
            let abs = range.unwrap_or_else(|| r.range.clone());
            emit_plain(display, segments, abs, &summary, Marks::default());
        }
        return Some(BlockExtra::Details { summary, open: html_details_open(t, &lower) });
    }
    // `</details>` close tag: zero-height chrome.
    if lower.starts_with("</details") {
        let rest = lower["</details".len()..].trim();
        if rest.is_empty() || rest == ">" || rest.starts_with('>') {
            return Some(BlockExtra::DetailsClose);
        }
    }
    None
}

/// `<h1` … `<h6` (open-tag prefix) → level. `None` for anything else.
fn html_heading_level(lower_trimmed: &str) -> Option<u8> {
    let b = lower_trimmed.as_bytes();
    if b.len() < 4 || b[0] != b'<' || b[1] != b'h' {
        return None;
    }
    if !(b'1'..=b'6').contains(&b[2]) {
        return None;
    }
    match b[3] {
        b'>' | b' ' | b'\t' | b'\n' => Some(b[2] - b'0'),
        _ => None,
    }
}

/// Inner text of the first `<summary>…</summary>`, whitespace-collapsed.
fn html_summary_text(t: &str, lower: &str) -> Option<String> {
    let (s, e) = html_summary_range(t, lower)?;
    let raw = t.get(s..e)?;
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then(|| collapsed)
}

/// `true` when the `<details …>` open tag carries a standalone `open`
/// attribute (`<details open>`, `<details open="">`, `<details open="open">`).
/// Mirrors GitHub's "open by default" switch.
fn html_details_open(_t: &str, lower: &str) -> bool {
    let after = "<details".len();
    let Some(gt_rel) = lower[after..].find('>') else {
        return false;
    };
    let attrs = &lower[after..after + gt_rel];
    attrs.split(|c: char| c.is_whitespace() || c == '/' || c == '=' || c == '"' || c == '\'').any(|tok| tok == "open")
}

/// Block index range owned by the `<details>` open row at `ix`
/// (`open..=close`, depth-counted for nesting). None when not a Details row
/// or when never closed.
pub fn details_block_range(p: &Projection, ix: usize) -> Option<(usize, usize)> {
    if !matches!(p.blocks.get(ix)?.extra, BlockExtra::Details { .. }) {
        return None;
    }
    let mut depth = 0usize;
    for (j, b) in p.blocks.iter().enumerate().skip(ix) {
        match &b.extra {
            BlockExtra::Details { .. } => depth += 1,
            BlockExtra::DetailsClose => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((ix, j));
                }
            }
            _ => {}
        }
    }
    None
}

/// Byte range of the summary inner text within `t` (`lower` mirrors `t`).
fn html_summary_range(t: &str, lower: &str) -> Option<(usize, usize)> {
    let open = lower.find("<summary")?;
    let after = open + "<summary".len();
    // Skip attributes on `<summary …>`.
    let gt = lower[after..].find('>')? + after;
    let close = lower[gt..].find("</summary>")? + gt;
    Some((gt + 1, close))
}

/// Leading indent (spaces/tabs, tabs count as 2) of the fence line.
/// Split-out nested code keeps its visual nesting; top-level is 0.
fn code_fence_indent(src: &str, range_start: usize, slice: &str) -> usize {
    let first_line = slice.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut n = first_line.len() - first_line.trim_start_matches([' ', '\t']).len();
    // `slice` may start mid-line (split out of a list); include the column
    // offset of the range start for the true visual indent.
    if range_start > 0 {
        let line_start = src[..range_start.min(src.len())].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let prefix = src.get(line_start..range_start.min(src.len())).unwrap_or("");
        if !prefix.contains('\n') {
            n += prefix.chars().filter(|c| *c == ' ').count() + prefix.chars().filter(|c| *c == '\t').count() * 2;
        }
    }
    n.min(12)
}

/// Strip the common leading indent (spaces/tabs) shared by all non-empty
/// lines. Split-out list code arrives indented; top-level code has none.
fn dedent_code(body: &str) -> String {    if !body.contains('\n') {
        return body.trim_start_matches([' ', '\t']).to_string();
    }
    let indent = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start_matches([' ', '\t']).len())
        .min()
        .unwrap_or(0);
    if indent == 0 {
        return body.to_string();
    }
    body.lines()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                l.char_indices()
                    .nth(indent)
                    .map(|(i, _)| l[i..].to_string())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn project_block(
    src: &str,
    r: &PaintRange,
    display: &mut String,
    segments: &mut Vec<Segment>,
    links: &mut Vec<String>,
) -> BlockExtra {
    let slice = r.slice(src);
    if r.is_blank(src) {
        return BlockExtra::Text;
    }
    if let Some((alt, img_src)) = sole_image(slice) {
        return BlockExtra::Image { alt, src: img_src };
    }
    match r.kind {
        BlockKind::Rule => BlockExtra::Rule,
        BlockKind::Html | BlockKind::Raw => {
            if let Some(extra) = project_html_block(src, r, display, segments) {
                extra
            } else {
                emit_plain(display, segments, r.range.clone(), slice, Marks::default());
                BlockExtra::Html
            }
        }
        BlockKind::Code => {
            let (lang, body) = notion::strip_fence(slice);
            let abs = body_abs_range(slice, &body, r.range.start);
            // Fences split out of list items carry the list indent
            // ("  ```sh\n  code\n  ```"). Dedent for display but remember
            // the fence indent so the block renders nested and the file
            // preserves it on edit.
            let indent = code_fence_indent(src, r.range.start, slice);
            let body = dedent_code(&body);
            emit_plain(display, segments, abs, &body, Marks::default());
            BlockExtra::Code { lang, indent }
        }
        BlockKind::Table => project_table(src, r, display, segments, links),
        BlockKind::List { ordered } => {
            let items = project_inlines(src, r, display, segments, links);
            BlockExtra::List {
                ordered,
                items: items.unwrap_or_default(),
            }
        }
        BlockKind::Heading(level) => {
            project_inlines(src, r, display, segments, links);
            restore_trailing_spaces(src, r.range.clone(), display, segments);
            restore_heading_suffix(src, r.range.clone(), display, segments);
            BlockExtra::Heading(level)
        }
        BlockKind::Quote => {
            project_inlines(src, r, display, segments, links);
            restore_trailing_spaces(src, r.range.clone(), display, segments);
            BlockExtra::Quote
        }
        BlockKind::Alert(kind) => {
            project_inlines(src, r, display, segments, links);
            restore_trailing_spaces(src, r.range.clone(), display, segments);
            BlockExtra::Alert(kind)
        }
        BlockKind::Paragraph => {
            // Incomplete list/heading markers must show as literal text until
            // the user types the trailing space that Notion uses to confirm.
            if crate::document::is_incomplete_list_marker(slice)
                || crate::document::is_incomplete_heading_marker(slice)
            {
                let line = slice.trim_end_matches(['\n', '\r']);
                let abs = r.range.start..r.range.start + line.len();
                emit_plain(display, segments, abs, line, Marks::default());
            } else {
                project_inlines(src, r, display, segments, links);
                restore_trailing_spaces(src, r.range.clone(), display, segments);
            }
            BlockExtra::Text
        }
    }
}

/// pulldown-cmark drops trailing spaces from `Text` events; keep them so the
/// caret can sit after a space at end of a block.
fn restore_trailing_spaces(
    src: &str,
    block: Range<usize>,
    display: &mut String,
    segments: &mut Vec<Segment>,
) {
    let slice = src.get(block.clone()).unwrap_or("");
    let trimmed = slice.trim_end_matches(['\n', '\r']);
    let content_end = block.start + trimmed.len();
    let last = segments
        .iter()
        .rev()
        .find(|s| s.source.end <= content_end && s.source.start >= block.start)
        .map(|s| s.source.end)
        .unwrap_or(block.start);
    if last >= content_end {
        return;
    }
    let gap = &src[last..content_end];
    if !gap.is_empty() && gap.bytes().all(|b| b == b' ' || b == b'\t') {
        emit_plain(display, segments, last..content_end, gap, Marks::default());
    }
}

/// ATX closers (`# Title #`) and spaces before them are dropped by cmark.
/// Restore the raw heading-body suffix so `#` / spaces typed at EOL stay visible.
fn restore_heading_suffix(
    src: &str,
    block: Range<usize>,
    display: &mut String,
    segments: &mut Vec<Segment>,
) {
    let slice = src.get(block.clone()).unwrap_or("");
    let (_, expected) = notion::strip_heading(slice);
    let expected = expected.trim_end_matches(['\n', '\r']);
    let shown_from = {
        let mut start = display.len();
        let mut found = false;
        for seg in segments.iter().rev() {
            if seg.source.start >= block.start && seg.source.end <= block.end {
                start = seg.display.start.min(start);
                found = true;
            } else if found {
                break;
            }
        }
        if found {
            start
        } else {
            display.len()
        }
    };
    let shown = display.get(shown_from..).unwrap_or("");
    if !expected.starts_with(shown) {
        return;
    }
    let suffix = &expected[shown.len()..];
    if suffix.is_empty() {
        return;
    }
    let line = slice.trim_end_matches(['\n', '\r']);
    let hashes = line.chars().take_while(|c| *c == '#').count();
    let mut body_at = hashes;
    if line.as_bytes().get(body_at) == Some(&b' ') {
        body_at += 1;
    }
    let body_src_start = block.start + body_at;
    let suffix_start = body_src_start + shown.len();
    let suffix_end = (suffix_start + suffix.len()).min(block.start + line.len());
    if suffix_end <= suffix_start {
        return;
    }
    let gap = &src[suffix_start..suffix_end];
    emit_plain(
        display,
        segments,
        suffix_start..suffix_end,
        gap,
        Marks::default(),
    );
}

/// pulldown-cmark drops trailing spaces on list item text, and may hide bodies
/// that look like nested markers (`- -`) or escapes. Restore the raw body.
fn restore_list_item_trailing_spaces(
    src: &str,
    item_source: &Range<usize>,
    item_d0: usize,
    display: &mut String,
    segments: &mut Vec<Segment>,
    checked: Option<bool>,
) {
    let slice = src.get(item_source.clone()).unwrap_or("");
    let line = slice.trim_end_matches(['\n', '\r']);
    let marker = list_marker_byte_len(line, checked);
    if marker > line.len() {
        return;
    }
    let body = &line[marker..];
    let shown = display.get(item_d0..).unwrap_or("");
    // Unescaped body for visible text (keep source mapping on the raw slice).
    let visible = unescape_md_punct(body);
    if shown.is_empty() && !visible.trim().is_empty() {
        emit_plain(
            display,
            segments,
            item_source.start + marker..item_source.start + marker + body.len(),
            &visible,
            Marks::default(),
        );
        return;
    }
    if !body.starts_with(shown) && !visible.starts_with(shown) {
        return;
    }
    // Trailing spaces only.
    if body.starts_with(shown) {
        let suffix = &body[shown.len()..];
        if suffix.is_empty() || !suffix.bytes().all(|b| b == b' ' || b == b'\t') {
            return;
        }
        let src_start = item_source.start + marker + shown.len();
        let src_end = src_start + suffix.len();
        emit_plain(
            display,
            segments,
            src_start..src_end,
            suffix,
            Marks::default(),
        );
    }
}

fn unescape_md_punct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            } else {
                out.push('\\');
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn list_marker_byte_len(line: &str, checked: Option<bool>) -> usize {
    let indent = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    let t = &line[indent..];
    let marker = if checked.is_some() {
        if t.starts_with("- [ ] ") || t.starts_with("- [x] ") || t.starts_with("- [X] ") {
            6
        } else if t.starts_with("* [ ] ") || t.starts_with("* [x] ") || t.starts_with("* [X] ") {
            6
        } else {
            // Task marker without trailing space yet
            5
        }
    } else if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        2
    } else {
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && (t[digits..].starts_with(". ") || t[digits..].starts_with(") ")) {
            digits + 2
        } else {
            0
        }
    };
    indent + marker
}

fn body_abs_range(slice: &str, body: &str, abs: usize) -> Range<usize> {
    if body.is_empty() {
        let (lang, _) = notion::strip_fence(slice);
        let prefix = if lang.is_empty() {
            4
        } else {
            3 + lang.len() + 1
        };
        let start = (abs + prefix).min(abs + slice.len());
        return start..start;
    }
    if let Some(i) = slice.find(body) {
        let start = abs + i;
        start..start + body.len()
    } else {
        abs..abs + slice.len()
    }
}

fn emit_plain(
    display: &mut String,
    segments: &mut Vec<Segment>,
    source: Range<usize>,
    text: &str,
    marks: Marks,
) {
    if text.is_empty() {
        segments.push(Segment {
            display: display.len()..display.len(),
            source,
            marks,
        });
        return;
    }
    let d0 = display.len();
    display.push_str(text);
    segments.push(Segment {
        display: d0..display.len(),
        source: source.start..source.start + text.len(),
        marks,
    });
}

struct OpenListItem {
    index: usize,
    d0: usize,
    src_start: usize,
    indent: usize,
    checked: Option<bool>,
    /// Frozen when a nested list starts so parent text does not swallow children.
    display_end: Option<usize>,
}

fn project_inlines(
    src: &str,
    r: &PaintRange,
    display: &mut String,
    segments: &mut Vec<Segment>,
    links: &mut Vec<String>,
) -> Option<Vec<ListItem>> {
    let slice = r.slice(src);
    let parser = Parser::new_ext(slice, gfm_options()).into_offset_iter();
    let mut marks = Marks::default();
    let mut items: Vec<Option<ListItem>> = Vec::new();
    let mut stack: Vec<OpenListItem> = Vec::new();
    let mut saw_list = false;
    let mut skip_alert_label = matches!(r.kind, BlockKind::Alert(_));
    let mut skip_alert_break = false;
    let mut after_item = false;

    for (event, range) in parser {
        let abs = r.range.start + range.start..r.range.start + range.end;
        match event {
            Event::Start(Tag::List(_)) => {
                saw_list = true;
                if let Some(open) = stack.last_mut() {
                    if open.display_end.is_none() {
                        open.display_end = Some(display.len());
                    }
                }
            }
            Event::Start(Tag::Item) => {
                if after_item {
                    let d0 = display.len();
                    display.push('\n');
                    segments.push(Segment {
                        display: d0..display.len(),
                        source: abs.start..abs.start,
                        marks: Marks::default(),
                    });
                }
                after_item = true;
                let index = items.len();
                items.push(None);
                stack.push(OpenListItem {
                    index,
                    d0: display.len(),
                    src_start: abs.start,
                    indent: list_indent_at(src, abs.start),
                    checked: None,
                    display_end: None,
                });
            }
            Event::End(TagEnd::Item) => {
                if let Some(open) = stack.pop() {
                    let src_end = r.range.start + range.end;
                    let first_line_end = src
                        .get(open.src_start..src_end)
                        .and_then(|s| s.find('\n'))
                        .map(|i| open.src_start + i)
                        .unwrap_or(src_end);
                    let restore_src = open.src_start..first_line_end;
                    if open.display_end.is_none() {
                        restore_list_item_trailing_spaces(
                            src,
                            &restore_src,
                            open.d0,
                            display,
                            segments,
                            open.checked,
                        );
                    }
                    let end = open.display_end.unwrap_or(display.len());
                    if let Some(slot) = items.get_mut(open.index) {
                        *slot = Some(ListItem {
                            display: open.d0..end,
                            source: open.src_start..src_end,
                            indent: open.indent,
                            checked: open.checked,
                        });
                    }
                }
            }
            Event::TaskListMarker(checked) => {
                if let Some(open) = stack.last_mut() {
                    open.checked = Some(checked);
                }
            }
            Event::Start(Tag::Strong) => marks.bold = true,
            Event::End(TagEnd::Strong) => marks.bold = false,
            Event::Start(Tag::Emphasis) => marks.italic = true,
            Event::End(TagEnd::Emphasis) => marks.italic = false,
            Event::Start(Tag::Strikethrough) => marks.strike = true,
            Event::End(TagEnd::Strikethrough) => marks.strike = false,
            Event::Start(Tag::Link { dest_url, .. }) => {
                let id = links.len() as u32;
                links.push(dest_url.to_string());
                marks.link = Some(id);
            }
            Event::End(TagEnd::Link) => marks.link = None,
            Event::Start(Tag::CodeBlock(_)) => {
                // Fenced code nested in a list item (e.g. an indented ``` block
                // under a bullet) still belongs to the List block — mark it as
                // code so it renders mono + pill instead of plain text.
                marks.code = true;
                if !display.is_empty() && !display.ends_with('\n') {
                    let d0 = display.len();
                    display.push('\n');
                    segments.push(Segment {
                        display: d0..display.len(),
                        source: abs.start..abs.start,
                        marks: Marks::default(),
                    });
                }
            }
            Event::End(TagEnd::CodeBlock) => marks.code = false,
            Event::Start(Tag::Heading { .. }) => {
                skip_alert_label = false;
            }
            Event::Start(Tag::BlockQuote(_)) if skip_alert_label => {}
            Event::Text(t) => {
                if skip_alert_label && is_alert_label(&t) {
                    skip_alert_label = false;
                    skip_alert_break = true;
                    continue;
                }
                skip_alert_label = false;
                let text = &slice[range.start.min(slice.len())..range.end.min(slice.len())];
                emit_plain(display, segments, abs, text, marks);
            }
            Event::Code(t) => {
                let text = t.as_ref();
                let inner = if range.end - range.start >= 2 {
                    r.range.start + range.start + 1..r.range.start + range.end - 1
                } else {
                    abs.clone()
                };
                emit_plain(
                    display,
                    segments,
                    inner,
                    text,
                    Marks {
                        code: true,
                        ..marks
                    },
                );
            }
            Event::SoftBreak | Event::HardBreak => {
                if skip_alert_label || skip_alert_break {
                    skip_alert_break = false;
                    continue;
                }
                let d0 = display.len();
                display.push('\n');
                segments.push(Segment {
                    display: d0..display.len(),
                    source: abs,
                    marks,
                });
            }
            Event::InlineHtml(t) | Event::Html(t) => {
                let tag = t.as_ref().trim();
                let lower = tag.to_ascii_lowercase();
                if lower == "<u>" || lower == "<u/>" {
                    marks.underline = true;
                } else if lower == "</u>" {
                    marks.underline = false;
                } else {
                    emit_plain(display, segments, abs, t.as_ref(), Marks::default());
                }
            }
            _ => {}
        }
    }

    if saw_list {
        Some(items.into_iter().flatten().collect())
    } else {
        None
    }
}

fn is_alert_label(t: &str) -> bool {
    let inner = t
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_start_matches('!');
    matches!(
        inner.to_ascii_uppercase().as_str(),
        "NOTE" | "TIP" | "IMPORTANT" | "WARNING" | "CAUTION"
    )
}

fn list_indent_at(src: &str, abs: usize) -> usize {
    let abs = abs.min(src.len());
    let line_start = src[..abs].rfind('\n').map(|i| i + 1).unwrap_or(0);
    list_indent(&src[line_start..])
}

fn list_indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count() / 2
}

/// Line separator used inside table cells in the display string so real `\n`
/// can stay the row separator. ASCII `\x1e` (record separator) is 1 byte so
/// caret math never lands inside a multi-byte char.
pub(crate) const TABLE_CELL_BR: char = '\u{001e}';

/// Flatten CR/LF inside a table cell so the display projection can keep using
/// `\n` as a row separator and `\t` as a cell separator.
pub(crate) fn flatten_table_cell_text(s: &str) -> Cow<'_, str> {
    flatten_table_cell_display(s)
}

pub(crate) fn flatten_table_cell_display(s: &str) -> Cow<'_, str> {
    if s.bytes().any(|b| b == b'\n' || b == b'\r') {
        Cow::Owned(s.replace('\n', "\u{001e}").replace('\r', "\u{001e}"))
    } else {
        Cow::Borrowed(s)
    }
}

pub(crate) fn flatten_table_cell_gfm(s: &str) -> Cow<'_, str> {
    if !s.contains(['\n', '\r', TABLE_CELL_BR]) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.replace('\n', "<br>")
            .replace('\r', "")
            .replace(TABLE_CELL_BR, "<br>"),
    )
}

pub fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn project_table(
    src: &str,
    r: &PaintRange,
    display: &mut String,
    segments: &mut Vec<Segment>,
    links: &mut Vec<String>,
) -> BlockExtra {
    let slice = r.slice(src);
    let mut cells = Vec::new();
    let mut row = 0usize;
    let mut col = 0usize;
    let mut cols = 0usize;
    let mut header = true;
    let mut cell_d0 = 0usize;
    let mut cell_src = 0..0;
    let mut marks = Marks::default();
    let parser = Parser::new_ext(slice, gfm_options()).into_offset_iter();

    for (event, range) in parser {
        let abs = r.range.start + range.start..r.range.start + range.end;
        match event {
            Event::Start(Tag::TableHead) => {
                header = true;
                row = 0;
                col = 0;
            }
            Event::Start(Tag::TableRow) => {
                if row > 0 || !cells.is_empty() {
                    let d0 = display.len();
                    display.push('\n');
                    segments.push(Segment {
                        display: d0..display.len(),
                        source: abs.start..abs.start,
                        marks: Marks::default(),
                    });
                }
                col = 0;
            }
            Event::Start(Tag::TableCell) => {
                if col > 0 {
                    let d0 = display.len();
                    display.push('\t');
                    segments.push(Segment {
                        display: d0..display.len(),
                        source: abs.start..abs.start,
                        marks: Marks::default(),
                    });
                }
                cell_d0 = display.len();
                cell_src = abs;
                marks = Marks::default();
            }
            Event::Start(Tag::Strong) => marks.bold = true,
            Event::End(TagEnd::Strong) => marks.bold = false,
            Event::Start(Tag::Emphasis) => marks.italic = true,
            Event::End(TagEnd::Emphasis) => marks.italic = false,
            Event::Start(Tag::Strikethrough) => marks.strike = true,
            Event::End(TagEnd::Strikethrough) => marks.strike = false,
            Event::Start(Tag::Link { dest_url, .. }) => {
                let id = links.len() as u32;
                links.push(dest_url.to_string());
                marks.link = Some(id);
            }
            Event::End(TagEnd::Link) => marks.link = None,
            Event::Code(t) => {
                let text = flatten_table_cell_text(t.as_ref());
                if !text.trim().is_empty() || !t.is_empty() {
                    emit_plain(
                        display,
                        segments,
                        abs,
                        text.as_ref(),
                        Marks {
                            code: true,
                            ..marks
                        },
                    );
                }
            }
            Event::Text(t) => {
                let text = &slice[range.start.min(slice.len())..range.end.min(slice.len())];
                let text = flatten_table_cell_text(text);
                if !text.trim().is_empty() || !t.is_empty() {
                    emit_plain(display, segments, abs, text.as_ref(), marks);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                // In-cell breaks must not use `\n` (that's the row separator).
                emit_plain(display, segments, abs, "\u{001e}", Marks::default());
            }
            Event::Html(t) | Event::InlineHtml(t) => {
                let lower = t.as_ref().trim().to_ascii_lowercase();
                if matches!(lower.as_str(), "<br>" | "<br/>" | "<br />") {
                    emit_plain(display, segments, abs, "\u{001e}", Marks::default());
                }
            }
            Event::End(TagEnd::TableCell) => {
                cells.push(TableCell {
                    display: cell_d0..display.len(),
                    source: cell_src.clone(),
                    header,
                    row,
                    col,
                });
                col += 1;
                cols = cols.max(col);
            }
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => {
                row += 1;
                header = false;
            }
            _ => {}
        }
    }

    BlockExtra::Table {
        cells,
        rows: row,
        cols,
    }
}

pub fn serialize_table(headers: &[String], rows: &[Vec<String>]) -> String {
    let cols = headers
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0))
        .max(1);
    let mut out = String::new();
    out.push('|');
    for i in 0..cols {
        out.push(' ');
        out.push_str(&flatten_table_cell_gfm(
            headers.get(i).map(|s| s.as_str()).unwrap_or(""),
        ));
        out.push_str(" |");
    }
    out.push('\n');
    out.push('|');
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    for row in rows {
        out.push('\n');
        out.push('|');
        for i in 0..cols {
            out.push(' ');
            out.push_str(&flatten_table_cell_gfm(
                row.get(i).map(|s| s.as_str()).unwrap_or(""),
            ));
            out.push_str(" |");
        }
    }
    out
}

pub const CODE_LANGS: &[&str] = &[
    "",
    "rust",
    "python",
    "javascript",
    "typescript",
    "go",
    "json",
    "html",
    "css",
    "bash",
    "markdown",
    "toml",
    "yaml",
    "sql",
    "c",
    "cpp",
];

pub const COLUMN_PX: f32 = 740.0;

pub fn wrap_cols_for(font_px: u32, wrap: bool) -> Option<usize> {
    if !wrap {
        return None;
    }
    let inner = COLUMN_PX - 64.0;
    let ch = (font_px as f32 * 0.55).max(6.0);
    Some((inner / ch).max(8.0) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_hides_hashes() {
        let p = project("# Hello");
        assert_eq!(p.display, "Hello");
        assert_eq!(p.to_source(0, Affinity::Inside), 2);
        assert_eq!(p.to_display(2), 0);
        assert_eq!(p.to_display(0), 0);
    }

    #[test]
    fn bold_hides_stars() {
        let p = project("hello **world**");
        assert_eq!(p.display, "hello world");
        assert!(p.marks_at(6, Affinity::Inside).bold);
        assert!(!p.marks_at(4, Affinity::Inside).bold);
        let src = p.to_source(6, Affinity::Inside);
        assert_eq!(&"hello **world**"[src..src + 1], "w");
        let outside = p.to_source(p.display.len(), Affinity::Outside);
        assert_eq!(outside, "hello **world**".len());
    }

    #[test]
    fn unclosed_stars_stay_visible() {
        let p = project("hello **world");
        assert!(p.display.contains("**"), "{}", p.display);
    }

    #[test]
    fn code_fence_hidden() {
        let src = "```rust\nfn main() {}\n```";
        let p = project(src);
        assert_eq!(p.display, "fn main() {}");
        match &p.blocks[0].extra {
            BlockExtra::Code { lang, .. } => assert_eq!(lang, "rust"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn list_hides_markers() {
        let p = project("- one\n- two");
        assert_eq!(p.display, "one\ntwo");
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("not list");
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn nested_list_keeps_parent() {
        let src = "- parent\n  - child\n    - grand\n- sibling";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("not list: {:?}", p.blocks[0].extra);
        };
        assert_eq!(items.len(), 4, "items={items:?} display={:?}", p.display);
        assert_eq!(&p.display[items[0].display.clone()], "parent");
        assert_eq!(&p.display[items[1].display.clone()], "child");
        assert_eq!(&p.display[items[2].display.clone()], "grand");
        assert_eq!(&p.display[items[3].display.clone()], "sibling");
        assert_eq!(items[0].indent, 0);
        assert_eq!(items[1].indent, 1);
        assert_eq!(items[2].indent, 2);
        assert_eq!(items[3].indent, 0);
    }

    #[test]
    fn two_blocks_separated() {
        let p = project("# H\n\npara");
        assert_eq!(p.display, "H\npara");
        assert_eq!(p.blocks.len(), 2);
        assert_eq!(p.to_display(0), 0);
        let d = p.to_display("p".len() + p.display.find('p').unwrap_or(2));
        let _ = d;
        assert!(p.display.contains("para"));
    }

    #[test]
    fn table_cells() {
        let src = "| a | b |\n| --- | --- |\n| 1 | 2 |";
        let p = project(src);
        let BlockExtra::Table { cells, cols, .. } = &p.blocks[0].extra else {
            panic!("{:?}", p.blocks[0].extra);
        };
        assert!(*cols >= 2, "{cells:?} display={}", p.display);
        assert!(p.display.contains('a'));
        assert!(p.display.contains('1'));
        assert!(!p.display.contains("---"));
    }

    #[test]
    fn table_multiline_cell_flattens_newlines() {
        assert_eq!(flatten_table_cell_text("foo\nbar"), "foo\u{001e}bar");
        assert_eq!(
            flatten_table_cell_text("foo\r\nbar"),
            "foo\u{001e}\u{001e}bar"
        );
        assert_eq!(flatten_table_cell_text("plain"), "plain");

        let src = "| a | b |\n| --- | --- |\n| 1 | 2 |";
        let p = project(src);
        let BlockExtra::Table { rows, cols, .. } = &p.blocks[0].extra else {
            panic!("{:?}", p.blocks[0].extra);
        };
        assert_eq!(*rows, 2);
        assert_eq!(*cols, 2);
        let table = &p.display[p.blocks[0].display.clone()];
        assert_eq!(
            table.matches('\n').count(),
            1,
            "one row separator: {table:?}"
        );
        assert!(table.contains('\t'), "{table:?}");

        let src = "| a | b |\n| --- | --- |\n| foo<br>bar | 2 |";
        let p = project(src);
        let BlockExtra::Table { rows, cols, .. } = &p.blocks[0].extra else {
            panic!("{:?}", p.blocks[0].extra);
        };
        assert_eq!(*rows, 2);
        assert_eq!(*cols, 2);
        let table = &p.display[p.blocks[0].display.clone()];
        assert_eq!(
            table.matches('\n').count(),
            1,
            "br in a cell is not a row break: {table:?}"
        );
        assert!(table.contains("foo"), "{table:?}");
        assert!(table.contains("bar"), "{table:?}");

        let gfm = serialize_table(&["h".into()], &[vec!["a\nb".into()]]);
        assert_eq!(gfm, "| h |\n| --- |\n| a<br>b |");
    }

    #[test]
    fn wrap_cols_positive() {
        assert!(wrap_cols_for(15, true).unwrap() > 20);
        assert_eq!(wrap_cols_for(15, false), None);
    }

    #[test]
    fn heading_end_stays_in_heading() {
        let src = "# Hello\n\npara";
        let p = project(src);
        let end = p.blocks[0].display.end;
        let off = p.to_source(end, Affinity::Inside);
        assert!(
            off <= p.blocks[0].source.end,
            "mapped to {off} past heading {:?}",
            p.blocks[0].source
        );
        assert_eq!(&src[..off], "# Hello");
        assert_eq!(p.to_display(off), end);
    }

    #[test]
    fn trailing_blank_is_empty_paragraph() {
        let src = "# Hello\n\n";
        let p = project(src);
        assert!(
            p.blocks.len() >= 2,
            "blocks={:?} display={:?}",
            p.blocks.iter().map(|b| b.kind).collect::<Vec<_>>(),
            p.display
        );
        let last = p.blocks.last().unwrap();
        assert_eq!(last.kind, BlockKind::Paragraph);
        assert_eq!(last.display.start, last.display.end);
    }

    #[test]
    fn trailing_spaces_kept() {
        let p = project("hello ");
        assert_eq!(p.display, "hello ");
        let p = project("# Hi ");
        assert_eq!(p.display, "Hi ");
        let p = project("- something ");
        assert_eq!(p.display, "something ");
        let p = project("- [ ] something ");
        assert_eq!(p.display, "something ");
        let p = project("1. something ");
        assert_eq!(p.display, "something ");
    }

    #[test]
    fn empty_alert_caret_is_in_body() {
        let src = "> [!NOTE]\n>";
        let p = project(src);
        assert!(
            matches!(p.blocks[0].kind, BlockKind::Alert(_)),
            "{:?}",
            p.blocks[0].kind
        );
        assert!(
            p.display.trim().is_empty(),
            "label is chrome, not display: {:?}",
            p.display
        );
        let at = p.to_source(p.blocks[0].display.start, Affinity::Inside);
        assert!(
            at > src.find("[!NOTE]").unwrap(),
            "caret after label, got {at} in {src:?}"
        );
        assert!(
            src[..at].ends_with("> ") || src[..at].ends_with('>'),
            "caret on body line: {:?} at={at}",
            &src[..at]
        );
    }

    #[test]
    fn heading_keeps_trailing_hash() {
        let p = project("# Hello there   #");
        assert_eq!(p.display, "Hello there   #");
        let p = project("# Hello there#");
        assert_eq!(p.display, "Hello there#");
    }

    #[test]
    fn table_cell_links_keep_marks() {
        let src = "| Font | Desc |\n| --- | --- |\n| [Zed](https://x.com) | plain |";
        let p = project(src);
        assert_eq!(p.links, vec!["https://x.com".to_string()]);
        let d = p.display.find("Zed").unwrap();
        assert_eq!(p.link_at(d).map(|(_, u)| u), Some("https://x.com"));
        assert!(p.marks_at(d, Affinity::Inside).link.is_some());
    }

    #[test]
    fn table_cell_code_and_bold_keep_marks() {
        let src = "| A |\n| --- |\n| `~/x` and **B** |";
        let p = project(src);
        let d = p.display.find("~/x").unwrap();
        assert!(p.marks_at(d, Affinity::Inside).code, "{:?}", p.display);
        let b = p.display.find('B').unwrap();
        assert!(p.marks_at(b, Affinity::Inside).bold);
    }

    #[test]
    fn nested_fence_in_list_splits_to_code_block() {
        let src = "- Install [Brew](https://brew.sh/)\n\n  ```sh\n  /bin/bash hi\n  ```\n";
        let p = project(src);
        // Bullet stays a list, fence becomes its own Code block with language.
        assert!(matches!(p.blocks[0].extra, BlockExtra::List { .. }), "{:?}", p.blocks[0].extra);
        let code = p.blocks.iter().find(|b| matches!(b.extra, BlockExtra::Code { .. }));
        assert!(code.is_some(), "no code block: {:?}", p.blocks.iter().map(|b| &b.extra).collect::<Vec<_>>());
        match &code.unwrap().extra {
            BlockExtra::Code { lang, .. } => assert_eq!(lang, "sh"),
            other => panic!("{other:?}"),
        }
        assert_eq!(p.display.lines().next().unwrap_or(""), "Install Brew");
        assert!(p.display.contains("/bin/bash hi"));
        // No merge of bullet text and code on one line.
        assert!(!p.display.contains(")brew") && !p.display.contains(")/bin"));
    }

    #[test]
    fn plain_fence_stays_code_block() {
        let src = "```\n/bin/bash hi\n```\n";
        let p = project(src);
        assert!(matches!(p.blocks[0].extra, BlockExtra::Code { .. }));
        match &p.blocks[0].extra {
            BlockExtra::Code { lang, .. } => assert_eq!(lang, ""),
            other => panic!("{other:?}"),
        }
        assert_eq!(p.display.trim_end(), "/bin/bash hi");
    }

    #[test]
    fn html_heading_renders_as_heading() {
        let p = project("<h1>🐟 Carlo's Dotfiles</h1>\n");
        assert_eq!(p.display, "🐟 Carlo's Dotfiles");
        assert!(matches!(p.blocks[0].extra, BlockExtra::HtmlHeading(1)));
        // Caret maps inside the tags, so typing preserves them.
        let s = p.to_source(0, Affinity::Inside);
        assert_eq!("<h1>🐟 Carlo's Dotfiles</h1>"[s..].chars().next(), Some('🐟'));
        let p = project("<h2>Table of Contents</h2>\n");
        assert_eq!(p.display, "Table of Contents");
        assert!(matches!(p.blocks[0].extra, BlockExtra::HtmlHeading(2)));
    }

    #[test]
    fn details_open_close_project() {
        let src = "<details>\n  <summary>\n    ⭐️ MacOS Improvements\n  </summary>\n\n- [x] Better drag\n\n</details>\n";
        let p = project(src);
        assert!(matches!(p.blocks[0].extra, BlockExtra::Details { .. }));
        assert_eq!(p.display.lines().next().unwrap_or(""), "⭐️ MacOS Improvements");
        let last = p.blocks.last().unwrap();
        assert!(matches!(last.extra, BlockExtra::DetailsClose));
        assert_eq!(last.display.start, last.display.end);
        let (a, b) = details_block_range(&p, 0).unwrap();
        assert_eq!((a, b), (0, p.blocks.len() - 1));
    }
}

#[cfg(test)]
mod ordered_marker_tests {
    use super::{list_sibling_index, ordered_marker, ListItem};

    fn item(indent: usize) -> ListItem {
        ListItem {
            display: 0..0,
            source: 0..0,
            indent,
            checked: None,
        }
    }

    #[test]
    fn cycles_1_a_i_by_indent() {
        assert_eq!(ordered_marker(0, 0), "1.");
        assert_eq!(ordered_marker(0, 1), "2.");
        assert_eq!(ordered_marker(1, 0), "a.");
        assert_eq!(ordered_marker(1, 1), "b.");
        assert_eq!(ordered_marker(2, 0), "i.");
        assert_eq!(ordered_marker(2, 1), "ii.");
        assert_eq!(ordered_marker(2, 3), "iv.");
        assert_eq!(ordered_marker(3, 0), "1.");
        assert_eq!(ordered_marker(4, 0), "a.");
        assert_eq!(ordered_marker(5, 0), "i.");
    }

    #[test]
    fn sibling_index_resets_under_parent() {
        let items = vec![
            item(0), // 1
            item(1), // a
            item(1), // b
            item(2), // i
            item(0), // 2
            item(1), // a again
        ];
        assert_eq!(list_sibling_index(&items, 0), 0);
        assert_eq!(list_sibling_index(&items, 1), 0);
        assert_eq!(list_sibling_index(&items, 2), 1);
        assert_eq!(list_sibling_index(&items, 3), 0);
        assert_eq!(list_sibling_index(&items, 4), 1);
        assert_eq!(list_sibling_index(&items, 5), 0);
    }
}
