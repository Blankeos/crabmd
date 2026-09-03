//! Command palette (cmd-shift-p / ctrl-shift-p).

use gpui::Modifiers;

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
    pub query: LineField,
    pub index: usize,
    pub mode: PaletteMode,
}

impl PaletteState {
    pub fn open() -> Self {
        Self {
            query: LineField::new(),
            index: 0,
            mode: PaletteMode::Root,
        }
    }

    /// Open straight into a submenu (e.g. Themes from settings).
    pub fn open_in(mode: PaletteMode) -> Self {
        Self {
            query: LineField::new(),
            index: 0,
            mode,
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
        filter_items(&items_for(self.mode, view_source), self.query.as_str())
    }
}

/// Single-line buffer with a real caret (not append-at-end).
///
/// Shared by the palette query, `:` command bar, `/`+`cmd-f` search bars and
/// the link draft so opt/cmd-backspace and opt/cmd-arrows edit where the
/// caret is instead of always at the end.
#[derive(Clone, Debug, Default)]
pub struct LineField {
    text: String,
    caret: usize,
}

impl LineField {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
    }

    fn clamp_caret(&mut self) {
        if self.caret > self.text.len() {
            self.caret = self.text.len();
        }
        while !self.text.is_char_boundary(self.caret) && self.caret > 0 {
            self.caret -= 1;
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        self.clamp_caret();
        self.text.insert(self.caret, ch);
        self.caret += ch.len_utf8();
    }

    pub fn insert_str(&mut self, s: &str) {
        self.clamp_caret();
        self.text.insert_str(self.caret, s);
        self.caret += s.len();
    }

    /// Plain backspace. False when empty so callers can fall through
    /// (e.g. palette backs out of submenus).
    pub fn backspace(&mut self) -> bool {
        self.clamp_caret();
        if self.caret == 0 {
            return false;
        }
        let prev = self.text[..self.caret]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.text.drain(prev..self.caret);
        self.caret = prev;
        true
    }

    /// Opt-backspace: delete the word before the caret.
    pub fn delete_word_back(&mut self) -> bool {
        self.clamp_caret();
        if self.caret == 0 {
            return false;
        }
        let bytes = self.text.as_bytes();
        let mut start = self.caret;
        while start > 0 && bytes[start - 1] == b' ' {
            start -= 1;
        }
        while start > 0 && bytes[start - 1] != b' ' {
            let prev = self.text[..start]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            start = prev;
        }
        if start == self.caret {
            return false;
        }
        self.text.drain(start..self.caret);
        self.caret = start;
        true
    }

    /// Cmd-backspace: delete everything before the caret (whole line when
    /// the caret is at the end).
    pub fn clear_back(&mut self) -> bool {
        self.clamp_caret();
        if self.caret == 0 {
            return false;
        }
        self.text.drain(..self.caret);
        self.caret = 0;
        true
    }

    pub fn move_left(&mut self) {
        self.clamp_caret();
        if self.caret > 0 {
            self.caret = self.text[..self.caret]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn move_right(&mut self) {
        self.clamp_caret();
        if self.caret < self.text.len() {
            self.caret = self.text[self.caret..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.caret + i)
                .unwrap_or(self.text.len());
        }
    }

    fn word_left_from(&self, mut ix: usize) -> usize {
        let bytes = self.text.as_bytes();
        while ix > 0 && bytes[ix - 1] == b' ' {
            ix -= 1;
        }
        while ix > 0 && bytes[ix - 1] != b' ' {
            ix -= 1;
        }
        while !self.text.is_char_boundary(ix) && ix > 0 {
            ix -= 1;
        }
        ix
    }

    fn word_right_from(&self, mut ix: usize) -> usize {
        let bytes = self.text.as_bytes();
        let len = self.text.len();
        while ix < len && bytes[ix] == b' ' {
            ix += 1;
        }
        while ix < len && bytes[ix] != b' ' {
            ix += 1;
        }
        while !self.text.is_char_boundary(ix) && ix < len {
            ix += 1;
        }
        ix
    }

    pub fn move_word_left(&mut self) {
        self.clamp_caret();
        self.caret = self.word_left_from(self.caret);
    }

    pub fn move_word_right(&mut self) {
        self.clamp_caret();
        self.caret = self.word_right_from(self.caret);
    }

    pub fn home(&mut self) {
        self.caret = 0;
    }

    pub fn end(&mut self) {
        self.caret = self.text.len();
    }

    /// Arrows / home / end with optional opt (word) / cmd (line) motion.
    /// Always consumed: there is nowhere else for them to go.
    pub fn caret_key(&mut self, key: &str, mods: Modifiers) -> bool {
        let word = mods.alt || (mods.control && !mods.platform);
        match key {
            "left" if word => {
                self.move_word_left();
                true
            }
            "right" if word => {
                self.move_word_right();
                true
            }
            "left" if mods.platform => {
                self.home();
                true
            }
            "right" if mods.platform => {
                self.end();
                true
            }
            "left" => {
                self.move_left();
                true
            }
            "right" => {
                self.move_right();
                true
            }
            "home" => {
                self.home();
                true
            }
            "end" => {
                self.end();
                true
            }
            _ => false,
        }
    }

    /// Backspace family (plain / opt-word / cmd-line). False when nothing
    /// was deleted so callers can fall through.
    pub fn delete_key(&mut self, key: &str, mods: Modifiers) -> bool {
        if key != "backspace" {
            return false;
        }
        if mods.platform && !mods.alt && !mods.control {
            if self.is_empty() {
                return false;
            }
            self.clear_back();
            // Cmd-backspace at line start clears nothing but still consumes.
            true
        } else if mods.alt && !mods.platform && !mods.control {
            self.delete_word_back()
        } else if !mods.platform && !mods.control && !mods.alt {
            self.backspace()
        } else {
            false
        }
    }

    /// Render with a visible caret (`▌`) at the caret, not the end.
    pub fn render(&self) -> String {
        let mut caret = self.caret.min(self.text.len());
        while !self.text.is_char_boundary(caret) && caret > 0 {
            caret -= 1;
        }
        format!("{}▌{}", &self.text[..caret], &self.text[caret..])
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
            hint: theme::appearance_hint(name),
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
