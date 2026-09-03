//! OpenCode-compatible theme loading (`"$schema": "https://opencode.ai/theme.json"`).

use std::collections::HashMap;

use gpui::{rgb, Hsla, Rgba};
use serde::Deserialize;

pub const DEFAULT_THEME: &str = "opencode";

pub const THEME_FILES: &[(&str, &str)] = &[
    ("aura", include_str!("../themes/aura.json")),
    ("ayu", include_str!("../themes/ayu.json")),
    ("carbonfox-light", include_str!("../themes/carbonfox-light.json")),
    ("carbonfox", include_str!("../themes/carbonfox.json")),
    ("catppuccin-frappe", include_str!("../themes/catppuccin-frappe.json")),
    ("catppuccin-light", include_str!("../themes/catppuccin-light.json")),
    ("catppuccin-macchiato", include_str!("../themes/catppuccin-macchiato.json")),
    ("catppuccin", include_str!("../themes/catppuccin.json")),
    ("cobalt2-light", include_str!("../themes/cobalt2-light.json")),
    ("cobalt2", include_str!("../themes/cobalt2.json")),
    ("crabcode-orange", include_str!("../themes/crabcode-orange.json")),
    ("cursor-light", include_str!("../themes/cursor-light.json")),
    ("cursor", include_str!("../themes/cursor.json")),
    ("dracula-light", include_str!("../themes/dracula-light.json")),
    ("dracula", include_str!("../themes/dracula.json")),
    ("everforest-light", include_str!("../themes/everforest-light.json")),
    ("everforest", include_str!("../themes/everforest.json")),
    ("flexoki-light", include_str!("../themes/flexoki-light.json")),
    ("flexoki", include_str!("../themes/flexoki.json")),
    ("github-light", include_str!("../themes/github-light.json")),
    ("github", include_str!("../themes/github.json")),
    ("grokday", include_str!("../themes/grokday.json")),
    ("groknight", include_str!("../themes/groknight.json")),
    ("gruvbox-light", include_str!("../themes/gruvbox-light.json")),
    ("gruvbox", include_str!("../themes/gruvbox.json")),
    ("kanagawa-light", include_str!("../themes/kanagawa-light.json")),
    ("kanagawa", include_str!("../themes/kanagawa.json")),
    ("lucent-orng-light", include_str!("../themes/lucent-orng-light.json")),
    ("lucent-orng", include_str!("../themes/lucent-orng.json")),
    ("material-light", include_str!("../themes/material-light.json")),
    ("material", include_str!("../themes/material.json")),
    ("matrix-light", include_str!("../themes/matrix-light.json")),
    ("matrix", include_str!("../themes/matrix.json")),
    ("mercury-light", include_str!("../themes/mercury-light.json")),
    ("mercury", include_str!("../themes/mercury.json")),
    ("monokai-light", include_str!("../themes/monokai-light.json")),
    ("monokai", include_str!("../themes/monokai.json")),
    ("nightowl", include_str!("../themes/nightowl.json")),
    ("nord-light", include_str!("../themes/nord-light.json")),
    ("nord", include_str!("../themes/nord.json")),
    ("one-dark-light", include_str!("../themes/one-dark-light.json")),
    ("one-dark", include_str!("../themes/one-dark.json")),
    ("opencode-light", include_str!("../themes/opencode-light.json")),
    ("opencode", include_str!("../themes/opencode.json")),
    ("orng-light", include_str!("../themes/orng-light.json")),
    ("orng", include_str!("../themes/orng.json")),
    ("osaka-jade-light", include_str!("../themes/osaka-jade-light.json")),
    ("osaka-jade", include_str!("../themes/osaka-jade.json")),
    ("palenight-light", include_str!("../themes/palenight-light.json")),
    ("palenight", include_str!("../themes/palenight.json")),
    ("rosepine-light", include_str!("../themes/rosepine-light.json")),
    ("rosepine", include_str!("../themes/rosepine.json")),
    ("solarized-light", include_str!("../themes/solarized-light.json")),
    ("solarized", include_str!("../themes/solarized.json")),
    ("synthwave84-light", include_str!("../themes/synthwave84-light.json")),
    ("synthwave84", include_str!("../themes/synthwave84.json")),
    ("tokyonight-light", include_str!("../themes/tokyonight-light.json")),
    ("tokyonight", include_str!("../themes/tokyonight.json")),
    ("vercel-light", include_str!("../themes/vercel-light.json")),
    ("vercel", include_str!("../themes/vercel.json")),
    ("vesper-light", include_str!("../themes/vesper-light.json")),
    ("vesper", include_str!("../themes/vesper.json")),
    ("zenburn-light", include_str!("../themes/zenburn-light.json")),
    ("zenburn", include_str!("../themes/zenburn.json")),
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
    let value: serde_json::Value = serde_json::from_str(json)?;
    if value.get("defs").is_some() && value.get("theme").is_some() {
        let file: ThemeFile = serde_json::from_value(value)?;
        return load_tui(name, &file);
    }
    // Same desktop-theme schema crabcode ships (`light`/`dark` overrides):
    // grokday, groknight, crabcode-orange.
    if value.get("light").is_some() && value.get("dark").is_some() {
        return load_desktop(name, &value);
    }
    anyhow::bail!("unsupported theme schema for `{name}`")
}

fn load_tui(name: &str, file: &ThemeFile) -> anyhow::Result<Palette> {
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

/// Desktop-theme schema (`light`/`dark` with `seeds` + `overrides`), the
/// same files crabcode ships for grokday / groknight / crabcode-orange.
#[derive(Debug, Deserialize)]
struct DesktopMode {
    #[serde(default)]
    seeds: HashMap<String, String>,
    #[serde(default)]
    overrides: HashMap<String, String>,
}

fn load_desktop(name: &str, value: &serde_json::Value) -> anyhow::Result<Palette> {
    let appearance = match value
        .get("appearance")
        .and_then(|v| v.as_str())
    {
        Some("light") => Appearance::Light,
        Some("dark") => Appearance::Dark,
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
    let mode: DesktopMode = serde_json::from_value(
        value
            .get(side)
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )?;
    let seed = |key: &str| mode.seeds.get(key).map(String::as_str);
    let over = |key: &str| mode.overrides.get(key).map(String::as_str);
    // Seeds first, overrides win (mirrors crabcode's get_colors_with).
    let pick = |seeds: &[&str], overs: &[&str], fallback: &str| -> Hsla {
        for key in overs {
            if let Some(hex) = over(key) {
                if let Some(c) = parse_hex(hex) {
                    return c;
                }
            }
        }
        for key in seeds {
            if let Some(hex) = seed(key) {
                if let Some(c) = parse_hex(hex) {
                    return c;
                }
            }
        }
        parse_hex(fallback).unwrap()
    };
    let text_base = pick(&[], &["text-base"], "#eeeeee");
    Ok(Palette {
        name: name.to_string(),
        appearance,
        background: pick(&["neutral"], &["background-base"], "#0a0a0a"),
        background_panel: pick(
            &[],
            &[
                "background-stronger",
                "background-strong",
                "surface-raised-stronger-non-alpha",
            ],
            "#141414",
        ),
        background_element: pick(
            &[],
            &["surface-raised-base-hover", "background-strong"],
            "#1e1e1e",
        ),
        border: pick(&[], &["border-base", "border-weak-base"], "#484848"),
        text: text_base,
        text_muted: pick(&[], &["text-weak"], "#808080"),
        primary: pick(&["interactive", "primary"], &[], "#fab283"),
        secondary: pick(&["primary"], &[], "#5c9cf5"),
        accent: pick(&["primary"], &["markdown-heading"], "#9d7cd8"),
        error: pick(&["error"], &[], "#e06c75"),
        warning: pick(&["warning"], &[], "#f5a742"),
        success: pick(&["success", "diffAdd"], &[], "#7fd88f"),
        info: pick(&["info"], &[], "#56b6c2"),
        markdown_text: pick(&[], &["markdown-text", "text-base"], "#eeeeee"),
        markdown_heading: pick(&[], &["markdown-heading", "text-strong"], "#9d7cd8"),
        markdown_link: pick(&["interactive"], &["markdown-link"], "#fab283"),
        markdown_code: pick(&[], &["markdown-code", "text-base"], "#7fd88f"),
        markdown_block_quote: pick(&[], &["markdown-block-quote", "text-weak"], "#e5c07b"),
        markdown_emph: pick(&[], &["markdown-emph", "text-weak"], "#e5c07b"),
        markdown_strong: pick(&[], &["markdown-strong", "text-strong"], "#f5a742"),
        markdown_horizontal_rule: pick(
            &[],
            &["markdown-horizontal-rule", "border-base"],
            "#808080",
        ),
        markdown_list_item: pick(&[], &["markdown-list-item", "text-base"], "#fab283"),
        markdown_code_block: pick(&[], &["markdown-code-block", "text-base"], "#eeeeee"),
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
    fn ships_the_full_set() {
        let names = list_theme_names();
        assert_eq!(names.len(), THEME_FILES.len());
        for want in [
            "opencode",
            "opencode-light",
            "catppuccin",
            "catppuccin-light",
            "tokyonight",
            "tokyonight-light",
            "github",
            "github-light",
            "grokday",
            "groknight",
            "aura",
            "dracula",
            "nightowl",
        ] {
            assert!(names.contains(&want), "missing theme {want}");
        }
    }

    #[test]
    fn all_bundled_themes_load() {
        for name in list_theme_names() {
            load_named(name)
                .unwrap_or_else(|err| panic!("theme {name} failed: {err:#}"));
        }
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
