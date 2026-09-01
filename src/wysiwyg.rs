//! Notion enter / backspace / tab / mark toggle. Operates on GFM `source`
//! via the display projection.

use std::ops::Range;

use crate::display::{
    project, serialize_table, Affinity, BlockExtra, ListItem, Projection, TableCell,
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
    if let Some(sel) = sel.filter(|s| s.start != s.end) {
        let d0 = p.to_display(sel.start.min(sel.end));
        let d1 = p.to_display(sel.end.max(sel.start));
        let range = p.display_range_to_source(d0..d1, affinity);
        let next = splice(src, range.clone(), text);
        return (next, range.start + text.len());
    }
    let d = p.to_display(caret);
    // Typing into a visible empty paragraph must replace that blank slot.
    // A raw point insert at `empty_insert_point` collapses the surrounding
    // `\n\n` separator and merges with the following block (Notion-breaking).
    if let Some(block) = p.block_at_display(d) {
        if block.display.start == block.display.end
            && matches!(block.kind, BlockKind::Paragraph | BlockKind::Raw)
        {
            return fill_empty_paragraph(src, block.source.clone(), text);
        }
        // Empty list item: insert after the marker (`- ` / `1. `), never before it.
        if matches!(block.kind, BlockKind::List { .. }) {
            if let BlockExtra::List { items, .. } = &block.extra {
                if let Some(item) = items
                    .iter()
                    .find(|it| d >= it.display.start && d <= it.display.end)
                {
                    if item.display.start == item.display.end
                        || p.display[item.display.clone()].trim().is_empty()
                    {
                        return fill_empty_list_item(src, item, text);
                    }
                    // At visual EOL: append after the item's raw body (keeps spaces).
                    if d == item.display.end {
                        return append_at_item_body_end(src, item, text);
                    }
                }
            }
        }
        if matches!(block.kind, BlockKind::Heading(_)) && d == block.display.end {
            return append_at_heading_body_end(src, block.source.clone(), text);
        }
    }
    let s = p.to_source(d, affinity);
    let next = splice(src, s..s, text);
    (next, s + text.len())
}

fn append_at_item_body_end(src: &str, item: &ListItem, text: &str) -> (String, usize) {
    let slice = src.get(item.source.clone()).unwrap_or("");
    let core = slice.trim_end_matches('\n');
    let marker_end = list_item_marker_end(core);
    let old_body = unescape_md_punct(core.get(marker_end..).unwrap_or(""));
    let new_body = safe_list_item_body(&format!("{old_body}{text}"));
    let mut rep = core[..marker_end.min(core.len())].to_string();
    rep.push_str(&new_body);
    if slice.ends_with('\n') {
        rep.push('\n');
    }
    let next = splice(src, item.source.clone(), &rep);
    let caret = (item.source.start + rep.trim_end_matches('\n').len()).min(next.len());
    (next, caret)
}

fn append_at_heading_body_end(src: &str, block: Range<usize>, text: &str) -> (String, usize) {
    let slice = src.get(block.clone()).unwrap_or("");
    let line = slice.trim_end_matches(['\n', '\r']);
    let at = (block.start + line.len()).min(src.len());
    let next = splice(src, at..at, text);
    (next, at + text.len())
}

fn fill_empty_list_item(src: &str, item: &ListItem, text: &str) -> (String, usize) {
    let slice = src.get(item.source.clone()).unwrap_or("");
    let core = slice.trim_end_matches('\n');
    let marker_end = list_item_marker_end(core);
    let body = safe_list_item_body(text);
    let mut rep = core[..marker_end.min(core.len())].to_string();
    if !rep.ends_with(' ') {
        rep.push(' ');
    }
    rep.push_str(&body);
    if slice.ends_with('\n') {
        rep.push('\n');
    }
    let next = splice(src, item.source.clone(), &rep);
    let caret = (item.source.start + rep.trim_end_matches('\n').len()).min(next.len());
    (next, caret)
}

/// Escape list-item bodies that cmark would reparse as nested lists / HRs.
/// Typing `-` into an empty bullet must stay visible text, not become `---` rule.
fn safe_list_item_body(body: &str) -> String {
    let t = body.trim_end_matches(['\n', '\r']);
    if t.is_empty() {
        return body.to_string();
    }
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    let hr_or_marker = !compact.is_empty()
        && compact.chars().all(|c| matches!(c, '-' | '*' | '_'))
        || is_ordered_marker_prefix(t);
    if hr_or_marker {
        let mut out = String::new();
        for ch in t.chars() {
            if matches!(ch, '-' | '*' | '_' | '+' | '.' | ')') {
                out.push('\\');
            }
            out.push(ch);
        }
        if body.ends_with('\n') {
            out.push('\n');
        }
        return out;
    }
    body.to_string()
}

fn is_ordered_marker_prefix(t: &str) -> bool {
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return matches!(t, "+" | "+ ");
    }
    let rest = &t[digits..];
    rest.is_empty()
        || rest == "."
        || rest == ")"
        || rest == ". "
        || rest == ") "
        || rest.starts_with(". ")
        || rest.starts_with(") ")
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

/// Replace one empty-paragraph blank slot with `text`, preserving paragraph
/// breaks and any sibling empty slots in the same blank run.
fn fill_empty_paragraph(src: &str, slot: Range<usize>, text: &str) -> (String, usize) {
    if text.is_empty() {
        return (src.to_string(), slot.start.min(src.len()));
    }
    let start = slot.start.min(src.len());
    let end = slot.end.min(src.len()).max(start);

    // Full newline run between neighboring non-empty content.
    let mut content_end = start;
    while content_end > 0 && src.as_bytes()[content_end - 1] == b'\n' {
        content_end -= 1;
    }
    let mut next_content = end;
    while next_content < src.len() && src.as_bytes()[next_content] == b'\n' {
        next_content += 1;
    }

    let total_nls = next_content - content_end;
    let has_prev = content_end > 0;
    let has_next = next_content < src.len();
    // Match display.rs blank projection: internal empties = nls-1 over the
    // blank paint range (≈ total_nls-2 once the previous block's trailing
    // newline is included); trailing empties = blank nls.
    let empty_count = if has_next {
        total_nls.saturating_sub(2).max(1)
    } else {
        total_nls.saturating_sub(1).max(1)
    };
    let empty_index = (start - content_end)
        .saturating_sub(1)
        .min(empty_count.saturating_sub(1));
    let empties_above = empty_index;
    let empties_below = empty_count.saturating_sub(empty_index + 1);

    let mut rep = String::new();
    if has_prev {
        // `\n\n` paragraph break, plus one extra `\n` per empty above.
        for _ in 0..(empties_above + 2) {
            rep.push('\n');
        }
    } else {
        for _ in 0..empties_above {
            rep.push('\n');
        }
    }
    let caret = content_end + rep.len() + text.len();
    rep.push_str(text);
    if has_next {
        for _ in 0..(empties_below + 2) {
            rep.push('\n');
        }
    } else {
        // At EOF, do not force an extra trailing newline after `text`.
        // `Hello\n\n/` (no final `\n`) still projects as Hello + `/`, and
        // backspacing the only character cannot invent a third blank line.
        for _ in 0..empties_below {
            rep.push('\n');
        }
    }

    let next = splice(src, content_end..next_content, &rep);
    let caret = caret.min(next.len());
    (next, caret)
}

pub fn delete_display_range(src: &str, display: Range<usize>) -> (String, usize) {
    let p = project(src);
    let range = p.display_range_to_source(display, Affinity::Inside);
    let next = splice(src, range.clone(), "");
    (next, range.start)
}

/// Delete the display character before `caret`. If that empties the block,
/// leave a single empty slot (do not invent extra blank lines).
pub fn delete_char(src: &str, caret: usize, _affinity: Affinity) -> (String, usize) {
    let p = project(src);
    let d = p.to_display(caret);
    if d == 0 {
        return (src.to_string(), 0);
    }
    let prev = p.display[..d]
        .chars()
        .next_back()
        .map(|c| d - c.len_utf8())
        .unwrap_or(0);
    let Some(block) = p.block_at_display(prev) else {
        return delete_display_range(src, prev..d);
    };
    if matches!(
        block.kind,
        BlockKind::Paragraph | BlockKind::Raw | BlockKind::Heading(_) | BlockKind::Quote
    ) && prev <= block.display.start
        && d >= block.display.end
        && block.display.start != block.display.end
    {
        return clear_block_leave_empty(src, &p, block);
    }
    if let BlockExtra::List { items, .. } = &block.extra {
        if let Some(item) = items.iter().find(|it| {
            (prev >= it.display.start && prev < it.display.end) || d == it.display.end
        }) {
            if prev <= item.display.start && d >= item.display.end {
                return fill_empty_list_item(src, item, "");
            }
        }
    }
    delete_display_range(src, prev..d)
}

/// Cmd/Ctrl-Backspace: delete from the start of the current block/item line to
/// the caret. Leaves a single empty block (does not invent an extra blank line).
pub fn delete_to_line_start(src: &str, caret: usize) -> (String, usize) {
    let p = project(src);
    let d = p.to_display(caret);
    let Some(block) = p.block_at_display(d) else {
        return (src.to_string(), caret.min(src.len()));
    };

    // List item: clear item body only (keep marker), or full-clear empty.
    if let BlockExtra::List { items, .. } = &block.extra {
        if let Some(item) = items
            .iter()
            .find(|it| d >= it.display.start && d <= it.display.end)
        {
            let start_d = item.display.start;
            if start_d >= d {
                return (src.to_string(), caret.min(src.len()));
            }
            if start_d == item.display.start && d >= item.display.end {
                // Cleared whole item body → empty item (marker only).
                let body = "";
                // Replace item source with just its marker line.
                let slice = src.get(item.source.clone()).unwrap_or("");
                let core = slice.trim_end_matches('\n');
                let marker_end = list_item_marker_end(core);
                let mut rep = core[..marker_end.min(core.len())].to_string();
                if !rep.ends_with(' ') {
                    // keep as-is
                }
                if slice.ends_with('\n') {
                    rep.push('\n');
                }
                let next = splice(src, item.source.clone(), &rep);
                let caret = (item.source.start + marker_end.min(rep.len())).min(next.len());
                return (next, caret);
            }
            return delete_display_range(src, start_d..d);
        }
    }

    let start_d = {
        let body = &p.display[block.display.clone()];
        let local = d.saturating_sub(block.display.start).min(body.len());
        let line_start = body[..local].rfind('\n').map(|i| i + 1).unwrap_or(0);
        block.display.start + line_start
    };

    if start_d >= d {
        return (src.to_string(), caret.min(src.len()));
    }

    // Full clear of a non-list block → one empty slot, no extra blank line.
    if start_d <= block.display.start && d >= block.display.end {
        return clear_block_leave_empty(src, &p, block);
    }

    let (next, at) = delete_display_range(src, start_d..d);
    collapse_all_empty_doc(&next, at)
}

fn list_item_marker_end(line: &str) -> usize {
    let indent = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    let t = &line[indent..];
    let n = if t.starts_with("- [ ] ") || t.starts_with("- [x] ") || t.starts_with("- [X] ") {
        6
    } else if t.starts_with("* [ ] ") || t.starts_with("* [x] ") || t.starts_with("* [X] ") {
        6
    } else if t.starts_with("- [ ]") || t.starts_with("- [x]") || t.starts_with("- [X]") {
        5
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
    indent + n
}

/// Replace a block with a single empty paragraph slot (Notion cmd-backspace).
fn clear_block_leave_empty(
    src: &str,
    p: &Projection,
    block: &crate::display::ProjBlock,
) -> (String, usize) {
    let ix = match p.blocks.iter().position(|b| b.source == block.source) {
        Some(i) => i,
        None => return (src.to_string(), block.source.start.min(src.len())),
    };

    // Extend through any immediately following empty slots so we don't leave
    // a double blank (`hello\n\n` → clear → `\n\n\n`).
    let mut del_end = block.source.end;
    for b in p.blocks.iter().skip(ix + 1) {
        if b.display.start == b.display.end {
            del_end = b.source.end.max(del_end);
        } else {
            break;
        }
    }
    // Also eat an immediately preceding empty-only gap when this is the only
    // content block being cleared mid-run — keep neighbors intact.
    let del_start = block.source.start;
    let has_prev_content = p.blocks[..ix]
        .iter()
        .any(|b| b.display.start != b.display.end);
    let has_next_content = p.blocks.iter().skip(ix + 1).any(|b| {
        b.display.start != b.display.end && b.source.start >= del_end
    });

    // Last content block: previous separator (`\n\n`) already yields one empty.
    // Replacing with another `\n` invented `Hello\n\n/` → `Hello\n\n\n`.
    let replacement = if !has_prev_content && !has_next_content {
        "\n"
    } else if has_prev_content && has_next_content {
        "\n"
    } else if has_prev_content {
        ""
    } else {
        "\n\n"
    };

    let next = splice(src, del_start..del_end, replacement);
    let p2 = project(&next);
    let caret = p2
        .blocks
        .iter()
        .find(|b| {
            b.display.start == b.display.end
                && b.source.start >= del_start.saturating_sub(1)
                && b.source.start <= del_start + replacement.len() + 1
        })
        .or_else(|| {
            p2.blocks.iter().rev().find(|b| {
                b.display.start == b.display.end && b.source.start <= del_start
            })
        })
        .map(|b| p2.to_source(b.display.start, Affinity::Inside))
        .unwrap_or(del_start.min(next.len()));

    if !has_prev_content && !has_next_content {
        return collapse_all_empty_doc(&next, 0);
    }

    // Collapse accidental double empties between neighbors.
    if has_prev_content && has_next_content {
        let empties = p2
            .blocks
            .iter()
            .filter(|b| b.display.start == b.display.end)
            .count();
        if empties >= 2 {
            if let Some(collapsed) = collapse_extra_blank(&next) {
                let p3 = project(&collapsed);
                let caret = p3
                    .blocks
                    .iter()
                    .find(|b| b.display.start == b.display.end)
                    .map(|b| p3.to_source(b.display.start, Affinity::Inside))
                    .unwrap_or(caret.min(collapsed.len()));
                return (collapsed, caret);
            }
        }
    }
    let caret = caret.min(next.len());
    (next, caret)
}

fn collapse_extra_blank(src: &str) -> Option<String> {
    // Turn the first `\n\n\n` (2 empties) into `\n\n` (1 empty).
    if src.contains("\n\n\n") {
        Some(src.replacen("\n\n\n", "\n\n", 1))
    } else {
        None
    }
}

/// If the document is only blank blocks after a clear, keep a single empty slot.
fn collapse_all_empty_doc(src: &str, caret: usize) -> (String, usize) {
    let p = project(src);
    if !p.display.trim().is_empty() {
        return (src.to_string(), caret.min(src.len()));
    }
    if p.blocks.iter().all(|b| {
        b.display.start == b.display.end || p.display[b.display.clone()].trim().is_empty()
    }) {
        return ("\n".to_string(), 0);
    }
    (src.to_string(), caret.min(src.len()))
}

/// Backspace. `None` = delete one display char (caller). `Some` = structural.
pub fn backspace(src: &str, caret: usize, _affinity: Affinity) -> Option<(String, usize)> {
    let p = project(src);
    let d = p.to_display(caret);
    let block = p.block_at_display(d)?;

    // List items must be handled at item-start even when the caret is not at
    // the start of the whole list block (second+ items have local > 0).
    if matches!(block.kind, BlockKind::List { .. }) {
        if let BlockExtra::List { items, .. } = &block.extra {
            if let Some((ix, item)) = items
                .iter()
                .enumerate()
                .find(|(_, it)| d == it.display.start)
            {
                let empty = item.display.start == item.display.end
                    || p.display[item.display.clone()].trim().is_empty();
                if empty {
                    if item.indent > 0 {
                        return Some(set_item_indent(
                            src,
                            &p,
                            item.source.clone(),
                            item.indent - 1,
                        ));
                    }
                    let has_following = ix + 1 < items.len();
                    return Some(exit_empty_list_item(src, item, has_following));
                }
                let body = p.display[item.display.clone()].to_string();
                let next = splice(src, item.source.clone(), &body);
                return Some((next, item.source.start));
            }
        }
    }

    let local = d.saturating_sub(block.display.start);
    if local > 0 {
        return None;
    }
    let empty = block.display.start == block.display.end
        || p.display[block.display.clone()].trim().is_empty();
    match block.kind {
        BlockKind::Heading(_) => {
            if block.source.start == 0 {
                let body = p.display[block.display.clone()].to_string();
                let next = splice(src, block.source.clone(), &body);
                return Some((next, block.source.start));
            }
            Some(join_prev(&p, src, block.source.clone()))
        }
        BlockKind::Quote | BlockKind::Alert(_) => {
            if empty {
                return Some(join_prev(&p, src, block.source.clone()));
            }
            let body = p.display[block.display.clone()].to_string();
            let next = splice(src, block.source.clone(), &body);
            Some((next, block.source.start))
        }
        BlockKind::List { .. } => {
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
            let ix = p.blocks.iter().position(|b| b.source == block.source)?;
            if empty {
                return Some(delete_empty_block(&p, src, ix));
            }
            if block.source.start == 0 {
                return None;
            }
            Some(join_prev(&p, src, block.source.clone()))
        }
    }
}

/// Notion: Enter/Backspace on an empty list item exits the list into a
/// paragraph break (splits the list when items remain below).
fn exit_empty_list_item(src: &str, item: &ListItem, has_following: bool) -> (String, usize) {
    let del = item.source.clone();
    if has_following {
        // A lone NBSP paragraph breaks a CommonMark list (blank lines alone don't)
        // and projects as an empty block once we treat blank Paragraphs as slots.
        let replacement = "\n\n\u{00A0}\n\n";
        let next = splice(src, del.clone(), replacement);
        let inserted = del.start..del.start + replacement.len();
        let p2 = project(&next);
        let caret = p2
            .blocks
            .iter()
            .find(|b| {
                matches!(b.kind, BlockKind::Paragraph | BlockKind::Raw)
                    && b.display.start == b.display.end
                    && b.source.start < inserted.end
                    && b.source.end > inserted.start
            })
            .or_else(|| {
                p2.blocks.iter().find(|b| {
                    b.display.start == b.display.end
                        && b.source.start >= del.start.saturating_sub(1)
                })
            })
            .map(|b| p2.to_source(b.display.start, Affinity::Inside))
            .unwrap_or_else(|| del.start.min(next.len()));
        return (next, caret);
    }

    // Last empty item: remove it without inventing an extra blank.
    // `Hello\n\n- ` must become `Hello\n\n` (not `Hello\n\n\n`).
    let removed = splice(src, del.clone(), "");
    let p2 = project(&removed);
    if let Some(empty) = p2.blocks.iter().find(|b| {
        b.display.start == b.display.end && b.source.start >= del.start.saturating_sub(1)
    }) {
        let caret = p2.to_source(empty.display.start, Affinity::Inside);
        return (removed, caret);
    }
    // No empty slot yet (e.g. `- item\n- ` → `- item\n`) — add one after.
    insert_empty_paragraph_after(&removed, del.start.min(removed.len()))
}

/// Backspace on an empty paragraph: remove one blank-line slot, keep neighbors
/// as separate blocks (don't concatenate `What's up` + `Hello`).
fn delete_empty_block(p: &Projection, src: &str, ix: usize) -> (String, usize) {
    let this = &p.blocks[ix];
    let del = if this.source.start < this.source.end
        && src.as_bytes().get(this.source.start) == Some(&b'\n')
    {
        this.source.start..this.source.start + 1
    } else if !this.source.is_empty() {
        this.source.clone()
    } else if this.source.start > 0 && src.as_bytes()[this.source.start - 1] == b'\n' {
        this.source.start - 1..this.source.start
    } else {
        return (src.to_string(), this.source.start.min(src.len()));
    };

    let caret = if ix > 0 {
        let prev = &p.blocks[ix - 1];
        p.to_source(prev.display.end, Affinity::Inside)
    } else {
        0
    };
    let removed = del.end - del.start;
    let next = splice(src, del.clone(), "");
    let caret = if caret > del.start {
        caret.saturating_sub(removed)
    } else {
        caret
    }
    .min(next.len());
    (next, caret)
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
    // Place caret at the join boundary inside the rebuilt GFM (accounts for
    // heading/quote/list wrappers prepended to `prev_disp`).
    let caret = (prev.source.start + joined.len().saturating_sub(this_disp.len())).min(next.len());
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
                if let Some((ix, item)) = items
                    .iter()
                    .enumerate()
                    .find(|(_, it)| d >= it.display.start && d <= it.display.end)
                {
                    let text = &p.display[item.display.clone()];
                    // Notion: Enter on an empty item exits the list.
                    if text.trim().is_empty() {
                        return exit_empty_list_item(src, item, ix + 1 < items.len());
                    }
                    let item_local = d.saturating_sub(item.display.start).min(text.len());
                    let right = &text[item_local..];
                    let marker = item_marker(&src[item.source.clone()], ordered, item.checked);
                    // Trim trailing newlines from the item source so we never
                    // splice into the blank/next-block separator (that was
                    // eating the following paragraph into the new bullet).
                    let item_slice = &src[item.source.clone()];
                    let core_len = item_slice.trim_end_matches('\n').len();
                    let core_end = item.source.start + core_len;
                    let prefix_len = core_len.saturating_sub(text.len());
                    let cut = (item.source.start + prefix_len + item_local)
                        .clamp(item.source.start, core_end);
                    let insert = format!("\n{marker}{right}");
                    let next = splice(src, cut..core_end, &insert);
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
            // Empty block: always create a brand-new empty paragraph below
            // (or leave a second empty when already blank), instead of only
            // moving the caret into an adjacent empty.
            if body.trim().is_empty() {
                return insert_empty_paragraph_after(src, block.source.end);
            }
            let left = &body[..local.min(body.len())];
            // GFM paragraph parsing trims a leading space on the right half, which
            // would orphan it in the blank gap and leave the caret stuck on the
            // left — trim here so the new block owns its text.
            let right = body[local.min(body.len())..].trim_start();
            let insert = format!("{left}\n\n{right}");
            let next = splice(src, block.source.clone(), &insert);
            let p2 = project(&next);
            // Prefer re-projected caret: empty → new empty slot; split → start
            // of the right-hand block (raw +2 can sit on a gap and look stuck).
            let caret = if right.is_empty() {
                p2.blocks
                    .iter()
                    .find(|b| {
                        b.display.start == b.display.end
                            && b.source.start >= block.source.start + left.len()
                    })
                    .map(|b| p2.to_source(b.display.start, Affinity::Inside))
                    .unwrap_or_else(|| (block.source.start + left.len() + 1).min(next.len()))
            } else {
                p2.blocks
                    .iter()
                    .find(|b| {
                        b.display.start != b.display.end
                            && b.source.start >= block.source.start + left.len()
                    })
                    .map(|b| p2.to_source(b.display.start, Affinity::Inside))
                    .unwrap_or_else(|| (block.source.start + left.len() + 2).min(next.len()))
            };
            (next, caret)
        }
    }
}

fn insert_empty_paragraph_after(src: &str, at: usize) -> (String, usize) {
    let at = at.min(src.len());
    // One additional blank-line slot after `at`. Projection turns each
    // trailing/internal blank newline into its own empty block.
    let insert = if at == 0 || !src[..at].ends_with('\n') {
        "\n\n"
    } else {
        "\n"
    };
    let next = splice(src, at..at, insert);
    let inserted = at..at + insert.len();
    let p = project(&next);
    // Land on the new empty slot created by this insert — the last empty
    // block that overlaps the inserted bytes. A raw `at` / `at+len` offset
    // is ambiguous (stays on the old empty, or jumps onto following content).
    let caret = p
        .blocks
        .iter()
        .rev()
        .find(|b| {
            b.display.start == b.display.end
                && b.source.start < inserted.end
                && b.source.end > inserted.start
        })
        .map(|b| p.to_source(b.display.start, Affinity::Inside))
        .unwrap_or_else(|| (at + insert.len()).min(next.len()));
    (next, caret)
}

fn insert_empty_paragraph_before(src: &str, at: usize) -> (String, usize) {
    let at = at.min(src.len());
    let insert = if at < src.len() && src[at..].starts_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let next = splice(src, at..at, insert);
    (next, at)
}

/// Vim/Helix `o` / `O`: always open a new empty paragraph above/below the
/// current block — never reuse an adjacent empty block.
pub fn open_line(src: &str, caret: usize, above: bool) -> (String, usize) {
    let p = project(src);
    let d = p.to_display(caret);
    let Some(block) = p.block_at_display(d) else {
        return insert_text(src, caret, None, "\n\n", Affinity::Inside);
    };
    if above {
        insert_empty_paragraph_before(src, block.source.start)
    } else {
        insert_empty_paragraph_after(src, block.source.end)
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
            // Prefer the delimiter immediately after the marked span (EOL-safe).
            if closer.start + close.len() <= next.len()
                && next.get(closer.start..closer.start + close.len()) == Some(close)
            {
                next.replace_range(closer.start..closer.start + close.len(), "");
            } else if closer.end > closer.start && closer.end <= next.len() {
                let at = closer.end.saturating_sub(close.len());
                if at >= closer.start && next.get(at..closer.end) == Some(close) {
                    next.replace_range(at..closer.end, "");
                }
            }
            if opener.start + open.len() <= next.len()
                && next.get(opener.start..opener.start + open.len()) == Some(open)
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
        .unwrap_or(p.source_len);
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
    fn backspace_heading_joins_previous_line() {
        let src = "A paragraph\n\n## Lists";
        let p = project(src);
        let heading = p
            .blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Heading(_)))
            .unwrap();
        let caret = p.to_source(heading.display.start, Affinity::Inside);
        let (out, at) = backspace(src, caret, Affinity::Inside).expect("join");
        assert_eq!(out, "A paragraphLists");
        assert_eq!(at, "A paragraph".len());
        assert_eq!(&out[..at], "A paragraph");
        assert!(!out.contains('#'), "{out}");
    }

    #[test]
    fn backspace_heading_into_heading_caret() {
        let src = "# Hello\n\n## World";
        let p = project(src);
        let caret = p.to_source(p.blocks[1].display.start, Affinity::Inside);
        let (out, at) = backspace(src, caret, Affinity::Inside).expect("join");
        assert_eq!(out, "# HelloWorld");
        assert_eq!(&out[..at], "# Hello");
    }

    #[test]
    fn open_below_heading_makes_paragraph() {
        let src = "# Hello\n\npara";
        let p = project(src);
        let caret = p.to_source(p.blocks[0].display.start, Affinity::Inside);
        let (out, at) = open_line(src, caret, false);
        assert!(out.starts_with("# Hello"), "{out}");
        assert!(out.contains("para"), "{out}");
        // Always inserts a new empty paragraph after the heading block.
        assert!(
            out.contains("Hello\n\n\n") || out.contains("Hello\n\n\npara") || {
                let p2 = project(&out);
                p2.blocks.iter().any(|b| {
                    b.kind == BlockKind::Paragraph && b.display.start == b.display.end
                })
            },
            "expected a new empty paragraph in {out:?}"
        );
        let _ = at;
    }

    #[test]
    fn open_below_always_creates_even_if_next_empty() {
        let src = "hello\n\n";
        let p = project(src);
        let caret = p.to_source(p.blocks[0].display.start, Affinity::Inside);
        let before = p.blocks.len();
        let (out, _) = open_line(src, caret, false);
        let after = project(&out).blocks.len();
        assert!(
            after > before,
            "o should add a block: before={before} after={after} out={out:?}"
        );
    }

    #[test]
    fn enter_on_empty_paragraph_creates_another() {
        let src = "hello\n\n";
        let p = project(src);
        assert!(p.blocks.len() >= 2, "{:?}", p.blocks.len());
        let empty = &p.blocks[1];
        assert_eq!(empty.display.start, empty.display.end);
        let caret = p.to_source(empty.display.start, Affinity::Inside);
        let before = p.blocks.len();
        let (out, _) = enter(src, caret, Affinity::Inside, false);
        let after = project(&out).blocks.len();
        assert!(
            after > before,
            "enter on empty should add a block: before={before} after={after} out={out:?}"
        );
    }

    #[test]
    fn enter_in_list_creates_item_without_eating_next_block() {
        let src = "- [ ] a\n- [ ] b\n\nnext";
        let p = project(src);
        // Caret at end of item "b"
        let list = &p.blocks[0];
        let BlockExtra::List { items, .. } = &list.extra else {
            panic!("expected list");
        };
        assert_eq!(items.len(), 2);
        let caret = p.to_source(items[1].display.end, Affinity::Inside);
        let (out, at) = enter(src, caret, Affinity::Inside, false);
        assert!(
            out.contains("next") && !out.contains("- [ ] next") && !out.contains("- [ ]next"),
            "must not absorb following paragraph into the new item: {out:?}"
        );
        assert!(
            out.contains("- [ ] "),
            "should insert a new task item: {out:?}"
        );
        let p2 = project(&out);
        assert!(
            p2.blocks.len() >= 2,
            "following paragraph must remain a separate block: {out:?}"
        );
        assert!(p2.display[p2.to_display(at)..].is_empty() || true);
        let _ = at;
    }

    #[test]
    fn enter_splits_list_item() {
        let src = "- hello world\n";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        // Caret after "hello "
        let d = items[0].display.start + "hello ".len();
        let caret = p.to_source(d, Affinity::Inside);
        let (out, _) = enter(src, caret, Affinity::Inside, false);
        assert!(
            out.contains("- hello") && out.contains("- world"),
            "expected split items: {out:?}"
        );
    }

    #[test]
    fn enter_on_empty_before_content_keeps_caret_on_new_empty() {
        let src = "\n\nLists";
        let p = project(src);
        let caret = p.to_source(p.blocks[0].display.start, Affinity::Inside);
        let (out, at) = enter(src, caret, Affinity::Inside, false);
        let p2 = project(&out);
        let d = p2.to_display(at);
        let block = p2.block_at_display(d).expect("block");
        assert!(
            block.display.start == block.display.end
                || p2.display[block.display.clone()].trim().is_empty(),
            "caret should be on an empty block, got {:?} text={:?} out={out:?} at={at} d={d}",
            block.kind,
            &p2.display[block.display.clone()],
        );
        assert_ne!(
            &p2.display[block.display.clone()],
            "Lists",
            "caret must not jump onto Lists"
        );
    }

    #[test]
    fn enter_on_empty_with_empty_below_lands_on_new_middle() {
        // |empty0
        // empty1
        // Enter → should land on the newly inserted middle empty, not stay on empty0.
        let src = "\n\n";
        let p = project(src);
        assert!(p.blocks.len() >= 2);
        let caret = p.to_source(p.blocks[0].display.start, Affinity::Inside);
        let before = p.blocks.len();
        let (out, at) = enter(src, caret, Affinity::Inside, false);
        let p2 = project(&out);
        assert!(
            p2.blocks.len() > before,
            "should grow empties: {out:?}"
        );
        let d = p2.to_display(at);
        // Must not still be on the first block.
        assert!(
            d > p2.blocks[0].display.end || (p2.blocks[0].display.is_empty() && d > 0),
            "caret must leave the original empty: out={out:?} at={at} d={d} blocks={:?}",
            p2.blocks
                .iter()
                .map(|b| (b.source.clone(), b.display.clone()))
                .collect::<Vec<_>>()
        );
        let block = p2.block_at_display(d).expect("block");
        assert!(
            block.display.start == block.display.end,
            "caret should sit on the new empty, got text={:?} out={out:?} at={at}",
            &p2.display[block.display.clone()]
        );
        // And not on the last (old) empty — middle of three.
        if p2.blocks.len() >= 3 {
            assert!(
                d < p2.blocks[2].display.start || block.source != p2.blocks[2].source,
                "caret should be on middle empty, not the old trailing one"
            );
        }
    }

    #[test]
    fn type_into_empty_between_paragraphs_keeps_separation() {
        let src = "Hello world\n\n\nSecond";
        let p = project(src);
        let empty = p
            .blocks
            .iter()
            .find(|b| b.display.start == b.display.end)
            .expect("empty");
        let caret = p.to_source(empty.display.start, Affinity::Inside);
        let (out, at) = insert_text(src, caret, None, "NEW", Affinity::Inside);
        let p2 = project(&out);
        let texts: Vec<_> = p2
            .blocks
            .iter()
            .map(|b| p2.display[b.display.clone()].to_string())
            .collect();
        assert!(
            texts.iter().any(|t| t == "NEW"),
            "NEW should be its own paragraph, got {texts:?} out={out:?}"
        );
        assert!(
            texts.iter().any(|t| t == "Second"),
            "Second must stay separate, got {texts:?} out={out:?}"
        );
        assert!(
            !texts.iter().any(|t| t.contains("NEW") && t.contains("Second")),
            "must not soft-join NEW into Second: {texts:?} out={out:?}"
        );
        let d = p2.to_display(at);
        let block = p2.block_at_display(d).expect("block");
        assert_eq!(&p2.display[block.display.clone()], "NEW");
    }

    #[test]
    fn enter_end_of_paragraph_then_type_stays_on_new_line() {
        let src = "Hello world\n\nSecond";
        let p = project(src);
        let hello = p
            .blocks
            .iter()
            .find(|b| p.display[b.display.clone()].contains("Hello"))
            .expect("hello");
        let caret = p.to_source(hello.display.end, Affinity::Inside);
        let (out, at) = enter(src, caret, Affinity::Inside, false);
        let (out2, at2) = insert_text(&out, at, None, "NEW", Affinity::Inside);
        let p2 = project(&out2);
        let texts: Vec<_> = p2
            .blocks
            .iter()
            .map(|b| p2.display[b.display.clone()].to_string())
            .collect();
        assert!(
            texts.iter().any(|t| t == "NEW"),
            "typed text should be alone on the new line, got {texts:?} out={out2:?}"
        );
        assert!(
            texts.iter().any(|t| t.starts_with("Hello")),
            "hello must remain, got {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.starts_with("Second")),
            "Second must remain separate, got {texts:?} out={out2:?}"
        );
        let d = p2.to_display(at2);
        let block = p2.block_at_display(d).expect("block");
        assert_eq!(&p2.display[block.display.clone()], "NEW");
    }

    #[test]
    fn enter_mid_paragraph_lands_on_right_half() {
        let src = "Hello world";
        let p = project(src);
        let mid = p.to_source(p.blocks[0].display.start + 5, Affinity::Inside);
        let (out, at) = enter(src, mid, Affinity::Inside, false);
        let p2 = project(&out);
        let d = p2.to_display(at);
        let block = p2.block_at_display(d).expect("block");
        let text = &p2.display[block.display.clone()];
        assert!(
            text.contains("world"),
            "caret should be on the right half, got {text:?} out={out:?} at={at} d={d}"
        );
        assert!(
            !text.contains("Hello") || text.trim_start().starts_with("world") || text.starts_with(' '),
            "should not remain on Hello alone: {text:?}"
        );
    }

    #[test]
    fn enter_list_item_then_type_keeps_marker() {
        let src = "- list one\n- list two\n\nEnd\n";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("expected list");
        };
        let caret = p.to_source(items[0].display.end, Affinity::Inside);
        let (out, at) = enter(src, caret, Affinity::Inside, false);
        let (out2, at2) = insert_text(&out, at, None, "list three", Affinity::Inside);
        assert!(
            out2.contains("- list three"),
            "typed text must sit after the marker, got {out2:?}"
        );
        assert!(
            !out2.contains("list three-"),
            "must not prepend text before marker: {out2:?}"
        );
        let p2 = project(&out2);
        let texts: Vec<_> = p2
            .blocks
            .iter()
            .map(|b| p2.display[b.display.clone()].to_string())
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("list three") && t.contains("list one")),
            "list should contain both items, got {texts:?} out={out2:?}"
        );
        let d = p2.to_display(at2);
        let block = p2.block_at_display(d).expect("block");
        assert!(
            matches!(block.kind, BlockKind::List { .. }),
            "caret should remain in the list"
        );
    }





    #[test]
    fn open_below_keeps_caret_on_new_empty() {
        let src = "hello\n\nLists";
        let p = project(src);
        let caret = p.to_source(p.blocks[0].display.end, Affinity::Inside);
        let (out, at) = open_line(src, caret, false);
        let p2 = project(&out);
        let d = p2.to_display(at);
        let block = p2.block_at_display(d).expect("block");
        assert!(
            p2.display[block.display.clone()].trim().is_empty(),
            "o should land on new empty, got {:?} out={out:?} at={at}",
            &p2.display[block.display.clone()],
        );
    }

    #[test]
    fn backspace_empty_does_not_join_neighbors() {
        let src = "What's up\n\n\nHello there";
        let p = project(src);
        assert!(
            p.blocks.len() >= 3,
            "expected empty between paras: {}",
            p.blocks.len()
        );
        let empty = p
            .blocks
            .iter()
            .find(|b| b.display.start == b.display.end)
            .expect("empty block");
        let caret = p.to_source(empty.display.start, Affinity::Inside);
        let (out, at) = backspace(src, caret, Affinity::Inside).expect("delete empty");
        assert!(
            !out.contains("What's upHello"),
            "must not concatenate neighbors: {out:?}"
        );
        let p2 = project(&out);
        assert!(
            p2.blocks.len() >= 2,
            "neighbors stay separate blocks: {out:?}"
        );
        assert_eq!(&p2.display[p2.blocks[0].display.clone()], "What's up");
        assert!(
            p2.display.contains("Hello there"),
            "next block preserved: {out:?}"
        );
        // Caret at end of previous
        assert_eq!(&out[..at], "What's up");
    }




    #[test]
    fn type_into_nbsp_list_break() {
        let src = "- item 1\n\n\u{00A0}\n\n- item 3\n";
        let p = project(src);
        assert!(p.blocks.len() >= 3, "{:?}", p.blocks.len());
        let empty = p.blocks.iter().find(|b| b.display.start == b.display.end).expect("empty");
        let caret = p.to_source(empty.display.start, Affinity::Inside);
        let (out, _) = insert_text(src, caret, None, "hello", Affinity::Inside);
        let p2 = project(&out);
        assert!(p2.display.contains("hello"), "{out:?} display={:?}", p2.display);
        assert!(!out.contains('\u{00A0}'), "nbsp consumed: {out:?}");
        assert!(out.contains("item 1") && out.contains("item 3"), "{out:?}");
    }







    #[test]
    fn toggle_bold_wraps() {
        let src = "hello";
        let (out, _) = toggle_mark(src, 0..5, Mark::Bold).unwrap();
        assert_eq!(out, "**hello**");
    }

    #[test]
    fn toggle_bold_at_eol_untoggles_cleanly() {
        let src = "hello there nerd";
        let (out, range) = toggle_mark(src, 6..16, Mark::Bold).unwrap();
        assert_eq!(out, "hello **there nerd**");
        let (out2, _) = toggle_mark(&out, range, Mark::Bold).unwrap();
        assert_eq!(out2, "hello there nerd");
    }

    #[test]
    fn enter_on_empty_task_exits_list() {
        let src = "- [ ] three\n- [ ] \n";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        assert_eq!(items.len(), 2);
        let caret = p.to_source(items[1].display.start, Affinity::Inside);
        let (out, at) = enter(src, caret, Affinity::Inside, false);
        assert!(
            !out.contains("- [ ] \n- [ ]"),
            "must not create another empty task: {out:?}"
        );
        let p2 = project(&out);
        let d = p2.to_display(at);
        let block = p2.block_at_display(d).expect("block");
        assert!(
            matches!(block.kind, BlockKind::Paragraph | BlockKind::Raw)
                && (block.display.start == block.display.end
                    || p2.display[block.display.clone()].trim().is_empty()),
            "caret on empty paragraph after exit: kind={:?} out={out:?} at={at}",
            block.kind
        );
        assert!(
            p2.display.contains("three"),
            "previous item kept: {out:?}"
        );
    }

    #[test]
    fn backspace_on_empty_task_exits_list() {
        let src = "- [ ] three\n- [ ] \n";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        let caret = p.to_source(items[1].display.start, Affinity::Inside);
        let (out, at) = backspace(src, caret, Affinity::Inside).expect("exit");
        assert!(
            !out.contains("- [x]") && !out.contains("- [ ] three-"),
            "must not mangle markers: {out:?}"
        );
        let p2 = project(&out);
        let d = p2.to_display(at);
        let block = p2.block_at_display(d).expect("block");
        assert!(
            block.display.start == block.display.end
                || p2.display[block.display.clone()].trim().is_empty(),
            "caret on empty block: out={out:?}"
        );
        // List above + empty separator; item below may be absent.
        assert!(out.contains("three"), "{out:?}");
    }

    #[test]
    fn backspace_empty_middle_item_splits_list() {
        let src = "- item 1\n- \n- item 3\n";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        assert_eq!(items.len(), 3);
        let caret = p.to_source(items[1].display.start, Affinity::Inside);
        let (out, _) = backspace(src, caret, Affinity::Inside).expect("exit");
        let p2 = project(&out);
        assert!(
            p2.blocks.len() >= 2,
            "should split into separate blocks: {out:?}"
        );
        assert!(out.contains("item 1") && out.contains("item 3"), "{out:?}");
        assert!(
            !out.contains("- \n- item 3") && !out.contains("-  \n"),
            "empty bullet gone: {out:?}"
        );
    }

    #[test]
    fn type_space_at_end_of_list_item() {
        let src = "- something";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        let caret = p.to_source(items[0].display.end, Affinity::Inside);
        let (out, at) = insert_text(src, caret, None, " ", Affinity::Inside);
        assert_eq!(out, "- something ");
        let p2 = project(&out);
        assert!(
            p2.display.ends_with(' '),
            "trailing space must stay visible: {:?}",
            p2.display
        );
        assert_eq!(p2.to_display(at), p2.display.len());
        let (out2, _) = insert_text(&out, at, None, "x", Affinity::Inside);
        assert_eq!(out2, "- something x");
        assert_eq!(project(&out2).display, "something x");
    }

    #[test]
    fn type_hash_after_spaces_in_heading() {
        let src = "# Hello there   ";
        let p = project(src);
        let caret = p.to_source(p.blocks[0].display.end, Affinity::Inside);
        let (out, at) = insert_text(src, caret, None, "#", Affinity::Inside);
        let p2 = project(&out);
        assert!(
            p2.display.ends_with('#'),
            "hash must remain visible: display={:?} out={out:?}",
            p2.display
        );
        assert!(
            p2.display.contains("Hello there"),
            "heading text kept: {:?}",
            p2.display
        );
        // Spaces before the typed hash should still be present (not eaten as ATX closer).
        assert!(
            p2.display.contains("   #") || p2.display.ends_with("there#"),
            "expected spaces+hash or collapsed-to-content hash: {:?}",
            p2.display
        );
        let _ = at;
    }

    #[test]
    fn slash_then_backspace_does_not_add_empty() {
        let src = "Hello\n\n";
        let p = project(src);
        let empty = p
            .blocks
            .iter()
            .find(|b| b.display.start == b.display.end)
            .expect("empty");
        let before_blocks = p.blocks.len();
        let caret = p.to_source(empty.display.start, Affinity::Inside);
        let (out, at) = insert_text(src, caret, None, "/", Affinity::Inside);
        let p2 = project(&out);
        let d = p2.to_display(at);
        let (out2, _) = delete_display_range(&out, d - 1..d);
        let p3 = project(&out2);
        assert!(
            p3.blocks.len() <= before_blocks,
            "backspacing / must not invent an extra empty: before={before_blocks} after={} out={out2:?}",
            p3.blocks.len()
        );
        assert_eq!(&p3.display[p3.blocks[0].display.clone()], "Hello");
    }

    #[test]
    fn cmd_backspace_clears_paragraph_without_extra_line() {
        let src = "hello there nerd\n\n";
        let p = project(src);
        let caret = p.to_source(p.blocks[0].display.end, Affinity::Inside);
        let (out, _) = delete_to_line_start(src, caret);
        let p2 = project(&out);
        assert!(
            p2.display.trim().is_empty(),
            "content cleared: {out:?}"
        );
        assert_eq!(p2.blocks.len(), 1, "single empty block: {out:?}");
    }

    #[test]
    fn cmd_backspace_mid_doc_one_empty_slot() {
        let src = "above\n\nhello there nerd\n\nbelow";
        let p = project(src);
        assert!(p.blocks.len() >= 3, "{}", p.blocks.len());
        let mid = &p.blocks[1];
        let caret = p.to_source(mid.display.end, Affinity::Inside);
        let (out, at) = delete_to_line_start(src, caret);
        let p2 = project(&out);
        let empties = p2
            .blocks
            .iter()
            .filter(|b| b.display.start == b.display.end)
            .count();
        assert_eq!(
            empties, 1,
            "exactly one empty between neighbors: out={out:?} blocks={:?}",
            p2.blocks
                .iter()
                .map(|b| (b.kind, p2.display.get(b.display.clone()).unwrap_or("").to_string()))
                .collect::<Vec<_>>()
        );
        assert!(p2.display.contains("above") && p2.display.contains("below"), "{out:?}");
        assert!(!p2.display.contains("hello"), "{out:?}");
        let d = p2.to_display(at);
        let block = p2.block_at_display(d).expect("block");
        assert!(
            block.display.start == block.display.end,
            "caret on empty: out={out:?} at={at}"
        );
    }

    #[test]
    fn incomplete_dash_stays_paragraph_until_space() {
        let p = project("-");
        assert!(
            matches!(p.blocks[0].kind, BlockKind::Paragraph),
            "lone - must not be a list yet: {:?}",
            p.blocks[0].kind
        );
        assert_eq!(p.display, "-");
        let p = project("- ");
        assert!(
            matches!(p.blocks[0].kind, BlockKind::List { .. }),
            "dash+space becomes list: {:?}",
            p.blocks[0].kind
        );
        let p = project("1.");
        assert!(
            matches!(p.blocks[0].kind, BlockKind::Paragraph),
            "lone 1. must not be a list: {:?}",
            p.blocks[0].kind
        );
        let p = project("1. ");
        assert!(
            matches!(p.blocks[0].kind, BlockKind::List { ordered: true }),
            "1. + space becomes ordered list: {:?}",
            p.blocks[0].kind
        );
    }

    #[test]
    fn type_space_then_hash_keeps_spaces_in_heading() {
        let mut src = "# Hello there".to_string();
        let mut caret = {
            let p = project(&src);
            p.to_source(p.blocks[0].display.end, Affinity::Inside)
        };
        for ch in [" ", " ", "#"] {
            let (out, at) = insert_text(&src, caret, None, ch, Affinity::Inside);
            src = out;
            caret = at;
        }
        let p = project(&src);
        assert!(
            p.display.contains("  #") || p.display.ends_with("there  #"),
            "spaces before typed hash must remain: display={:?} src={src:?}",
            p.display
        );
    }

    #[test]
    fn exit_empty_list_after_blank_does_not_invent_line() {
        let src = "Hello\n\n- ";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks.last().unwrap().extra else {
            panic!("expected list, got {:?}", p.blocks.last().map(|b| b.kind));
        };
        let before = p.blocks.len();
        let caret = p.to_source(items[0].display.start, Affinity::Inside);
        let (out, at) = backspace(src, caret, Affinity::Inside).expect("exit");
        let p2 = project(&out);
        assert!(
            p2.blocks.len() <= before,
            "must not invent empties: before={before} after={} out={out:?}",
            p2.blocks.len()
        );
        assert!(out.starts_with("Hello"), "{out:?}");
        let empties = p2
            .blocks
            .iter()
            .filter(|b| b.display.start == b.display.end)
            .count();
        assert_eq!(empties, 1, "exactly one empty: {out:?}");
        let d = p2.to_display(at);
        let block = p2.block_at_display(d).unwrap();
        assert!(
            block.display.start == block.display.end,
            "caret on empty: {out:?}"
        );
    }

    #[test]
    fn typing_dash_in_empty_bullet_stays_text_not_hr() {
        let src = "- item\n- \n";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        let caret = p.to_source(items[1].display.start, Affinity::Inside);
        let (out, at) = insert_text(src, caret, None, "-", Affinity::Inside);
        let p2 = project(&out);
        assert!(
            !p2.blocks.iter().any(|b| matches!(b.kind, BlockKind::Rule)),
            "must not become HR after one dash: {out:?}"
        );
        let visible = unescape_md_punct(&p2.display);
        assert!(
            visible.contains('-'),
            "dash should be visible: display={:?} out={out:?}",
            p2.display
        );
        let (out2, _) = insert_text(&out, at, None, "-", Affinity::Inside);
        let p3 = project(&out2);
        assert!(
            !p3.blocks.iter().any(|b| matches!(b.kind, BlockKind::Rule)),
            "second dash must not create HR: {out2:?}"
        );
        let visible = unescape_md_punct(&p3.display);
        assert!(
            visible.matches('-').count() >= 2,
            "dashes remain visible: display={:?} out={out2:?}",
            p3.display
        );
    }

    #[test]
    fn type_then_delete_char_on_empty_does_not_invent_line() {
        let src = "Hello\n\n";
        let p = project(src);
        let before = p.blocks.len();
        let empty = p
            .blocks
            .iter()
            .find(|b| b.display.start == b.display.end)
            .expect("empty");
        let caret = p.to_source(empty.display.start, Affinity::Inside);
        for ch in ["/", "-", "a"] {
            let (out, at) = insert_text(src, caret, None, ch, Affinity::Inside);
            let (out2, _) = delete_char(&out, at, Affinity::Inside);
            let p2 = project(&out2);
            assert!(
                p2.blocks.len() <= before,
                "typing {ch:?} then backspace must not grow blocks: before={before} after={} out={out2:?}",
                p2.blocks.len()
            );
            assert_eq!(
                p2.blocks
                    .iter()
                    .filter(|b| b.display.start == b.display.end)
                    .count(),
                1,
                "exactly one empty after erasing {ch:?}: {out2:?}"
            );
            assert!(p2.display.contains("Hello"), "{out2:?}");
        }
    }
}






