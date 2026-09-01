//! GFM paint helper + small source-buffer utilities.
//!
//! The document is the markdown file bytes (`source`). `parse_ranges` is a
//! pure paint helper: given source, return covering ranges + kind. Never store
//! blocks as the document.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use pulldown_cmark::{Event, Options, Parser, Tag};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub fn gfm_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl AlertKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "NOTE",
            Self::Tip => "TIP",
            Self::Important => "IMPORTANT",
            Self::Warning => "WARNING",
            Self::Caution => "CAUTION",
        }
    }

    #[allow(dead_code)]
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_uppercase().as_str() {
            "NOTE" => Some(Self::Note),
            "TIP" => Some(Self::Tip),
            "IMPORTANT" => Some(Self::Important),
            "WARNING" => Some(Self::Warning),
            "CAUTION" => Some(Self::Caution),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading(u8),
    List { ordered: bool },
    Code,
    Quote,
    Alert(AlertKind),
    Table,
    Rule,
    Html,
    Raw,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub id: u64,
    pub kind: BlockKind,
    pub source: String,
}

impl Block {
    pub fn paragraph(source: impl Into<String>) -> Self {
        Self {
            id: next_id(),
            kind: BlockKind::Paragraph,
            source: source.into(),
        }
    }

    pub fn with_kind(kind: BlockKind, source: impl Into<String>) -> Self {
        Self {
            id: next_id(),
            kind,
            source: source.into(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.source.trim().is_empty()
    }
}

/// A GFM block's exact byte range in `source`. Paint-only — not the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintRange {
    pub kind: BlockKind,
    pub range: Range<usize>,
}

impl PaintRange {
    pub fn slice<'a>(&self, src: &'a str) -> &'a str {
        let start = self.range.start.min(src.len());
        let end = self.range.end.min(src.len()).max(start);
        &src[start..end]
    }

    pub fn is_blank(&self, src: &str) -> bool {
        self.slice(src).trim().is_empty()
    }
}

/// Parse GFM into paint ranges that cover `0..src.len()`.
/// Unparsed gaps (blank lines, leftovers) are [`BlockKind::Raw`].
pub fn parse_ranges(src: &str) -> Vec<PaintRange> {
    if src.is_empty() {
        return vec![PaintRange {
            kind: BlockKind::Paragraph,
            range: 0..0,
        }];
    }

    let parser = Parser::new_ext(src, gfm_options()).into_offset_iter();
    let mut covered: Vec<(usize, usize, BlockKind)> = Vec::new();
    let mut depth = 0usize;
    let mut current_start = 0usize;
    let mut current_kind = BlockKind::Raw;

    for (event, range) in parser {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    current_start = range.start;
                    current_kind = kind_from_tag(&tag);
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    covered.push((current_start, range.end, current_kind));
                }
            }
            Event::Rule if depth == 0 => {
                covered.push((range.start, range.end, BlockKind::Rule));
            }
            _ => {}
        }
    }

    covered.sort_by_key(|(start, _, _)| *start);

    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    for (start, end, kind) in covered {
        let start = start.min(src.len()).max(cursor);
        let end = end.min(src.len()).max(start);
        if start > cursor {
            ranges.push(PaintRange {
                kind: BlockKind::Raw,
                range: cursor..start,
            });
        }
        if end > start {
            ranges.push(PaintRange {
                kind,
                range: start..end,
            });
        }
        cursor = cursor.max(end);
    }
    if cursor < src.len() {
        ranges.push(PaintRange {
            kind: BlockKind::Raw,
            range: cursor..src.len(),
        });
    }
    if ranges.is_empty() {
        ranges.push(PaintRange {
            kind: BlockKind::Paragraph,
            range: 0..src.len(),
        });
    }
    ranges
}

fn contains_caret(range: &Range<usize>, caret: usize, src_len: usize) -> bool {
    if range.start == range.end {
        return caret == range.start;
    }
    if caret >= src_len {
        return range.end >= src_len && caret >= range.start;
    }
    caret >= range.start && caret < range.end
}

fn intersects(range: &Range<usize>, hot: &Range<usize>) -> bool {
    range.start < hot.end && hot.start < range.end
}

/// One continuous raw span covering every paint range that intersects `caret`
/// or `sel` (including gaps between them).
pub fn raw_span(
    ranges: &[PaintRange],
    src_len: usize,
    caret: usize,
    sel: Option<Range<usize>>,
) -> Range<usize> {
    let caret = caret.min(src_len);
    let hot = sel.map(|s| {
        let a = s.start.min(s.end).min(src_len);
        let b = s.start.max(s.end).min(src_len);
        a..b
    });
    let mut first = None;
    let mut last = None;
    for (i, r) in ranges.iter().enumerate() {
        let hit = match &hot {
            Some(h) if h.start != h.end => {
                intersects(&r.range, h) || contains_caret(&r.range, caret, src_len)
            }
            _ => contains_caret(&r.range, caret, src_len),
        };
        if hit {
            if first.is_none() {
                first = Some(i);
            }
            last = Some(i);
        }
    }
    match (first, last) {
        (Some(a), Some(b)) => ranges[a].range.start..ranges[b].range.end,
        _ => caret..caret,
    }
}

/// Replace `source[range]` with `insert`.
pub fn splice(source: &str, range: Range<usize>, insert: &str) -> String {
    let start = range.start.min(source.len());
    let end = range.end.min(source.len()).max(start);
    let mut out = String::with_capacity(source.len() - (end - start) + insert.len());
    out.push_str(&source[..start]);
    out.push_str(insert);
    out.push_str(&source[end..]);
    out
}

/// `/` query on the logical line containing `offset` (insert, line start).
pub fn slash_query_at(source: &str, offset: usize) -> Option<&str> {
    let offset = offset.min(source.len());
    let start = source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = source[offset..]
        .find('\n')
        .map(|i| offset + i)
        .unwrap_or(source.len());
    slash_query(&source[start..end])
}

/// Parse GFM into top-level blocks, preserving source slices (plus gap raw).
pub fn parse_blocks(src: &str) -> Vec<Block> {
    if src.trim().is_empty() {
        return Vec::new();
    }

    let parser = Parser::new_ext(src, gfm_options()).into_offset_iter();
    let mut covered: Vec<(usize, usize, BlockKind)> = Vec::new();
    let mut depth = 0usize;
    let mut current_start = 0usize;
    let mut current_kind = BlockKind::Raw;

    for (event, range) in parser {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    current_start = range.start;
                    current_kind = kind_from_tag(&tag);
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    covered.push((current_start, range.end, current_kind));
                }
            }
            Event::Rule if depth == 0 => {
                covered.push((range.start, range.end, BlockKind::Rule));
            }
            _ => {}
        }
    }

    covered.sort_by_key(|(start, _, _)| *start);

    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    for (start, end, kind) in covered {
        let start = start.min(src.len());
        let end = end.min(src.len()).max(start);
        if start > cursor {
            push_gap(&mut blocks, &src[cursor..start]);
        }
        let source = src[start..end].trim_end_matches(['\n', '\r']).to_string();
        if !source.trim().is_empty() {
            blocks.push(Block {
                id: next_id(),
                kind,
                source,
            });
        }
        cursor = end;
    }
    if cursor < src.len() {
        push_gap(&mut blocks, &src[cursor..]);
    }
    blocks
}

fn push_gap(blocks: &mut Vec<Block>, gap: &str) {
    let trimmed = gap.trim();
    if trimmed.is_empty() {
        return;
    }
    blocks.push(Block {
        id: next_id(),
        kind: BlockKind::Raw,
        source: trimmed.to_string(),
    });
}

fn kind_from_tag(tag: &Tag<'_>) -> BlockKind {
    match tag {
        Tag::Paragraph => BlockKind::Paragraph,
        Tag::Heading { level, .. } => BlockKind::Heading(*level as u8),
        Tag::List(start) => BlockKind::List {
            ordered: start.is_some(),
        },
        Tag::CodeBlock(_) => BlockKind::Code,
        Tag::BlockQuote(kind) => match kind {
            Some(pulldown_cmark::BlockQuoteKind::Note) => BlockKind::Alert(AlertKind::Note),
            Some(pulldown_cmark::BlockQuoteKind::Tip) => BlockKind::Alert(AlertKind::Tip),
            Some(pulldown_cmark::BlockQuoteKind::Important) => {
                BlockKind::Alert(AlertKind::Important)
            }
            Some(pulldown_cmark::BlockQuoteKind::Warning) => BlockKind::Alert(AlertKind::Warning),
            Some(pulldown_cmark::BlockQuoteKind::Caution) => BlockKind::Alert(AlertKind::Caution),
            None => BlockKind::Quote,
        },
        Tag::Table(_) => BlockKind::Table,
        Tag::HtmlBlock => BlockKind::Html,
        _ => BlockKind::Raw,
    }
}

/// Join blocks into a GFM file. Empty trailing paragraphs are omitted.
pub fn serialize(blocks: &[Block]) -> String {
    let parts: Vec<&str> = blocks
        .iter()
        .map(|b| b.source.trim_end())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    let mut out = parts.join("\n\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Ensure there is a trailing empty paragraph so the user always has a place
/// to type. Not written to disk.
pub fn ensure_trailing_paragraph(blocks: &mut Vec<Block>) {
    match blocks.last() {
        Some(b) if b.kind == BlockKind::Paragraph && b.is_empty() => {}
        _ => blocks.push(Block::paragraph("")),
    }
}

/// Replace `index` with the blocks parsed from `source`.
/// Returns the index of the last inserted block.
pub fn replace_block(blocks: &mut Vec<Block>, index: usize, source: String) -> usize {
    if index >= blocks.len() {
        blocks.push(Block::paragraph(source));
        return blocks.len() - 1;
    }
    let parsed = parse_blocks(&source);
    if parsed.is_empty() {
        blocks[index] = Block::paragraph("");
        return index;
    }
    let n = parsed.len();
    blocks.splice(index..index + 1, parsed);
    index + n - 1
}

pub fn slash_query(source: &str) -> Option<&str> {
    let t = source.trim();
    let rest = t.strip_prefix('/')?;
    if rest.contains('\n') {
        return None;
    }
    Some(rest.trim_start())
}

/// A markdown link or autolink found in block source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundLink {
    pub text: String,
    pub url: String,
}

/// `![alt](src)` when the block is only an image (plus whitespace).
pub fn sole_image(source: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() != 1 {
        return None;
    }
    parse_image_line(lines[0])
}

fn parse_image_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("![")?;
    let close = rest.find(']')?;
    let alt = rest[..close].to_string();
    let rest = rest[close + 1..].strip_prefix('(')?;
    let close = rest.find(')')?;
    let inner = rest[..close].trim();
    let src = inner
        .split_whitespace()
        .next()
        .unwrap_or(inner)
        .trim_matches('"')
        .to_string();
    if src.is_empty() {
        return None;
    }
    Some((alt, src))
}

/// Toggle `- [ ]` / `- [x]` on `line_index`. None if that line is not a task.
///
/// Preserves a trailing newline on `source` so splicing a list block back into
/// the document does not eat the blank-line separator before the next block
/// (`item 2\n\n## Quote` must not become `item 2\n## Quote`).
pub fn toggle_task_line(source: &str, line_index: usize) -> Option<String> {
    let trailing_nl = source.ends_with('\n');
    let mut lines: Vec<String> = source.lines().map(|l| l.to_string()).collect();
    let line = lines.get(line_index)?;
    let (prefix, checked, rest) = split_task_line(line)?;
    let mark = if checked { " " } else { "x" };
    lines[line_index] = format!("{prefix}[{mark}]{rest}");
    let mut out = lines.join("\n");
    if trailing_nl {
        out.push('\n');
    }
    Some(out)
}

/// `(prefix, checked, rest)` where rest includes the space after `]`.
pub fn split_task_line(line: &str) -> Option<(String, bool, String)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let after_indent = &line[i..];
    let marker_end = if after_indent.starts_with("- ")
        || after_indent.starts_with("* ")
        || after_indent.starts_with("+ ")
    {
        i + 2
    } else {
        let digits = after_indent
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        let rest = &after_indent[digits..];
        if !rest.starts_with(". ") && !rest.starts_with(") ") {
            return None;
        }
        i + digits + 2
    };
    if marker_end + 3 > line.len() {
        return None;
    }
    let boxy = &line[marker_end..];
    let (checked, rest) = if boxy.starts_with("[ ]") {
        (false, &boxy[3..])
    } else if boxy.starts_with("[x]") || boxy.starts_with("[X]") {
        (true, &boxy[3..])
    } else {
        return None;
    };
    Some((line[..marker_end].to_string(), checked, rest.to_string()))
}

pub fn has_task_line(source: &str) -> bool {
    source.lines().any(|l| split_task_line(l).is_some())
}

pub fn extract_links(source: &str) -> Vec<FoundLink> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < source.len() {
        if source[i..].starts_with("![") {
            if let Some((_, _, end)) = parse_md_link_at(&source[i + 1..]) {
                i += 1 + end;
                continue;
            }
        }
        if source[i..].starts_with('[') {
            if let Some((text, url, end)) = parse_md_link_at(&source[i..]) {
                if !url.is_empty() && !url.starts_with('#') {
                    out.push(FoundLink { text, url });
                }
                i += end;
                continue;
            }
        }
        if source[i..].starts_with('<') {
            if let Some((url, end)) = parse_autolink_at(&source[i..]) {
                out.push(FoundLink {
                    text: url.clone(),
                    url,
                });
                i += end;
                continue;
            }
        }
        if source[i..].starts_with("http://") || source[i..].starts_with("https://") {
            let url = take_bare_url(&source[i..]);
            i += url.len();
            out.push(FoundLink {
                text: url.clone(),
                url,
            });
            continue;
        }
        i += source[i..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
    }
    out
}

fn parse_md_link_at(s: &str) -> Option<(String, String, usize)> {
    if !s.starts_with('[') {
        return None;
    }
    let close = s.find(']')?;
    let text = s[1..close].to_string();
    let after = s.get(close + 1..)?;
    if !after.starts_with('(') {
        return None;
    }
    let end_paren = after.find(')')?;
    let inner = after[1..end_paren].trim();
    let url = inner.split_whitespace().next().unwrap_or("").to_string();
    Some((text, url, close + 1 + end_paren + 1))
}

fn parse_autolink_at(s: &str) -> Option<(String, usize)> {
    let rest = s.strip_prefix('<')?;
    let close = rest.find('>')?;
    let inner = &rest[..close];
    if inner.starts_with("http://") || inner.starts_with("https://") {
        Some((inner.to_string(), close + 2))
    } else {
        None
    }
}

fn take_bare_url(s: &str) -> String {
    let n = s
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ')' && *c != ']' && *c != '>')
        .count();
    let mut url: String = s.chars().take(n).collect();
    while url.ends_with(['.', ',', ';', ':', '!', '?', '"', '\'']) {
        url.pop();
    }
    url
}

fn strip_one_trailing_newline(s: &str) -> &str {
    s.strip_suffix('\n')
        .map(|p| p.strip_suffix('\r').unwrap_or(p))
        .unwrap_or(s)
}

/// Join `current` onto the last line of `prev` as one nvim/hx buffer.
/// Strips a single trailing newline from `prev` so the heading lands on the
/// same line as the previous text. No extra newline is inserted.
pub fn join_line(prev: &str, current: &str) -> (String, usize) {
    let prev = strip_one_trailing_newline(prev);
    let caret = prev.len();
    let mut out = String::with_capacity(prev.len() + current.len());
    out.push_str(prev);
    out.push_str(current);
    (out, caret)
}

/// Join two block sources with backspace semantics (see [`join_line`]).
#[allow(dead_code)]
pub fn merge_blocks(prev: &str, current: &str) -> String {
    join_line(prev, current).0
}

/// Byte offset of the join point (end of the previous source, after stripping
/// one trailing newline).
#[allow(dead_code)]
pub fn merge_caret(prev: &str, current: &str) -> usize {
    join_line(prev, current).1
}

/// In-block backspace at column 0: delete the newline before `offset`.
/// `None` if `offset` is 0 or the previous character is not a newline.
pub fn join_line_at(source: &str, offset: usize) -> Option<(String, usize)> {
    if offset == 0 || offset > source.len() {
        return None;
    }
    if source.as_bytes()[offset - 1] != b'\n' {
        return None;
    }
    let skip = if offset >= 2 && source.as_bytes()[offset - 2] == b'\r' {
        2
    } else {
        1
    };
    let caret = offset - skip;
    let mut out = String::with_capacity(source.len() - skip);
    out.push_str(&source[..caret]);
    out.push_str(&source[offset..]);
    Some((out, caret))
}

/// Index to select after removing `ix` from a list of `len` blocks.
/// `None` if there is nothing to remove (`len == 0`).
pub fn delete_block_index(len: usize, ix: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if len == 1 {
        return Some(0);
    }
    let ix = ix.min(len - 1);
    if ix + 1 == len {
        Some(ix.saturating_sub(1))
    } else {
        Some(ix)
    }
}

pub fn infer_kind(source: &str) -> BlockKind {
    parse_blocks(source)
        .into_iter()
        .next()
        .map(|b| b.kind)
        .unwrap_or(BlockKind::Paragraph)
}

pub fn alert_icon_name(kind: AlertKind) -> &'static str {
    match kind {
        AlertKind::Note => "info",
        AlertKind::Tip => "lightbulb",
        AlertKind::Important => "star",
        AlertKind::Warning => "triangle-alert",
        AlertKind::Caution => "circle-alert",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KITCHEN: &str = r#"# Title

A paragraph with **bold**, *italic*, ~~strike~~, `code`, and a [link](https://example.com).

## Subhead

- bullet
- nested parent

1. one
2. two

- [ ] todo
- [x] done

> a quote

> [!NOTE]
> This is a note.

> [!WARNING]
> Careful.

```rust
fn main() {}
```

| a | b |
| --- | --- |
| 1 | 2 |

---

https://github.com/blankeos/crabmd

Visit <https://spec.commonmark.org>.
"#;

    #[test]
    fn kitchen_sink_kinds() {
        let blocks = parse_blocks(KITCHEN);
        let kinds: Vec<_> = blocks.iter().map(|b| b.kind).collect();
        assert!(
            kinds.contains(&BlockKind::Heading(1)),
            "missing h1: {kinds:?}"
        );
        assert!(kinds.contains(&BlockKind::Heading(2)));
        assert!(kinds.contains(&BlockKind::Paragraph));
        assert!(kinds.contains(&BlockKind::List { ordered: false }));
        assert!(kinds.contains(&BlockKind::List { ordered: true }));
        assert!(kinds.contains(&BlockKind::Quote));
        assert!(kinds.contains(&BlockKind::Alert(AlertKind::Note)));
        assert!(kinds.contains(&BlockKind::Alert(AlertKind::Warning)));
        assert!(kinds.contains(&BlockKind::Code));
        assert!(kinds.contains(&BlockKind::Table));
        assert!(kinds.contains(&BlockKind::Rule));
    }

    #[test]
    fn serialize_is_valid_gfm() {
        let blocks = parse_blocks(KITCHEN);
        let out = serialize(&blocks);
        let again = parse_blocks(&out);
        let kinds: Vec<_> = again.iter().map(|b| b.kind).collect();
        assert!(kinds.contains(&BlockKind::Alert(AlertKind::Note)));
        assert!(kinds.contains(&BlockKind::Table));
        assert!(kinds.contains(&BlockKind::Code));
        assert!(out.contains("~~strike~~"));
        assert!(out.contains("- [ ] todo"));
        assert!(out.contains("- [x] done"));
        assert!(out.contains("> [!NOTE]"));
        assert!(out.contains("| a | b |"));
        assert!(out.contains("```rust"));
    }

    #[test]
    fn empty_parses_to_nothing() {
        assert!(parse_blocks("").is_empty());
        assert!(parse_blocks("   \n\n").is_empty());
        assert_eq!(serialize(&[]), "");
    }

    #[test]
    fn task_list_round_trip() {
        let src = "- [ ] open\n- [x] closed\n";
        let blocks = parse_blocks(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, BlockKind::List { ordered: false });
        let out = serialize(&blocks);
        assert!(out.contains("- [ ] open"));
        assert!(out.contains("- [x] closed"));
    }

    #[test]
    fn alert_templates_parse() {
        for kind in [
            AlertKind::Note,
            AlertKind::Tip,
            AlertKind::Important,
            AlertKind::Warning,
            AlertKind::Caution,
        ] {
            let src = format!("> [!{}]\n> body\n", kind.as_str());
            let blocks = parse_blocks(&src);
            assert_eq!(
                blocks[0].kind,
                BlockKind::Alert(kind),
                "failed for {}",
                kind.as_str()
            );
        }
    }

    #[test]
    fn replace_block_splits_paragraphs() {
        let mut blocks = parse_blocks("hello\n");
        replace_block(&mut blocks, 0, "one\n\ntwo".into());
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, BlockKind::Paragraph);
        assert_eq!(blocks[1].kind, BlockKind::Paragraph);
    }

    #[test]
    fn slash_query_detects() {
        assert_eq!(slash_query("/"), Some(""));
        assert_eq!(slash_query("/head"), Some("head"));
        assert_eq!(slash_query("  /table"), Some("table"));
        assert_eq!(slash_query("not a slash"), None);
        assert_eq!(slash_query("/ heading"), Some("heading"));
    }

    #[test]
    fn toggle_task_flips_box() {
        let src = "- [ ] open\n- [x] done";
        let out = toggle_task_line(src, 0).unwrap();
        assert_eq!(out, "- [x] open\n- [x] done");
        let out = toggle_task_line(&out, 1).unwrap();
        assert_eq!(out, "- [x] open\n- [ ] done");
        assert!(toggle_task_line("- bullet", 0).is_none());
    }

    #[test]
    fn toggle_task_keeps_trailing_newline() {
        let src = "- [x] item 1\n- [x] item 2\n";
        let out = toggle_task_line(src, 0).unwrap();
        assert_eq!(out, "- [ ] item 1\n- [x] item 2\n");
        // Splicing this into `list\n\n## Quote` must keep the blank separator.
        let doc = format!("{src}\n## Quote\nbody\n");
        let start = 0;
        let end = src.len();
        let spliced = splice(&doc, start..end, &out);
        assert!(
            spliced.contains("item 2\n\n## Quote"),
            "separator collapsed: {spliced:?}"
        );
    }

    #[test]
    fn sole_image_detects() {
        assert_eq!(
            sole_image("![cat](cat.png)\n"),
            Some(("cat".into(), "cat.png".into()))
        );
        assert_eq!(sole_image("hello ![x](y.png)"), None);
        assert_eq!(sole_image("![a](b.png)\n\nmore"), None);
    }

    #[test]
    fn extract_links_skips_images() {
        let links = extract_links(
            "see [gfm](https://example.com) and ![pic](pic.png) plus <https://spec.commonmark.org> and https://zed.dev.",
        );
        let urls: Vec<_> = links.iter().map(|l| l.url.as_str()).collect();
        assert!(urls.contains(&"https://example.com"));
        assert!(urls.contains(&"https://spec.commonmark.org"));
        assert!(urls.contains(&"https://zed.dev"));
        assert!(!urls.iter().any(|u| u.contains("pic.png")));
    }

    #[test]
    fn merge_blocks_concat() {
        assert_eq!(merge_blocks("", "b"), "b");
        assert_eq!(merge_blocks("a", ""), "a");
        assert_eq!(merge_blocks("hello", "world"), "helloworld");
        assert_eq!(merge_blocks("hello\n", "world"), "helloworld");
        assert_eq!(merge_caret("hello", "world"), 5);
        assert_eq!(merge_caret("", "world"), 0);
        assert_eq!(merge_caret("ab\n", "c"), 2);
    }

    #[test]
    fn join_line_strips_one_newline() {
        let (out, caret) = join_line("hello", "## Quota and hsh");
        assert_eq!(out, "hello## Quota and hsh");
        assert_eq!(caret, 5);
        let (out, caret) = join_line("hello\n", "## Quota and hsh");
        assert_eq!(out, "hello## Quota and hsh");
        assert_eq!(caret, 5);
        let (out, caret) = join_line("hello\n\n", "world");
        assert_eq!(out, "hello\nworld");
        assert_eq!(caret, 6);
        let (out, caret) = join_line("ab\r\n", "cd");
        assert_eq!(out, "abcd");
        assert_eq!(caret, 2);
    }

    #[test]
    fn join_line_at_deletes_newline() {
        let (out, caret) = join_line_at("aa\nbb", 3).unwrap();
        assert_eq!(out, "aabb");
        assert_eq!(caret, 2);
        let (out, caret) = join_line_at("## Quota and hsh\nnext", 17).unwrap();
        assert_eq!(out, "## Quota and hshnext");
        assert_eq!(caret, 16);
        assert!(join_line_at("aabb", 2).is_none());
        assert!(join_line_at("aa\nbb", 0).is_none());
        let (out, caret) = join_line_at("aa\r\nbb", 4).unwrap();
        assert_eq!(out, "aabb");
        assert_eq!(caret, 2);
    }

    #[test]
    fn delete_block_picks_next_or_prev() {
        assert_eq!(delete_block_index(0, 0), None);
        assert_eq!(delete_block_index(1, 0), Some(0));
        assert_eq!(delete_block_index(3, 0), Some(0));
        assert_eq!(delete_block_index(3, 1), Some(1));
        assert_eq!(delete_block_index(3, 2), Some(1));
        assert_eq!(delete_block_index(2, 1), Some(0));
    }

    #[test]
    fn parse_ranges_cover_source() {
        let src = "# H\n\npara";
        let ranges = parse_ranges(src);
        assert_eq!(ranges.first().unwrap().range.start, 0);
        assert_eq!(ranges.last().unwrap().range.end, src.len());
        let kinds: Vec<_> = ranges.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&BlockKind::Heading(1)), "{kinds:?}");
        assert!(kinds.contains(&BlockKind::Paragraph), "{kinds:?}");
        let joined: String = ranges.iter().map(|r| r.slice(src)).collect();
        assert_eq!(joined, src);
    }

    #[test]
    fn raw_span_merges_adjacent_selection() {
        let src = "# H\n\npara";
        let ranges = parse_ranges(src);
        let heading = ranges
            .iter()
            .find(|r| r.kind == BlockKind::Heading(1))
            .unwrap();
        let para = ranges
            .iter()
            .find(|r| r.kind == BlockKind::Paragraph)
            .unwrap();
        let only = raw_span(&ranges, src.len(), heading.range.start, None);
        assert_eq!(only, heading.range);
        let span = raw_span(
            &ranges,
            src.len(),
            heading.range.start,
            Some(heading.range.start..para.range.start + 1),
        );
        assert_eq!(span.start, heading.range.start);
        assert_eq!(span.end, para.range.end);
        assert!(span.end > heading.range.end);
    }

    #[test]
    fn splice_replaces_range() {
        assert_eq!(splice("hello", 1..4, "i"), "hio");
        assert_eq!(splice("# H\n\npara", 0..3, "## X"), "## X\n\npara");
    }

    #[test]
    fn slash_query_at_line_start() {
        assert_eq!(slash_query_at("/head", 1), Some("head"));
        assert_eq!(slash_query_at("para\n/h1", 6), Some("h1"));
        assert_eq!(slash_query_at("not /a slash", 4), None);
        assert_eq!(slash_query_at("hello", 0), None);
    }
}
