//! OpenCode-compatible theme loading (`"$schema": "https://opencode.ai/theme.json"`).

use std::collections::HashMap;

use gpui::{rgb, Hsla, Rgba};
use serde::Deserialize;

pub const DEFAULT_THEME: &str = "opencode";

pub const THEME_FILES: &[(&str, &str)] = &[
    ("opencode", include_str!("../themes/opencode.json")),
    (
        "opencode-light",
        include_str!("../themes/opencode-light.json"),
    ),
    ("catppuccin", include_str!("../themes/catppuccin.json")),
    (
        "catppuccin-light",
        include_str!("../themes/catppuccin-light.json"),
    ),
    ("tokyonight", include_str!("../themes/tokyonight.json")),
    (
        "tokyonight-light",
        include_str!("../themes/tokyonight-light.json"),
    ),
    ("github", include_str!("../themes/github.json")),
    ("github-light", include_str!("../themes/github-light.json")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Palette {
    pub name: String,
    pub appearance: Appearance,
    pub background: Hsla,
    pub background_panel: Hsla,
    pub background_element: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub primary: Hsla,
    pub secondary: Hsla,
    pub accent: Hsla,
    pub error: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
    pub info: Hsla,
    pub markdown_text: Hsla,
    pub markdown_heading: Hsla,
    pub markdown_link: Hsla,
    pub markdown_code: Hsla,
    pub markdown_block_quote: Hsla,
    pub markdown_emph: Hsla,
    pub markdown_strong: Hsla,
    pub markdown_horizontal_rule: Hsla,
    pub markdown_list_item: Hsla,
    pub markdown_code_block: Hsla,
}

impl Palette {
    pub fn alert_color(&self, kind: crate::document::AlertKind) -> Hsla {
        use crate::document::AlertKind::*;
        match kind {
            Note => self.info,
            Tip => self.success,
            Important => self.accent,
            Warning => self.warning,
            Caution => self.error,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    #[serde(default)]
    defs: HashMap<String, String>,
    theme: HashMap<String, serde_json::Value>,
    #[serde(default)]
    appearance: Option<String>,
}

pub fn list_theme_names() -> Vec<&'static str> {
    THEME_FILES.iter().map(|(name, _)| *name).collect()
}

pub fn load_named(name: &str) -> anyhow::Result<Palette> {
    let name = name.trim();
    let json = THEME_FILES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, j)| *j)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown theme `{name}`. available: {}",
                list_theme_names().join(", ")
            )
        })?;
    load_json(name, json)
}

pub fn load_json(name: &str, json: &str) -> anyhow::Result<Palette> {
    let file: ThemeFile = serde_json::from_str(json)?;
    let appearance = match file.appearance.as_deref() {
        Some("light") => Appearance::Light,
        _ => {
            if name.ends_with("-light") {
                Appearance::Light
            } else {
                Appearance::Dark
            }
        }
    };
    let side = match appearance {
        Appearance::Dark => "dark",
        Appearance::Light => "light",
    };

    let resolve = |key: &str, fallback: &str| -> Hsla {
        color_of(&file, key, side).unwrap_or_else(|| parse_hex(fallback).unwrap())
    };

    Ok(Palette {
        name: name.to_string(),
        appearance,
        background: resolve("background", "#0a0a0a"),
        background_panel: resolve("backgroundPanel", "#141414"),
        background_element: resolve("backgroundElement", "#1e1e1e"),
        border: resolve("border", "#484848"),
        text: resolve("text", "#eeeeee"),
        text_muted: resolve("textMuted", "#808080"),
        primary: resolve("primary", "#fab283"),
        secondary: resolve("secondary", "#5c9cf5"),
        accent: resolve("accent", "#9d7cd8"),
        error: resolve("error", "#e06c75"),
        warning: resolve("warning", "#f5a742"),
        success: resolve("success", "#7fd88f"),
        info: resolve("info", "#56b6c2"),
        markdown_text: resolve("markdownText", "#eeeeee"),
        markdown_heading: resolve("markdownHeading", "#9d7cd8"),
        markdown_link: resolve("markdownLink", "#fab283"),
        markdown_code: resolve("markdownCode", "#7fd88f"),
        markdown_block_quote: resolve("markdownBlockQuote", "#e5c07b"),
        markdown_emph: resolve("markdownEmph", "#e5c07b"),
        markdown_strong: resolve("markdownStrong", "#f5a742"),
        markdown_horizontal_rule: resolve("markdownHorizontalRule", "#808080"),
        markdown_list_item: resolve("markdownListItem", "#fab283"),
        markdown_code_block: resolve("markdownCodeBlock", "#eeeeee"),
    })
}

fn color_of(file: &ThemeFile, key: &str, side: &str) -> Option<Hsla> {
    let value = file.theme.get(key)?;
    let token = match value {
        serde_json::Value::String(s) => s.as_str(),
        serde_json::Value::Object(map) => map
            .get(side)
            .and_then(|v| v.as_str())
            .or_else(|| map.get("dark").and_then(|v| v.as_str()))?,
        _ => return None,
    };
    if let Some(hex) = file.defs.get(token) {
        return parse_hex(hex);
    }
    parse_hex(token)
}

pub fn parse_hex(input: &str) -> Option<Hsla> {
    let hex = input.trim().trim_start_matches('#');
    match hex.len() {
        3 => {
            let n = u32::from_str_radix(hex, 16).ok()?;
            let r = (n >> 8) & 0xF;
            let g = (n >> 4) & 0xF;
            let b = n & 0xF;
            let packed = (r << 20) | (r << 16) | (g << 12) | (g << 8) | (b << 4) | b;
            Some(rgb(packed).into())
        }
        6 => {
            let n = u32::from_str_radix(hex, 16).ok()?;
            Some(rgb(n).into())
        }
        8 => {
            let n = u32::from_str_radix(hex, 16).ok()?;
            let r = ((n >> 24) & 0xFF) as f32 / 255.0;
            let g = ((n >> 16) & 0xFF) as f32 / 255.0;
            let b = ((n >> 8) & 0xFF) as f32 / 255.0;
            let a = (n & 0xFF) as f32 / 255.0;
            Some(Rgba { r, g, b, a }.into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_the_small_set() {
        let names = list_theme_names();
        assert_eq!(
            names,
            [
                "opencode",
                "opencode-light",
                "catppuccin",
                "catppuccin-light",
                "tokyonight",
                "tokyonight-light",
                "github",
                "github-light",
            ]
        );
    }

    #[test]
    fn opencode_loads_and_maps_markdown_keys() {
        let p = load_named("opencode").unwrap();
        assert_eq!(p.appearance, Appearance::Dark);
        assert_eq!(p.name, "opencode");
        let heading = parse_hex("#9d7cd8").unwrap();
        assert_eq!(p.markdown_heading, heading);
        let text = parse_hex("#eeeeee").unwrap();
        assert_eq!(p.markdown_text, text);
    }

    #[test]
    fn light_variant_uses_light_side() {
        let p = load_named("github-light").unwrap();
        assert_eq!(p.appearance, Appearance::Light);
    }
}
