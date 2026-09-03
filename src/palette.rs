//! Command palette (cmd-k / ctrl-k).

use crate::config::EditorKind;
use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteMode {
    Root,
    Themes,
    Editors,
}

#[derive(Clone, Debug)]
pub struct PaletteState {
    pub query: String,
    pub index: usize,
    pub mode: PaletteMode,
}

impl PaletteState {
    pub fn open() -> Self {
        Self {
            query: String::new(),
            index: 0,
            mode: PaletteMode::Root,
        }
    }

    pub fn set_mode(&mut self, mode: PaletteMode) {
        self.mode = mode;
        self.query.clear();
        self.index = 0;
    }

    pub fn move_by(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.index = 0;
            return;
        }
        let next = self.index as isize + delta;
        self.index = next.rem_euclid(len as isize) as usize;
    }

    pub fn items(&self, view_source: bool) -> Vec<PaletteItem> {
        filter_items(&items_for(self.mode, view_source), &self.query)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    OpenThemes,
    OpenEditors,
    ToggleFullWidth,
    ToggleSource,
    OpenSettings,
    SetTheme(&'static str),
    SetEditor(EditorKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    pub label: &'static str,
    pub hint: &'static str,
    pub action: PaletteAction,
}

pub fn items_for(mode: PaletteMode, view_source: bool) -> Vec<PaletteItem> {
    match mode {
        PaletteMode::Root => root_commands(view_source),
        PaletteMode::Themes => theme_commands(),
        PaletteMode::Editors => editor_commands(),
    }
}

pub fn root_commands(view_source: bool) -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            label: "Change Theme…",
            hint: "appearance",
            action: PaletteAction::OpenThemes,
        },
        PaletteItem {
            label: "Change Editor…",
            hint: "helix / vim / notion",
            action: PaletteAction::OpenEditors,
        },
        PaletteItem {
            label: "Toggle Full Width",
            hint: "column",
            action: PaletteAction::ToggleFullWidth,
        },
        PaletteItem {
            label: if view_source {
                "Show Rendered"
            } else {
                "Show Markdown Source"
            },
            hint: "source",
            action: PaletteAction::ToggleSource,
        },
        PaletteItem {
            label: "Settings",
            hint: "preferences",
            action: PaletteAction::OpenSettings,
        },
    ]
}

pub fn theme_commands() -> Vec<PaletteItem> {
    theme::list_theme_names()
        .into_iter()
        .map(|name| PaletteItem {
            label: name,
            hint: "theme",
            action: PaletteAction::SetTheme(name),
        })
        .collect()
}

pub fn editor_commands() -> Vec<PaletteItem> {
    [
        (EditorKind::Helix, "Helix", "modal"),
        (EditorKind::Vim, "Vim", "modal"),
        (EditorKind::Notion, "Notion", "wysiwyg"),
    ]
    .into_iter()
    .map(|(kind, label, hint)| PaletteItem {
        label,
        hint,
        action: PaletteAction::SetEditor(kind),
    })
    .collect()
}

pub fn filter_items(items: &[PaletteItem], query: &str) -> Vec<PaletteItem> {
    let q = query.trim();
    if q.is_empty() {
        return items.to_vec();
    }
    let q = q.to_ascii_lowercase();
    items
        .iter()
        .copied()
        .filter(|item| {
            item.label.to_ascii_lowercase().contains(&q)
                || item.hint.to_ascii_lowercase().contains(&q)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_is_case_insensitive() {
        let items = root_commands(false);
        let hit = filter_items(&items, "THEME");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].label, "Change Theme…");
        let src = filter_items(&items, "source");
        assert_eq!(src[0].action, PaletteAction::ToggleSource);
        assert!(filter_items(&items, "zzzz").is_empty());
    }

    #[test]
    fn source_label_flips() {
        assert_eq!(root_commands(false)[3].label, "Show Markdown Source");
        assert_eq!(root_commands(true)[3].label, "Show Rendered");
    }
}
