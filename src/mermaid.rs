//! Live Mermaid diagrams: pure-Rust `mermaid-rs-renderer` (same crate Zed
//! uses — no browser/JS) lays the block out to SVG, then GPUI's built-in
//! `SvgRenderer` rasterizes it to a `RenderImage`.
//!
//! Renders run off-thread and are cached here keyed by block source text,
//! mirroring `zorite`'s `MermaidStore`. Diagrams are themed at render time
//! from the active [`crate::theme::Palette`], so a theme switch clears the
//! cache (they re-render on next paint).

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{Hsla, RenderImage, SharedString, SvgRenderer};

use crate::theme::Palette;

/// Texture is rasterized at natural SVG size; display at half so the type
/// reads beside 14-16px body text. The 2:1 ratio doubles as crispness DPR
/// (1:1 device px on a 2x display).
pub const RASTER_SCALE: f32 = 1.0;
const DISPLAY_SCALE: f32 = 0.5;

type Image = (Arc<RenderImage>, f32, f32);

enum Slot {
    Loading,
    Ready(Image),
    Failed(String),
}

#[derive(Default)]
pub struct MermaidStore {
    slots: HashMap<SharedString, Slot>,
}

impl MermaidStore {
    pub fn get(&self, source: &SharedString) -> Option<Image> {
        match self.slots.get(source) {
            Some(Slot::Ready((img, w, h))) => Some((img.clone(), *w, *h)),
            _ => None,
        }
    }

    pub fn error(&self, source: &SharedString) -> Option<String> {
        match self.slots.get(source) {
            Some(Slot::Failed(e)) => Some(e.clone()),
            _ => None,
        }
    }

    /// Claim `source` for rendering. `false` when a slot already exists.
    pub fn begin(&mut self, source: SharedString) -> bool {
        if self.slots.contains_key(&source) {
            return false;
        }
        self.slots.insert(source, Slot::Loading);
        true
    }

    pub fn finish(&mut self, source: SharedString, result: Result<Arc<RenderImage>, String>) {
        let slot = match result {
            Ok(image) => {
                let size = image.size(0);
                Slot::Ready((
                    image,
                    size.width.0 as f32 * DISPLAY_SCALE,
                    size.height.0 as f32 * DISPLAY_SCALE,
                ))
            }
            Err(e) => Slot::Failed(e),
        };
        self.slots.insert(source, slot);
    }

    pub fn clear(&mut self) {
        self.slots.clear();
    }
}

fn hex(color: Hsla, bg: Hsla) -> String {
    let bg = bg.to_rgb();
    let c = color.to_rgb();
    let blend = |fg: f32, b: f32| fg * c.a + b * (1.0 - c.a);
    let to = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        to(blend(c.r, bg.r)),
        to(blend(c.g, bg.g)),
        to(blend(c.b, bg.b))
    )
}

/// Build a `mermaid_rs_renderer::Theme` from the live palette. Call on the
/// main thread; the result is plain `String`s so it crosses threads.
pub fn theme_for(pal: &Palette) -> mermaid_rs_renderer::Theme {
    let bg = hex(pal.background_panel, pal.background_panel);
    let text = hex(pal.markdown_text, pal.background_panel);
    let border = hex(pal.text_muted, pal.background_panel);
    let line = hex(pal.text_muted, pal.background_panel);
    let fill = hex(pal.background_element, pal.background_panel);

    let mut t = mermaid_rs_renderer::Theme::modern();
    t.background = bg.clone();
    t.text_color = text.clone();
    t.primary_color = fill.clone();
    t.primary_text_color = text.clone();
    t.primary_border_color = border.clone();
    t.secondary_color = hex(pal.background_element, pal.background_panel);
    t.tertiary_color = hex(pal.border, pal.background_panel);
    t.line_color = line.clone();
    t.edge_label_background = bg.clone();
    t.cluster_background = hex(pal.background, pal.background_panel);
    t.cluster_border = hex(pal.border, pal.background_panel);
    t.sequence_actor_fill = fill.clone();
    t.sequence_actor_border = border.clone();
    t.sequence_actor_line = line.clone();
    t.sequence_note_fill = hex(pal.background_element, pal.background_panel);
    t.sequence_note_border = border.clone();
    t.sequence_activation_fill = hex(pal.border, pal.background_panel);
    t.sequence_activation_border = border.clone();
    let accent = pal.accent;
    let rotated = |i: usize, n: usize| {
        let mut c = accent;
        c.h = (accent.h + i as f32 / n as f32).fract();
        hex(c, pal.background_panel)
    };
    t.pie_colors = std::array::from_fn(|i| rotated(i, 12));
    t.pie_title_text_color = text.clone();
    t.pie_legend_text_color = text.clone();
    t.pie_stroke_color = border.clone();
    t.pie_outer_stroke_color = border.clone();
    t.git_colors = std::array::from_fn(|i| rotated(i, 8));
    t.git_inv_colors = std::array::from_fn(|i| rotated(i, 8));
    t.git_commit_label_color = text.clone();
    t.git_commit_label_background = fill.clone();
    t.git_tag_label_color = text;
    t.git_tag_label_background = fill;
    t.git_tag_label_border = border;
    t
}

/// Mermaid source → SVG → `RenderImage`. Pure CPU, safe off-thread.
pub fn render_to_image(
    source: &str,
    theme: mermaid_rs_renderer::Theme,
    svg: &SvgRenderer,
    scale: f32,
) -> Result<Arc<RenderImage>, String> {
    let options = mermaid_rs_renderer::RenderOptions {
        theme,
        layout: mermaid_rs_renderer::LayoutConfig::default(),
    };
    let svg_text =
        mermaid_rs_renderer::render_with_options(source, options).map_err(|e| e.to_string())?;
    svg.render_single_frame(svg_text.as_bytes(), scale)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_diagram_errors() {
        let svg = SvgRenderer::new(Arc::new(crate::assets::Assets));
        let theme = mermaid_rs_renderer::Theme::modern();
        assert!(render_to_image("not a diagram ((((", theme, &svg, 1.0).is_err());
    }

    #[test]
    fn store_begin_finish_round_trip() {
        let mut store = MermaidStore::default();
        let key: SharedString = "graph TD;A-->B".into();
        assert!(store.begin(key.clone()));
        assert!(!store.begin(key.clone()));
        assert!(store.get(&key).is_none());
    }
}
