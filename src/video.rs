//! Inline video playback: pure-Rust `yscv-video` (no FFmpeg/GStreamer
//! system libs — the `CrabMD` bundle stays self-contained) decodes MP4
//! (H.264/HEVC) off-thread into `RenderImage` frames, mirroring `zorite`'s
//! `MermaidStore` pattern. Playback is silent (yscv exposes audio metadata
//! only) and scrubbable; `Open` still shells to the system player for audio.
//!
//! Why not `gpui-video-player` (the awesome-gpui entry)? It needs a system
//! GStreamer install + per-video thread/playbin pipeline, and assumes tight
//! NV12 layouts — hostile to a scrolling doc and to a self-contained bundle.
//! The `gpui-video` fork trades that for FFmpeg dev libraries + CPAL — same
//! bundle problem. `yscv-video` is pure Rust with optional HW backends we
//! leave off, so `Mp4VideoReader::open` + `next_frame` just works everywhere.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::RenderImage;

/// Cap decoded frames per clip: 10s at 30fps. Each 640-wide RGBA frame is
/// ~1MB; beyond this the card shows the first run with a "preview" note.
pub const MAX_FRAMES: usize = 300;
/// Longest edge after downscale — keeps GPU atlas + memory bounded.
pub const MAX_EDGE: usize = 640;
/// Frame tick when timestamps are missing (~30fps).
pub const FALLBACK_FRAME_DT: Duration = Duration::from_millis(33);

pub struct VideoClip {
    pub frames: Vec<Arc<RenderImage>>,
    pub width: u32,
    pub height: u32,
    pub duration_us: u64,
    pub fps: f32,
    pub truncated: bool,
}

enum Slot {
    Loading,
    Ready(VideoClip, Vec<u64>),
    Failed(String),
}

/// Per-clip playhead. `generation` invalidates stale timer loops when the
/// user pauses/seeks or the clip is replaced.
pub struct VideoPlay {
    pub playing: bool,
    pub frame: usize,
    pub generation: u64,
}

impl Default for VideoPlay {
    fn default() -> Self {
        Self {
            playing: false,
            frame: 0,
            generation: 0,
        }
    }
}

#[derive(Default)]
pub struct VideoStore {
    slots: HashMap<String, Slot>,
    plays: HashMap<String, VideoPlay>,
}

impl VideoStore {
    pub fn key_for(path: &Path) -> String {
        path.to_string_lossy().to_string()
    }

    pub fn remote_key(url: &str) -> String {
        format!("remote:{url}")
    }

    pub fn get(&self, key: &str) -> Option<(&VideoClip, &[u64])> {
        match self.slots.get(key) {
            Some(Slot::Ready(clip, ts)) => Some((clip, ts)),
            _ => None,
        }
    }

    pub fn error(&self, key: &str) -> Option<String> {
        match self.slots.get(key) {
            Some(Slot::Failed(e)) => Some(e.clone()),
            _ => None,
        }
    }

    /// Claim `key` for decoding. `false` when a slot already exists.
    pub fn begin(&mut self, key: String) -> bool {
        if self.slots.contains_key(&key) {
            return false;
        }
        self.slots.insert(key, Slot::Loading);
        true
    }

    pub fn finish(&mut self, key: String, result: Result<(VideoClip, Vec<u64>), String>) {
        let slot = match result {
            Ok((clip, ts)) => Slot::Ready(clip, ts),
            Err(e) => Slot::Failed(e),
        };
        self.slots.insert(key, slot);
    }

    pub fn play_state(&self, key: &str) -> (bool, usize) {
        self.plays
            .get(key)
            .map(|p| (p.playing, p.frame))
            .unwrap_or((false, 0))
    }

    pub fn toggle(&mut self, key: &str) -> (bool, u64) {
        let n = self.frame_count(key);
        let p = self.plays.entry(key.to_string()).or_default();
        p.playing = !p.playing;
        if p.playing && n.is_some_and(|n| p.frame + 1 >= n) {
            p.frame = 0;
        }
        p.generation += 1;
        (p.playing, p.generation)
    }

    /// Scrub the playhead without invalidating the timer loop — the loop
    /// keeps playing from the new position. Used by the scrub bar.
    pub fn scrub(&mut self, key: &str, frame: usize) {
        let n = self.frame_count(key).unwrap_or(1);
        let p = self.plays.entry(key.to_string()).or_default();
        p.frame = frame.min(n.saturating_sub(1));
    }

    /// Per-frame tick from the clip's own timestamps (falls back to fps,
    /// then 30fps). Captured once when playback starts.
    pub fn frame_dt(&self, key: &str) -> Duration {
        match self.slots.get(key) {
            Some(Slot::Ready(clip, _)) => {
                let n = clip.frames.len().max(1) as u32;
                if clip.duration_us > 0 && n > 0 {
                    Duration::from_micros(clip.duration_us / n as u64).clamp(
                        Duration::from_millis(8),
                        Duration::from_secs(1),
                    )
                } else if clip.fps > 0.0 {
                    Duration::from_secs_f32((1.0 / clip.fps).clamp(1.0 / 120.0, 1.0))
                } else {
                    FALLBACK_FRAME_DT
                }
            }
            _ => FALLBACK_FRAME_DT,
        }
    }

    /// Advance one frame iff still playing under `gen`. `false` stops the
    /// timer loop (paused, superseded, or end of clip — which also pauses).
    pub fn advance_if_active(&mut self, key: &str, gen: u64) -> bool {
        let n = match self.frame_count(key) {
            Some(n) => n,
            None => return false,
        };
        let p = self.plays.entry(key.to_string()).or_default();
        if !p.playing || p.generation != gen {
            return false;
        }
        if p.frame + 1 >= n {
            p.playing = false;
            p.generation += 1;
            return false;
        }
        p.frame += 1;
        true
    }

    fn frame_count(&self, key: &str) -> Option<usize> {
        match self.slots.get(key) {
            Some(Slot::Ready(clip, _)) => Some(clip.frames.len()),
            _ => None,
        }
    }
}

/// Nearest-neighbor RGB8 downscale to `MAX_EDGE`. Keeps the preview light
/// without pulling `image` resize features.
pub fn downscale_rgb8(
    rgb: &[u8],
    w: usize,
    h: usize,
    max_edge: usize,
) -> (Vec<u8>, usize, usize) {
    let longest = w.max(h);
    if longest <= max_edge || w == 0 || h == 0 {
        return (rgb.to_vec(), w, h);
    }
    let scale = max_edge as f64 / longest as f64;
    let nw = ((w as f64 * scale).round() as usize).max(1);
    let nh = ((h as f64 * scale).round() as usize).max(1);
    let mut out = vec![0u8; nw * nh * 3];
    for y in 0..nh {
        let sy = ((y as f64 / scale).floor() as usize).min(h - 1);
        for x in 0..nw {
            let sx = ((x as f64 / scale).floor() as usize).min(w - 1);
            let s = (sy * w + sx) * 3;
            let d = (y * nw + x) * 3;
            out[d..d + 3].copy_from_slice(&rgb[s..s + 3]);
        }
    }
    (out, nw, nh)
}

/// RGB8 → `RenderImage` (RGBA + atlas BGRA swap, same as
/// `SvgRenderer::render_single_frame`).
pub fn rgb8_to_image(rgb: &[u8], w: u32, h: u32) -> Option<Arc<RenderImage>> {
    if w == 0 || h == 0 || rgb.len() < w as usize * h as usize * 3 {
        return None;
    }
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for px in rgb.chunks_exact(3) {
        rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    let mut buf = image::RgbaImage::from_raw(w, h, rgba)?;
    for pixel in buf.chunks_mut(4) {
        gpui::swap_rgba_pa_to_bgra(pixel);
    }
    Some(Arc::new(RenderImage::new(smallvec::smallvec![
        image::Frame::new(buf)
    ])))
}

/// Blocking decode: MP4 → preview clip. Runs inside `background_spawn`.
pub fn decode_file(path: &Path) -> Result<(VideoClip, Vec<u64>), String> {
    let mut reader =
        yscv_video::Mp4VideoReader::open(path).map_err(|e| format!("open failed: {e}"))?;
    let mut frames = Vec::new();
    let mut stamps = Vec::new();
    let mut dw = 0u32;
    let mut dh = 0u32;
    let mut truncated = false;
    loop {
        if frames.len() >= MAX_FRAMES {
            truncated = true;
            break;
        }
        let next = reader.next_frame().map_err(|e| format!("decode: {e}"))?;
        let Some(f) = next else { break };
        if f.width == 0 || f.height == 0 || f.rgb8_data.is_empty() {
            continue;
        }
        let (rgb, nw, nh) = downscale_rgb8(&f.rgb8_data, f.width, f.height, MAX_EDGE);
        let (w, h) = (nw as u32, nh as u32);
        dw = w;
        dh = h;
        if let Some(img) = rgb8_to_image(&rgb, w, h) {
            frames.push(img);
            stamps.push(f.timestamp_us);
        }
    }
    if frames.is_empty() {
        return Err("no frames decoded".to_string());
    }
    // Some MP4s carry no usable timestamps (yscv reports all zeros).
    // Anything under ~1ms/frame is bogus — synthesize 30fps so pacing
    // and the time label behave.
    if stamps.last().copied().unwrap_or(0) < frames.len() as u64 * 1_000 {
        stamps = (0..frames.len()).map(|i| i as u64 * 33_333).collect();
    }
    let duration_us = stamps.last().copied().unwrap_or(0).max(1);
    let secs = duration_us as f32 / 1_000_000.0;
    let fps = if secs > 0.0 {
        frames.len() as f32 / secs
    } else {
        30.0
    };
    Ok((
        VideoClip {
            frames,
            width: dw,
            height: dh,
            duration_us,
            fps: fps.clamp(1.0, 120.0),
            truncated,
        },
        stamps,
    ))
}

/// `41.2s` / `0:03` time labels for the scrub bar.
pub fn fmt_time(us: u64) -> String {
    let s = us / 1_000_000;
    if s < 60 {
        format!("0:{s:02}")
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Temp file for a remote URL so it decodes through the same path.
pub fn remote_cache_path(url: &str) -> std::path::PathBuf {
    let mut hash = 0u64;
    for b in url.bytes() {
        hash = hash.wrapping_mul(1099511628211).wrapping_add(b as u64);
    }
    std::env::temp_dir()
        .join("crabmd-videos")
        .join(format!("{hash:016x}.mp4"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_caps_longest_edge() {
        let rgb = vec![7u8; 1200 * 300 * 3];
        let (out, w, h) = downscale_rgb8(&rgb, 1200, 300, 640);
        assert_eq!((w, h), (640, 160));
        assert_eq!(out.len(), 640 * 160 * 3);
    }

    #[test]
    fn downscale_keeps_small_frames() {
        let rgb = vec![1u8; 100 * 80 * 3];
        let (out, w, h) = downscale_rgb8(&rgb, 100, 80, 640);
        assert_eq!((w, h), (100, 80));
        assert_eq!(out.len(), rgb.len());
    }

    #[test]
    fn rgb8_to_image_rejects_bad_dims() {
        assert!(rgb8_to_image(&[0u8; 12], 0, 2).is_none());
        assert!(rgb8_to_image(&[0u8; 3], 2, 2).is_none());
        let rgb = vec![200u8; 4 * 2 * 3];
        assert!(rgb8_to_image(&rgb, 4, 2).is_some());
    }

    #[test]
    fn store_begin_finish_round_trip() {
        let mut store = VideoStore::default();
        assert!(store.begin("a".to_string()));
        assert!(!store.begin("a".to_string()));
        assert!(store.get("a").is_none());
        store.finish("a".to_string(), Err("boom".to_string()));
        assert_eq!(store.error("a").as_deref(), Some("boom"));
    }

    #[test]
    fn fmt_time_labels() {
        assert_eq!(fmt_time(3_500_000), "0:03");
        assert_eq!(fmt_time(75_000_000), "1:15");
    }

    #[test]
    fn decode_real_mp4() {
        let path = std::path::Path::new("examples/random_small.mp4");
        if !path.exists() {
            return;
        }
        let (clip, stamps) = decode_file(path).expect("decode sample clip");
        assert!(!clip.frames.is_empty());
        assert_eq!(clip.frames.len(), stamps.len());
        // Zero-timestamp containers get synthesized 30fps pacing.
        assert!(clip.duration_us > 0);
        assert!((clip.fps - 30.0).abs() < 5.0);
        let mut store = VideoStore::default();
        store.begin("t".to_string());
        store.finish("t".to_string(), Ok((clip, stamps)));
        let dt = store.frame_dt("t");
        assert!(dt >= std::time::Duration::from_millis(8));
        assert!(dt <= std::time::Duration::from_secs(1));
    }
}
