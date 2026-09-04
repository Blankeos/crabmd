//! Bundled SVGs from `assets/icons/` (iconmate, lucide, preset `normal`).
//! App icon: `assets/app-icon.png` is the source of truth.

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::anyhow;
use gpui::{AssetSource, Result, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        Self::get(path)
            .map(|f| Some(f.data))
            .ok_or_else(|| anyhow!("could not find asset at path \"{path}\""))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect())
    }
}

pub fn path(name: &str) -> String {
    format!("icons/{name}.svg")
}

pub const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app-icon.png");

/// Bundled text fonts (OFL): IBM Plex Sans (UI/prose) + JetBrains Mono
/// (code). Registered via `cx.text_system().add_fonts` at startup so the
/// defaults work without a system install — user `font_family` overrides in
/// `config.toml` still resolve from system fonts as before.
pub fn load_bundled_fonts(cx: &gpui::App) {
    let fonts = [
        // IBM Plex Sans (fontsource latin subset)
        &include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf")[..],
        &include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Italic.ttf")[..],
        &include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Medium.ttf")[..],
        &include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-MediumItalic.ttf")[..],
        &include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf")[..],
        &include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBoldItalic.ttf")[..],
        &include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Bold.ttf")[..],
        &include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-BoldItalic.ttf")[..],
        // JetBrains Mono (official static TTFs)
        &include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf")[..],
        &include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Italic.ttf")[..],
        &include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Medium.ttf")[..],
        &include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-MediumItalic.ttf")[..],
        &include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-SemiBold.ttf")[..],
        &include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-SemiBoldItalic.ttf")[..],
        &include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Bold.ttf")[..],
        &include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-BoldItalic.ttf")[..],
    ];
    let fonts: Vec<Cow<'static, [u8]>> =
        fonts.into_iter().map(|b| Cow::Borrowed(b)).collect();
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        eprintln!("crabmd: bundled fonts failed to load: {err:#}");
    }
}

/// X11/Wayland window icon (GPUI `WindowOptions.icon` is documented X11-only).
pub fn window_icon() -> Option<Arc<image::RgbaImage>> {
    let img = image::load_from_memory(APP_ICON_PNG).ok()?;
    Some(Arc::new(img.to_rgba8()))
}

/// macOS Dock / app-switcher icon for unpackaged `cargo r` as well as bundles.
#[cfg(target_os = "macos")]
pub fn apply_dock_icon() {
    use cocoa::appkit::{NSApp, NSApplication, NSImage};
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSData;
    unsafe {
        let data = NSData::dataWithBytes_length_(
            nil,
            APP_ICON_PNG.as_ptr() as *const std::ffi::c_void,
            APP_ICON_PNG.len() as u64,
        );
        let image = NSImage::initWithData_(NSImage::alloc(nil), data);
        if image != nil {
            let app: id = NSApp();
            if app != nil {
                app.setApplicationIconImage_(image);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply_dock_icon() {}
