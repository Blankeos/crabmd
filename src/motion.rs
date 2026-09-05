//! Motions on a single source buffer. No GPUI types.
//!
//! `wrap_cols = None` uses logical `\n` lines. `Some(n)` splits each logical
//! line into visual rows of `n` characters (approximate wrap for tests / when
//! the textarea width is unknown). There is no block edge: `j`/`k` walk visual
//! (or logical) rows of the whole file and stop at the buffer ends.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Down,
    Up,
    WordForward,
    WordBack,
    WordEnd,
    /// Whitespace-separated WORD (Vim/Helix `W`).
    WordForwardWs,
    /// Whitespace-separated WORD back (`B`).
    WordBackWs,
    /// End of whitespace-separated WORD (`E`).
    WordEndWs,
    LineStart,
    LineFirstNonBlank,
    LineEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Blank,
    Word,
    Punct,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Blank
    } else if c.is_ascii_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

fn clamp_off(source: &str, offset: usize) -> usize {
    if offset >= source.len() {
        source.len()
    } else {
        let mut i = offset;
        while i > 0 && !source.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

fn next_char(source: &str, offset: usize) -> Option<(usize, char)> {
    let offset = clamp_off(source, offset);
    source[offset..].chars().next().map(|c| (c.len_utf8(), c))
}

fn prev_char(source: &str, offset: usize) -> Option<(usize, char)> {
    let offset = clamp_off(source, offset);
    source[..offset]
        .chars()
        .next_back()
        .map(|c| (c.len_utf8(), c))
}

/// Logical line bounds: `start..end` where `end` is the newline or `len`.
pub fn logical_line_range(source: &str, offset: usize) -> Range<usize> {
    let offset = clamp_off(source, offset);
    let start = source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = source[offset..]
        .find('\n')
        .map(|i| offset + i)
        .unwrap_or(source.len());
    start..end
}

pub fn first_non_blank_in(source: &str, range: Range<usize>) -> usize {
    let line = &source[range.clone()];
    let skip = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    let mut i = range.start;
    for _ in 0..skip {
        i += source[i..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
    }
    i.min(range.end)
}

/// Visual (or logical) rows as byte ranges, newline not included.
pub fn visual_rows(source: &str, wrap_cols: Option<usize>) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    if source.is_empty() {
        rows.push(0..0);
        return rows;
    }
    let mut start = 0usize;
    for (i, c) in source.char_indices() {
        if c == '\n' {
            rows.push(start..i);
            start = i + 1;
        }
    }
    rows.push(start..source.len());
    let Some(cols) = wrap_cols.filter(|n| *n > 0) else {
        return rows;
    };
    let mut out = Vec::new();
    for row in rows {
        let line = &source[row.clone()];
        if line.is_empty() {
            out.push(row);
            continue;
        }
        let mut col = 0usize;
        let mut seg = row.start;
        for (off, c) in line.char_indices() {
            if col == cols {
                out.push(seg..row.start + off);
                seg = row.start + off;
                col = 0;
            }
            col += 1;
            let _ = c;
        }
        out.push(seg..row.end);
    }
    out
}

fn row_index(rows: &[Range<usize>], offset: usize) -> usize {
    // Caret at `row.end` is EOL of that row (the `\n` offset), except at a
    // soft-wrap boundary where `row.end == next.start` — that offset is the
    // first column of the next visual row.
    for (i, r) in rows.iter().enumerate() {
        if offset < r.start {
            return i.saturating_sub(1);
        }
        if offset < r.end {
            return i;
        }
        if offset == r.end {
            if let Some(next) = rows.get(i + 1) {
                if next.start == r.end {
                    continue;
                }
            }
            return i;
        }
    }
    rows.len().saturating_sub(1)
}

fn col_in_row(source: &str, row: &Range<usize>, offset: usize) -> usize {
    let offset = offset.clamp(row.start, row.end);
    source[row.start..offset].chars().count()
}

fn offset_at_col(source: &str, row: &Range<usize>, col: usize) -> usize {
    let mut n = 0;
    for (off, _) in source[row.start..row.end].char_indices() {
        if n == col {
            return row.start + off;
        }
        n += 1;
    }
    row.end
}

fn apply_once(source: &str, offset: usize, motion: Motion, wrap_cols: Option<usize>) -> usize {
    let offset = clamp_off(source, offset);
    match motion {
        Motion::Left => {
            // Vim default (no whichwrap): stay on this logical line of the
            // full buffer. Counts that would wrap just stop.
            let line = logical_line_range(source, offset);
            if offset <= line.start {
                line.start
            } else if let Some((n, _)) = prev_char(source, offset) {
                (offset - n).max(line.start)
            } else {
                line.start
            }
        }
        Motion::Right => {
            let line = logical_line_range(source, offset);
            if offset >= line.end {
                line.end
            } else if let Some((n, c)) = next_char(source, offset) {
                if c == '\n' {
                    offset
                } else {
                    (offset + n).min(line.end)
                }
            } else {
                line.end
            }
        }
        Motion::Down | Motion::Up => {
            let rows = visual_rows(source, wrap_cols);
            let ix = row_index(&rows, offset);
            let col = col_in_row(source, &rows[ix], offset);
            if motion == Motion::Down {
                if ix + 1 >= rows.len() {
                    return offset;
                }
                offset_at_col(source, &rows[ix + 1], col)
            } else {
                if ix == 0 {
                    return offset;
                }
                offset_at_col(source, &rows[ix - 1], col)
            }
        }
        Motion::LineStart => logical_line_range(source, offset).start,
        Motion::LineEnd => logical_line_range(source, offset).end,
        Motion::LineFirstNonBlank => {
            let range = logical_line_range(source, offset);
            first_non_blank_in(source, range)
        }
        Motion::WordForward => word_forward(source, offset),
        Motion::WordBack => word_back(source, offset),
        Motion::WordEnd => word_end(source, offset),
        Motion::WordForwardWs => word_forward_ws(source, offset),
        Motion::WordBackWs => word_back_ws(source, offset),
        Motion::WordEndWs => word_end_ws(source, offset),
    }
}

/// Byte range of the word (or punctuation run) under `offset`.
/// Empty when the caret sits on whitespace / empty text.
pub fn word_range_at(source: &str, offset: usize) -> Range<usize> {
    let offset = clamp_off(source, offset);
    if source.is_empty() {
        return 0..0;
    }
    let probe = if offset >= source.len() {
        // Prefer the char before EOF so double-click at end selects the last word.
        match prev_char(source, offset) {
            Some((n, _)) => offset - n,
            None => return offset..offset,
        }
    } else {
        offset
    };
    let Some((_, c)) = next_char(source, probe) else {
        return offset..offset;
    };
    // Caret resting on the line break (Normal-mode `$` / `gl` parks on the
    // newline): text objects target the line's last word instead of no-op.
    // Mid-line blanks still return empty.
    let probe = if c == '\n' {
        match prev_char(source, probe) {
            Some((n, pc)) if !pc.is_whitespace() => probe - n,
            _ => return offset..offset,
        }
    } else {
        probe
    };
    let Some((_, c)) = next_char(source, probe) else {
        return offset..offset;
    };
    let cls = class(c);
    if cls == Class::Blank {
        return offset..offset;
    }
    let start = skip_while_back(source, probe, |ch| class(ch) == cls);
    let end = skip_while(source, probe, |ch| class(ch) == cls);
    start..end
}

/// Byte range of the whitespace-separated WORD under `offset`.
/// Empty when the caret sits on whitespace / empty text.
pub fn big_word_range_at(source: &str, offset: usize) -> Range<usize> {
    let offset = clamp_off(source, offset);
    if source.is_empty() {
        return 0..0;
    }
    let probe = if offset >= source.len() {
        match prev_char(source, offset) {
            Some((n, _)) => offset - n,
            None => return offset..offset,
        }
    } else {
        offset
    };
    let Some((_, c)) = next_char(source, probe) else {
        return offset..offset;
    };
    // Same EOL back-off as `word_range_at`: a caret parked on the line
    // break targets the line's last WORD.
    let probe = if c == '\n' {
        match prev_char(source, probe) {
            Some((n, pc)) if !pc.is_whitespace() => probe - n,
            _ => return offset..offset,
        }
    } else {
        probe
    };
    let Some((_, c)) = next_char(source, probe) else {
        return offset..offset;
    };
    if c.is_whitespace() {
        return offset..offset;
    }
    let start = skip_while_back(source, probe, |ch| !ch.is_whitespace());
    let end = skip_while(source, probe, |ch| !ch.is_whitespace());
    start..end
}

/// Byte offsets of the enclosing delimiter pair around `offset`:
/// `(open_start, close_start)` (each the delimiter's own offset).
/// Backward scan finds the unmatched `open`; forward scan from there finds
/// its match, counting nesting. `None` when unbalanced.
pub fn pair_around(
    source: &str,
    offset: usize,
    open: char,
    close: char,
) -> Option<(usize, usize)> {
    if open == close {
        return quote_around(source, offset, open);
    }
    let offset = clamp_off(source, offset);
    // Backward: nearest unmatched `open` at or before `offset`.
    let mut depth = 0usize;
    let mut open_pos: Option<usize> = None;
    let mut i = offset;
    while i > 0 {
        let Some((n, c)) = prev_char(source, i) else {
            break;
        };
        let pos = i - n;
        if c == close {
            depth += 1;
        } else if c == open {
            if depth == 0 {
                open_pos = Some(pos);
                break;
            }
            depth -= 1;
        }
        i = pos;
    }
    let open_pos = open_pos?;
    // Forward: its match, counting nested opens.
    let mut depth = 0usize;
    let mut i = open_pos + open.len_utf8();
    while i < source.len() {
        let Some((n, c)) = next_char(source, i) else {
            break;
        };
        if c == open {
            depth += 1;
        } else if c == close {
            if depth == 0 {
                return Some((open_pos, i));
            }
            depth -= 1;
        }
        i += n;
    }
    None
}

/// Byte offsets of the enclosing same-char quote (e.g. `"`) around `offset`:
/// `(open_start, close_start)`. `None` when fewer than two quotes straddle it.
pub fn quote_around(source: &str, offset: usize, q: char) -> Option<(usize, usize)> {
    let offset = clamp_off(source, offset);
    let before = &source[..offset];
    let open_pos = before.rfind(q)?;
    let after = &source[offset..];
    let rel = after.find(q)?;
    Some((open_pos, offset + rel))
}

fn skip_while(source: &str, mut offset: usize, pred: impl Fn(char) -> bool) -> usize {
    while let Some((n, c)) = next_char(source, offset) {
        if !pred(c) {
            break;
        }
        offset += n;
    }
    offset
}

fn skip_while_back(source: &str, mut offset: usize, pred: impl Fn(char) -> bool) -> usize {
    while let Some((n, c)) = prev_char(source, offset) {
        if !pred(c) {
            break;
        }
        offset -= n;
    }
    offset
}

fn word_forward(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    if offset >= source.len() {
        return source.len();
    }
    let Some((_, c)) = next_char(source, offset) else {
        return source.len();
    };
    let start_class = class(c);
    let mut i = skip_while(source, offset, |ch| class(ch) == start_class);
    i = skip_while(source, i, |ch| class(ch) == Class::Blank);
    i
}

fn word_back(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    if offset == 0 {
        return 0;
    }
    let mut i = offset;
    if let Some((_, c)) = prev_char(source, i) {
        if class(c) == Class::Blank {
            i = skip_while_back(source, i, |ch| class(ch) == Class::Blank);
        }
    }
    if let Some((_, c)) = prev_char(source, i) {
        let cls = class(c);
        i = skip_while_back(source, i, |ch| class(ch) == cls);
    }
    i
}

fn word_end(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    if offset >= source.len() {
        return source.len();
    }
    let mut i = offset;
    if let Some((n, c)) = next_char(source, i) {
        let next_i = i + n;
        if next_i < source.len() {
            if let Some((_, nxt)) = next_char(source, next_i) {
                if class(c) != Class::Blank && class(nxt) == class(c) {
                    i = next_i;
                } else {
                    i = skip_while(source, next_i, |ch| class(ch) == Class::Blank);
                }
            }
        } else {
            return source.len();
        }
    }
    i = skip_while(source, i, |ch| class(ch) == Class::Blank);
    if i >= source.len() {
        return source.len();
    }
    if let Some((_, c)) = next_char(source, i) {
        let cls = class(c);
        let end = skip_while(source, i, |ch| class(ch) == cls);
        if end > i {
            if let Some((n, _)) = prev_char(source, end) {
                return end - n;
            }
        }
    }
    i
}

fn word_forward_ws(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    if offset >= source.len() {
        return source.len();
    }
    let i = skip_while(source, offset, |ch| !ch.is_whitespace());
    skip_while(source, i, |ch| ch.is_whitespace())
}

fn word_back_ws(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    if offset == 0 {
        return 0;
    }
    let mut i = offset;
    if let Some((_, c)) = prev_char(source, i) {
        if c.is_whitespace() {
            i = skip_while_back(source, i, |ch| ch.is_whitespace());
        }
    }
    skip_while_back(source, i, |ch| !ch.is_whitespace())
}

fn word_end_ws(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    if offset >= source.len() {
        return source.len();
    }
    let mut i = offset;
    if let Some((n, c)) = next_char(source, i) {
        let next_i = i + n;
        if next_i < source.len() {
            if let Some((_, nxt)) = next_char(source, next_i) {
                if !c.is_whitespace() && !nxt.is_whitespace() {
                    i = next_i;
                } else {
                    i = skip_while(source, next_i, |ch| ch.is_whitespace());
                }
            }
        } else {
            return source.len();
        }
    }
    i = skip_while(source, i, |ch| ch.is_whitespace());
    if i >= source.len() {
        return source.len();
    }
    let end = skip_while(source, i, |ch| !ch.is_whitespace());
    if end > i {
        if let Some((n, _)) = prev_char(source, end) {
            return end - n;
        }
    }
    i
}

/// Insert/Notion whichwrap: Left at column 0 crosses onto the previous line's
/// end; Right at EOL crosses onto the next line's start.
pub fn whichwrap(source: &str, offset: usize, motion: Motion) -> Option<usize> {
    let offset = clamp_off(source, offset);
    match motion {
        Motion::Left => {
            let line = logical_line_range(source, offset);
            if offset > line.start || offset == 0 {
                return None;
            }
            Some(offset - 1)
        }
        Motion::Right => {
            let line = logical_line_range(source, offset);
            if offset < line.end || offset >= source.len() {
                return None;
            }
            if source.as_bytes()[offset] == b'\n' {
                Some(offset + 1)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Apply `count` repetitions of `motion` on the full buffer. `j`/`k` at the
/// last/first visual row stay put (no leftover / no block edge).
pub fn apply_motion(
    source: &str,
    offset: usize,
    motion: Motion,
    count: usize,
    wrap_cols: Option<usize>,
) -> usize {
    let count = count.max(1);
    let mut offset = clamp_off(source, offset);
    for _ in 0..count {
        let next = apply_once(source, offset, motion, wrap_cols);
        if next == offset {
            break;
        }
        offset = next;
    }
    offset
}

/// Inclusive visual-line selection: current logical line, including its
/// newline when one follows. Repeat / `j` in V-LINE extends one line.
pub fn visual_line_range(source: &str, offset: usize) -> Range<usize> {
    logical_line_delete_range(source, offset)
}

/// Extend a visual-line selection by one logical line in `dir` (>0 down).
pub fn extend_visual_line(source: &str, sel: Range<usize>, dir: i8) -> Range<usize> {
    let start = sel.start.min(sel.end).min(source.len());
    let end = sel.start.max(sel.end).min(source.len());
    if dir >= 0 {
        let at = if end < source.len() {
            end
        } else {
            end.saturating_sub(1)
        };
        let next = logical_line_delete_range(source, at);
        start.min(next.start)..end.max(next.end)
    } else {
        let at = start.saturating_sub(1);
        let prev = logical_line_delete_range(source, at);
        prev.start.min(start)..end.max(prev.end)
    }
}

pub fn delete_range(source: &str, range: Range<usize>) -> (String, usize) {
    let start = clamp_off(source, range.start.min(range.end));
    let end = clamp_off(source, range.start.max(range.end));
    let mut out = String::with_capacity(source.len() - (end - start));
    out.push_str(&source[..start]);
    out.push_str(&source[end..]);
    (out, start)
}

pub fn delete_char_at(source: &str, offset: usize) -> (String, usize) {
    let offset = clamp_off(source, offset);
    if offset >= source.len() {
        return (source.to_string(), offset);
    }
    let n = source[offset..]
        .chars()
        .next()
        .map(|c| c.len_utf8())
        .unwrap_or(0);
    delete_range(source, offset..offset + n)
}

/// Helix `x` / Vim visual-line / `dd`: the logical line, including its newline
/// when one follows (so deleting the line removes it).
pub fn logical_line_delete_range(source: &str, offset: usize) -> Range<usize> {
    let mut range = logical_line_range(source, offset);
    if range.end < source.len() && source.as_bytes()[range.end] == b'\n' {
        range.end += 1;
    } else if range.start > 0
        && source.as_bytes()[range.start - 1] == b'\n'
        && range.end == source.len()
    {
        range.start -= 1;
    }
    range
}

pub fn open_line_below(source: &str, offset: usize) -> (String, usize) {
    let end = logical_line_range(source, offset).end;
    let mut out = String::with_capacity(source.len() + 1);
    out.push_str(&source[..end]);
    out.push('\n');
    out.push_str(&source[end..]);
    (out, end + 1)
}

pub fn open_line_above(source: &str, offset: usize) -> (String, usize) {
    let start = logical_line_range(source, offset).start;
    let mut out = String::with_capacity(source.len() + 1);
    out.push_str(&source[..start]);
    out.push('\n');
    out.push_str(&source[start..]);
    (out, start)
}

/// Byte range covering the grapheme under a Normal-mode block caret.
/// At end-of-line / EOF, covers the last character of the line (vim sits on
/// it). Empty line → collapsed `offset..offset`.
pub fn block_caret_range(source: &str, offset: usize) -> Range<usize> {
    let offset = clamp_off(source, offset);
    if let Some((n, c)) = next_char(source, offset) {
        if c != '\n' {
            return offset..offset + n;
        }
    }
    if let Some((n, c)) = prev_char(source, offset) {
        if c != '\n' {
            return offset - n..offset;
        }
    }
    offset..offset
}

pub fn after_caret(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    match next_char(source, offset) {
        Some((n, _)) => offset + n,
        None => offset,
    }
}

/// Vim/Helix `a`: move one display char forward but never onto the next
/// logical line (so end-of-block append does not jump to the next block).
pub fn after_caret_same_line(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    let line = logical_line_range(source, offset);
    match next_char(source, offset) {
        Some((n, c)) if c != '\n' && offset + n <= line.end => offset + n,
        _ => offset.min(line.end),
    }
}

/// Fold a digit into a pending count. `None` + `0` is not a count (caller
/// treats `0` as line-start). `Some(n)` + digit is `n * 10 + digit`.
pub fn push_count(pending: Option<usize>, digit: u8) -> Option<usize> {
    let d = digit as usize;
    match pending {
        None if digit == 0 => None,
        None => Some(d),
        Some(n) => Some(n.saturating_mul(10).saturating_add(d)),
    }
}

pub fn take_count(pending: &mut Option<usize>) -> usize {
    pending.take().unwrap_or(1).max(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindKind {
    Forward,
    Backward,
    Till,
    TillBack,
}

impl FindKind {
    pub fn reverse(self) -> Self {
        match self {
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
            Self::Till => Self::TillBack,
            Self::TillBack => Self::Till,
        }
    }
}

/// Till landing: the character before `found` (forward) or after (backward).
pub fn find_char(
    source: &str,
    offset: usize,
    ch: char,
    kind: FindKind,
    count: usize,
    line_only: bool,
) -> Option<usize> {
    let offset = clamp_off(source, offset);
    let bound = if line_only {
        logical_line_range(source, offset)
    } else {
        0..source.len()
    };
    let count = count.max(1);
    match kind {
        FindKind::Forward | FindKind::Till => {
            let mut found = 0usize;
            let mut i = offset;
            if let Some((n, _)) = next_char(source, i) {
                if i < bound.end {
                    i += n;
                }
            }
            while i < bound.end {
                let Some((n, c)) = next_char(source, i) else {
                    break;
                };
                if c == ch {
                    found += 1;
                    if found == count {
                        if kind == FindKind::Till {
                            return prev_char(source, i).map(|(k, _)| i - k);
                        }
                        return Some(i);
                    }
                }
                i += n;
            }
            None
        }
        FindKind::Backward | FindKind::TillBack => {
            let mut found = 0usize;
            let mut i = offset;
            while i > bound.start {
                let Some((n, c)) = prev_char(source, i) else {
                    break;
                };
                let pos = i - n;
                if c == ch {
                    found += 1;
                    if found == count {
                        if kind == FindKind::TillBack {
                            return Some(pos + n);
                        }
                        return Some(pos);
                    }
                }
                i = pos;
            }
            None
        }
    }
}

/// Vim `J`: join current line with the next `count` lines (min 1 following).
/// Newline + indent become a single space when both sides have content.
/// Empty lines between are eaten so a heading `J` pulls the next paragraph up.
pub fn join_next_lines(source: &str, offset: usize, count: usize) -> (String, usize) {
    let count = count.max(1);
    let mut src = source.to_string();
    let mut caret = clamp_off(&src, offset);
    for _ in 0..count {
        match join_once(&src, caret) {
            Some((next, c)) => {
                src = next;
                caret = c;
            }
            None => break,
        }
    }
    (src, caret)
}

fn join_once(source: &str, offset: usize) -> Option<(String, usize)> {
    let offset = clamp_off(source, offset);
    let line = logical_line_range(source, offset);
    if line.end >= source.len() {
        return None;
    }
    // Skip the newline at line.end and any following empty lines so a block
    // separator `\n\n` does not eat an extra J.
    let mut take_from = line.end;
    if source.as_bytes()[take_from] != b'\n' {
        return None;
    }
    take_from += 1;
    if take_from >= 1 && take_from <= source.len() {
        // skip \r already handled by logical lines
    }
    while take_from < source.len() && source.as_bytes()[take_from] == b'\n' {
        take_from += 1;
    }
    if take_from > source.len() {
        return None;
    }
    let next_line = logical_line_range(source, take_from.min(source.len()));
    let left = source[..line.end].trim_end();
    let right = source[next_line.start..next_line.end].trim_start();
    let space = if !left.is_empty() && !right.is_empty() {
        " "
    } else {
        ""
    };
    let rest = if next_line.end < source.len() {
        &source[next_line.end..]
    } else {
        ""
    };
    let caret = left.len() + if space.is_empty() { 0 } else { 1 };
    let mut out = String::with_capacity(left.len() + space.len() + right.len() + rest.len());
    out.push_str(left);
    out.push_str(space);
    out.push_str(right);
    out.push_str(rest);
    let caret = caret.min(out.len());
    Some((out, caret))
}

/// Join every logical line overlapping `range` (visual `J`).
pub fn join_range(source: &str, range: Range<usize>) -> (String, usize) {
    let start = clamp_off(source, range.start.min(range.end));
    let end = clamp_off(source, range.start.max(range.end));
    let first = logical_line_range(source, start);
    let last = logical_line_range(source, end.saturating_sub(if end > start { 1 } else { 0 }));
    let mut n = 0usize;
    let mut i = first.start;
    while i <= last.start && i < source.len() {
        n += 1;
        let line = logical_line_range(source, i);
        if line.end >= source.len() {
            break;
        }
        i = line.end + 1;
        if n > 10_000 {
            break;
        }
    }
    let joins = n.saturating_sub(1).max(1);
    join_next_lines(source, start, joins)
}

/// Replace `count` characters starting at `offset` with `ch` (vim `3rx`).
pub fn replace_chars(source: &str, offset: usize, count: usize, ch: char) -> (String, usize) {
    let offset = clamp_off(source, offset);
    let count = count.max(1);
    let mut i = offset;
    let mut n = 0usize;
    for _ in 0..count {
        match next_char(source, i) {
            Some((k, c)) if c != '\n' => {
                i += k;
                n += 1;
            }
            _ => break,
        }
    }
    if n == 0 {
        return (source.to_string(), offset);
    }
    let repl: String = std::iter::repeat(ch).take(n).collect();
    let mut out = String::with_capacity(source.len() - (i - offset) + repl.len());
    out.push_str(&source[..offset]);
    out.push_str(&repl);
    out.push_str(&source[i..]);
    (out, offset)
}

/// Helix: replace every non-newline grapheme (char) in `range` with `ch`.
pub fn replace_selection(source: &str, range: Range<usize>, ch: char) -> (String, usize) {
    let start = clamp_off(source, range.start.min(range.end));
    let end = clamp_off(source, range.start.max(range.end));
    if start == end {
        return replace_chars(source, start, 1, ch);
    }
    let mut out = String::with_capacity(source.len());
    out.push_str(&source[..start]);
    for c in source[start..end].chars() {
        if c == '\n' {
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out.push_str(&source[end..]);
    (out, start)
}

/// Next / previous blank-line-separated region (a GFM block in serialize).
/// Paragraph = start of the next/previous non-empty line after a blank line,
/// or start of a block. We treat a line after `\n\n` as a paragraph start,
/// plus offset 0.
pub fn paragraph_jump(source: &str, offset: usize, dir: i8, count: usize) -> usize {
    let count = count.max(1);
    let mut offset = clamp_off(source, offset);
    for _ in 0..count {
        let next = if dir >= 0 {
            next_paragraph(source, offset)
        } else {
            prev_paragraph(source, offset)
        };
        if next == offset {
            break;
        }
        offset = next;
    }
    offset
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut out = vec![0];
    for (i, c) in source.char_indices() {
        if c == '\n' && i + 1 <= source.len() {
            out.push(i + 1);
        }
    }
    if out.last() != Some(&source.len()) && source.ends_with('\n') {
        // trailing empty line start is already pushed
    }
    out
}

fn line_is_blank(source: &str, start: usize) -> bool {
    let end = source[start..]
        .find('\n')
        .map(|i| start + i)
        .unwrap_or(source.len());
    source[start..end].trim().is_empty()
}

fn next_paragraph(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    let starts = line_starts(source);
    let mut ix = starts
        .iter()
        .position(|&s| s > offset)
        .unwrap_or(starts.len());
    // Advance to a blank line (or stay), then the next non-blank.
    let mut seen_blank = line_is_blank(source, logical_line_range(source, offset).start)
        || (offset > 0
            && source.as_bytes().get(offset.saturating_sub(1)) == Some(&b'\n')
            && source.as_bytes().get(offset.saturating_sub(2)) == Some(&b'\n'));
    while ix < starts.len() {
        let s = starts[ix];
        if s >= source.len() {
            break;
        }
        let blank = line_is_blank(source, s);
        if seen_blank && !blank {
            return s;
        }
        if blank {
            seen_blank = true;
        }
        ix += 1;
    }
    source.len()
}

fn prev_paragraph(source: &str, offset: usize) -> usize {
    let offset = clamp_off(source, offset);
    let starts = line_starts(source);
    // Paragraph starts: offset 0 and any non-blank line preceded by a blank.
    let mut paras: Vec<usize> = Vec::new();
    let mut prev_blank = true;
    for &s in &starts {
        if s >= source.len() {
            continue;
        }
        let blank = line_is_blank(source, s);
        if !blank && prev_blank {
            paras.push(s);
        }
        prev_blank = blank;
    }
    paras
        .iter()
        .copied()
        .rev()
        .find(|&s| s < offset)
        .unwrap_or(0)
}

/// Next/previous ATX heading line (`#` after optional indent).
pub fn heading_jump(source: &str, offset: usize, dir: i8, count: usize) -> usize {
    let count = count.max(1);
    let mut offset = clamp_off(source, offset);
    for _ in 0..count {
        let next = if dir >= 0 {
            next_heading(source, offset)
        } else {
            prev_heading(source, offset)
        };
        if next == offset {
            break;
        }
        offset = next;
    }
    offset
}

fn is_heading_line(source: &str, start: usize) -> bool {
    let line = &source[start..];
    let t = line.trim_start_matches([' ', '\t']);
    t.starts_with('#')
        && t.as_bytes()
            .get(1)
            .map(|b| *b == b'#' || *b == b' ' || *b == b'\n')
            .unwrap_or(true)
}

fn next_heading(source: &str, offset: usize) -> usize {
    let starts = line_starts(source);
    for &s in &starts {
        if s > offset && s < source.len() && is_heading_line(source, s) {
            return s;
        }
    }
    offset
}

fn prev_heading(source: &str, offset: usize) -> usize {
    let starts = line_starts(source);
    starts
        .iter()
        .copied()
        .rev()
        .find(|&s| s < offset && s < source.len() && is_heading_line(source, s))
        .unwrap_or(offset)
}

/// First match of `query` at or after `from`, wrapping if `wrap`.
pub fn search_next(source: &str, from: usize, query: &str, wrap: bool) -> Option<Range<usize>> {
    if query.is_empty() {
        return None;
    }
    let from = from.min(source.len());
    if let Some(rel) = source[from..].find(query) {
        let start = from + rel;
        return Some(start..start + query.len());
    }
    if wrap && from > 0 {
        if let Some(rel) = source[..from].find(query) {
            return Some(rel..rel + query.len());
        }
    }
    None
}

/// Previous match ending at or before `from`, wrapping if `wrap`.
pub fn search_prev(source: &str, from: usize, query: &str, wrap: bool) -> Option<Range<usize>> {
    if query.is_empty() {
        return None;
    }
    let from = from.min(source.len());
    let hay = if from >= query.len() {
        &source[..from.saturating_sub(0)]
    } else {
        ""
    };
    // Exclusive of a match that *starts* at from (we want strictly previous).
    let limit = from;
    if let Some(start) = source[..limit].rfind(query) {
        if start + query.len() <= limit || start < from {
            return Some(start..start + query.len());
        }
    }
    let _ = hay;
    if wrap {
        if let Some(start) = source.rfind(query) {
            if start >= from {
                return Some(start..start + query.len());
            }
        }
    }
    None
}

/// Last line start of the document (`G`).
pub fn last_line_start(source: &str) -> usize {
    logical_line_range(source, source.len()).start
}

/// Line start of 1-based line number (`count G`).
pub fn line_start_n(source: &str, n: usize) -> usize {
    let n = n.max(1);
    let mut line = 1usize;
    let mut i = 0usize;
    loop {
        let r = logical_line_range(source, i);
        if line == n {
            return r.start;
        }
        if r.end >= source.len() {
            return r.start;
        }
        i = r.end + 1;
        line += 1;
        if line > 1_000_000 {
            return r.start;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn off(src: &str, motion: Motion, at: usize, count: usize) -> usize {
        apply_motion(src, at, motion, count, None)
    }

    #[test]
    fn hl_chars() {
        let s = "ab";
        assert_eq!(off(s, Motion::Right, 0, 1), 1);
        assert_eq!(off(s, Motion::Right, 0, 2), 2);
        assert_eq!(off(s, Motion::Left, 2, 1), 1);
        assert_eq!(off(s, Motion::Left, 0, 3), 0);
    }

    #[test]
    fn hl_do_not_wrap_lines_or_blocks() {
        let s = "ab\ncd";
        assert_eq!(off(s, Motion::Left, 0, 1), 0);
        assert_eq!(off(s, Motion::Left, 0, 5), 0);
        assert_eq!(off(s, Motion::Right, 2, 1), 2);
        assert_eq!(off(s, Motion::Right, 2, 5), 2);
        assert_eq!(off(s, Motion::Right, 0, 5), 2);
        assert_eq!(off(s, Motion::Right, 1, 5), 2);
        assert_eq!(off(s, Motion::Left, 3, 1), 3);
        assert_eq!(off(s, Motion::Left, 3, 9), 3);
        assert_eq!(apply_motion(s, 0, Motion::Left, 3, None), 0);
        assert_eq!(apply_motion(s, 4, Motion::Right, 9, None), 5);
        assert_eq!(apply_motion(s, 3, Motion::Left, 1, None), 3);
        let long = "abcdefghij";
        assert_eq!(apply_motion(long, 0, Motion::Right, 20, Some(4)), 10);
        assert_eq!(apply_motion(long, 4, Motion::Left, 1, Some(4)), 3);
        assert_eq!(apply_motion(long, 0, Motion::Left, 3, Some(4)), 0);
        assert_eq!(apply_motion(s, 0, Motion::Up, 1, None), 0);
        assert_eq!(apply_motion(s, 4, Motion::Down, 1, None), 4);
    }

    #[test]
    fn block_caret_covers_char_or_last_at_eol() {
        assert_eq!(block_caret_range("ab", 0), 0..1);
        assert_eq!(block_caret_range("ab", 1), 1..2);
        assert_eq!(block_caret_range("ab", 2), 1..2);
        assert_eq!(block_caret_range("", 0), 0..0);
        assert_eq!(block_caret_range("ab\ncd", 2), 1..2);
        assert_eq!(block_caret_range("ab\ncd", 3), 3..4);
        assert_eq!(block_caret_range("ab\ncd", 5), 4..5);
        assert_eq!(block_caret_range("\n", 0), 0..0);
        assert_eq!(block_caret_range("\n", 1), 1..1);
    }

    #[test]
    fn jk_logical_lines() {
        let s = "aa\nbb\ncc";
        assert_eq!(off(s, Motion::Down, 0, 1), 3);
        assert_eq!(off(s, Motion::Down, 1, 1), 4);
        assert_eq!(off(s, Motion::Down, 0, 2), 6);
        assert_eq!(off(s, Motion::Up, 6, 1), 3);
        assert_eq!(off(s, Motion::Up, 4, 1), 1);
    }

    #[test]
    fn down_from_eol_does_not_skip_next_line() {
        // Caret at the `\n` (EOL of "aa") must stay on that row so j/Down
        // lands on "bb", not the line after.
        let s = "aa\nbb\ncc";
        assert_eq!(off(s, Motion::Down, 2, 1), 5);
        assert_eq!(off(s, Motion::Down, 2, 2), 8);
        let trailing = "hello world\nhello\n";
        let at = "hello world".len();
        let next = off(trailing, Motion::Down, at, 1);
        assert_eq!(
            next,
            "hello world\nhello".len(),
            "EOL of a long line clamps onto the shorter line, not the trailing empty, got {next}"
        );
        assert_eq!(off(trailing, Motion::Down, 0, 1), "hello world\n".len());
    }

    #[test]
    fn down_from_end_of_paragraph_lands_in_next_paragraph() {
        use crate::display::project;
        let src = "hello world\n\nhello\n";
        let p = project(src);
        assert!(p.blocks.len() >= 2, "blocks={}", p.blocks.len());
        let at = p.blocks[0].display.end;
        let next = apply_motion(&p.display, at, Motion::Down, 1, None);
        let second = p
            .blocks
            .iter()
            .find(|b| b.display.start != b.display.end && b.display.start >= at)
            .expect("second paragraph");
        assert!(
            next >= second.display.start && next <= second.display.end,
            "down from {at} landed at {next}, second={:?}, display={:?}",
            second.display,
            p.display
        );
    }

    #[test]
    fn jk_stops_at_buffer_edges() {
        let s = "aa\nbb";
        assert_eq!(apply_motion(s, 0, Motion::Up, 1, None), 0);
        assert_eq!(apply_motion(s, 4, Motion::Down, 1, None), 4);
        assert_eq!(apply_motion(s, 4, Motion::Down, 3, None), 4);
        let one = "only";
        assert_eq!(apply_motion(one, 2, Motion::Down, 1, None), 2);
    }

    #[test]
    fn wrap_jk_walks_visual_rows() {
        let s = "abcdefghij";
        assert_eq!(apply_motion(s, 0, Motion::Down, 1, Some(4)), 4);
        assert_eq!(apply_motion(s, 3, Motion::Down, 1, Some(4)), 7);
        assert_eq!(apply_motion(s, 4, Motion::Down, 1, Some(4)), 8);
        assert_eq!(apply_motion(s, 8, Motion::Down, 1, Some(4)), 8);
        assert_eq!(apply_motion(s, 0, Motion::Up, 1, Some(4)), 0);
    }

    #[test]
    fn j_from_heading_into_next_paragraph() {
        let s = "# H\n\npara";
        let a = apply_motion(s, 0, Motion::Down, 1, None);
        assert_eq!(a, 4, "j from heading lands on the blank line");
        let b = apply_motion(s, a, Motion::Down, 1, None);
        assert!(
            s[b..].starts_with("para"),
            "second j lands in para, got {:?}",
            &s[b..]
        );
        assert_eq!(apply_motion(s, 0, Motion::Right, 20, Some(4)), 3);
        assert_eq!(apply_motion(s, 5, Motion::Left, 9, Some(4)), 5);
    }

    #[test]
    fn visual_line_is_exactly_one_logical_line() {
        let s = "# H\n\npara\nnext";
        let range = visual_line_range(s, 0);
        assert_eq!(&s[range.clone()], "# H\n");
        assert_eq!(range, 0..4);
        let para = s.find("para").unwrap();
        let range = visual_line_range(s, para);
        assert_eq!(&s[range.clone()], "para\n");
        let ext = extend_visual_line(s, range.clone(), 1);
        assert!(s[ext.clone()].contains("para"));
        assert!(s[ext.clone()].contains("next"));
        assert!(!s[ext].contains("# H"));
    }

    #[test]
    fn word_motions_skip_punctuation_attached() {
        let s = "foo,bar baz";
        assert_eq!(off(s, Motion::WordForward, 0, 1), 3);
        assert_eq!(off(s, Motion::WordForwardWs, 0, 1), 8);
        assert_eq!(&s[8..], "baz");
        assert_eq!(off(s, Motion::WordBackWs, 8, 1), 0);
        let e = off(s, Motion::WordEndWs, 0, 1);
        assert_eq!(&s[e..=e], "r");
        let e2 = off(s, Motion::WordEndWs, e, 1);
        assert_eq!(&s[e2..=e2], "z");
    }

    #[test]
    fn zero_caret_and_dollar() {
        let s = "  hi\nxy";
        assert_eq!(off(s, Motion::LineStart, 4, 1), 0);
        assert_eq!(off(s, Motion::LineFirstNonBlank, 0, 1), 2);
        assert_eq!(off(s, Motion::LineEnd, 2, 1), 4);
        assert_eq!(off(s, Motion::LineStart, 6, 1), 5);
        assert_eq!(off(s, Motion::LineEnd, 5, 1), 7);
    }

    #[test]
    fn words() {
        let s = "foo bar,baz";
        assert_eq!(off(s, Motion::WordForward, 0, 1), 4);
        assert_eq!(off(s, Motion::WordForward, 0, 2), 7);
        assert_eq!(off(s, Motion::WordBack, 5, 1), 4);
        assert_eq!(off(s, Motion::WordBack, 4, 1), 0);
        let e = off(s, Motion::WordEnd, 0, 1);
        assert_eq!(&s[e..=e], "o");
    }

    #[test]
    fn counts() {
        assert_eq!(push_count(None, 0), None);
        assert_eq!(push_count(None, 5), Some(5));
        assert_eq!(push_count(Some(1), 0), Some(10));
        assert_eq!(push_count(Some(1), 2), Some(12));
        let mut p = Some(3);
        assert_eq!(take_count(&mut p), 3);
        assert_eq!(take_count(&mut p), 1);
        let s = "a\nb\nc\nd";
        assert_eq!(off(s, Motion::Down, 0, 2), 4);
    }

    #[test]
    fn delete_char_and_line() {
        let (out, caret) = delete_char_at("abc", 1);
        assert_eq!(out, "ac");
        assert_eq!(caret, 1);
        let s = "aa\nbb\ncc";
        let range = logical_line_delete_range(s, 4);
        let (out, _) = delete_range(s, range);
        assert_eq!(out, "aa\ncc");
        let (out, caret) = open_line_below("aa\nbb", 1);
        assert_eq!(out, "aa\n\nbb");
        assert_eq!(caret, 3);
        let (out, caret) = open_line_above("aa\nbb", 4);
        assert_eq!(out, "aa\n\nbb");
        assert_eq!(caret, 3);
        assert_eq!(after_caret("ab", 0), 1);
        assert_eq!(after_caret("ab", 2), 2);
    }

    #[test]
    fn x_does_not_imply_block_delete() {
        let src = "# Title\n\npara";
        let (out, _) = delete_char_at(src, 0);
        assert!(out.contains("Title"));
        assert!(out.contains("para"));
        let range = logical_line_delete_range("# Title", 0);
        let (out, _) = delete_range("# Title", range);
        assert_eq!(out, "");
    }

    #[test]
    fn find_char_on_line_only() {
        let s = "abca\nxyz";
        assert_eq!(find_char(s, 0, 'a', FindKind::Forward, 1, true), Some(3));
        assert_eq!(find_char(s, 0, 'a', FindKind::Forward, 2, true), None); // second a would wrap the line
        assert_eq!(find_char(s, 0, 'x', FindKind::Forward, 1, true), None); // vim: does not leave the line
        assert_eq!(find_char(s, 0, 'c', FindKind::Till, 1, true), Some(1)); // t c lands on b, before c
        assert_eq!(find_char(s, 3, 'a', FindKind::Backward, 1, true), Some(0));
        assert_eq!(find_char(s, 0, 'b', FindKind::Forward, 1, true), Some(1));
        let s = "aaaba";
        assert_eq!(find_char(s, 0, 'a', FindKind::Forward, 2, true), Some(2)); // 2fa
        assert_eq!(find_char(s, 4, 'a', FindKind::Backward, 2, true), Some(1));
        let t = find_char(s, 0, 'b', FindKind::Till, 1, true).unwrap();
        assert_eq!(t, 2); // t b lands on last a before b
        let tb = find_char(s, 4, 'b', FindKind::TillBack, 1, true).unwrap();
        assert_eq!(tb, 4); // T b from last a lands just after b
                           // Helix: f/t search the rest of the buffer, not the current line.
        let s = "abca\nxyz";
        assert_eq!(find_char(s, 0, 'x', FindKind::Forward, 1, false), Some(5));
        assert_eq!(find_char(s, 5, 'a', FindKind::Backward, 1, false), Some(3));
    }

    #[test]
    fn join_next_across_blank_and_blocks() {
        let s = "# Title\n\nA paragraph\n";
        let (out, caret) = join_next_lines(s, 0, 1);
        assert_eq!(out, "# Title A paragraph\n");
        assert_eq!(&out[caret..].chars().next().unwrap(), &'A');
        let s = "hello\n  world";
        let (out, _) = join_next_lines(s, 0, 1);
        assert_eq!(out, "hello world");
        let s = "a\nb\nc\nd";
        let (out, _) = join_next_lines(s, 0, 3);
        assert_eq!(out, "a b c d");
        let (out, _) = join_range("aa\nbb\ncc", 0..5);
        assert_eq!(out, "aa bb\ncc");
        let (out, _) = join_range("aa\nbb\ncc", 0..8);
        assert_eq!(out, "aa bb cc");
    }

    #[test]
    fn replace_char_and_selection() {
        let (out, caret) = replace_chars("hello", 1, 1, 'x');
        assert_eq!(out, "hxllo");
        assert_eq!(caret, 1);
        let (out, _) = replace_chars("hello", 0, 3, 'x');
        assert_eq!(out, "xxxlo");
        let (out, _) = replace_selection("hello", 0..5, 'x');
        assert_eq!(out, "xxxxx");
        let (out, _) = replace_selection("ab\ncd", 0..5, 'z');
        assert_eq!(out, "zz\nzz");
    }

    #[test]
    fn paragraph_and_heading_jumps() {
        let s = "# A\n\npara one\n\n## B\n\npara two\n";
        let p1 = paragraph_jump(s, 0, 1, 1);
        assert!(s[p1..].starts_with("para one"), "{:?}", &s[p1..]);
        let p2 = paragraph_jump(s, 0, 1, 2);
        assert!(
            s[p2..].starts_with("## B") || s[p2..].starts_with("para two"),
            "{:?}",
            &s[p2..]
        );
        let h = heading_jump(s, 0, 1, 1);
        assert!(s[h..].starts_with("## B"), "{:?}", &s[h..]);
        let back = heading_jump(s, h, -1, 1);
        assert_eq!(back, 0);
        let prev = paragraph_jump(s, p1, -1, 1);
        assert_eq!(prev, 0);
    }

    #[test]
    fn search_wraps() {
        let s = "foo bar foo";
        assert_eq!(search_next(s, 0, "foo", true), Some(0..3));
        assert_eq!(search_next(s, 1, "foo", true), Some(8..11));
        assert_eq!(search_next(s, 9, "foo", true), Some(0..3)); // wrap
        assert_eq!(search_prev(s, 8, "foo", true), Some(0..3));
        assert_eq!(search_prev(s, 1, "foo", true), Some(8..11)); // wrap
        assert_eq!(search_next(s, 0, "nope", true), None);
        assert_eq!(search_next(s, 0, "", true), None);
    }

    #[test]
    fn whichwrap_crosses_lines_in_insert() {
        let s = "aa\nbb";
        assert_eq!(whichwrap(s, 3, Motion::Left), Some(2));
        assert_eq!(whichwrap(s, 2, Motion::Right), Some(3));
        assert_eq!(whichwrap(s, 1, Motion::Left), None);
        assert_eq!(whichwrap(s, 0, Motion::Left), None);
    }

    #[test]
    fn after_caret_same_line_stops_at_eol() {
        let s = "ab\ncd";
        // On 'b' (offset 1) → after it at 2 (EOL), not onto next line.
        assert_eq!(after_caret_same_line(s, 1), 2);
        // Already at EOL → stay.
        assert_eq!(after_caret_same_line(s, 2), 2);
        // Unclamped after_caret would cross the newline:
        assert_eq!(after_caret(s, 2), 3);
    }
}

#[cfg(test)]
mod word_select_tests {
    use super::{big_word_range_at, pair_around, quote_around, word_range_at};

    #[test]
    fn word_range_selects_word_under_caret() {
        assert_eq!(word_range_at("hello world", 1), 0..5);
        assert_eq!(word_range_at("hello world", 4), 0..5);
        assert_eq!(word_range_at("hello world", 5), 5..5); // space
        assert_eq!(word_range_at("hello world", 6), 6..11);
        assert_eq!(word_range_at("hello world", 11), 6..11);
        assert_eq!(word_range_at("a", 0), 0..1);
        assert_eq!(word_range_at("  ", 1), 1..1);
    }

    #[test]
    fn big_word_ignores_punctuation() {
        assert_eq!(big_word_range_at("foo-bar baz", 2), 0..7);
        assert_eq!(big_word_range_at("foo-bar baz", 8), 8..11);
        assert_eq!(big_word_range_at("foo bar", 3), 3..3);
    }

    #[test]
    fn word_range_at_eol_targets_last_word() {
        // Caret parked on the line break (Normal `$`/`gl`): miw/viw still
        // selects the line's last word instead of no-op.
        assert_eq!(word_range_at("Title\nnext", 5), 0..5);
        assert_eq!(big_word_range_at("Title\nnext", 5), 0..5);
        // Mid-line blanks still select nothing.
        assert_eq!(word_range_at("a b", 1), 1..1);
        // Empty line break backs onto whitespace → nothing.
        assert_eq!(word_range_at("a\n\nb", 2), 2..2);
    }

    #[test]
    fn pair_finds_enclosing_with_nesting() {
        let s = "a (b (c) d) e";
        assert_eq!(pair_around(s, 9, '(', ')'), Some((2, 10)));
        // Inside the inner pair.
        assert_eq!(pair_around(s, 6, '(', ')'), Some((5, 7)));
        // Outside everything.
        assert_eq!(pair_around("(a", 0, '(', ')'), None);
        assert_eq!(pair_around("a )", 3, '(', ')'), None);
    }

    #[test]
    fn quotes_straddle_caret() {
        let s = "say \"hi there\" ok";
        assert_eq!(quote_around(s, 6, '"'), Some((4, 13)));
        assert_eq!(quote_around(s, 0, '"'), None);
    }
}
