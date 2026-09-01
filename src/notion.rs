//! Notion-mode content: edit visible text, commit back to GFM.

use crate::document::{AlertKind, Block, BlockKind};

/// Text shown in the Notion editor for a block (no GFM fence / prefix).
pub fn edit_text(block: &Block) -> String {
    match block.kind {
        BlockKind::Heading(_) => strip_heading(&block.source).1,
        BlockKind::Quote => strip_quote(&block.source),
        BlockKind::Alert(_) => strip_alert_body(&block.source),
        BlockKind::Code => strip_fence(&block.source).1,
        BlockKind::List { .. } => strip_list(&block.source),
        BlockKind::Rule => String::new(),
        BlockKind::Paragraph | BlockKind::Table | BlockKind::Html | BlockKind::Raw => {
            block.source.clone()
        }
    }
}

/// Rebuild GFM from Notion-edited content, preserving kind / language / checks.
pub fn commit(block: &Block, edit: &str) -> String {
    match block.kind {
        BlockKind::Heading(level) => wrap_heading(level, edit),
        BlockKind::Quote => wrap_quote(edit),
        BlockKind::Alert(kind) => wrap_alert(kind, edit),
        BlockKind::Code => {
            let (lang, _) = strip_fence(&block.source);
            wrap_fence(&lang, edit)
        }
        BlockKind::List { ordered } => wrap_list(ordered, edit, &block.source),
        BlockKind::Rule => block.source.clone(),
        BlockKind::Paragraph | BlockKind::Table | BlockKind::Html | BlockKind::Raw => {
            edit.to_string()
        }
    }
}

pub fn uses_raw_exception(kind: BlockKind) -> bool {
    matches!(kind, BlockKind::Table | BlockKind::Html | BlockKind::Raw)
}

pub fn strip_heading(source: &str) -> (u8, String) {
    let line = source.trim_start_matches('\n');
    let hashes = line.chars().take_while(|c| *c == '#').count();
    let level = hashes.clamp(1, 6) as u8;
    let rest = line.get(hashes..).unwrap_or("");
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    (level, rest.to_string())
}

pub fn wrap_heading(level: u8, text: &str) -> String {
    let n = level.clamp(1, 6) as usize;
    format!("{} {}", "#".repeat(n), text.trim_start())
}

pub fn strip_quote(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            line.strip_prefix("> ")
                .or_else(|| line.strip_prefix('>'))
                .unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn wrap_quote(text: &str) -> String {
    if text.is_empty() {
        return "> ".into();
    }
    text.lines()
        .map(|l| format!("> {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn strip_alert_body(source: &str) -> String {
    let mut lines = source.lines().peekable();
    if lines
        .peek()
        .is_some_and(|l| l.contains("[!") && l.contains(']'))
    {
        lines.next();
    }
    lines
        .map(|line| {
            line.strip_prefix("> ")
                .or_else(|| line.strip_prefix('>'))
                .unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn wrap_alert(kind: AlertKind, body: &str) -> String {
    let mut out = format!("> [!{}]", kind.as_str());
    if body.is_empty() {
        out.push_str("\n> ");
        return out;
    }
    for line in body.lines() {
        out.push_str("\n> ");
        out.push_str(line);
    }
    out
}

pub fn strip_fence(source: &str) -> (String, String) {
    let t = source.trim();
    let rest = t.strip_prefix("```").unwrap_or(t);
    let mut lines = rest.lines();
    let first = lines.next().unwrap_or("");
    let lang = first.trim().to_string();
    let mut body: Vec<&str> = lines.collect();
    if body.last().is_some_and(|l| l.trim() == "```") {
        body.pop();
    }
    (lang, body.join("\n"))
}

pub fn wrap_fence(lang: &str, body: &str) -> String {
    if lang.is_empty() {
        format!("```\n{body}\n```")
    } else {
        format!("```{lang}\n{body}\n```")
    }
}

fn strip_list_marker(line: &str) -> &str {
    let t = line.trim_start();
    let after_ul = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .or_else(|| t.strip_prefix("+ "));
    if let Some(rest) = after_ul {
        return strip_task_box(rest);
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let rest = &t[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return strip_task_box(rest);
        }
    }
    t
}

fn strip_task_box(s: &str) -> &str {
    s.strip_prefix("[ ] ")
        .or_else(|| s.strip_prefix("[x] "))
        .or_else(|| s.strip_prefix("[X] "))
        .or_else(|| s.strip_prefix("[ ]"))
        .or_else(|| s.strip_prefix("[x]"))
        .or_else(|| s.strip_prefix("[X]"))
        .unwrap_or(s)
}

pub fn strip_list(source: &str) -> String {
    source
        .lines()
        .map(strip_list_marker)
        .collect::<Vec<_>>()
        .join("\n")
}

fn original_line_prefix(line: &str) -> Option<String> {
    if let Some((_, checked, _)) = crate::document::split_task_line(line) {
        let mark = if checked { "x" } else { " " };
        let indent = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect::<String>();
        return Some(format!("{indent}- [{mark}] "));
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    let after = &line[i..];
    if after.starts_with("- ") || after.starts_with("* ") || after.starts_with("+ ") {
        return Some(line[..i + 2].to_string());
    }
    let digits = after.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let rest = &after[digits..];
        if rest.starts_with(". ") || rest.starts_with(") ") {
            return Some(line[..i + digits + 2].to_string());
        }
    }
    None
}

pub fn wrap_list(ordered: bool, plain: &str, original: &str) -> String {
    let originals: Vec<&str> = original.lines().collect();
    plain
        .lines()
        .enumerate()
        .map(|(i, text)| {
            if let Some(orig) = originals.get(i).and_then(|l| original_line_prefix(l)) {
                format!("{orig}{text}")
            } else if ordered {
                format!("{}. {text}", i + 1)
            } else {
                format!("- {text}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_round_trip() {
        let block = Block::with_kind(BlockKind::Heading(2), "## Hello");
        assert_eq!(edit_text(&block), "Hello");
        assert_eq!(commit(&block, "World"), "## World");
        let (lvl, text) = strip_heading("# Title");
        assert_eq!(lvl, 1);
        assert_eq!(text, "Title");
        assert_eq!(wrap_heading(1, "Title"), "# Title");
    }

    #[test]
    fn paragraph_round_trip() {
        let block = Block::paragraph("hello **there**");
        assert_eq!(edit_text(&block), "hello **there**");
        assert_eq!(commit(&block, "new"), "new");
    }

    #[test]
    fn list_round_trip() {
        let src = "- [ ] open\n- [x] done";
        let block = Block::with_kind(BlockKind::List { ordered: false }, src);
        assert_eq!(edit_text(&block), "open\ndone");
        let out = commit(&block, "open\ndone");
        assert!(out.contains("- [ ] open"));
        assert!(out.contains("- [x] done"));
        let ol = Block::with_kind(BlockKind::List { ordered: true }, "1. one\n2. two");
        assert_eq!(edit_text(&ol), "one\ntwo");
        let out = commit(&ol, "one\ntwo");
        assert!(out.contains("1. one"));
        assert!(out.contains("2. two"));
    }

    #[test]
    fn quote_code_alert_round_trip() {
        let q = Block::with_kind(BlockKind::Quote, "> said");
        assert_eq!(edit_text(&q), "said");
        assert_eq!(commit(&q, "said"), "> said");
        let c = Block::with_kind(BlockKind::Code, "```rust\nfn main() {}\n```");
        assert_eq!(edit_text(&c), "fn main() {}");
        assert_eq!(commit(&c, "fn main() {}"), "```rust\nfn main() {}\n```");
        let a = Block::with_kind(BlockKind::Alert(AlertKind::Note), "> [!NOTE]\n> body");
        assert_eq!(edit_text(&a), "body");
        let out = commit(&a, "body");
        assert!(out.contains("[!NOTE]"));
        assert!(out.contains("> body"));
    }
}
