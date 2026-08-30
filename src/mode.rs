//! Modal layer (Helix / Vim / Notion). No GPUI types.

use crate::config::EditorKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Select,
    Visual,
    VisualLine,
}

impl Mode {
    pub fn status(self, editor: EditorKind) -> &'static str {
        if editor == EditorKind::Notion {
            return "NOTION";
        }
        match self {
            Self::Normal => "NOR",
            Self::Insert => "INS",
            Self::Select => "SEL",
            Self::Visual => "VIS",
            Self::VisualLine => "V-LINE",
        }
    }

    pub fn is_insert(self) -> bool {
        matches!(self, Self::Insert)
    }

    pub fn is_visual(self) -> bool {
        matches!(self, Self::Select | Self::Visual | Self::VisualLine)
    }

    pub fn extends_selection(self) -> bool {
        self.is_visual()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caret {
    Start,
    End,
    Offset(usize),
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExCommand {
    Write,
    WriteQuit,
    Cancel,
    Unknown(String),
}

/// Parse a `:` command-line body (no leading colon).
pub fn parse_ex(input: &str) -> ExCommand {
    let t = input.trim();
    if t.is_empty() {
        return ExCommand::Cancel;
    }
    let cmd = t.split_whitespace().next().unwrap_or(t);
    match cmd {
        "w" | "write" => ExCommand::Write,
        "wq" | "x" | "xit" => ExCommand::WriteQuit,
        _ => ExCommand::Unknown(t.to_string()),
    }
}

pub fn slash_move(index: usize, len: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let next = index as i32 + delta;
    if next < 0 {
        len - 1
    } else if next as usize >= len {
        0
    } else {
        next as usize
    }
}

pub fn clamp_index(index: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        index.min(len - 1)
    }
}

/// Blocks that should not grow via `o`/`O` (a new paragraph is inserted instead).
pub fn block_should_not_grow(kind: crate::document::BlockKind) -> bool {
    use crate::document::BlockKind::*;
    matches!(kind, Rule | Html | Raw | Table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EditorKind;
    use crate::document::BlockKind;

    #[test]
    fn status_labels() {
        assert_eq!(Mode::Normal.status(EditorKind::Helix), "NOR");
        assert_eq!(Mode::Select.status(EditorKind::Helix), "SEL");
        assert_eq!(Mode::Visual.status(EditorKind::Vim), "VIS");
        assert_eq!(Mode::VisualLine.status(EditorKind::Vim), "V-LINE");
        assert_eq!(Mode::Insert.status(EditorKind::Vim), "INS");
        assert_eq!(Mode::Normal.status(EditorKind::Notion), "NOTION");
        assert!(Mode::Select.is_visual());
        assert!(!Mode::Normal.is_visual());
    }

    #[test]
    fn slash_index_wraps() {
        assert_eq!(slash_move(0, 4, -1), 3);
        assert_eq!(slash_move(3, 4, 1), 0);
        assert_eq!(slash_move(1, 4, 1), 2);
        assert_eq!(clamp_index(9, 3), 2);
        assert_eq!(clamp_index(0, 0), 0);
    }

    #[test]
    fn rule_does_not_grow() {
        assert!(block_should_not_grow(BlockKind::Rule));
        assert!(!block_should_not_grow(BlockKind::Paragraph));
        assert!(!block_should_not_grow(BlockKind::Code));
    }

    #[test]
    fn parse_ex_commands() {
        assert_eq!(parse_ex(""), ExCommand::Cancel);
        assert_eq!(parse_ex("   "), ExCommand::Cancel);
        assert_eq!(parse_ex("w"), ExCommand::Write);
        assert_eq!(parse_ex("write"), ExCommand::Write);
        assert_eq!(parse_ex("  w  "), ExCommand::Write);
        assert_eq!(parse_ex("wq"), ExCommand::WriteQuit);
        assert_eq!(parse_ex("x"), ExCommand::WriteQuit);
        assert_eq!(parse_ex("nope"), ExCommand::Unknown("nope".into()));
        assert_eq!(parse_ex("foo bar"), ExCommand::Unknown("foo bar".into()));
    }
}
