//! User settings: editor + theme + fonts. Stored at ~/.config/crabmd/config.toml.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditorKind {
    #[default]
    Helix,
    Vim,
    Notion,
}

impl EditorKind {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Helix => "helix",
            Self::Vim => "vim",
            Self::Notion => "notion",
        }
    }

    pub fn is_modal(self) -> bool {
        !matches!(self, Self::Notion)
    }
}

/// One font face + size. Shared by UI / markdown / buffer slots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontSpec {
    pub family: String,
    pub size: u32,
}

impl FontSpec {
    pub fn new(family: impl Into<String>, size: u32) -> Self {
        Self {
            family: family.into(),
            size: size.clamp(8, 48),
        }
    }

    pub fn clamp(mut self) -> Self {
        self.size = self.size.clamp(8, 48);
        if self.family.trim().is_empty() {
            self.family = default_ui_font().family;
        }
        self
    }
}

pub fn default_ui_font() -> FontSpec {
    FontSpec::new("IBM Plex Sans", 13)
}

pub fn default_markdown_font() -> FontSpec {
    FontSpec::new("IBM Plex Sans", 15)
}

pub fn default_buffer_font() -> FontSpec {
    FontSpec::new("JetBrains Mono", 14)
}

fn default_wrap() -> bool {
    true
}

fn default_theme() -> String {
    theme::DEFAULT_THEME.to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Helix, Vim, or Notion. Accepts legacy `keymap =` on load.
    #[serde(default, alias = "keymap")]
    pub editor: EditorKind,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// When true, `j`/`k` count wrapped visual lines in source view.
    /// Preview always wraps; this only affects source view. Default on.
    #[serde(default = "default_wrap")]
    pub wrap_motions: bool,
    /// When true, markdown is not constrained to `COLUMN_PX`. Default off.
    #[serde(default)]
    pub full_width: bool,
    /// Titlebar, footer, settings chrome.
    #[serde(default = "default_ui_font")]
    pub ui_font: FontSpec,
    /// Prose / headings / lists / quotes.
    #[serde(default = "default_markdown_font")]
    pub markdown_font: FontSpec,
    /// Fenced code blocks (buffer / mono).
    #[serde(default = "default_buffer_font")]
    pub buffer_font: FontSpec,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            editor: EditorKind::Helix,
            theme: default_theme(),
            wrap_motions: true,
            full_width: false,
            ui_font: default_ui_font(),
            markdown_font: default_markdown_font(),
            buffer_font: default_buffer_font(),
        }
    }
}

/// Wire format that also accepts the old flat `font_family` / `font_size` keys.
#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default, alias = "keymap")]
    editor: EditorKind,
    #[serde(default = "default_theme")]
    theme: String,
    #[serde(default = "default_wrap")]
    wrap_motions: bool,
    #[serde(default)]
    full_width: bool,
    #[serde(default)]
    ui_font: Option<FontSpec>,
    #[serde(default)]
    markdown_font: Option<FontSpec>,
    #[serde(default)]
    buffer_font: Option<FontSpec>,
    /// Legacy single-family key.
    #[serde(default)]
    font_family: Option<String>,
    /// Legacy single-size key.
    #[serde(default)]
    font_size: Option<u32>,
}

impl From<RawConfig> for Config {
    fn from(raw: RawConfig) -> Self {
        let legacy = raw
            .font_family
            .as_ref()
            .map(|fam| FontSpec::new(fam.clone(), raw.font_size.unwrap_or(15)));
        Self {
            editor: raw.editor,
            theme: raw.theme,
            wrap_motions: raw.wrap_motions,
            full_width: raw.full_width,
            ui_font: raw.ui_font.unwrap_or_else(default_ui_font).clamp(),
            markdown_font: raw
                .markdown_font
                .or_else(|| legacy.clone())
                .unwrap_or_else(default_markdown_font)
                .clamp(),
            buffer_font: raw
                .buffer_font
                .or(legacy)
                .unwrap_or_else(default_buffer_font)
                .clamp(),
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/crabmd"))
}

pub fn config_path() -> Option<PathBuf> {
    Some(config_dir()?.join("config.toml"))
}

pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return Config::default();
    };
    toml::from_str::<RawConfig>(&raw)
        .map(Config::from)
        .unwrap_or_default()
}

pub fn save(config: &Config) -> anyhow::Result<()> {
    let Some(dir) = config_dir() else {
        anyhow::bail!("HOME is not set");
    };
    fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    let body = toml::to_string_pretty(config)?;
    fs::write(path, body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_toml() {
        let cfg = Config {
            editor: EditorKind::Vim,
            theme: "catppuccin".into(),
            wrap_motions: false,
            full_width: true,
            ui_font: FontSpec::new("SF Pro", 13),
            markdown_font: FontSpec::new("Menlo", 15),
            buffer_font: FontSpec::new("JetBrainsMono Nerd Font", 14),
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(s.contains("editor"));
        assert!(s.contains("vim"));
        assert!(s.contains("ui_font"));
        assert!(s.contains("markdown_font"));
        assert!(s.contains("buffer_font"));
        assert!(!s.contains("font_family"));
        let back: Config = toml::from_str::<RawConfig>(&s).map(Config::from).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn default_is_helix_opencode_wrap_on() {
        let cfg: Config = toml::from_str::<RawConfig>("")
            .map(Config::from)
            .unwrap_or_default();
        assert_eq!(cfg.editor, EditorKind::Helix);
        assert_eq!(cfg.theme, "opencode");
        assert!(cfg.wrap_motions);
        assert!(!cfg.full_width);
        assert_eq!(cfg.markdown_font.family, "IBM Plex Sans");
        assert_eq!(cfg.markdown_font.size, 15);
        assert_eq!(cfg.buffer_font.size, 14);
    }

    #[test]
    fn migrates_legacy_font_keys() {
        let cfg: Config = toml::from_str::<RawConfig>(
            r#"
editor = "vim"
font_family = "Menlo"
font_size = 18
"#,
        )
        .map(Config::from)
        .unwrap();
        assert_eq!(cfg.editor, EditorKind::Vim);
        assert_eq!(cfg.markdown_font.family, "Menlo");
        assert_eq!(cfg.markdown_font.size, 18);
        assert_eq!(cfg.buffer_font.family, "Menlo");
        assert_eq!(cfg.buffer_font.size, 18);
        // UI keeps its own default unless overridden.
        assert_eq!(cfg.ui_font.family, default_ui_font().family);
    }

    #[test]
    fn reads_legacy_keymap() {
        let cfg: Config = toml::from_str::<RawConfig>("keymap = \"vim\"\ntheme = \"github\"\n")
            .map(Config::from)
            .unwrap();
        assert_eq!(cfg.editor, EditorKind::Vim);
        assert_eq!(cfg.theme, "github");
        assert!(cfg.wrap_motions);
    }

    #[test]
    fn notion_value() {
        let cfg: Config = toml::from_str::<RawConfig>("editor = \"notion\"\n")
            .map(Config::from)
            .unwrap();
        assert_eq!(cfg.editor, EditorKind::Notion);
        assert_eq!(cfg.editor.as_str(), "notion");
        assert!(!cfg.editor.is_modal());
    }
}
