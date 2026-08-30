//! Notion enter / backspace / tab / mark toggle. Operates on GFM `source`
//! via the display projection.

use std::ops::Range;

use crate::display::{
    project, serialize_table, Affinity, BlockExtra, Projection, TableCell,
};
use crate::document::{splice, BlockKind};
use crate::notion;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Strike,
    Code,
}

impl Mark {
    fn wrap(self) -> (&'static str, &'static str) {
        match self {
            Self::Bold => ("**", "**"),
            Self::Italic => ("*", "*"),
            Self::Strike => ("~~", "~~"),
            Self::Code => ("`", "`"),
        }
    }

    fn has(self, marks: crate::display::Marks) -> bool {
        match self {
            Self::Bold => marks.bold,
            Self::Italic => marks.italic,
            Self::Strike => marks.strike,
            Self::Code => marks.code,
        }
    }
}

pub fn insert_text(
    src: &str,
    caret: usize,
    sel: Option<Range<usize>>,
    text: &str,
    affinity: Affinity,
) -> (String, usize) {
    let p = project(src);
    let range = if let Some(sel) = sel.filter(|s| s.start != s.end) {
        let d0 = p.to_display(sel.start.min(sel.end));
        let d1 = p.to_display(sel.end.max(sel.start));
        p.display_range_to_source(d0..d1, affinity)
    } else {
        let d = p.to_display(caret);
        let s = p.to_source(d, affinity);
        s..s
    };
    let next = splice(src, range.clone(), text);
    (next, range.start + text.len())
}

pub fn delete_display_range(src: &str, display: Range<usize>) -> (String, usize) {
    let p = project(src);
    let range = p.display_range_to_source(display, Affinity::Inside);
    let next = splice(src, range.clone(), "");
    (next, range.start)
}

/// Backspace. `None` = delete one display char (caller). `Some` = structural.
pub fn backspace(src: &str, caret: usize, _affinity: Affinity) -> Option<(String, usize)> {
    let p = project(src);
    let d = p.to_display(caret);
    let block = p.block_at_display(d)?;
    let local = d.saturating_sub(block.display.start);
    if local > 0 {
        return None;
    }
    let empty = block.display.start == block.display.end
        || p.display[block.display.clone()].trim().is_empty();
    match block.kind {
        BlockKind::Heading(_) | BlockKind::Quote | BlockKind::Alert(_) => {
            if empty {
                return Some(join_prev(&p, src, block.source.clone()));
            }
            let body = p.display[block.display.clone()].to_string();
            let next = splice(src, block.source.clone(), &body);
            Some((next, block.source.start + body.len().min(body.len())))
        }
        BlockKind::List { .. } => {
            if let Some(item) = match &block.extra {
                BlockExtra::List { items, .. } => items.iter().find(|it| d == it.display.start),
                _ => None,
            } {
                if item.display.start == item.display.end
                    || p.display[item.display.clone()].trim().is_empty()
                {
                    if item.indent > 0 {
                        return Some(set_item_indent(src, &p, item.source.clone(), item.indent - 1));
                    }
                    let body = p.display[item.display.clone()].to_string();
                    let next = splice(src, item.source.clone(), &body);
                    return Some((next, item.source.start));
                }
                let body = p.display[item.display.clone()].to_string();
                let next = splice(src, item.source.clone(), &body);
                return Some((next, item.source.start));
            }
            if empty {
                return Some(join_prev(&p, src, block.source.clone()));
            }
            None
        }
        BlockKind::Code => {
            if empty {
                let (lang_body, body) = {
                    let slice = &src[block.source.clone()];
                    notion::strip_fence(slice)
                };
                let _ = lang_body;
                let next = splice(src, block.source.clone(), &body);
                Some((next, block.source.start))
            } else if local == 0 {
                let body = p.display[block.display.clone()].to_string();
                let next = splice(src, block.source.clone(), &body);
                Some((next, block.source.start))
            } else {
                None
            }
        }
        BlockKind::Rule | BlockKind::Html => Some(join_prev(&p, src, block.source.clone())),
        BlockKind::Paragraph | BlockKind::Raw | BlockKind::Table => {
            if block.source.start == 0 {
                return None;
            }
            Some(join_prev(&p, src, block.source.clone()))
        }
    }
}

fn join_prev(p: &Projection, src: &str, block_src: Range<usize>) -> (String, usize) {
    let ix = p
        .blocks
        .iter()
        .position(|b| b.source == block_src)
        .unwrap_or(0);
    if ix == 0 {
        let next = splice(src, block_src, "");
        return (next, 0);
    }
    let prev = &p.blocks[ix - 1];
    let this = &p.blocks[ix];
    let this_disp = p.display[this.display.clone()].to_string();
    let prev_disp = p.display[prev.display.clone()].to_string();
    let joined = match prev.kind {
        BlockKind::Heading(level) => notion::wrap_heading(level, &format!("{prev_disp}{this_disp}")),
        BlockKind::Quote => notion::wrap_quote(&format!("{prev_disp}{this_disp}")),
        BlockKind::Alert(k) => notion::wrap_alert(k, &format!("{prev_disp}{this_disp}")),
        BlockKind::List { ordered } => {
            let slice = &src[prev.source.clone()];
            notion::wrap_list(ordered, &format!("{prev_disp}{this_disp}"), slice)
        }
        BlockKind::Code => {
            let (lang, body) = notion::strip_fence(&src[prev.source.clone()]);
            notion::wrap_fence(&lang, &format!("{body}{this_disp}"))
        }
        _ => format!("{prev_disp}{this_disp}"),
    };
    let span = prev.source.start..this.source.end;
    let next = splice(src, span, &joined);
    let caret = (prev.source.start + prev_disp.len()).min(next.len());
    (next, caret)
}

fn set_item_indent(
    src: &str,
    _p: &Projection,
    item_src: Range<usize>,
    indent: usize,
) -> (String, usize) {
    let line = &src[item_src.clone()];
    let trimmed = line.trim_start();
    let pad = "  ".repeat(indent);
    let next_line = format!("{pad}{trimmed}");
    let next = splice(src, item_src.clone(), &next_line);
    (next, item_src.start + pad.len() + marker_len(trimmed))
}

fn marker_len(line: &str) -> usize {
    let t = line.trim_start();
    if t.starts_with("- [ ] ") || t.starts_with("- [x] ") || t.starts_with("- [X] ") {
        return line.len() - t.len() + 6;
    }
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        return line.len() - t.len() + 2;
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && (t[digits..].starts_with(". ") || t[digits..].starts_with(") ")) {
        return line.len() - t.len() + digits + 2;
    }
    0
}

pub fn enter(src: &str, caret: usize, affinity: Affinity, hard: bool) -> (String, usize) {
    let p = project(src);
    let d = p.to_display(caret);
    let Some(block) = p.block_at_display(d) else {
        let (n, c) = insert_text(src, caret, None, "\n\n", affinity);
        return (n, c);
    };
    if hard && matches!(block.kind, BlockKind::Paragraph | BlockKind::Quote) {
        return insert_text(src, caret, None, "  \n", affinity);
    }
    let local = d.saturating_sub(block.display.start);
    let body = &p.display[block.display.clone()];
    match block.kind {
        BlockKind::Code => insert_text(src, caret, None, "\n", affinity),
        BlockKind::Heading(level) => {
            let left = &body[..local.min(body.len())];
            let right = &body[local.min(body.len())..];
            let left_gfm = notion::wrap_heading(level, left);
            let right_gfm = if right.is_empty() {
                String::new()
            } else {
                right.to_string()
            };
            let insert = if right_gfm.is_empty() {
                format!("{left_gfm}\n\n")
            } else {
                format!("{left_gfm}\n\n{right_gfm}")
            };
            let next = splice(src, block.source.clone(), &insert);
            let caret = (block.source.start + left_gfm.len() + 2).min(next.len());
            (next, caret)
        }
        BlockKind::Quote | BlockKind::Alert(_) => {
            let empty_line = body
                .get(..local)
                .and_then(|s| s.rsplit('\n').next())
                .unwrap_or(body)
                .is_empty()
                || (local == body.len() && body.ends_with('\n'))
                || body.is_empty();
            if empty_line && local == body.len() {
                let trimmed = body.trim_end_matches('\n');
                let gfm = match block.kind {
                    BlockKind::Quote => notion::wrap_quote(trimmed),
                    BlockKind::Alert(k) => notion::wrap_alert(k, trimmed),
                    _ => trimmed.to_string(),
                };
                let insert = format!("{gfm}\n\n");
                let next = splice(src, block.source.clone(), &insert);
                let caret = (block.source.start + insert.len()).min(next.len());
                (next, caret)
            } else {
                insert_text(src, caret, None, "\n", affinity)
            }
        }
        BlockKind::List { ordered } => {
            if let BlockExtra::List { items, .. } = &block.extra {
                if let Some(item) = items.iter().find(|it| d >= it.display.start && d <= it.display.end)
                {
                    let empty = item.display.start == item.display.end
                        || p.display[item.display.clone()].trim().is_empty();
                    if empty {
                        if item.indent > 0 {
                            return set_item_indent(src, &p, item.source.clone(), item.indent - 1);
                        }
                        let next = splice(src, item.source.clone(), "");
                        let caret = item.source.start.min(next.len());
                        return (next, caret);
                    }
                    let item_local = d.saturating_sub(item.display.start);
                    let text = &p.display[item.display.clone()];
                    let right = &text[item_local.min(text.len())..];
                    let marker = item_marker(&src[item.source.clone()], ordered, item.checked);
                    let insert = format!("\n{marker}{right}");
                    let cut = item.source.start
                        + (item.source.len().saturating_sub(text.len() - item_local));
                    let cut = cut.clamp(item.source.start, item.source.end);
                    let next = splice(src, cut..item.source.end, &insert);
                    let caret = (cut + 1 + marker.len()).min(next.len());
                    return (next, caret);
                }
            }
            insert_text(src, caret, None, "\n", affinity)
        }
        BlockKind::Table => insert_text(src, caret, None, "\n", affinity),
        BlockKind::Rule | BlockKind::Html => {
            let insert = "\n\n";
            let next = splice(src, block.source.end..block.source.end, insert);
            (next, block.source.end + insert.len())
        }
        BlockKind::Paragraph | BlockKind::Raw => {
            let left = &body[..local.min(body.len())];
            let right = &body[local.min(body.len())..];
            let insert = format!("{left}\n\n{right}");
            let next = splice(src, block.source.clone(), &insert);
            (next, block.source.start + left.len() + 2)
        }
    }
}

fn item_marker(item_src: &str, ordered: bool, checked: Option<bool>) -> String {
    let indent: String = item_src.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    if let Some(c) = checked {
        let mark = if c { "x" } else { " " };
        return format!("{indent}- [{mark}] ");
    }
    if ordered {
        format!("{indent}1. ")
    } else {
        format!("{indent}- ")
    }
}

pub fn tab(src: &str, caret: usize, shift: bool) -> Option<(String, usize)> {
    let p = project(src);
    let d = p.to_display(caret);
    let block = p.block_at_display(d)?;
    match &block.extra {
        BlockExtra::List { items, .. } => {
            let item = items.iter().find(|it| d >= it.display.start && d <= it.display.end)?;
            let ix = items.iter().position(|it| it.source == item.source)?;
            if shift {
                if item.indent == 0 {
                    let body = p.display[item.display.clone()].to_string();
                    let next = splice(src, item.source.clone(), &body);
                    return Some((next, item.source.start));
                }
                return Some(set_item_indent(src, &p, item.source.clone(), item.indent - 1));
            }
            if ix == 0 {
                return None;
            }
            let prev = &items[ix - 1];
            if item.indent > prev.indent {
                return None;
            }
            Some(set_item_indent(src, &p, item.source.clone(), item.indent + 1))
        }
        BlockExtra::Table { cells, rows, cols } => {
            Some(table_tab(src, &p, cells, *rows, *cols, d, shift))
        }
        BlockExtra::Code { .. } => {
            let (n, c) = insert_text(src, caret, None, "\t", Affinity::Inside);
            Some((n, c))
        }
        _ => {
            let (n, c) = insert_text(src, caret, None, "\t", Affinity::Inside);
            Some((n, c))
        }
    }
}

fn table_tab(
    src: &str,
    p: &Projection,
    cells: &[TableCell],
    rows: usize,
    cols: usize,
    d: usize,
    shift: bool,
) -> (String, usize) {
    let Some(cur) = cells.iter().find(|c| d >= c.display.start && d <= c.display.end) else {
        return (src.to_string(), p.to_source(d, Affinity::Inside));
    };
    if shift {
        if let Some(prev) = cells.iter().rev().find(|c| {
            c.row < cur.row || (c.row == cur.row && c.col < cur.col)
        }) {
            return (src.to_string(), p.to_source(prev.display.start, Affinity::Inside));
        }
        return (src.to_string(), p.to_source(cur.display.start, Affinity::Inside));
    }
    if let Some(next) = cells.iter().find(|c| {
        c.row > cur.row || (c.row == cur.row && c.col > cur.col)
    }) {
        return (src.to_string(), p.to_source(next.display.start, Affinity::Inside));
    }
    let (headers, body) = table_strings(p, cells, cols);
    let mut body = body;
    body.push(vec![String::new(); cols.max(1)]);
    let gfm = serialize_table(&headers, &body);
    let block = p.block_at_display(d).unwrap();
    let next = splice(src, block.source.clone(), &gfm);
    let p2 = project(&next);
    let caret = p2
        .blocks
        .iter()
        .find(|b| b.kind == BlockKind::Table)
        .and_then(|b| match &b.extra {
            BlockExtra::Table { cells, .. } => cells.last().map(|c| {
                p2.to_source(c.display.start, Affinity::Inside)
            }),
            _ => None,
        })
        .unwrap_or(block.source.start + gfm.len());
    let _ = rows;
    (next, caret)
}

fn table_strings(p: &Projection, cells: &[TableCell], cols: usize) -> (Vec<String>, Vec<Vec<String>>) {
    let mut headers = vec![String::new(); cols.max(1)];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for c in cells {
        let text = p.display.get(c.display.clone()).unwrap_or("").replace('\t', "").to_string();
        if c.header {
            if c.col < headers.len() {
                headers[c.col] = text;
            }
        } else {
            let body_row = c.row.saturating_sub(1);
            while rows.len() <= body_row {
                rows.push(vec![String::new(); cols.max(1)]);
            }
            if c.col < rows[body_row].len() {
                rows[body_row][c.col] = text;
            }
        }
    }
    (headers, rows)
}

pub fn toggle_mark(
    src: &str,
    sel: Range<usize>,
    mark: Mark,
) -> Option<(String, Range<usize>)> {
    let p = project(src);
    let d0 = p.to_display(sel.start.min(sel.end));
    let d1 = p.to_display(sel.end.max(sel.start));
    if d0 == d1 {
        return None;
    }
    let all = (d0..d1).all(|i| {
        if i >= p.display.len() {
            return false;
        }
        mark.has(p.marks_at(i, Affinity::Inside))
    });
    let (open, close) = mark.wrap();
    if all {
        let mut next = src.to_string();
        let segs: Vec<_> = p
            .segments
            .iter()
            .filter(|s| s.display.start < d1 && s.display.end > d0 && mark.has(s.marks))
            .cloned()
            .collect();
        for seg in segs.into_iter().rev() {
            let closer = p_segments_closer(&p, &seg);
            let opener = p_segments_opener(&p, &seg);
            if closer.end <= next.len() && closer.start >= close.len() {
                let at = closer.end.saturating_sub(close.len());
                if next.get(at..closer.end) == Some(close) {
                    next.replace_range(at..closer.end, "");
                }
            }
            if opener.start + open.len() <= next.len() && next.get(opener.start..opener.start + open.len()) == Some(open)
            {
                next.replace_range(opener.start..opener.start + open.len(), "");
            }
        }
        let p2 = project(&next);
        let caret = p2.to_source(d0.min(p2.display.len()), Affinity::Inside);
        let end = p2.to_source(d1.min(p2.display.len()), Affinity::Inside);
        Some((next, caret..end))
    } else {
        let range = p.display_range_to_source(d0..d1, Affinity::Inside);
        let mut next = src.to_string();
        next.insert_str(range.end, close);
        next.insert_str(range.start, open);
        let start = range.start + open.len();
        let end = range.end + open.len();
        Some((next, start..end))
    }
}

fn p_segments_opener(p: &Projection, seg: &crate::display::Segment) -> Range<usize> {
    let prev = p
        .segments
        .iter()
        .rev()
        .find(|s| s.display.end <= seg.display.start)
        .map(|s| s.source.end)
        .unwrap_or(seg.source.start);
    prev..seg.source.start
}

fn p_segments_closer(p: &Projection, seg: &crate::display::Segment) -> Range<usize> {
    let next = p
        .segments
        .iter()
        .find(|s| s.display.start >= seg.display.end)
        .map(|s| s.source.start)
        .unwrap_or(seg.source.end);
    seg.source.end..next
}

pub fn apply_link(src: &str, sel: Range<usize>, url: &str) -> (String, usize) {
    let p = project(src);
    let d0 = p.to_display(sel.start.min(sel.end));
    let d1 = p.to_display(sel.end.max(sel.start));
    let text = p.display.get(d0..d1).unwrap_or("").to_string();
    let range = p.display_range_to_source(d0..d1, Affinity::Inside);
    let replacement = if url.is_empty() {
        text
    } else {
        format!("[{text}]({url})")
    };
    let next = splice(src, range.clone(), &replacement);
    (next, range.start + replacement.len())
}

pub fn set_code_lang(src: &str, caret: usize, lang: &str) -> Option<(String, usize)> {
    let p = project(src);
    let d = p.to_display(caret);
    let block = p.block_at_display(d)?;
    let BlockExtra::Code { .. } = &block.extra else {
        return None;
    };
    let body = p.display[block.display.clone()].to_string();
    let gfm = notion::wrap_fence(lang, &body);
    let next = splice(src, block.source.clone(), &gfm);
    Some((next, block.source.start + 3 + lang.len() + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_into_heading() {
        let (out, caret) = insert_text("# Hi", 4, None, "!", Affinity::Inside);
        assert_eq!(out, "# Hi!");
        assert_eq!(&out[..caret], "# Hi!");
    }

    #[test]
    fn enter_splits_heading() {
        let src = "# Hello";
        let p = project(src);
        let caret = p.to_source(p.display.len(), Affinity::Inside);
        let (out, _) = enter(src, caret, Affinity::Inside, false);
        assert!(out.starts_with("# Hello"), "{out}");
        assert!(out.contains("\n\n"), "{out:?}");
    }

    #[test]
    fn backspace_empty_heading_joins() {
        let src = "para\n\n# ";
        let p = project(src);
        let d = p.blocks.last().unwrap().display.start;
        let caret = p.to_source(d, Affinity::Inside);
        let (out, _) = backspace(src, caret, Affinity::Inside).expect("join");
        assert!(out.contains("para"), "{out}");
        assert!(!out.contains('#'), "{out}");
    }

    #[test]
    fn toggle_bold_wraps() {
        let src = "hello";
        let (out, _) = toggle_mark(src, 0..5, Mark::Bold).unwrap();
        assert_eq!(out, "**hello**");
    }
}
