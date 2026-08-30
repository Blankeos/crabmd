//! Image drop/paste: unique filenames next to the markdown file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "tif", "tiff",
];

pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.iter().any(|ok| e.eq_ignore_ascii_case(ok)))
}

pub fn unique_filename(existing: &HashSet<String>, preferred: &str) -> String {
    if !existing.contains(preferred) {
        return preferred.to_string();
    }
    let path = Path::new(preferred);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    for n in 2..10_000 {
        let candidate = format!("{stem}-{n}{ext}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{stem}-dup{ext}")
}

pub fn timestamped_name(stamp: &str, ext: &str) -> String {
    let ext = ext.trim_start_matches('.');
    format!("crabmd-image-{stamp}.{ext}")
}

pub fn now_stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub fn dir_names(dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                names.insert(name.to_string());
            }
        }
    }
    names
}

/// Copy or write `bytes` beside `md_path`. Returns the relative filename used.
pub fn write_beside(md_path: &Path, preferred: &str, bytes: &[u8]) -> anyhow::Result<String> {
    let dir = md_path.parent().unwrap_or_else(|| Path::new("."));
    let mut existing = dir_names(dir);
    let name = unique_filename(&existing, preferred);
    existing.insert(name.clone());
    let dest = dir.join(&name);
    std::fs::write(&dest, bytes)?;
    Ok(name)
}

/// Drop a file: reuse the original name when it does not collide; otherwise -2, -3.
/// If the drop is already in the markdown directory, do not copy.
pub fn place_dropped(md_path: &Path, src: &Path) -> anyhow::Result<String> {
    let dir = md_path
        .parent()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .unwrap_or_else(|| {
            md_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        });
    let src_abs = std::fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());
    let preferred = src
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("image.png")
        .to_string();

    if src_abs.parent() == Some(dir.as_path()) {
        return Ok(preferred);
    }

    let bytes = std::fs::read(&src_abs)?;
    write_beside(md_path, &preferred, &bytes)
}

pub fn gfm_image(alt: &str, filename: &str) -> String {
    format!("![{alt}]({filename})")
}

pub fn resolve_beside(md_path: &Path, src: &str) -> PathBuf {
    if Path::new(src).is_absolute() {
        PathBuf::from(src)
    } else {
        md_path.parent().unwrap_or_else(|| Path::new(".")).join(src)
    }
}

#[allow(dead_code)]
pub fn ext_from_mime_or_name(hint: &str) -> &'static str {
    let h = hint.to_ascii_lowercase();
    if h.contains("jpeg") || h.contains("jpg") {
        "jpg"
    } else if h.contains("gif") {
        "gif"
    } else if h.contains("webp") {
        "webp"
    } else if h.contains("svg") {
        "svg"
    } else if h.contains("bmp") {
        "bmp"
    } else {
        "png"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_when_free() {
        let existing = HashSet::new();
        assert_eq!(unique_filename(&existing, "shot.png"), "shot.png");
    }

    #[test]
    fn unique_adds_counter() {
        let existing = HashSet::from(["shot.png".into(), "shot-2.png".into()]);
        assert_eq!(unique_filename(&existing, "shot.png"), "shot-3.png");
    }

    #[test]
    fn timestamp_pattern() {
        let name = timestamped_name("20260830-040000", "png");
        assert_eq!(name, "crabmd-image-20260830-040000.png");
    }

    #[test]
    fn image_exts() {
        assert!(is_image_path(Path::new("a.PNG")));
        assert!(is_image_path(Path::new("a.webp")));
        assert!(!is_image_path(Path::new("a.md")));
    }

    #[test]
    fn gfm_line() {
        assert_eq!(gfm_image("cat", "cat.png"), "![cat](cat.png)");
    }
}
