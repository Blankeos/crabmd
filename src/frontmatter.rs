//! YAML frontmatter (`---` block) for `.md` / `.mdx`.
//!
//! The leading `---` block parses into [`Prop`]s, which become first-class
//! `Property` document nodes (one per key) — so values get the full editor:
//! caret, insert mode, motions, undo, search. [`quote_if_needed`] serializes
//! values back; [`plain_inlines`] builds the initial inline run.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prop {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub props: Vec<Prop>,
}

impl Frontmatter {
    pub fn is_empty(&self) -> bool {
        self.props.is_empty()
    }
}

/// Split leading `---\n…\n---` off `src`. Returns `(body_start, inner_yaml)`.
pub fn split(src: &str) -> Option<(usize, &str)> {
    let mut lines = src.split_inclusive('\n').peekable();
    let first = lines.peek()?.trim_end_matches(['\n', '\r']).trim();
    if first != "---" {
        return None;
    }
    let mut inner_start = 0usize;
    let mut first_line = true;
    let mut pos = 0usize;
    for line in src.split_inclusive('\n') {
        let len = line.len();
        if first_line {
            inner_start = len;
            pos = len;
            first_line = false;
            continue;
        }
        let trimmed = line.trim_end_matches(['\n', '\r']).trim();
        if trimmed == "---" || trimmed == "..." {
            let end = pos + len;
            return Some((end, &src[inner_start..pos]));
        }
        pos += len;
    }
    None
}

pub fn parse(src: &str) -> Option<(Frontmatter, usize)> {
    let (body_start, inner) = split(src)?;
    Some((parse_inner(inner), body_start))
}

fn parse_inner(inner: &str) -> Frontmatter {
    let mut props: Vec<Prop> = Vec::new();
    let mut pending_key: Option<String> = None;
    let mut pending_items: Vec<String> = Vec::new();

    let flush = |props: &mut Vec<Prop>, key: String, items: Vec<String>| {
        let value = items.join(", ");
        props.push(Prop { key, value });
    };

    for raw_line in inner.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        // `- item` continuation of a `key:` list.
        let stripped = line.trim_start();
        if stripped.starts_with("- ") || stripped == "-" {
            if pending_key.is_some() {
                let item = stripped
                    .strip_prefix("- ")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                pending_items.push(item);
            }
            continue;
        }
        // `key: value`
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim();
            if key.is_empty() || key.contains(' ') && !key.starts_with('"') {
                // Not a key line (e.g. free text) — ignore.
                continue;
            }
            let key = key.trim_matches('"').trim_matches('\'').to_string();
            if key.is_empty() {
                continue;
            }
            if let Some(pk) = pending_key.take() {
                flush(&mut props, pk, std::mem::take(&mut pending_items));
            }
            let mut value = line[colon + 1..].trim().to_string();
            // Strip matching quotes.
            if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                value = value[1..value.len() - 1].to_string();
            }
            if value.is_empty() || value == "|" || value == ">" {
                // Possibly a block list / literal follows.
                pending_key = Some(key);
            } else {
                props.push(Prop { key, value });
            }
        } else if pending_key.is_some() {
            // Indented continuation (`|` literal) — property values are
            // single-line, so fold onto one line with spaces.
            if let Some(last) = pending_items.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            } else {
                pending_items.push(line.trim().to_string());
            }
        }
    }
    if let Some(pk) = pending_key.take() {
        flush(&mut props, pk, pending_items);
    }
    Frontmatter { props }
}

/// One `key: value` document line for a parsed prop. Shared by
/// `Doc::from_gfm` so parse → node stays symmetric with node → YAML.
pub fn line_for_prop(key: &str, value: &str) -> String {
    if value.is_empty() {
        format!("{key}: ")
    } else {
        format!("{key}: {value}")
    }
}

/// Normalize one edited `Property` row back to YAML. The row is the full
/// `key: value` line as plain text: re-split on the first colon, quote the
/// value, and keep no-colon rows as bare keys so they survive a reload.
pub fn row_to_yaml(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "untitled: ".to_string();
    }
    if let Some(colon) = trimmed.find(':') {
        let key = trimmed[..colon].trim();
        let value = trimmed[colon + 1..].trim();
        if key.is_empty() {
            trimmed.to_string()
        } else {
            format!("{key}: {}", quote_if_needed(value))
        }
    } else {
        format!("{trimmed}: ")
    }
}

/// Leading `Property` rows → `---\n…\n---\n` block.
pub fn serialize_block(rows: &[String]) -> String {
    let mut out = String::from("---\n");
    for r in rows {
        out.push_str(&row_to_yaml(r));
        out.push('\n');
    }
    out.push_str("---\n");
    out
}

/// Quote a scalar when YAML would otherwise misread it.
pub fn quote_if_needed(v: &str) -> String {
    // Property values are single-line (newlines folded on import).
    let v = v.replace('\n', " ");
    if v.is_empty() {
        return String::new();
    }
    let needs = v.contains(": ")
        || v.contains(" #")
        || v.starts_with(['#', '-', '?', ':', '*', '&', '!', '|', '>', '%', '@', '`', '{', '}', '[', ']', ','])
        || v != v.trim();
    if needs {
        format!("\"{}\"", v.replace('"', "\\\""))
    } else {
        v
    }
}

/// Initial inline run for a parsed value: plain text, no marks.
pub fn plain_inlines(value: &str) -> Vec<crate::tree::Inline> {
    let flat = value.replace(['\n', '\r'], " ");
    if flat.is_empty() {
        Vec::new()
    } else {
        vec![crate::tree::Inline {
            text: flat,
            marks: crate::display::Marks::default(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_leading_block() {
        let src = "---\ntitle: Hi\n---\n\n# Body\n";
        let (start, inner) = split(src).unwrap();
        assert!(inner.contains("title: Hi"));
        assert!(src[start..].contains("# Body"));
    }

    #[test]
    fn no_frontmatter() {
        assert!(split("# Just a doc\n").is_none());
        assert!(split("---\nno close").is_none());
    }

    #[test]
    fn parses_example() {
        let src = "---\ntitle: Terminal Keymaps\ndescription: Line jumps\n---\n\n# Hi\n";
        let (fm, _) = parse(src).unwrap();
        assert_eq!(fm.props.len(), 2);
        assert_eq!(fm.props[0].key, "title");
        assert_eq!(fm.props[1].value, "Line jumps");
    }

    #[test]
    fn roundtrip_block_list() {
        let src = "---\ntags:\n  - a\n  - b\n---\n\nHi\n";
        let (fm, _) = parse(src).unwrap();
        assert_eq!(fm.props[0].value, "a, b");
    }

    #[test]
    fn hr_not_frontmatter() {
        // `---` later in the doc is a rule, not frontmatter.
        assert!(split("# T\n\n---\n\ntext\n").is_none());
    }

    #[test]
    fn folds_literal_newlines() {
        let src = "---\ndesc: |\n  line one\n  line two\n---\n\nHi\n";
        let (fm, _) = parse(src).unwrap();
        assert_eq!(fm.props[0].value, "line one line two");
    }
}
