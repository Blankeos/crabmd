//! Lightweight token highlighter for fenced code blocks.

use std::ops::Range;

use gpui::HighlightStyle;

use crate::theme::Palette;

#[derive(Clone, Copy)]
enum Kind {
    Keyword,
    String,
    Comment,
    Number,
    Function,
}

pub fn highlights(lang: &str, text: &str, p: &Palette) -> Vec<(Range<usize>, HighlightStyle)> {
    if text.is_empty() {
        return Vec::new();
    }
    let lang = normalize_lang(lang);
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < text.len() {
        if let Some((end, kind)) = scan_token(lang, text, bytes, i) {
            if end > i {
                out.push((i..end, style(kind, p)));
                i = end;
                continue;
            }
        }
        i = text[i..]
            .chars()
            .next()
            .map(|c| i + c.len_utf8())
            .unwrap_or(text.len());
    }
    out
}

fn normalize_lang(lang: &str) -> &str {
    match lang.trim() {
        "js" | "javascript" | "jsx" | "mjs" | "cjs" | "ts" | "typescript" | "tsx" => "javascript",
        "rs" | "rust" => "rust",
        "py" | "python" => "python",
        "rb" | "ruby" => "ruby",
        "sh" | "bash" | "zsh" | "shell" => "bash",
        "yml" | "yaml" => "yaml",
        "c++" | "cc" | "h" | "hpp" | "cpp" => "cpp",
        "cs" | "csharp" => "csharp",
        "md" | "markdown" => "markdown",
        other => other,
    }
}

fn style(kind: Kind, p: &Palette) -> HighlightStyle {
    let color = match kind {
        Kind::Keyword => p.accent,
        Kind::String => p.success,
        Kind::Comment => p.text_muted,
        Kind::Number => p.warning,
        Kind::Function => p.info,
    };
    HighlightStyle {
        color: Some(color),
        ..Default::default()
    }
}

fn scan_token(lang: &str, text: &str, bytes: &[u8], i: usize) -> Option<(usize, Kind)> {
    let c = *bytes.get(i)? as char;
    if lang == "html" || lang == "xml" {
        if text[i..].starts_with("<!--") {
            let end = text[i + 4..]
                .find("-->")
                .map(|n| i + 4 + n + 3)
                .unwrap_or(text.len());
            return Some((end, Kind::Comment));
        }
        if c == '<' {
            let end = text[i..].find('>').map(|n| i + n + 1).unwrap_or(text.len());
            return Some((end, Kind::Keyword));
        }
    }
    if matches!(
        lang,
        "css" | "javascript" | "rust" | "go" | "c" | "cpp" | "csharp" | "json"
    ) && text[i..].starts_with("//")
    {
        return Some((eol(text, i), Kind::Comment));
    }
    if matches!(
        lang,
        "css" | "javascript" | "rust" | "go" | "c" | "cpp" | "csharp" | "sql"
    ) && text[i..].starts_with("/*")
    {
        let end = text[i + 2..]
            .find("*/")
            .map(|n| i + 2 + n + 2)
            .unwrap_or(text.len());
        return Some((end, Kind::Comment));
    }
    if matches!(lang, "python" | "bash" | "yaml" | "toml" | "ruby") && c == '#' {
        return Some((eol(text, i), Kind::Comment));
    }
    if lang == "python" && (text[i..].starts_with("\"\"\"") || text[i..].starts_with("'''")) {
        let q = &text[i..i + 3];
        let end = text[i + 3..]
            .find(q)
            .map(|n| i + 3 + n + 3)
            .unwrap_or(text.len());
        return Some((end, Kind::String));
    }
    if c == '"' || c == '\'' || (c == '`' && lang == "javascript") {
        return Some((scan_string(text, i, c), Kind::String));
    }
    if c.is_ascii_digit() {
        let mut j = i + 1;
        while j < text.len() {
            let ch = text[j..].chars().next()?;
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' {
                j += ch.len_utf8();
            } else {
                break;
            }
        }
        return Some((j, Kind::Number));
    }
    if c == '_' || c.is_ascii_alphabetic() {
        let mut j = i + 1;
        while j < text.len() {
            let ch = text[j..].chars().next()?;
            if ch.is_ascii_alphanumeric() || ch == '_' {
                j += ch.len_utf8();
            } else {
                break;
            }
        }
        let word = &text[i..j];
        if is_keyword(lang, word) {
            return Some((j, Kind::Keyword));
        }
        let rest = text[j..].trim_start();
        if rest.starts_with('(') {
            return Some((j, Kind::Function));
        }
        return None;
    }
    None
}

fn scan_string(text: &str, start: usize, quote: char) -> usize {
    let j = start + quote.len_utf8();
    let mut esc = false;
    for (off, ch) in text[j..].char_indices() {
        if esc {
            esc = false;
            continue;
        }
        if ch == '\\' {
            esc = true;
            continue;
        }
        if ch == quote {
            return j + off + ch.len_utf8();
        }
        if ch == '\n' && quote != '`' {
            return j + off;
        }
    }
    text.len()
}

fn eol(text: &str, i: usize) -> usize {
    text[i..].find('\n').map(|n| i + n).unwrap_or(text.len())
}

fn is_keyword(lang: &str, word: &str) -> bool {
    match lang {
        "rust" => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        "python" => matches!(
            word,
            "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "False"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "None"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "True"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        "javascript" => matches!(
            word,
            "async"
                | "await"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "debugger"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "function"
                | "if"
                | "import"
                | "in"
                | "instanceof"
                | "let"
                | "new"
                | "null"
                | "return"
                | "static"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typeof"
                | "var"
                | "void"
                | "while"
                | "yield"
                | "of"
        ),
        "go" => matches!(
            word,
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "type"
                | "var"
        ),
        "bash" => matches!(
            word,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "for"
                | "in"
                | "do"
                | "done"
                | "case"
                | "esac"
                | "while"
                | "until"
                | "function"
                | "return"
                | "local"
                | "export"
                | "unset"
        ),
        "sql" => matches!(
            word,
            "SELECT"
                | "FROM"
                | "WHERE"
                | "AND"
                | "OR"
                | "INSERT"
                | "INTO"
                | "VALUES"
                | "UPDATE"
                | "SET"
                | "DELETE"
                | "CREATE"
                | "TABLE"
                | "JOIN"
                | "LEFT"
                | "RIGHT"
                | "INNER"
                | "ON"
                | "AS"
                | "NULL"
                | "NOT"
                | "ORDER"
                | "BY"
                | "GROUP"
                | "LIMIT"
                | "select"
                | "from"
                | "where"
                | "and"
                | "or"
                | "insert"
                | "into"
                | "values"
                | "update"
                | "set"
                | "delete"
                | "create"
                | "table"
                | "join"
                | "left"
                | "right"
                | "inner"
                | "on"
                | "as"
                | "null"
                | "not"
                | "order"
                | "by"
                | "group"
                | "limit"
        ),
        "toml" | "yaml" => matches!(word, "true" | "false" | "null" | "yes" | "no"),
        "json" => matches!(word, "true" | "false" | "null"),
        "css" => matches!(
            word,
            "important" | "from" | "to" | "and" | "or" | "not" | "only"
        ),
        "c" | "cpp" | "csharp" => matches!(
            word,
            "auto"
                | "break"
                | "case"
                | "char"
                | "const"
                | "continue"
                | "default"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "extern"
                | "float"
                | "for"
                | "goto"
                | "if"
                | "int"
                | "long"
                | "register"
                | "return"
                | "short"
                | "signed"
                | "sizeof"
                | "static"
                | "struct"
                | "switch"
                | "typedef"
                | "union"
                | "unsigned"
                | "void"
                | "volatile"
                | "while"
                | "class"
                | "namespace"
                | "new"
                | "delete"
                | "this"
                | "public"
                | "private"
                | "protected"
                | "virtual"
                | "bool"
                | "true"
                | "false"
                | "using"
                | "template"
                | "typename"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    #[test]
    fn rust_keywords_and_strings() {
        let p = theme::load_named("opencode").unwrap();
        let hs = highlights("rust", "fn main() { let x = \"hi\"; }", &p);
        assert!(hs
            .iter()
            .any(|(r, _)| &"fn main() { let x = \"hi\"; }"[r.clone()] == "fn"));
        assert!(hs
            .iter()
            .any(|(r, _)| &"fn main() { let x = \"hi\"; }"[r.clone()] == "\"hi\""));
    }
}
