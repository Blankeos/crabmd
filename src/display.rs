//! GFM `source` → visible projection. Syntax stays in the file; the caret walks
//! `display`. Re-parse after every edit — markdown-markdown is just the parser.

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::document::{
    gfm_options, parse_ranges, sole_image, AlertKind, BlockKind, PaintRange,
};
use crate::notion;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<u32>,
}

impl Marks {
    pub fn any(self) -> bool {
        self.bold || self.italic || self.strike || self.code || self.link.is_some()
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
                .or_else(|| {
                    self.segments
                        .iter()
                        .find(|s| d == s.display.start)
                })
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
                            if affinity == Affinity::Inside && seg.marks.any() && !next.marks.any() {
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
                            return next.source.start;
                        }
                        if let Some(block) = self.block_at_display(d.saturating_sub(1).max(seg.display.start)) {
                            return block.source.end.min(self.source_len);
                        }
                        return seg.source.end;
                    }
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
        let item = items.iter().find(|it| d >= it.display.start && d <= it.display.end)?;
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

fn content_point(src: &str, r: &PaintRange) -> Range<usize> {
    let slice = r.slice(src);
    match r.kind {
        BlockKind::Heading(_) => {
            let (_, text) = notion::strip_heading(slice);
            let start = r.range.end.saturating_sub(text.len()).min(src.len());
            start..start
        }
        _ => {
            let p = r.range.start.min(src.len());
            p..p
        }
    }
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
        return BlockExtra::Image {
            alt,
            src: img_src,
        };
    }
    match r.kind {
        BlockKind::Rule => BlockExtra::Rule,
        BlockKind::Html | BlockKind::Raw => {
            emit_plain(display, segments, r.range.clone(), slice, Marks::default());
            BlockExtra::Html
        }
        BlockKind::Code => {
            let (lang, body) = notion::strip_fence(slice);
            let abs = body_abs_range(slice, &body, r.range.start);
            emit_plain(
                display,
                segments,
                abs,
                &body,
                Marks {
                    code: true,
                    ..Marks::default()
                },
            );
            BlockExtra::Code { lang }
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
            BlockExtra::Heading(level)
        }
        BlockKind::Quote => {
            project_inlines(src, r, display, segments, links);
            BlockExtra::Quote
        }
        BlockKind::Alert(kind) => {
            project_inlines(src, r, display, segments, links);
            BlockExtra::Alert(kind)
        }
        BlockKind::Paragraph => {
            project_inlines(src, r, display, segments, links);
            BlockExtra::Text
        }
    }
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
    let mut items: Vec<ListItem> = Vec::new();
    let mut item_d0 = 0usize;
    let mut item_src: Option<Range<usize>> = None;
    let mut item_indent = 0usize;
    let mut item_checked: Option<bool> = None;
    let mut saw_list = false;
    let mut skip_alert_label = matches!(r.kind, BlockKind::Alert(_));
    let mut after_item = false;

    for (event, range) in parser {
        let abs = r.range.start + range.start..r.range.start + range.end;
        match event {
            Event::Start(Tag::List(_)) => {
                saw_list = true;
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
                item_d0 = display.len();
                item_src = Some(abs);
                item_indent = list_indent(&slice[range.start.min(slice.len())..]);
                item_checked = None;
            }
            Event::End(TagEnd::Item) => {
                if let Some(src_r) = item_src.take() {
                    items.push(ListItem {
                        display: item_d0..display.len(),
                        source: src_r.start..r.range.start + range.end,
                        indent: item_indent,
                        checked: item_checked,
                    });
                }
            }
            Event::TaskListMarker(checked) => {
                item_checked = Some(checked);
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
            Event::Start(Tag::Heading { .. }) => {
                skip_alert_label = false;
            }
            Event::Start(Tag::BlockQuote(_)) if skip_alert_label => {}
            Event::Text(t) => {
                if skip_alert_label && is_alert_label(&t) {
                    skip_alert_label = false;
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
                let d0 = display.len();
                display.push('\n');
                segments.push(Segment {
                    display: d0..display.len(),
                    source: abs,
                    marks,
                });
            }
            Event::InlineHtml(t) | Event::Html(t) => {
                emit_plain(display, segments, abs, t.as_ref(), Marks::default());
            }
            _ => {}
        }
    }

    if saw_list {
        Some(items)
    } else {
        None
    }
}

fn is_alert_label(t: &str) -> bool {
    let u = t.trim().trim_start_matches('!').to_ascii_uppercase();
    matches!(
        u.as_str(),
        "NOTE" | "TIP" | "IMPORTANT" | "WARNING" | "CAUTION"
    )
}

fn list_indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count() / 2
}

fn project_table(
    src: &str,
    r: &PaintRange,
    display: &mut String,
    segments: &mut Vec<Segment>,
    _links: &mut Vec<String>,
) -> BlockExtra {
    let slice = r.slice(src);
    let mut cells = Vec::new();
    let mut row = 0usize;
    let mut col = 0usize;
    let mut cols = 0usize;
    let mut header = true;
    let mut cell_d0 = 0usize;
    let mut cell_src = 0..0;
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
            }
            Event::Text(t) => {
                let text = &slice[range.start.min(slice.len())..range.end.min(slice.len())];
                if !text.trim().is_empty() || !t.is_empty() {
                    emit_plain(display, segments, abs, text, Marks::default());
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
        out.push_str(headers.get(i).map(|s| s.as_str()).unwrap_or(""));
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
            out.push_str(row.get(i).map(|s| s.as_str()).unwrap_or(""));
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
            BlockExtra::Code { lang } => assert_eq!(lang, "rust"),
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
    fn wrap_cols_positive() {
        assert!(wrap_cols_for(15, true).unwrap() > 20);
        assert_eq!(wrap_cols_for(15, false), None);
    }
}
