//! Slash menu: typing `/` inserts GFM blocks, including GitHub alerts.

use crate::document::{AlertKind, Block, BlockKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashItem {
    pub keys: &'static [&'static str],
    pub label: &'static str,
    pub hint: &'static str,
    pub template: &'static str,
    pub kind: BlockKind,
    pub icon: &'static str,
}

pub const ITEMS: &[SlashItem] = &[
    SlashItem {
        keys: &["h1", "heading1", "title"],
        label: "Heading 1",
        hint: "#",
        template: "# ",
        kind: BlockKind::Heading(1),
        icon: "heading",
    },
    SlashItem {
        keys: &["h2", "heading2"],
        label: "Heading 2",
        hint: "##",
        template: "## ",
        kind: BlockKind::Heading(2),
        icon: "heading",
    },
    SlashItem {
        keys: &["h3", "heading3"],
        label: "Heading 3",
        hint: "###",
        template: "### ",
        kind: BlockKind::Heading(3),
        icon: "heading",
    },
    SlashItem {
        keys: &["ul", "list", "bullet"],
        label: "Bullet list",
        hint: "-",
        template: "- ",
        kind: BlockKind::List { ordered: false },
        icon: "list",
    },
    SlashItem {
        keys: &["ol", "numbered", "ordered"],
        label: "Numbered list",
        hint: "1.",
        template: "1. ",
        kind: BlockKind::List { ordered: true },
        icon: "list-ordered",
    },
    SlashItem {
        keys: &["todo", "task", "check"],
        label: "Task list",
        hint: "- [ ]",
        template: "- [ ] ",
        kind: BlockKind::List { ordered: false },
        icon: "list-todo",
    },
    SlashItem {
        keys: &["code", "fence"],
        label: "Code block",
        hint: "```",
        template: "```\n\n```",
        kind: BlockKind::Code,
        icon: "code",
    },
    SlashItem {
        keys: &["table"],
        label: "Table",
        hint: "| |",
        template: "| Column 1 | Column 2 | Column 3 |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |",
        kind: BlockKind::Table,
        icon: "table",
    },
    SlashItem {
        keys: &["quote", "blockquote"],
        label: "Quote",
        hint: ">",
        template: "> ",
        kind: BlockKind::Quote,
        icon: "quote",
    },
    SlashItem {
        keys: &["hr", "divider", "rule"],
        label: "Divider",
        hint: "---",
        template: "---",
        kind: BlockKind::Rule,
        icon: "minus",
    },
    SlashItem {
        keys: &["alert", "note"],
        label: "Alert: Note",
        hint: "[!NOTE]",
        template: "> [!NOTE]\n> ",
        kind: BlockKind::Alert(AlertKind::Note),
        icon: "info",
    },
    SlashItem {
        keys: &["tip"],
        label: "Alert: Tip",
        hint: "[!TIP]",
        template: "> [!TIP]\n> ",
        kind: BlockKind::Alert(AlertKind::Tip),
        icon: "lightbulb",
    },
    SlashItem {
        keys: &["important"],
        label: "Alert: Important",
        hint: "[!IMPORTANT]",
        template: "> [!IMPORTANT]\n> ",
        kind: BlockKind::Alert(AlertKind::Important),
        icon: "star",
    },
    SlashItem {
        keys: &["warning", "warn"],
        label: "Alert: Warning",
        hint: "[!WARNING]",
        template: "> [!WARNING]\n> ",
        kind: BlockKind::Alert(AlertKind::Warning),
        icon: "triangle-alert",
    },
    SlashItem {
        keys: &["caution"],
        label: "Alert: Caution",
        hint: "[!CAUTION]",
        template: "> [!CAUTION]\n> ",
        kind: BlockKind::Alert(AlertKind::Caution),
        icon: "circle-alert",
    },
];

pub fn filter(query: &str) -> Vec<&'static SlashItem> {
    let q = query.trim().to_ascii_lowercase();
    ITEMS
        .iter()
        .filter(|item| {
            if q.is_empty() {
                return true;
            }
            item.label.to_ascii_lowercase().contains(&q)
                || item.keys.iter().any(|k| k.contains(&q) || q.contains(k))
        })
        .collect()
}

pub fn to_block(item: &SlashItem) -> Block {
    Block::with_kind(item.kind, item.template)
}

pub fn selected<'a>(items: &[&'a SlashItem], index: usize) -> Option<&'a SlashItem> {
    if items.is_empty() {
        return None;
    }
    items.get(index.min(items.len() - 1)).copied()
}

pub fn clamp_index(index: usize, len: usize) -> usize {
    crate::mode::clamp_index(index, len)
}

pub fn move_index(index: usize, len: usize, delta: i32) -> usize {
    crate::mode::slash_move(index, len, delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_is_in_the_menu() {
        let hits = filter("alert");
        assert!(hits.iter().any(|i| i.label.contains("Note")));
        let note = filter("note")[0];
        assert!(note.template.contains("[!NOTE]"));
    }

    #[test]
    fn heading_query() {
        let hits = filter("h1");
        assert_eq!(hits[0].template, "# ");
        assert_eq!(hits[0].icon, "heading");
    }

    #[test]
    fn filter_and_index() {
        let items = filter("");
        assert!(items.len() > 5);
        assert_eq!(selected(&items, 0).unwrap().label, items[0].label);
        assert_eq!(
            selected(&items, 99).unwrap().label,
            items[items.len() - 1].label
        );
        let i = move_index(0, items.len(), -1);
        assert_eq!(i, items.len() - 1);
        let hits = filter("head");
        assert!(hits
            .iter()
            .all(|h| h.label.to_ascii_lowercase().contains("head")
                || h.keys.iter().any(|k| k.contains("head"))));
        assert_eq!(hits[0].icon, "heading");
    }
}
