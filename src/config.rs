//! User settings: editor + theme + wrap. Stored at ~/.config/crabmd/config.toml.

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

/// Monospace families confirmed present on this Mac. Default is Menlo
/// (gpui-component / macOS).
pub const FONT_FAMILIES: &[&str] = &[
    "Menlo",
    "Monaco",
    "JetBrainsMono Nerd Font",
    "Zed Mono",
    "Courier New",
];

pub const FONT_SIZES: &[u32] = &[13, 14, 15, 16, 18, 20];

pub fn default_font_family() -> String {
    "Menlo".into()
}

pub fn default_font_size() -> u32 {
    15
}

/// Old name kept as a type alias so existing call sites can migrate.
#[allow(dead_code)]
pub type Keymap = EditorKind;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Helix, Vim, or Notion. Accepts legacy `keymap =` on load.
    #[serde(default, alias = "keymap")]
    pub editor: EditorKind,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// When true, `j`/`k` count wrapped visual lines. Default on.
    #[serde(default = "default_wrap")]
    pub wrap_motions: bool,
    /// Raw textarea (Helix/Vim focused / visual span) font family.
    #[serde(default = "default_font_family")]
    pub font_family: String,
    /// Raw textarea font size in pixels (13–20).
    #[serde(default = "default_font_size")]
    pub font_size: u32,
}

fn default_theme() -> String {
    theme::DEFAULT_THEME.to_string()
}

fn default_wrap() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            editor: EditorKind::Helix,
            theme: default_theme(),
            wrap_motions: true,
            font_family: default_font_family(),
            font_size: default_font_size(),
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
    toml::from_str(&raw).unwrap_or_default()
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
            font_family: "Menlo".into(),
            font_size: 15,
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(s.contains("editor"));
        assert!(s.contains("vim"));
        assert!(s.contains("catppuccin"));
        assert!(s.contains("wrap_motions"));
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn default_is_helix_opencode_wrap_on() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.editor, EditorKind::Helix);
        assert_eq!(cfg.theme, "opencode");
        assert!(cfg.wrap_motions);
        assert_eq!(cfg.font_family, "Menlo");
        assert_eq!(cfg.font_size, 15);
    }

    #[test]
    fn reads_legacy_keymap() {
        let cfg: Config = toml::from_str("keymap = \"vim\"\ntheme = \"github\"\n").unwrap();
        assert_eq!(cfg.editor, EditorKind::Vim);
        assert_eq!(cfg.theme, "github");
        assert!(cfg.wrap_motions);
    }

    #[test]
    fn notion_value() {
        let cfg: Config = toml::from_str("editor = \"notion\"\n").unwrap();
        assert_eq!(cfg.editor, EditorKind::Notion);
        assert_eq!(cfg.editor.as_str(), "notion");
        assert!(!cfg.editor.is_modal());
    }
}
