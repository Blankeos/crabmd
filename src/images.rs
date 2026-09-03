//! Image drop/paste: unique filenames next to the markdown file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "tif", "tiff",
];

/// GitHub-style video attachments (`![alt](clip.mp4)`, `<video src=…>`).
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "webm", "m4v", "ogv", "ogg"];

pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.iter().any(|ok| e.eq_ignore_ascii_case(ok)))
}

/// Image or video file (paste/drop peer of images).
pub fn is_media_path(path: &Path) -> bool {
    is_image_path(path) || is_video_path(path)
}

pub fn is_video_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTS.iter().any(|ok| e.eq_ignore_ascii_case(ok)))
}

/// True when an `![alt](src)` target is a video (extension or URL guess).
pub fn is_video_src(src: &str) -> bool {
    let base = src.split(['?', '#']).next().unwrap_or(src);
    Path::new(base)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTS.iter().any(|ok| e.eq_ignore_ascii_case(ok)))
}

/// `<video src="clip.mp4">…</video>` (or bare `<video src=…>`) -> src.
pub fn parse_video_src(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let tag = lower.find("<video")?;
    let rest = &raw[tag..];
    let src_ix = rest.to_ascii_lowercase().find("src")?;
    let after = rest[src_ix + 3..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let quote = after.as_bytes().first()?;
    if *quote == b'"' || *quote == b'\'' {
        let end = after[1..].find(*quote as char)?;
        Some(after[1..1 + end].to_string())
    } else {
        let end = after
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(after.len());
        let src = after[..end].trim();
        (!src.is_empty()).then(|| src.to_string())
    }
}

/// `<img src="pic.png" …>` (anywhere in the raw HTML) -> (src, alt).
pub fn parse_img_src(raw: &str) -> Option<(String, String)> {
    let lower = raw.to_ascii_lowercase();
    let tag = lower.find("<img")?;
    let rest = &raw[tag..];
    let end = rest.find('>').unwrap_or(rest.len());
    let tag_text = &rest[..end];
    Some((tag_attr(tag_text, "src")?, tag_attr(tag_text, "alt").unwrap_or_default()))
}

/// `attr="v"` / `attr='v'` / `attr=v` inside a single HTML tag.
fn tag_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = lower.as_str();
    let mut offset = 0usize;
    loop {
        let ix = search.find(name)?;
        let abs = offset + ix;
        // Attribute name must not be a suffix of a longer name (`data-src`).
        let before = tag[..abs].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            search = &search[ix + name.len()..];
            offset = abs + name.len();
            continue;
        }
        let after = tag[abs + name.len()..].trim_start();
        let after = after.strip_prefix('=')?.trim_start();
        let quote = after.as_bytes().first()?;
        if *quote == b'"' || *quote == b'\'' {
            let end = after[1..].find(*quote as char)?;
            return Some(after[1..1 + end].to_string());
        }
        let end = after
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(after.len());
        let v = after[..end].trim();
        return (!v.is_empty()).then(|| v.to_string());
    }
}

/// Remote `http(s)` media that can never resolve to a local file.
pub fn is_remote_src(src: &str) -> bool {
    src.starts_with("http://") || src.starts_with("https://")
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

/// `1536` → `1.5 KB` for video/image meta lines.
pub fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{} {}", bytes, UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Best-effort tag name of a raw HTML block (`<video …>` → `video`).
pub fn html_tag(raw: &str) -> Option<String> {
    let t = raw.trim_start();
    let t = t.strip_prefix('<')?;
    let t = t.trim_start_matches('/');
    let end = t
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(t.len());
    let tag = t[..end].to_ascii_lowercase();
    (!tag.is_empty()).then(|| tag)
}

/// Generic `src`/`href` of any HTML block (video, iframe, embed, source…).
pub fn parse_html_src(raw: &str) -> Option<String> {
    parse_video_src(raw).or_else(|| {
        parse_img_src(raw)
            .map(|(s, _)| s)
            .or_else(|| tag_attr_pub(raw, "src").or_else(|| tag_attr_pub(raw, "href")))
    })
}

/// `attr="v"` inside any tag in `raw` (public sibling of `tag_attr`).
pub fn tag_attr_pub(raw: &str, name: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let mut search = lower.as_str();
    let mut offset = 0usize;
    loop {
        let ix = search.find(name)?;
        let abs = offset + ix;
        let before = raw[..abs].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            search = &search[ix + name.len()..];
            offset = abs + name.len();
            continue;
        }
        let after = raw[abs + name.len()..].trim_start();
        let after = after.strip_prefix('=')?.trim_start();
        let quote = after.as_bytes().first()?;
        if *quote == b'"' || *quote == b'\'' {
            let end = after[1..].find(*quote as char)?;
            return Some(after[1..1 + end].to_string());
        }
        let end = after
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(after.len());
        let v = after[..end].trim();
        return (!v.is_empty()).then(|| v.to_string());
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

    #[test]
    fn video_exts() {
        assert!(is_video_path(Path::new("clip.mp4")));
        assert!(is_video_path(Path::new("clip.MOV")));
        assert!(!is_video_path(Path::new("clip.png")));
        assert!(is_media_path(Path::new("clip.webm")));
        assert!(is_media_path(Path::new("pic.png")));
        assert!(!is_media_path(Path::new("notes.md")));
        assert!(is_video_src("clip.mp4"));
        assert!(is_video_src("https://x.test/v.mov?dl=1"));
        assert!(!is_video_src("pic.png"));
    }

    #[test]
    fn video_tag_src() {
        assert_eq!(
            parse_video_src(r#"<video src="clip.mp4" controls></video>"#),
            Some("clip.mp4".to_string())
        );
        assert_eq!(
            parse_video_src("<video src=clip.mp4>"),
            Some("clip.mp4".to_string())
        );
        assert_eq!(parse_video_src("![a](b.png)"), None);
    }

    #[test]
    fn img_tag_src() {
        assert_eq!(
            parse_img_src(r#"<p><img width="100%" src="https://x.test/b.png" alt="B"></p>"#),
            Some(("https://x.test/b.png".to_string(), "B".to_string()))
        );
        assert_eq!(
            parse_img_src("<img src=pic.png>"),
            Some(("pic.png".to_string(), String::new()))
        );
        assert_eq!(parse_img_src("<video src=c.mp4>"), None);
    }
}
