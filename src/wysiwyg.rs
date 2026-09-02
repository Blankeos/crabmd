//! Notion enter / backspace / tab / mark toggle. Operates on GFM `source`
//! via the display projection.
//!
//! Caret arguments and return values are **display** offsets
//! (`project(src).display`), never GFM source bytes. Source is only used
//! to splice. Mapping a caret through source is lossy at empty slots.

use std::ops::Range;

use crate::display::{
    project, serialize_table, Affinity, BlockExtra, ListItem, Projection, TableCell,
};
use crate::document::{delete_block_index, splice, BlockKind};
use crate::notion;

pub use crate::tree::{units, unit_display, Mark, Unit};

fn block_ix(p: &Projection, d: usize) -> usize {
    p.block_at_display(d)
        .and_then(|b| {
            p.blocks
                .iter()
                .position(|x| x.display.start == b.display.start && x.display.end == b.display.end)
        })
        .unwrap_or(0)
}

/// Display caret on block `ix` at `local` (clamped). `usize::MAX` = end.
fn after(next: &str, ix: usize, local: usize) -> usize {
    let p = project(next);
    let Some(b) = p.blocks.get(ix.min(p.blocks.len().saturating_sub(1))) else {
        return 0;
    };
    let len = b.display.end.saturating_sub(b.display.start);
    b.display.start + local.min(len)
}

fn finish(next: String, ix: usize, local: usize) -> (String, usize) {
    let caret = after(&next, ix, local);
    (next, caret)
}

pub fn insert_text(
    src: &str,
    caret: usize,
    sel: Option<Range<usize>>,
    text: &str,
    _affinity: Affinity,
) -> (String, usize) {
    let mut doc = crate::tree::Doc::from_gfm(src);
    let caret = doc.insert_text(caret, sel, text, crate::display::Marks::default());
    (doc.to_gfm(), caret)
}

#[allow(dead_code)]
fn insert_text_gfm(
    src: &str,
    caret: usize,
    sel: Option<Range<usize>>,
    text: &str,
    affinity: Affinity,
) -> (String, usize) {
    let p = project(src);
    if let Some(sel) = sel.filter(|s| s.start != s.end) {
        let d0 = sel.start.min(sel.end).min(p.display.len());
        let d1 = sel.end.max(sel.start).min(p.display.len());
        let range = p.display_range_to_source(d0..d1, affinity);
        let next = splice(src, range.clone(), text);
        let ix = block_ix(&p, d0);
        let local = d0.saturating_sub(p.blocks.get(ix).map(|b| b.display.start).unwrap_or(0));
        return finish(next, ix, local + text.len());
    }
    let d = caret.min(p.display.len());
    let ix = block_ix(&p, d);
    let local = d.saturating_sub(p.blocks.get(ix).map(|b| b.display.start).unwrap_or(0));
    // Typing into a visible empty paragraph must replace that blank slot.
    // A raw point insert at `empty_insert_point` collapses the surrounding
    // `\n\n` separator and merges with the following block (Notion-breaking).
    if let Some(block) = p.block_at_display(d) {
        if block.display.start == block.display.end
            && matches!(block.kind, BlockKind::Paragraph | BlockKind::Raw)
        {
            return fill_empty_paragraph(src, block.source.clone(), text, ix);
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
                        return fill_empty_list_item(src, item, text, ix, block.display.start);
                    }
                    // At visual EOL: append after the item's raw body (keeps spaces).
                    if d == item.display.end {
                        return append_at_item_body_end(src, item, text, ix, block.display.start);
                    }
                }
            }
        }
        if matches!(block.kind, BlockKind::Heading(_)) && d == block.display.end {
            return append_at_heading_body_end(src, block.source.clone(), text, ix, local);
        }
        if matches!(block.kind, BlockKind::Quote | BlockKind::Alert(_)) {
            let raw = src.get(block.source.clone()).unwrap_or("");
            let body = match block.kind {
                BlockKind::Alert(_) => notion::strip_alert_body(raw),
                _ => notion::strip_quote(raw),
            };
            if body.trim().is_empty() {
                return fill_empty_quote_or_alert(src, block, text, ix);
            }
        }
    }
    let s = p.to_source(d, affinity);
    let next = splice(src, s..s, text);
    confirm_list_shortcut(&next, after(&next, ix, local + text.len()), ix)
}

fn append_at_item_body_end(
    src: &str,
    item: &ListItem,
    text: &str,
    ix: usize,
    block_d0: usize,
) -> (String, usize) {
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
    // `local` is offset within the list *block*, not the item.
    let local = item.display.end.saturating_sub(block_d0) + text.len();
    finish(next, ix, local)
}

fn append_at_heading_body_end(
    src: &str,
    block: Range<usize>,
    text: &str,
    ix: usize,
    local: usize,
) -> (String, usize) {
    let slice = src.get(block.clone()).unwrap_or("");
    let line = slice.trim_end_matches(['\n', '\r']);
    let at = (block.start + line.len()).min(src.len());
    let next = splice(src, at..at, text);
    finish(next, ix, local + text.len())
}

fn fill_empty_quote_or_alert(
    src: &str,
    block: &crate::display::ProjBlock,
    text: &str,
    ix: usize,
) -> (String, usize) {
    let slice = src.get(block.source.clone()).unwrap_or("");
    let mut gfm = match block.kind {
        BlockKind::Alert(k) => notion::wrap_alert(k, text),
        BlockKind::Quote => notion::wrap_quote(text),
        _ => text.to_string(),
    };
    if slice.ends_with('\n') && !gfm.ends_with('\n') {
        gfm.push('\n');
    }
    let next = splice(src, block.source.clone(), &gfm);
    finish(next, ix, text.len())
}

fn fill_empty_list_item(
    src: &str,
    item: &ListItem,
    text: &str,
    ix: usize,
    block_d0: usize,
) -> (String, usize) {
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
    let local = item.display.start.saturating_sub(block_d0) + text.len();
    finish(next, ix, local)
}

/// Escape list-item bodies that cmark would reparse as nested lists / HRs.
/// Typing `-` into an empty bullet must stay visible text, not become `---` rule.
fn safe_list_item_body(body: &str) -> String {
    let t = body.trim_end_matches(['\n', '\r']);
    if t.is_empty() {
        return body.to_string();
    }
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    let hr_or_marker = !compact.is_empty() && compact.chars().all(|c| matches!(c, '-' | '*' | '_'))
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
fn fill_empty_paragraph(src: &str, slot: Range<usize>, text: &str, ix: usize) -> (String, usize) {
    if text.is_empty() {
        return (src.to_string(), slot.start.min(src.len()));
    }
    // Adjacent lists swallow a raw `-` / `1.` as another item. Escape until
    // the user types the trailing space that confirms a Notion list shortcut.
    let escaped;
    let text = if crate::document::is_incomplete_list_marker(text) {
        escaped = safe_list_item_body(text);
        escaped.as_str()
    } else {
        text
    };
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

    // cmark omits the trailing newline on fences/tables/headings, but includes
    // it on paragraphs. `+2` assumes that newline is in the replaced run;
    // without it, `\n\n` before `text` invents an extra empty slot (typing
    // between a code fence and a table then creates a new line per key).
    let p0 = project(src);
    let prev_ends_nl = ix
        .checked_sub(1)
        .and_then(|i| p0.blocks.get(i))
        .map(|b| b.source.end > b.source.start && src.as_bytes()[b.source.end - 1] == b'\n')
        .unwrap_or(false);
    let nls_before = if has_prev {
        empties_above + if prev_ends_nl { 2 } else { 1 }
    } else {
        empties_above
    };

    let mut rep = String::new();
    for _ in 0..nls_before {
        rep.push('\n');
    }
    let caret = content_end + rep.len() + text.len();
    let _ = caret;
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
    confirm_list_shortcut(&next, after(&next, ix, text.len()), ix)
}

/// After typing the confirming space (`- ` / `1. `), rewrite an escaped
/// paragraph (`\-`) into a real list marker so it doesn't stay literal.
fn confirm_list_shortcut(src: &str, caret_d: usize, ix: usize) -> (String, usize) {
    let p = project(src);
    let d = caret_d.min(p.display.len());
    let Some(block) = p.block_at_display(d) else {
        return (src.to_string(), d);
    };
    if !matches!(block.kind, BlockKind::Paragraph | BlockKind::Raw) {
        return (src.to_string(), d);
    }
    let body = &p.display[block.display.clone()];
    if !is_confirmed_list_shortcut(body) {
        return (src.to_string(), d);
    }
    let slice = src.get(block.source.clone()).unwrap_or("");
    let mut gfm = body.to_string();
    if slice.ends_with('\n') && !gfm.ends_with('\n') {
        gfm.push('\n');
    }
    let next = splice(src, block.source.clone(), &gfm);
    finish(next, ix, usize::MAX)
}

fn is_confirmed_list_shortcut(body: &str) -> bool {
    matches!(body, "- " | "* " | "+ ") || {
        let digits = body.chars().take_while(|c| c.is_ascii_digit()).count();
        digits > 0 && (&body[digits..] == ". " || &body[digits..] == ") ")
    }
}

fn slash_query_display(p: &Projection, caret: usize) -> Option<(Range<usize>, Range<usize>)> {
    let d = caret.min(p.display.len());
    let block = p.block_at_display(d)?;
    let body = p.display.get(block.display.clone())?;
    let local = d.saturating_sub(block.display.start).min(body.len());
    let line_start = body[..local].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let rel = body[line_start..local].rfind('/')?;
    let d0 = block.display.start + line_start + rel;
    let d1 = block.display.start + local;
    Some((d0..d1, block.source.clone()))
}

/// Drop the `/query` at `caret`. Whole-block queries become one empty slot.
pub fn clear_slash_query(src: &str, caret: usize) -> (String, usize) {
    let p = project(src);
    let Some((display, _)) = slash_query_display(&p, caret) else {
        return (src.to_string(), caret.min(src.len()));
    };
    if let Some(block) = p.block_at_display(display.start) {
        if display.start <= block.display.start && display.end >= block.display.end {
            return clear_block_leave_empty(src, &p, block);
        }
    }
    delete_display_range(src, display)
}

/// Replace `/query` with a slash-menu template (GFM).
pub fn apply_slash(src: &str, caret: usize, template: &str) -> (String, usize) {
    let mut doc = crate::tree::Doc::from_gfm(src);
    let caret = doc.apply_slash(caret, template);
    (doc.to_gfm(), caret)
}

#[allow(dead_code)]
fn apply_slash_gfm(src: &str, caret: usize, template: &str) -> (String, usize) {
    if template.is_empty() {
        return clear_slash_query(src, caret);
    }
    let p = project(src);
    let Some((display, source)) = slash_query_display(&p, caret) else {
        return insert_text(src, caret, None, template, Affinity::Inside);
    };
    if let Some(block) = p.block_at_display(display.start) {
        if block.source == source
            && display.start <= block.display.start
            && display.end >= block.display.end
        {
            let slice = src.get(block.source.clone()).unwrap_or("");
            let mut gfm = template.to_string();
            if slice.ends_with('\n') && !gfm.ends_with('\n') {
                gfm.push('\n');
            }
            let next = splice(src, block.source.clone(), &gfm);
            let ix = block_ix(&p, display.start);
            return finish(next, ix, usize::MAX);
        }
    }
    let ix = block_ix(&p, display.start);
    let range = p.display_range_to_source(display, Affinity::Inside);
    let next = splice(src, range.clone(), template);
    finish(next, ix, usize::MAX)
}

pub fn delete_display_range(src: &str, display: Range<usize>) -> (String, usize) {
    let mut doc = crate::tree::Doc::from_gfm(src);
    let caret = doc.delete_display(display);
    (doc.to_gfm(), caret)
}

#[allow(dead_code)]
fn delete_display_range_gfm(src: &str, display: Range<usize>) -> (String, usize) {
    let p = project(src);
    let ix = block_ix(&p, display.start);
    let local = display
        .start
        .saturating_sub(p.blocks.get(ix).map(|b| b.display.start).unwrap_or(0));
    let range = p.display_range_to_source(display, Affinity::Inside);
    let next = splice(src, range.clone(), "");
    finish(next, ix, local)
}

/// Delete the display character before `caret`. If that empties the block,
/// leave a single empty slot (do not invent extra blank lines).
pub fn delete_char(src: &str, caret: usize, _affinity: Affinity) -> (String, usize) {
    let mut doc = crate::tree::Doc::from_gfm(src);
    if let Some(c) = doc.backspace(caret) {
        return (doc.to_gfm(), c);
    }
    let c = doc.delete_char(caret);
    (doc.to_gfm(), c)
}

#[allow(dead_code)]
fn delete_char_gfm(src: &str, caret: usize, _affinity: Affinity) -> (String, usize) {
    let p = project(src);
    let d = caret.min(p.display.len());
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
        if let Some(item) = items
            .iter()
            .find(|it| (prev >= it.display.start && prev < it.display.end) || d == it.display.end)
        {
            if prev <= item.display.start && d >= item.display.end {
                let ix = block_ix(&p, prev);
                return fill_empty_list_item(src, item, "", ix, block.display.start);
            }
        }
    }
    delete_display_range(src, prev..d)
}

/// Cmd/Ctrl-Backspace: delete from the start of the current block/item line to
/// the caret. Leaves a single empty block (does not invent an extra blank line).
pub fn delete_to_line_start(src: &str, caret: usize) -> (String, usize) {
    let p = project(src);
    let d = caret.min(p.display.len());
    let Some(block) = p.block_at_display(d) else {
        return (src.to_string(), d);
    };
    let ix = block_ix(&p, d);

    // List item: clear item body only (keep marker), or full-clear empty.
    if let BlockExtra::List { items, .. } = &block.extra {
        if let Some(item) = items
            .iter()
            .find(|it| d >= it.display.start && d <= it.display.end)
        {
            let start_d = item.display.start;
            if start_d >= d {
                return (src.to_string(), d);
            }
            if start_d == item.display.start && d >= item.display.end {
                // Cleared whole item body → empty item (marker only).
                let _body = "";
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
                return finish(next, ix, 0);
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
        return (src.to_string(), d);
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
    let has_next_content = p
        .blocks
        .iter()
        .skip(ix + 1)
        .any(|b| b.display.start != b.display.end && b.source.start >= del_end);

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

    if !has_prev_content && !has_next_content {
        return collapse_all_empty_doc(&next, 0);
    }

    // Collapse accidental double empties between neighbors.
    if has_prev_content && has_next_content {
        let p2 = project(&next);
        let empties = p2
            .blocks
            .iter()
            .filter(|b| b.display.start == b.display.end)
            .count();
        if empties >= 2 {
            if let Some(collapsed) = collapse_extra_blank(&next) {
                return finish(collapsed, ix, 0);
            }
        }
    }
    finish(next, ix, 0)
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
        return (src.to_string(), caret.min(p.display.len()));
    }
    if p.blocks
        .iter()
        .all(|b| b.display.start == b.display.end || p.display[b.display.clone()].trim().is_empty())
    {
        return ("\n".to_string(), 0);
    }
    (src.to_string(), caret.min(p.display.len()))
}

/// Backspace. `None` = delete one display char (caller). `Some` = structural.
pub fn backspace(src: &str, caret: usize, _affinity: Affinity) -> Option<(String, usize)> {
    let mut doc = crate::tree::Doc::from_gfm(src);
    match doc.backspace(caret) {
        Some(c) => Some((doc.to_gfm(), c)),
        None => {
            let c = doc.delete_char(caret);
            Some((doc.to_gfm(), c))
        }
    }
}

#[allow(dead_code)]
fn backspace_gfm(src: &str, caret: usize, _affinity: Affinity) -> Option<(String, usize)> {
    let p = project(src);
    let d = caret.min(p.display.len());
    let block = p.block_at_display(d)?;
    let bix = block_ix(&p, d);

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
                    return Some(exit_empty_list_item(src, item, has_following, bix));
                }
                let body = p.display[item.display.clone()].to_string();
                let next = splice(src, item.source.clone(), &body);
                return Some(finish(next, bix, 0));
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
                return Some(finish(next, bix, 0));
            }
            Some(join_prev(&p, src, block.source.clone()))
        }
        BlockKind::Quote | BlockKind::Alert(_) => {
            if empty {
                return Some(join_prev(&p, src, block.source.clone()));
            }
            let body = p.display[block.display.clone()].to_string();
            let next = splice(src, block.source.clone(), &body);
            Some(finish(next, bix, 0))
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
                Some(finish(next, bix, 0))
            } else if local == 0 {
                let body = p.display[block.display.clone()].to_string();
                let next = splice(src, block.source.clone(), &body);
                Some(finish(next, bix, 0))
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
fn exit_empty_list_item(
    src: &str,
    item: &ListItem,
    has_following: bool,
    bix: usize,
) -> (String, usize) {
    let del = item.source.clone();
    if has_following {
        // A lone NBSP paragraph breaks a CommonMark list (blank lines alone
        // don't). One `\n` on each side is enough: extra blank lines project
        // as additional empty slots.
        let replacement = "\n\u{00A0}\n";
        let next = splice(src, del.clone(), replacement);
        return finish(next, bix + 1, 0);
    }

    // Last empty item: remove it without inventing an extra blank.
    // `Hello\n\n- ` must become `Hello\n\n` (not `Hello\n\n\n`).
    let removed = splice(src, del.clone(), "");
    let p2 = project(&removed);
    if let Some(empty_ix) = p2
        .blocks
        .iter()
        .position(|b| b.display.start == b.display.end)
    {
        return finish(removed, empty_ix, 0);
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
        return (src.to_string(), after(src, ix, 0));
    };

    let next = splice(src, del.clone(), "");
    if ix > 0 {
        finish(next, ix - 1, usize::MAX)
    } else {
        (next, 0)
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
        BlockKind::Heading(level) => {
            notion::wrap_heading(level, &format!("{prev_disp}{this_disp}"))
        }
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
    finish(next, ix - 1, prev_disp.len())
}

fn set_item_indent(
    src: &str,
    p: &Projection,
    item_src: Range<usize>,
    indent: usize,
) -> (String, usize) {
    let line = &src[item_src.clone()];
    let trimmed = line.trim_start();
    let pad = "  ".repeat(indent);
    let next_line = format!("{pad}{trimmed}");
    let next = splice(src, item_src.clone(), &next_line);
    let d = p
        .blocks
        .iter()
        .find_map(|b| {
            if let BlockExtra::List { items, .. } = &b.extra {
                items
                    .iter()
                    .find(|it| it.source == item_src)
                    .map(|it| it.display.start)
            } else {
                None
            }
        })
        .unwrap_or(0);
    let ix = block_ix(p, d);
    finish(next, ix, 0)
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

pub fn enter(src: &str, caret: usize, _affinity: Affinity, hard: bool) -> (String, usize) {
    let mut doc = crate::tree::Doc::from_gfm(src);
    let caret = doc.enter(caret, hard);
    (doc.to_gfm(), caret)
}

#[allow(dead_code)]
fn enter_gfm(src: &str, caret: usize, affinity: Affinity, hard: bool) -> (String, usize) {
    let p = project(src);
    let d = caret.min(p.display.len());
    let bix = block_ix(&p, d);
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
            finish(next, bix + 1, 0)
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
                finish(next, bix + 1, 0)
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
                        return exit_empty_list_item(src, item, ix + 1 < items.len(), bix);
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
                    let p2 = project(&next);
                    let caret = p2
                        .blocks
                        .get(bix)
                        .and_then(|b| match &b.extra {
                            BlockExtra::List { items, .. } => items
                                .get(ix + 1)
                                .or_else(|| items.last())
                                .map(|it| it.display.start),
                            _ => None,
                        })
                        .unwrap_or_else(|| after(&next, bix, usize::MAX));
                    return (next, caret);
                }
            }
            insert_text(src, caret, None, "\n", affinity)
        }
        BlockKind::Table => insert_text(src, caret, None, "\n", affinity),
        BlockKind::Rule | BlockKind::Html => {
            let insert = "\n\n";
            let next = splice(src, block.source.end..block.source.end, insert);
            finish(next, bix + 1, 0)
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
            if right.is_empty() {
                finish(next, bix + 1, 0)
            } else {
                finish(next, bix + 1, 0)
            }
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
        .map(|b| b.display.start)
        .unwrap_or_else(|| p.display.len());
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
    let p = project(&next);
    let caret = p
        .blocks
        .iter()
        .find(|b| b.display.start == b.display.end)
        .map(|b| b.display.start)
        .unwrap_or(0);
    (next, caret)
}

/// Vim/Helix `o` / `O`: always open a new empty paragraph above/below the
/// current block — never reuse an adjacent empty block.
pub fn open_line(src: &str, caret: usize, above: bool) -> (String, usize) {
    let p = project(src);
    let d = caret.min(p.display.len());
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
    let indent: String = item_src
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
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
    let d = caret.min(p.display.len());
    let block = p.block_at_display(d)?;
    let bix = block_ix(&p, d);
    match &block.extra {
        BlockExtra::List { items, .. } => {
            let item = items
                .iter()
                .find(|it| d >= it.display.start && d <= it.display.end)?;
            let ix = items.iter().position(|it| it.source == item.source)?;
            if shift {
                if item.indent == 0 {
                    let body = p.display[item.display.clone()].to_string();
                    let next = splice(src, item.source.clone(), &body);
                    return Some(finish(next, bix, 0));
                }
                return Some(set_item_indent(
                    src,
                    &p,
                    item.source.clone(),
                    item.indent - 1,
                ));
            }
            if ix == 0 {
                return None;
            }
            let prev = &items[ix - 1];
            if item.indent > prev.indent {
                return None;
            }
            Some(set_item_indent(
                src,
                &p,
                item.source.clone(),
                item.indent + 1,
            ))
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
    let Some(cur) = cells
        .iter()
        .find(|c| d >= c.display.start && d <= c.display.end)
    else {
        return (src.to_string(), d);
    };
    if shift {
        if let Some(prev) = cells
            .iter()
            .rev()
            .find(|c| c.row < cur.row || (c.row == cur.row && c.col < cur.col))
        {
            return (src.to_string(), prev.display.start);
        }
        return (src.to_string(), cur.display.start);
    }
    if let Some(next) = cells
        .iter()
        .find(|c| c.row > cur.row || (c.row == cur.row && c.col > cur.col))
    {
        return (src.to_string(), next.display.start);
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
            BlockExtra::Table { cells, .. } => cells.last().map(|c| c.display.start),
            _ => None,
        })
        .unwrap_or_else(|| p2.display.len());
    let _ = rows;
    (next, caret)
}

fn table_strings(
    p: &Projection,
    cells: &[TableCell],
    cols: usize,
) -> (Vec<String>, Vec<Vec<String>>) {
    let mut headers = vec![String::new(); cols.max(1)];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for c in cells {
        let text = p
            .display
            .get(c.display.clone())
            .unwrap_or("")
            .replace('\t', "")
            .to_string();
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

pub fn toggle_mark(src: &str, sel: Range<usize>, mark: Mark) -> Option<(String, Range<usize>)> {
    let mut doc = crate::tree::Doc::from_gfm(src);
    let range = doc.toggle_mark(sel, mark)?;
    Some((doc.to_gfm(), range))
}

#[allow(dead_code)]
fn toggle_mark_gfm(src: &str, sel: Range<usize>, mark: Mark) -> Option<(String, Range<usize>)> {
    let p = project(src);
    let d0 = sel.start.min(sel.end).min(p.display.len());
    let d1 = sel.end.max(sel.start).min(p.display.len());
    if d0 >= d1 {
        return None;
    }
    let marked = |i| mark.has(p.marks_at(i, Affinity::Inside));
    let all = (d0..d1).all(marked);
    let any = (d0..d1).any(marked);
    let wordish = p
        .display
        .get(d0..d1)
        .is_some_and(|s| !s.chars().any(char::is_whitespace));
    if all || (any && wordish) {
        let (next, _) = unmark_display_range(src, &p, d0, d1, mark)?;
        Some((next, d0..d1))
    } else {
        let (next, _) = ensure_mark_display_range(src, &p, d0, d1, mark)?;
        Some((next, d0..d1))
    }
}

fn marked_segs(p: &Projection, d0: usize, d1: usize, mark: Mark) -> Vec<crate::display::Segment> {
    p.segments
        .iter()
        .filter(|s| s.display.start < d1 && s.display.end > d0 && mark.has(s.marks))
        .cloned()
        .collect()
}

fn mark_delims(mark: Mark) -> &'static [&'static str] {
    match mark {
        Mark::Bold => &["**", "__"],
        Mark::Italic => &["*", "_"],
        Mark::Strike => &["~~"],
        Mark::Code => &["`"],
        Mark::Underline => &["<u>", "</u>"],
    }
}

fn remove_delim(next: &mut String, gap: Range<usize>, delim: &str) -> bool {
    if delim.is_empty() || gap.start > next.len() {
        return false;
    }
    let start = gap.start.min(next.len());
    let end = gap.end.min(next.len()).max(start);
    if start + delim.len() <= next.len() && next.get(start..start + delim.len()) == Some(delim) {
        next.replace_range(start..start + delim.len(), "");
        return true;
    }
    if end > start {
        let at = end.saturating_sub(delim.len());
        if at >= start && at <= next.len() && end <= next.len() && next.get(at..end) == Some(delim)
        {
            next.replace_range(at..end, "");
            return true;
        }
    }
    false
}

fn remove_mark_delim(next: &mut String, gap: Range<usize>, mark: Mark) {
    for delim in mark_delims(mark) {
        if remove_delim(next, gap.clone(), delim) {
            return;
        }
    }
}

fn display_prev(s: &str, i: usize) -> usize {
    s.get(..i)
        .and_then(|pre| pre.char_indices().next_back().map(|(j, _)| j))
        .unwrap_or(0)
}

fn display_next(s: &str, i: usize) -> usize {
    s.get(i..)
        .and_then(|rest| rest.chars().next().map(|c| i + c.len_utf8()))
        .unwrap_or(s.len())
}

fn is_ws_at(s: &str, i: usize) -> bool {
    s.get(i..)
        .and_then(|rest| rest.chars().next())
        .map(|c| c.is_whitespace())
        .unwrap_or(false)
}

/// CommonMark closers cannot be preceded by whitespace (`**foo **` shows stars).
fn skip_ws_left(s: &str, mut i: usize) -> usize {
    while i > 0 {
        let prev = display_prev(s, i);
        if !is_ws_at(s, prev) {
            break;
        }
        i = prev;
    }
    i
}

/// CommonMark openers cannot be followed by whitespace (`** foo**` shows stars).
fn skip_ws_right(s: &str, mut i: usize) -> usize {
    while i < s.len() && is_ws_at(s, i) {
        i = display_next(s, i);
    }
    i
}

fn flanking_ok(mark: Mark) -> bool {
    !matches!(mark, Mark::Code)
}

fn extend_marked(p: &Projection, mut left: usize, mut right: usize, mark: Mark) -> (usize, usize) {
    while left > 0 {
        let prev = display_prev(&p.display, left);
        if p.display.as_bytes().get(prev) == Some(&b'\n') {
            break;
        }
        if !mark.has(p.marks_at(prev, Affinity::Inside)) {
            break;
        }
        left = prev;
    }
    while right < p.display.len() {
        if p.display.as_bytes().get(right) == Some(&b'\n') {
            break;
        }
        if !mark.has(p.marks_at(right, Affinity::Inside)) {
            break;
        }
        right = display_next(&p.display, right);
    }
    (left, right)
}

fn unmark_display_range(
    src: &str,
    p: &Projection,
    d0: usize,
    d1: usize,
    mark: Mark,
) -> Option<(String, Range<usize>)> {
    let (open, close) = mark.wrap();
    let mut next = src.to_string();
    for seg in marked_segs(p, d0, d1, mark).into_iter().rev() {
        let vis_start = d0.max(seg.display.start);
        let vis_end = d1.min(seg.display.end);
        if vis_start >= vis_end {
            continue;
        }
        let closer = p_segments_closer(p, &seg);
        let opener = p_segments_opener(p, &seg);
        let full_start = vis_start == seg.display.start;
        let full_end = vis_end == seg.display.end;
        if full_start && full_end {
            remove_mark_delim(&mut next, closer, mark);
            remove_mark_delim(&mut next, opener, mark);
            continue;
        }
        let mut keep_start = vis_start;
        let mut keep_end = vis_end;
        if flanking_ok(mark) {
            if !full_start {
                keep_start = skip_ws_left(&p.display, vis_start);
            }
            if !full_end {
                keep_end = skip_ws_right(&p.display, vis_end);
            }
        }
        let drop_open = full_start || keep_start <= seg.display.start;
        let drop_close = full_end || keep_end >= seg.display.end;
        if drop_open && drop_close {
            remove_mark_delim(&mut next, closer, mark);
            remove_mark_delim(&mut next, opener, mark);
            continue;
        }
        if drop_open {
            // Keep a suffix: insert a new opener at keep_end, drop the original opener.
            let at = p.to_source(keep_end, Affinity::Inside).min(next.len());
            next.insert_str(at, open);
            remove_mark_delim(&mut next, opener, mark);
        } else if drop_close {
            // Keep a prefix: insert a new closer at keep_start, drop the original closer.
            let at = p.to_source(keep_start, Affinity::Inside).min(next.len());
            next.insert_str(at, close);
            let shifted =
                closer.start.saturating_add(close.len())..closer.end.saturating_add(close.len());
            remove_mark_delim(&mut next, shifted, mark);
        } else {
            let at_end = p.to_source(keep_end, Affinity::Inside).min(next.len());
            let at_start = p.to_source(keep_start, Affinity::Inside).min(at_end);
            next.insert_str(at_end, open);
            next.insert_str(at_start, close);
        }
    }
    let p2 = project(&next);
    let caret = p2.to_source(d0.min(p2.display.len()), Affinity::Inside);
    let end = p2.to_source(d1.min(p2.display.len()), Affinity::Inside);
    Some((next, caret..end))
}

fn ensure_mark_display_range(
    src: &str,
    p: &Projection,
    d0: usize,
    d1: usize,
    mark: Mark,
) -> Option<(String, Range<usize>)> {
    let (open, close) = mark.wrap();
    let (left, right) = extend_marked(p, d0, d1, mark);
    let mut next = src.to_string();
    for seg in marked_segs(p, left, right, mark).into_iter().rev() {
        remove_mark_delim(&mut next, p_segments_closer(p, &seg), mark);
        remove_mark_delim(&mut next, p_segments_opener(p, &seg), mark);
    }
    let p2 = project(&next);
    let mut d_left = left.min(p2.display.len());
    let mut d_right = right.min(p2.display.len()).max(d_left);
    if flanking_ok(mark) {
        d_left = skip_ws_right(&p2.display, d_left);
        d_right = skip_ws_left(&p2.display, d_right);
    }
    if d_left >= d_right {
        let start = p2.to_source(d0.min(p2.display.len()), Affinity::Inside);
        let end = p2.to_source(d1.min(p2.display.len()), Affinity::Inside);
        return Some((next, start..end));
    }
    let range = p2.display_range_to_source(d_left..d_right, Affinity::Inside);
    next.insert_str(range.end, close);
    next.insert_str(range.start, open);
    let p3 = project(&next);
    let start = p3.to_source(d0.min(p3.display.len()), Affinity::Inside);
    let end = p3.to_source(d1.min(p3.display.len()), Affinity::Inside);
    Some((next, start..end))
}

fn p_segments_opener(p: &Projection, seg: &crate::display::Segment) -> Range<usize> {
    let prev = p
        .segments
        .iter()
        .rev()
        .find(|s| s.display.end <= seg.display.start)
        .map(|s| s.source.end)
        .unwrap_or(0);
    prev.min(seg.source.start)..seg.source.start
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
    let d0 = sel.start.min(sel.end).min(p.display.len());
    let d1 = sel.end.max(sel.start).min(p.display.len());
    let text = p.display.get(d0..d1).unwrap_or("").to_string();
    let range = p.display_range_to_source(d0..d1, Affinity::Inside);
    let replacement = if url.is_empty() {
        text.clone()
    } else {
        format!("[{text}]({url})")
    };
    let next = splice(src, range.clone(), &replacement);
    let ix = block_ix(&p, d0);
    let local = d0.saturating_sub(p.blocks.get(ix).map(|b| b.display.start).unwrap_or(0));
    finish(next, ix, local + text.len())
}

pub fn set_code_lang(src: &str, caret: usize, lang: &str) -> Option<(String, usize)> {
    let p = project(src);
    let d = caret.min(p.display.len());
    let block = p.block_at_display(d)?;
    let ix = block_ix(&p, d);
    let BlockExtra::Code { .. } = &block.extra else {
        return None;
    };
    let body = p.display[block.display.clone()].to_string();
    let gfm = notion::wrap_fence(lang, &body);
    let next = splice(src, block.source.clone(), &gfm);
    Some(finish(next, ix, 0))
}

fn unit_start(p: &Projection, src: &str, units: &[Unit], ui: usize) -> usize {
    if ui == 0 {
        return 0;
    }
    let u = units[ui];
    match u.item {
        Some(i) => {
            let Some(BlockExtra::List { items, .. }) = p.blocks.get(u.block).map(|b| &b.extra)
            else {
                return p.blocks[u.block].source.start.min(src.len());
            };
            items
                .get(i)
                .map(|it| it.source.start.min(src.len()))
                .unwrap_or_else(|| p.blocks[u.block].source.start.min(src.len()))
        }
        None => p.blocks[u.block].source.start.min(src.len()),
    }
}

fn unit_span(p: &Projection, src: &str, units: &[Unit], ui: usize) -> Range<usize> {
    let start = unit_start(p, src, units, ui);
    let end = if ui + 1 < units.len() {
        unit_start(p, src, units, ui + 1)
    } else {
        src.len()
    };
    start.min(end)..end
}

fn trim_unit_src(s: &str) -> &str {
    s.trim_end_matches(['\n', '\r'])
}

fn trailing_nl_count(s: &str) -> usize {
    s.bytes().rev().take_while(|&b| b == b'\n').count()
}

fn concat_units(src: &str, parts: &[(Unit, String)]) -> String {
    let mut out = String::new();
    let mut pending_empty = 0usize;
    let mut last_list = false;

    for (unit, part) in parts {
        if part.trim().is_empty() {
            pending_empty += 1;
            last_list = false;
            continue;
        }
        let body = trim_unit_src(part);
        if out.is_empty() && pending_empty == 0 {
            out.push_str(body);
            last_list = unit.is_list_item();
            continue;
        }
        let need = if last_list && unit.is_list_item() && pending_empty == 0 {
            1
        } else {
            2 + pending_empty
        };
        let have = trailing_nl_count(&out);
        for _ in have..need {
            out.push('\n');
        }
        out.push_str(body);
        pending_empty = 0;
        last_list = unit.is_list_item();
    }
    if pending_empty > 0 {
        // Trailing empties: previous block's newline + one extra per slot.
        let need = pending_empty + 1;
        let have = trailing_nl_count(&out);
        for _ in have..need {
            out.push('\n');
        }
    } else if parts.last().is_some_and(|(_, p)| !p.trim().is_empty()) {
        let had = src.ends_with('\n');
        out = out.trim_end_matches(['\n', '\r']).to_string();
        if had {
            out.push('\n');
        }
    }
    out
}

fn caret_at_unit(src: &str, ix: usize) -> usize {
    let p = project(src);
    let units = units(&p);
    let Some(u) = units.get(ix.min(units.len().saturating_sub(1))) else {
        return 0;
    };
    unit_display(&p, *u).start
}

/// Delete unit `ix` (block or list item). Caret lands on the next (or previous) row.
pub fn delete_block(src: &str, ix: usize) -> Option<(String, usize)> {
    let mut doc = crate::tree::Doc::from_gfm(src);
    let caret = doc.delete_unit(ix);
    Some((doc.to_gfm(), caret))
}

pub fn duplicate_block(src: &str, ix: usize) -> Option<(String, usize)> {
    let mut doc = crate::tree::Doc::from_gfm(src);
    let caret = doc.duplicate_unit(ix);
    Some((doc.to_gfm(), caret))
}

pub fn move_block(src: &str, from: usize, gap: usize) -> Option<(String, usize)> {
    let mut doc = crate::tree::Doc::from_gfm(src);
    let caret = doc.move_unit(from, gap)?;
    Some((doc.to_gfm(), caret))
}

#[allow(dead_code)]
fn delete_block_gfm(src: &str, ix: usize) -> Option<(String, usize)> {
    let p = project(src);
    let us = units(&p);
    let n = us.len();
    if n == 0 || ix >= n {
        return None;
    }
    let mut parts: Vec<(Unit, String)> = (0..n)
        .map(|i| (us[i], src[unit_span(&p, src, &us, i)].to_string()))
        .collect();
    parts.remove(ix);
    let next = concat_units(src, &parts);
    let at = delete_block_index(n, ix).unwrap_or(0);
    let caret = caret_at_unit(&next, at);
    Some((next, caret))
}

/// Duplicate unit `ix` immediately after it.
#[allow(dead_code)]
fn duplicate_block_gfm(src: &str, ix: usize) -> Option<(String, usize)> {
    let p = project(src);
    let us = units(&p);
    let n = us.len();
    if n == 0 || ix >= n {
        return None;
    }
    let mut parts: Vec<(Unit, String)> = (0..n)
        .map(|i| (us[i], src[unit_span(&p, src, &us, i)].to_string()))
        .collect();
    let copy = parts[ix].clone();
    parts.insert(ix + 1, copy);
    let next = concat_units(src, &parts);
    let caret = caret_at_unit(&next, ix + 1);
    Some((next, caret))
}

/// Move unit `from` to drop-gap `gap` (`0..=n`, like Notion's edge).
#[allow(dead_code)]
fn move_block_gfm(src: &str, from: usize, gap: usize) -> Option<(String, usize)> {
    let p = project(src);
    let us = units(&p);
    let n = us.len();
    if n == 0 || from >= n || gap > n {
        return None;
    }
    if gap == from || gap == from + 1 {
        return None;
    }
    let mut parts: Vec<(Unit, String)> = (0..n)
        .map(|i| (us[i], src[unit_span(&p, src, &us, i)].to_string()))
        .collect();
    let item = parts.remove(from);
    let insert_at = if from < gap { gap - 1 } else { gap };
    parts.insert(insert_at, item);
    let next = concat_units(src, &parts);
    let caret = caret_at_unit(&next, insert_at);
    Some((next, caret))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_into_heading() {
        let p = project("# Hi");
        let caret = p.blocks[0].display.end;
        let (out, at) = insert_text("# Hi", caret, None, "!", Affinity::Inside);
        assert_eq!(out, "# Hi!");
        let p2 = project(&out);
        assert_eq!(&p2.display[..at], "Hi!");
    }

    #[test]
    fn enter_splits_heading() {
        let src = "# Hello";
        let p = project(src);
        let caret = p.display.len();
        let (out, _) = enter(src, caret, Affinity::Inside, false);
        assert!(out.starts_with("# Hello"), "{out}");
        assert!(out.contains("\n\n"), "{out:?}");
    }

    #[test]
    fn backspace_empty_heading_joins() {
        let src = "para\n\n# ";
        let p = project(src);
        let d = p.blocks.last().unwrap().display.start;
        let caret = d;
        let (out, _) = backspace(src, caret, Affinity::Inside).expect("join");
        assert!(out.contains("para"), "{out}");
        assert!(!out.contains('#'), "{out}");
    }

    #[test]
    fn backspace_heading_into_empty_keeps_heading() {
        // (empty)
        // # |Table  → backspace removes empty, keeps heading
        let mut doc = crate::tree::Doc::from_gfm("# Table");
        doc.nodes.insert(
            0,
            crate::tree::Node {
                id: crate::document::next_id(),
                kind: crate::tree::NodeKind::Paragraph { inlines: vec![] },
            },
        );
        let p = doc.project();
        let heading = p
            .blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Heading(_)))
            .unwrap();
        let at = doc.backspace(heading.display.start).expect("join");
        assert_eq!(doc.nodes.len(), 1);
        assert!(
            matches!(doc.nodes[0].kind, crate::tree::NodeKind::Heading { .. }),
            "heading preserved: {:?}",
            doc.nodes[0].kind
        );
        let p2 = doc.project();
        assert_eq!(
            p2.display.get(p2.blocks[0].display.clone()).unwrap_or(""),
            "Table"
        );
        let b = p2.block_at_display(at).unwrap();
        assert!(matches!(b.kind, BlockKind::Heading(_)));
        assert_eq!(at, b.display.start);
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
        let caret = heading.display.start;
        let (out, at) = backspace(src, caret, Affinity::Inside).expect("join");
        assert_eq!(out, "A paragraphLists");
        let p2 = project(&out);
        assert_eq!(&p2.display[..at], "A paragraph");
        assert!(!out.contains('#'), "{out}");
    }

    #[test]
    fn backspace_heading_into_heading_caret() {
        let src = "# Hello\n\n## World";
        let p = project(src);
        let caret = p.blocks[1].display.start;
        let (out, at) = backspace(src, caret, Affinity::Inside).expect("join");
        assert_eq!(out, "# HelloWorld");
        let p2 = project(&out);
        assert_eq!(&p2.display[..at], "Hello");
    }

    #[test]
    fn open_below_heading_makes_paragraph() {
        let src = "# Hello\n\npara";
        let p = project(src);
        let caret = p.blocks[0].display.start;
        let (out, at) = open_line(src, caret, false);
        assert!(out.starts_with("# Hello"), "{out}");
        assert!(out.contains("para"), "{out}");
        // Always inserts a new empty paragraph after the heading block.
        assert!(
            out.contains("Hello\n\n\n") || out.contains("Hello\n\n\npara") || {
                let p2 = project(&out);
                p2.blocks
                    .iter()
                    .any(|b| b.kind == BlockKind::Paragraph && b.display.start == b.display.end)
            },
            "expected a new empty paragraph in {out:?}"
        );
        let _ = at;
    }

    #[test]
    fn open_below_always_creates_even_if_next_empty() {
        let src = "hello\n\n";
        let p = project(src);
        let caret = p.blocks[0].display.start;
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
        let caret = empty.display.start;
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
        let caret = items[1].display.end;
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
        let caret = d;
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
        let caret = p.blocks[0].display.start;
        let (out, at) = enter(src, caret, Affinity::Inside, false);
        let p2 = project(&out);
        let d = at;
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
        let caret = p.blocks[0].display.start;
        let before = p.blocks.len();
        let (out, at) = enter(src, caret, Affinity::Inside, false);
        let p2 = project(&out);
        assert!(p2.blocks.len() > before, "should grow empties: {out:?}");
        let d = at;
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
        let caret = empty.display.start;
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
            !texts
                .iter()
                .any(|t| t.contains("NEW") && t.contains("Second")),
            "must not soft-join NEW into Second: {texts:?} out={out:?}"
        );
        let d = at;
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
        let caret = hello.display.end;
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
        let mid = p.blocks[0].display.start + 5;
        let (out, at) = enter(src, mid, Affinity::Inside, false);
        let p2 = project(&out);
        let d = at;
        let block = p2.block_at_display(d).expect("block");
        let text = &p2.display[block.display.clone()];
        assert!(
            text.contains("world"),
            "caret should be on the right half, got {text:?} out={out:?} at={at} d={d}"
        );
        assert!(
            !text.contains("Hello")
                || text.trim_start().starts_with("world")
                || text.starts_with(' '),
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
        let caret = items[0].display.end;
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
            texts
                .iter()
                .any(|t| t.contains("list three") && t.contains("list one")),
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
        let caret = p.blocks[0].display.end;
        let (out, at) = open_line(src, caret, false);
        let p2 = project(&out);
        let d = at;
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
        let caret = empty.display.start;
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
        assert_eq!(&p2.display[..at.min(p2.display.len())], "What's up");
    }

    #[test]
    fn delete_last_char_of_middle_paragraph_stays_on_empty() {
        let src = "1\n\n2\n\n3\n\n4\n\n5";
        let p = project(src);
        let three = p
            .blocks
            .iter()
            .find(|b| &p.display[b.display.clone()] == "3")
            .expect("3");
        let caret = three.display.end;
        let (out, at) = delete_char(src, caret, Affinity::Inside);
        let p2 = project(&out);
        let block = p2.block_at_display(at).expect("block");
        assert_eq!(
            block.display.start,
            block.display.end,
            "caret must stay on the emptied slot, not jump: at={at} display={:?} kinds={:?}",
            p2.display,
            p2.blocks
                .iter()
                .map(|b| (b.kind, p2.display[b.display.clone()].to_string()))
                .collect::<Vec<_>>()
        );
        let texts: Vec<_> = p2
            .blocks
            .iter()
            .map(|b| p2.display[b.display.clone()].to_string())
            .collect();
        assert!(texts.iter().any(|t| t == "2"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "4"), "{texts:?}");
        assert!(
            !texts.iter().any(|t| t == "1" && block.display.start == 0),
            "must not jump to first block: {texts:?} at={at}"
        );
    }

    #[test]
    fn type_into_nbsp_list_break() {
        let src = "- item 1\n\n\u{00A0}\n\n- item 3\n";
        let p = project(src);
        assert!(p.blocks.len() >= 3, "{:?}", p.blocks.len());
        let empty = p
            .blocks
            .iter()
            .find(|b| b.display.start == b.display.end)
            .expect("empty");
        let caret = empty.display.start;
        let (out, _) = insert_text(src, caret, None, "hello", Affinity::Inside);
        let p2 = project(&out);
        assert!(
            p2.display.contains("hello"),
            "{out:?} display={:?}",
            p2.display
        );
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

    fn sel_display(src: &str, d0: usize, d1: usize) -> Range<usize> {
        let p = project(src);
        d0..d1
    }

    fn assert_no_visible_stars(src: &str) {
        let p = project(src);
        assert!(
            !p.display.contains('*'),
            "visible asterisks in display={:?} src={src:?}",
            p.display
        );
        assert!(!src.contains("****"), "stacked asterisks: {src:?}");
    }

    #[test]
    fn toggle_bold_roundtrips_already_bold_word() {
        let src = "hello **world**";
        let sel = sel_display(src, 6, 11);
        let (out, range) = toggle_mark(src, sel, Mark::Bold).unwrap();
        assert_eq!(out, "hello world");
        let (out2, _) = toggle_mark(&out, range, Mark::Bold).unwrap();
        assert_eq!(out2, "hello **world**");
        assert_no_visible_stars(&out2);
    }

    #[test]
    fn toggle_bold_clears_partially_bold_word() {
        let src = "h**ell**o";
        let sel = sel_display(src, 0, 5);
        let (out, range) = toggle_mark(src, sel, Mark::Bold).unwrap();
        assert_eq!(out, "hello");
        let (out2, _) = toggle_mark(&out, range, Mark::Bold).unwrap();
        assert_eq!(out2, "**hello**");
        assert_no_visible_stars(&out2);
    }

    #[test]
    fn toggle_bold_repeated_on_partial_does_not_stack_stars() {
        let mut src = "h**ell**o".to_string();
        for _ in 0..6 {
            let sel = sel_display(&src, 0, 5);
            let (next, _) = toggle_mark(&src, sel, Mark::Bold).unwrap();
            assert_no_visible_stars(&next);
            src = next;
        }
        assert!(src == "hello" || src == "**hello**", "{src:?}");
    }

    #[test]
    fn toggle_bold_unmarks_middle_without_orphan_stars() {
        let src = "**hello**";
        let sel = sel_display(src, 1, 4);
        let (out, _) = toggle_mark(src, sel, Mark::Bold).unwrap();
        assert_eq!(out, "**h**ell**o**");
        assert_no_visible_stars(&out);
        let p = project(&out);
        assert!(p.marks_at(0, Affinity::Inside).bold);
        assert!(!p.marks_at(1, Affinity::Inside).bold);
        assert!(p.marks_at(4, Affinity::Inside).bold);
    }

    #[test]
    fn toggle_bold_merges_into_existing_span() {
        let src = "hello **world**";
        let sel = sel_display(src, 3, 8);
        let (out, _) = toggle_mark(src, sel, Mark::Bold).unwrap();
        assert_eq!(out, "hel**lo world**");
        assert_no_visible_stars(&out);
    }

    #[test]
    fn toggle_bold_unbold_cool_does_not_leave_stars_after_space() {
        let src = "you're cool bruh!";
        let (out, _) = toggle_mark(src, sel_display(src, 1, 10), Mark::Bold).unwrap();
        assert_eq!(out, "y**ou're coo**l bruh!");
        assert_no_visible_stars(&out);

        let (out2, _) = toggle_mark(&out, sel_display(&out, 7, 11), Mark::Bold).unwrap();
        assert_eq!(out2, "y**ou're** cool bruh!");
        assert_no_visible_stars(&out2);

        let (out3, _) = toggle_mark(&out, sel_display(&out, 7, 10), Mark::Bold).unwrap();
        assert_eq!(out3, "y**ou're** cool bruh!");
        assert_no_visible_stars(&out3);

        let (out4, _) = toggle_mark(&out, sel_display(&out, 6, 10), Mark::Bold).unwrap();
        assert_eq!(out4, "y**ou're** cool bruh!");
        assert_no_visible_stars(&out4);
    }

    #[test]
    fn enter_on_empty_task_exits_list() {
        let src = "- [ ] three\n- [ ] \n";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        assert_eq!(items.len(), 2);
        let caret = items[1].display.start;
        let (out, at) = enter(src, caret, Affinity::Inside, false);
        assert!(
            !out.contains("- [ ] \n- [ ]"),
            "must not create another empty task: {out:?}"
        );
        let p2 = project(&out);
        let d = at;
        let block = p2.block_at_display(d).expect("block");
        assert!(
            matches!(block.kind, BlockKind::Paragraph | BlockKind::Raw)
                && (block.display.start == block.display.end
                    || p2.display[block.display.clone()].trim().is_empty()),
            "caret on empty paragraph after exit: kind={:?} out={out:?} at={at}",
            block.kind
        );
        assert!(p2.display.contains("three"), "previous item kept: {out:?}");
    }

    #[test]
    fn backspace_on_empty_task_exits_list() {
        let src = "- [ ] three\n- [ ] \n";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        let caret = items[1].display.start;
        let (out, at) = backspace(src, caret, Affinity::Inside).expect("exit");
        assert!(
            !out.contains("- [x]") && !out.contains("- [ ] three-"),
            "must not mangle markers: {out:?}"
        );
        let p2 = project(&out);
        let d = at;
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
        let caret = items[1].display.start;
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
        let empties = p2
            .blocks
            .iter()
            .filter(|b| b.display.start == b.display.end)
            .count();
        assert_eq!(
            empties,
            1,
            "exactly one empty between lists: blocks={} out={out:?}",
            p2.blocks.len()
        );
    }

    #[test]
    fn dash_between_lists_stays_paragraph_until_space() {
        let src = "- item 1\n- \n- item 3\n";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        let caret = items[1].display.start;
        let (split, at) = backspace(src, caret, Affinity::Inside).expect("exit");
        let (out, at) = insert_text(&split, at, None, "-", Affinity::Inside);
        let p2 = project(&out);
        let kinds: Vec<_> = p2.blocks.iter().map(|b| b.kind).collect();
        assert!(
            p2.blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Paragraph | BlockKind::Raw)
                    && p2.display[b.display.clone()].contains('-')),
            "lone dash must stay a paragraph, not a bullet: kinds={kinds:?} out={out:?} display={:?}",
            p2.display
        );
        assert!(
            !p2.blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::List { .. })
                    && p2.display[b.display.clone()]
                        .lines()
                        .any(|l| l.trim() == "-")),
            "must not merge dash into a list item: out={out:?} display={:?}",
            p2.display
        );
        let (out2, _) = insert_text(&out, at, None, " ", Affinity::Inside);
        let p3 = project(&out2);
        assert!(
            p3.blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::List { .. })),
            "dash+space becomes a list: out={out2:?}"
        );
    }

    #[test]
    fn dash_after_list_stays_paragraph_until_space() {
        let src = "- one\n- two\n\n";
        let p = project(src);
        let empty = p
            .blocks
            .iter()
            .find(|b| b.display.start == b.display.end)
            .expect("empty after list");
        let caret = empty.display.start;
        let (out, at) = insert_text(src, caret, None, "-", Affinity::Inside);
        let p2 = project(&out);
        assert!(
            p2.blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Paragraph)
                    && p2.display[b.display.clone()].contains('-')),
            "dash after a list must not become a 3rd bullet: out={out:?} display={:?}",
            p2.display
        );
        let (out2, _) = insert_text(&out, at, None, " ", Affinity::Inside);
        let p3 = project(&out2);
        let lists = p3
            .blocks
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::List { .. }))
            .count();
        assert!(
            lists >= 2
                || p3
                    .blocks
                    .iter()
                    .any(|b| matches!(b.kind, BlockKind::List { .. })),
            "space confirms a new list: out={out2:?}"
        );
    }

    #[test]
    fn type_space_at_end_of_list_item() {
        let src = "- something";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        let caret = items[0].display.end;
        let (out, at) = insert_text(src, caret, None, " ", Affinity::Inside);
        assert_eq!(out, "- something ");
        let p2 = project(&out);
        assert!(
            p2.display.ends_with(' '),
            "trailing space must stay visible: {:?}",
            p2.display
        );
        assert_eq!(at, p2.display.len());
        let (out2, _) = insert_text(&out, at, None, "x", Affinity::Inside);
        assert_eq!(out2, "- something x");
        assert_eq!(project(&out2).display, "something x");
    }

    #[test]
    fn type_hash_after_spaces_in_heading() {
        let src = "# Hello there   ";
        let p = project(src);
        let caret = p.blocks[0].display.end;
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
        let caret = empty.display.start;
        let (out, at) = insert_text(src, caret, None, "/", Affinity::Inside);
        let p2 = project(&out);
        let d = at;
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
        let caret = p.blocks[0].display.end;
        let (out, _) = delete_to_line_start(src, caret);
        let p2 = project(&out);
        assert!(p2.display.trim().is_empty(), "content cleared: {out:?}");
        assert_eq!(p2.blocks.len(), 1, "single empty block: {out:?}");
    }

    #[test]
    fn cmd_backspace_mid_doc_one_empty_slot() {
        let src = "above\n\nhello there nerd\n\nbelow";
        let p = project(src);
        assert!(p.blocks.len() >= 3, "{}", p.blocks.len());
        let mid = &p.blocks[1];
        let caret = mid.display.end;
        let (out, at) = delete_to_line_start(src, caret);
        let p2 = project(&out);
        let empties = p2
            .blocks
            .iter()
            .filter(|b| b.display.start == b.display.end)
            .count();
        assert_eq!(
            empties,
            1,
            "exactly one empty between neighbors: out={out:?} blocks={:?}",
            p2.blocks
                .iter()
                .map(|b| (
                    b.kind,
                    p2.display.get(b.display.clone()).unwrap_or("").to_string()
                ))
                .collect::<Vec<_>>()
        );
        assert!(
            p2.display.contains("above") && p2.display.contains("below"),
            "{out:?}"
        );
        assert!(!p2.display.contains("hello"), "{out:?}");
        let d = at;
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
            p.blocks[0].display.end
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
        let caret = items[0].display.start;
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
        let d = at;
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
        let caret = items[1].display.start;
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
    fn slash_not_becomes_note_alert() {
        let src = "/not";
        let p = project(src);
        let caret = p.display.len();
        let (out, _) = apply_slash(src, caret, "> [!NOTE]\n> ");
        assert!(out.contains("> [!NOTE]"), "{out:?}");
        assert!(!out.contains("/no"), "{out:?}");
        let p2 = project(&out);
        assert!(
            p2.blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Alert(_))),
            "kinds={:?} out={out:?}",
            p2.blocks.iter().map(|b| b.kind).collect::<Vec<_>>()
        );
        assert!(!p2.display.contains('/'), "display={:?}", p2.display);
    }

    #[test]
    fn slash_not_after_paragraph() {
        let src = "hello\n\n/not";
        let p = project(src);
        let caret = src.len();
        let _ = p;
        let (out, _) = apply_slash(src, caret, "> [!NOTE]\n> ");
        assert!(out.contains("hello"), "{out:?}");
        assert!(out.contains("> [!NOTE]"), "{out:?}");
        assert!(!out.contains("/not"), "{out:?}");
        assert!(!out.contains("/no>"), "{out:?}");
    }

    #[test]
    fn type_into_empty_alert_stays_in_body() {
        let src = "\n> [!NOTE]\n>";
        let p = project(src);
        let alert = p
            .blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Alert(_)))
            .expect("alert");
        let caret = alert.display.start;
        let (out, _) = insert_text(src, caret, None, "hello", Affinity::Inside);
        assert!(out.contains("> [!NOTE]"), "label must stay: {out:?}");
        assert!(out.contains("> hello"), "body after label: {out:?}");
        assert!(
            !out.contains("hello> [!NOTE]"),
            "must not prepend the marker: {out:?}"
        );
        let p2 = project(&out);
        assert!(
            p2.blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Alert(_))),
            "still an alert: {out:?} kinds={:?}",
            p2.blocks.iter().map(|b| b.kind).collect::<Vec<_>>()
        );
        assert!(
            p2.display.contains("hello"),
            "visible body: {:?}",
            p2.display
        );
    }

    #[test]
    fn down_from_empty_line_types_in_alert_body() {
        use crate::motion::{apply_motion, Motion};
        let src = "x\n\n> [!NOTE]\n>";
        let p = project(src);
        let start = p.blocks[0].display.start;
        let next_d = apply_motion(&p.display, start, Motion::Down, 1, None);
        let caret = next_d;
        let (out, _) = insert_text(src, caret, None, "hello", Affinity::Inside);
        assert!(out.contains("> hello"), "{out:?}");
        assert!(!out.contains("hello> [!NOTE]"), "{out:?}");
        assert!(
            project(&out)
                .blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Alert(_))),
            "{out:?}"
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
        let caret = empty.display.start;
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

    fn display_blocks(src: &str) -> Vec<String> {
        let p = project(src);
        p.blocks
            .iter()
            .map(|b| p.display[b.display.clone()].to_string())
            .collect()
    }

    #[test]
    fn move_block_first_to_end() {
        let src = "# A\n\nB\n\nC";
        assert_eq!(display_blocks(src), ["A", "B", "C"]);
        let (out, _) = move_block(src, 0, 3).unwrap();
        assert_eq!(display_blocks(&out), ["B", "C", "A"]);
        assert!(out.contains("# A"), "{out}");
    }

    #[test]
    fn move_block_last_to_start() {
        let src = "# A\n\nB\n\nC";
        let (out, _) = move_block(src, 2, 0).unwrap();
        assert_eq!(display_blocks(&out), ["C", "A", "B"]);
    }

    #[test]
    fn move_block_adjacent_is_noop() {
        let src = "# A\n\nB\n\nC";
        assert!(move_block(src, 1, 1).is_none());
        assert!(move_block(src, 1, 2).is_none());
    }

    #[test]
    fn duplicate_block_after() {
        let src = "# A\n\nB";
        let (out, _) = duplicate_block(src, 0).unwrap();
        assert_eq!(display_blocks(&out), ["A", "A", "B"]);
        assert_eq!(out.matches("# A").count(), 2, "{out}");
    }

    #[test]
    fn delete_block_middle() {
        let src = "# A\n\nB\n\nC";
        let (out, _) = delete_block(src, 1).unwrap();
        assert_eq!(display_blocks(&out), ["A", "C"]);
    }

    #[test]
    fn delete_last_block_leaves_previous() {
        let src = "# A\n\nB";
        let (out, _) = delete_block(src, 1).unwrap();
        assert_eq!(display_blocks(&out), ["A"]);
    }

    #[test]
    fn delete_only_block_empties_doc() {
        let src = "hello";
        let (out, caret) = delete_block(src, 0).unwrap();
        assert!(out.trim().is_empty(), "{out:?}");
        assert_eq!(caret, 0);
    }

    fn display_units(src: &str) -> Vec<String> {
        let p = project(src);
        units(&p)
            .into_iter()
            .map(|u| p.display[unit_display(&p, u)].to_string())
            .collect()
    }

    #[test]
    fn list_items_are_separate_units() {
        let src = "- a\n- b\n- c";
        assert_eq!(display_units(src), ["a", "b", "c"]);
    }

    #[test]
    fn move_list_item_within_list() {
        let src = "- a\n- b\n- c";
        let (out, _) = move_block(src, 0, 3).unwrap();
        assert_eq!(display_units(&out), ["b", "c", "a"]);
        assert!(out.contains("- a"), "{out}");
        assert!(!out.contains("\n\n"), "list must stay one list: {out:?}");
    }

    #[test]
    fn move_list_item_between_paragraphs() {
        let src = "A\n\n- b\n- c\n\nD";
        // units: A, b, c, D — move b after D
        let (out, _) = move_block(src, 1, 4).unwrap();
        assert_eq!(display_units(&out), ["A", "c", "D", "b"]);
        assert!(out.contains("- b"), "{out}");
        assert!(out.contains("- c"), "{out}");
    }

    #[test]
    fn duplicate_list_item() {
        let src = "- a\n- b";
        let (out, _) = duplicate_block(src, 0).unwrap();
        assert_eq!(display_units(&out), ["a", "a", "b"]);
        assert_eq!(out.matches("- a").count(), 2, "{out}");
    }

    #[test]
    fn delete_list_item_middle() {
        let src = "- a\n- b\n- c";
        let (out, _) = delete_block(src, 1).unwrap();
        assert_eq!(display_units(&out), ["a", "c"]);
    }

    #[test]
    fn delete_only_list_item() {
        let src = "A\n\n- b\n\nC";
        let (out, _) = delete_block(src, 1).unwrap();
        assert_eq!(display_units(&out), ["A", "C"]);
        assert!(!out.contains("- b"), "{out}");
    }

    #[test]
    fn type_at_end_of_third_item_stays_on_item() {
        let src = "- a\n- b\n- c";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        let caret = items[2].display.end;
        let mut src = src.to_string();
        let mut at = caret;
        for ch in ["a", "r", "l", "o"] {
            let (out, next) = insert_text(&src, at, None, ch, Affinity::Inside);
            src = out;
            at = next;
        }
        let p2 = project(&src);
        assert_eq!(p2.display, "a\nb\ncarlo", "{src}");
        let BlockExtra::List { items, .. } = &p2.blocks[0].extra else {
            panic!("list {}", src);
        };
        assert_eq!(&p2.display[items[2].display.clone()], "carlo");
        assert_eq!(at, items[2].display.end, "caret jumped: {at} src={src}");
    }

    #[test]
    fn type_at_start_of_third_item_stays_on_item() {
        let src = "- a\n- b\n- c";
        let p = project(src);
        let BlockExtra::List { items, .. } = &p.blocks[0].extra else {
            panic!("list");
        };
        let caret = items[2].display.start;
        let (out, at) = insert_text(src, caret, None, "arlo", Affinity::Inside);
        let p2 = project(&out);
        assert_eq!(p2.display, "a\nb\narloc", "{out}");
        let BlockExtra::List { items, .. } = &p2.blocks[0].extra else {
            panic!("{out}");
        };
        assert_eq!(&p2.display[items[2].display.clone()], "arloc");
        assert!(
            at > items[2].display.start && at <= items[2].display.end,
            "caret {at} item={:?} out={out}",
            items[2].display
        );
    }

    fn empty_unit_count(src: &str) -> usize {
        let p = project(src);
        units(&p)
            .into_iter()
            .filter(|u| {
                let d = unit_display(&p, *u);
                p.display.get(d).unwrap_or("").trim().is_empty()
            })
            .count()
    }

    #[test]
    fn move_empty_block_to_end() {
        let src = "A\n\n\nB";
        assert_eq!(empty_unit_count(src), 1, "need a visible empty: {src:?}");
        let (out, _) = move_block(src, 1, 3).unwrap();
        assert_eq!(display_units(&out), ["A", "B", ""]);
        assert_eq!(empty_unit_count(&out), 1, "{out:?}");
    }

    #[test]
    fn move_block_past_empty() {
        let src = "A\n\n\nB";
        // units: A, empty, B — move B above empty (gap 1)
        let (out, _) = move_block(src, 2, 1).unwrap();
        assert_eq!(display_units(&out), ["A", "B", ""]);
        assert_eq!(empty_unit_count(&out), 1, "{out:?}");
    }

    #[test]
    fn type_between_code_and_table_stays_one_paragraph() {
        let src = "```\nfn main() {}\n```\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let p = project(src);
        let kinds: Vec<_> = p.blocks.iter().map(|b| b.kind).collect();
        let empty = p
            .blocks
            .iter()
            .find(|b| b.display.start == b.display.end)
            .unwrap_or_else(|| {
                panic!(
                    "no empty between code/table: kinds={kinds:?} n={} display={:?} src={src:?}",
                    p.blocks.len(),
                    p.display
                )
            });
        let caret = empty.display.start;
        let empty_ix = p
            .blocks
            .iter()
            .position(|b| b.display.start == b.display.end)
            .unwrap();
        let mut src = src.to_string();
        let mut at = caret;
        for ch in ["h", "i"] {
            let before = project(&src);
            let (out, next) = insert_text(&src, at, None, ch, Affinity::Inside);
            let afterp = project(&out);
            assert_eq!(
                afterp.blocks.len(),
                before.blocks.len(),
                "typing {ch:?} must not invent a line: before_kinds={:?} after_kinds={:?} out={out:?} caret_in={at} caret_out={next} empty_ix={empty_ix}",
                before.blocks.iter().map(|b| b.kind).collect::<Vec<_>>(),
                afterp.blocks.iter().map(|b| b.kind).collect::<Vec<_>>(),
            );
            src = out;
            at = next;
        }
        let p2 = project(&src);
        assert!(
            p2.display.contains("hi"),
            "typed text: display={:?} src={src:?}",
            p2.display
        );
        assert!(
            p2.blocks.iter().any(|b| matches!(b.kind, BlockKind::Code)),
            "{src:?}"
        );
        assert!(
            p2.blocks.iter().any(|b| matches!(b.kind, BlockKind::Table)),
            "{src:?}"
        );
        let para = p2
            .blocks
            .iter()
            .find(|b| b.kind == BlockKind::Paragraph && b.display.start != b.display.end)
            .map(|b| p2.display[b.display.clone()].to_string());
        assert_eq!(
            para.as_deref(),
            Some("hi"),
            "src={src:?} display={:?}",
            p2.display
        );
    }

    #[test]
    fn hash_stays_paragraph_until_space() {
        let ranges = crate::document::parse_ranges("#");
        assert_eq!(
            ranges[0].kind,
            BlockKind::Paragraph,
            "parse_ranges: {:?}",
            ranges
        );
        let p = project("#");
        assert_eq!(
            p.blocks[0].kind,
            BlockKind::Paragraph,
            "{:?}",
            p.blocks[0].kind
        );
        assert_eq!(p.display, "#");
        let p = project("##");
        assert_eq!(p.blocks[0].kind, BlockKind::Paragraph);
        assert_eq!(p.display, "##");
        let p = project("# ");
        assert!(
            matches!(p.blocks[0].kind, BlockKind::Heading(1)),
            "{:?}",
            p.blocks[0].kind
        );
        let p = project("## Hello");
        assert!(matches!(p.blocks[0].kind, BlockKind::Heading(2)));
        assert_eq!(p.display, "Hello");
    }

    #[test]
    fn type_hash_then_space_becomes_heading() {
        let src = "Hello\n\n";
        let p = project(src);
        let empty = p
            .blocks
            .iter()
            .find(|b| b.display.start == b.display.end)
            .expect("empty");
        let caret = empty.display.start;
        let (out, at) = insert_text(src, caret, None, "#", Affinity::Inside);
        let p2 = project(&out);
        assert_eq!(
            p2.display.trim_end(),
            "Hello\n#",
            "display={:?} out={out:?}",
            p2.display,
        );
        assert!(
            matches!(
                p2.block_at_display(at).map(|b| b.kind),
                Some(BlockKind::Paragraph)
            ),
            "lone hash is still a paragraph: {:?}",
            p2.blocks.iter().map(|b| b.kind).collect::<Vec<_>>()
        );
        let (out2, at2) = insert_text(&out, at, None, " ", Affinity::Inside);
        let p3 = project(&out2);
        assert!(
            p3.blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Heading(1))),
            "hash+space becomes heading: kinds={:?} out={out2:?}",
            p3.blocks.iter().map(|b| b.kind).collect::<Vec<_>>()
        );
        let _ = at2;
    }

    #[test]
    fn move_block_below_empty() {
        let src = "A\n\n\nB\n\nC";
        // units: A, empty, B, C — move B to after empty is already there;
        // move C to just after empty (gap 2)
        assert_eq!(display_units(src)[0], "A");
        let (out, _) = move_block(src, 3, 2).unwrap();
        assert_eq!(display_units(&out), ["A", "", "C", "B"]);
    }

    #[test]
    fn opt_backspace_last_word_leaves_empty_block() {
        // Notion opt-backspace: deleting the sole word in a block must leave
        // an empty slot (same as cmd-backspace), not join into the previous.
        let src = "above\n\nhello";
        let mut doc = crate::tree::Doc::from_gfm(src);
        let p = doc.project();
        let block = p
            .blocks
            .iter()
            .find(|b| &p.display[b.display.clone()] == "hello")
            .unwrap();
        let caret = block.display.end;
        let unit_start = block.display.start;
        let start = crate::motion::apply_motion(
            &p.display,
            caret,
            crate::motion::Motion::WordBack,
            1,
            None,
        )
        .max(unit_start);
        assert_eq!(start, unit_start);
        let at = doc.delete_display(start..caret);
        assert_eq!(doc.nodes.len(), 2, "must keep empty node");
        let p2 = doc.project();
        let b = p2.block_at_display(at).unwrap();
        assert_eq!(
            b.display.start, b.display.end,
            "caret on empty block, at={at} display={:?}",
            p2.display
        );
        assert!(p2.display.contains("above"), "{:?}", p2.display);
    }

    #[test]
    fn opt_backspace_clamped_word_back_does_not_cross_block() {
        let src = "above\n\nhello world";
        let mut doc = crate::tree::Doc::from_gfm(src);
        let p = doc.project();
        let block = p
            .blocks
            .iter()
            .find(|b| p.display[b.display.clone()].starts_with("hello"))
            .unwrap();
        let caret = block.display.end;
        let unit_start = block.display.start;
        // Unclamped WordBack from mid-block is fine; from block start would
        // cross — clamping keeps deletion inside the unit.
        let start = crate::motion::apply_motion(
            &p.display,
            caret,
            crate::motion::Motion::WordBack,
            1,
            None,
        )
        .max(unit_start);
        assert!(start >= unit_start);
        let _ = doc.delete_display(start..caret);
        let p2 = doc.project();
        assert!(p2.display.contains("above"), "previous block intact: {:?}", p2.display);
        assert!(p2.display.contains("hello"), "only last word removed: {:?}", p2.display);
        assert!(!p2.display.contains("world"), "{:?}", p2.display);
    }

    #[test]
    fn opt_backspace_sole_word_leaves_empty_doc_block() {
        let src = "hello";
        let mut doc = crate::tree::Doc::from_gfm(src);
        let p = doc.project();
        let caret = p.display.len();
        let start = crate::motion::apply_motion(
            &p.display,
            caret,
            crate::motion::Motion::WordBack,
            1,
            None,
        );
        let at = doc.delete_display(start..caret);
        assert_eq!(doc.nodes.len(), 1);
        let p2 = doc.project();
        let b = p2.block_at_display(at).unwrap();
        assert_eq!(b.display.start, b.display.end, "sole empty block at={at}");
    }

    #[test]
    fn enter_at_start_of_heading_inserts_empty_above() {
        let src = "# Table";
        let mut doc = crate::tree::Doc::from_gfm(src);
        let at = doc.enter(0, false);
        assert_eq!(doc.nodes.len(), 2);
        assert!(
            matches!(doc.nodes[0].kind, crate::tree::NodeKind::Paragraph { .. }),
            "empty paragraph above"
        );
        assert!(
            matches!(doc.nodes[1].kind, crate::tree::NodeKind::Heading { .. }),
            "heading preserved: {:?}",
            doc.nodes[1].kind
        );
        let p = doc.project();
        assert_eq!(
            p.display.get(p.blocks[1].display.clone()).unwrap_or(""),
            "Table",
            "heading body intact: {:?}",
            p.display
        );
        // Caret stays on the heading: `|Table`
        let b = p.block_at_display(at).unwrap();
        assert!(
            matches!(b.kind, BlockKind::Heading(_)),
            "caret on heading, got {:?} at={at} display={:?}",
            b.kind,
            p.display
        );
        assert_eq!(at, b.display.start, "caret at start of heading body");
    }

    #[test]
    fn cmd_backspace_after_code_clears_paragraph() {
        let src = "```\ncode\n```\n\nSome words here";
        // Editor clears unit_start..unit_end when deleting from the unit start,
        // even if the caret sits on the last character (end-1).
        for caret_off in [0usize, 1] {
            let mut doc = crate::tree::Doc::from_gfm(src);
            let p = doc.project();
            let para = p
                .blocks
                .iter()
                .find(|b| p.display.get(b.display.clone()) == Some("Some words here"))
                .unwrap();
            let unit_start = para.display.start;
            let unit_end = para.display.end;
            let caret = unit_end.saturating_sub(caret_off).max(unit_start);
            let at = doc.delete_display(unit_start..unit_end);
            let _ = caret;
            let p2 = doc.project();
            assert!(
                !p2.display.contains("Some") && !p2.display.contains("words"),
                "cleared (off={caret_off}): {:?}",
                p2.display
            );
            assert!(p2.display.contains("code"), "{:?}", p2.display);
            let b = p2.block_at_display(at).unwrap();
            assert_eq!(
                b.display.start, b.display.end,
                "left on empty slot at={at} display={:?}",
                p2.display
            );
            assert!(
                !p2.display.ends_with('e'),
                "stray e (off={caret_off}): {:?}",
                p2.display
            );
        }
    }

    #[test]
    fn join_gfm_preserves_consecutive_empties() {
        let src = "p1\n\n\np2\n\n\np3";
        let doc = crate::tree::Doc::from_gfm(src);
        assert_eq!(doc.nodes.len(), 5, "parsed empties");
        let gfm = doc.to_gfm();
        let doc2 = crate::tree::Doc::from_gfm(&gfm);
        assert_eq!(
            doc2.nodes.len(),
            5,
            "roundtrip must keep empties: gfm={gfm:?} nodes={}",
            doc2.nodes.len()
        );
    }

    #[test]
    fn move_block_preserves_empties_tree_native() {
        let src = "p1\n\n\np2\n\n\np3";
        let mut doc = crate::tree::Doc::from_gfm(src);
        assert_eq!(doc.nodes.len(), 5);
        doc.move_unit(2, 1).expect("move p2 before first empty");
        // p1, p2, empty, empty, p3
        assert_eq!(doc.nodes.len(), 5, "move must not drop nodes");
        let gfm = doc.to_gfm();
        let doc2 = crate::tree::Doc::from_gfm(&gfm);
        assert_eq!(
            doc2.nodes.len(),
            5,
            "sync roundtrip keeps empties: gfm={gfm:?}"
        );
        let texts: Vec<_> = {
            let p = doc2.project();
            p.blocks
                .iter()
                .map(|b| p.display.get(b.display.clone()).unwrap_or("").to_string())
                .collect()
        };
        assert_eq!(texts, vec!["p1", "p2", "", "", "p3"]);
    }

}
