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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::RenderImage;

/// Cap decoded frames per clip: 10s at 30fps. Each 640-wide RGBA frame is
/// ~1MB; beyond this the card shows the first run with a "preview" note.
pub const MAX_FRAMES: usize = 300;
/// Frames per progressive batch: first paint lands after ~12 frames instead
/// of all 300, so remote clips play while the rest still decodes.
pub const DECODE_BATCH: usize = 12;
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
    /// Range-fetched preview (first ~2MB) playing while the full clip still
    /// buffers — browser-style progressive playback. Replaced by the full
    /// decode when the download completes.
    pub preview: bool,
    /// Stable totals known upfront (sample-table count + moov duration),
    /// so the scrub bar / time label don't grow with each decoded batch.
    /// `None` when the container couldn't be probed (fallback: decoded `n`).
    /// When the clip is `truncated` (capped at `MAX_FRAMES`) these describe
    /// the full video, not the playable preview — the UI then keeps the
    /// denominator on the decoded run with a "preview of first N" note.
    pub total_frames: Option<usize>,
    pub total_duration_us: Option<u64>,
}

impl VideoClip {
    /// Stable timeline length for the scrub bar. While batches stream in,
    /// the decoded run grows — but the denominator stays pinned to the
    /// container total, so the playhead doesn't jump per chunk. Truncated
    /// (`MAX_FRAMES`-capped) previews keep the denominator on the decoded
    /// run with a "preview of first N" note.
    pub fn timeline_frames(&self) -> usize {
        let n = self.frames.len().max(1);
        if self.truncated {
            return n;
        }
        match self.total_frames {
            Some(t) if t >= self.frames.len() && t <= MAX_FRAMES => t.max(1),
            _ => n,
        }
    }

    /// Stable total duration label. Same pinning as `timeline_frames`.
    pub fn timeline_duration_us(&self) -> u64 {
        if self.truncated {
            return self.duration_us;
        }
        match self.total_duration_us {
            Some(d) if d >= self.duration_us => d,
            _ => self.duration_us,
        }
    }

    /// Decoded (buffered) fraction of the timeline — the YouTube-style grey
    /// track behind the playhead. `None` while nothing is buffered.
    pub fn buffered_frac(&self) -> Option<f32> {
        let total = self.timeline_frames();
        if total <= 1 {
            return None;
        }
        Some((self.frames.len().saturating_sub(1) as f32 / (total - 1) as f32).clamp(0.0, 1.0))
    }
}

enum Slot {
    Loading,
    Ready(VideoClip, Vec<u64>),
    Failed(String),
    /// Probed a remote URL that turned out not to be video (image, HTML…).
    /// Distinct from `Failed` so callers can fall back to image/link UI
    /// instead of showing a video error card.
    NotVideo,
}

/// Marker error from the probe path (never surfaced in UI).
pub const NOT_VIDEO: &str = "not-video";

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
    /// Live download progress per remote key: (bytes received, total if
    /// known from the HEAD Content-Length). Drives the YouTube-style
    /// buffering bar while the clip streams in.
    progress: HashMap<String, (u64, Option<u64>)>,
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

    /// Append a progressive decode batch. Creates the Ready slot on the
    /// first batch (still `preview`), extends it while `finished` is false,
    /// and clears `preview` on the final batch. Returns `false` when there
    /// is nothing to extend (full clip already swapped in, or an error won
    /// the race) so forwarders can drop late batches.
    /// Totals (`total_frames` / `total_duration_us`) are set once from the
    /// first batch and kept — later batches carry the same values, and
    /// never shrink the denominator the scrub bar uses.
    pub fn append_batch(&mut self, key: &str, batch: DecodeBatch) -> bool {
        let preview = !batch.finished;
        match self.slots.get_mut(key) {
            Some(Slot::Loading) => {
                let clip = VideoClip {
                    frames: batch.frames,
                    width: batch.width,
                    height: batch.height,
                    duration_us: batch.duration_us,
                    fps: batch.fps,
                    truncated: batch.truncated,
                    preview,
                    total_frames: batch.total_frames,
                    total_duration_us: batch.total_duration_us,
                };
                self.slots.insert(key.to_string(), Slot::Ready(clip, batch.stamps));
                true
            }
            Some(Slot::Ready(clip, stamps)) if clip.preview => {
                clip.frames.extend(batch.frames);
                stamps.extend(batch.stamps);
                clip.width = batch.width;
                clip.height = batch.height;
                clip.duration_us = batch.duration_us;
                clip.fps = batch.fps;
                clip.truncated = batch.truncated;
                clip.preview = preview;
                // Keep the first-seen totals so the timeline stays stable
                // while batches stream in.
                if clip.total_frames.is_none() {
                    clip.total_frames = batch.total_frames;
                }
                if clip.total_duration_us.is_none() {
                    clip.total_duration_us = batch.total_duration_us;
                }
                true
            }
            _ => false,
        }
    }

    /// Clear the `preview` flag without adding frames — used when the decode
    /// thread disconnects after publishing a partial clip (error tail), so
    /// playback stops at the buffer end instead of stalling forever.
    pub fn finalize_preview(&mut self, key: &str) {
        if let Some(Slot::Ready(clip, _)) = self.slots.get_mut(key) {
            clip.preview = false;
        }
    }

    /// True while the slot is claimed but neither ready nor failed — the
    /// streaming/buffering window where the progress bar shows.
    pub fn is_loading(&self, key: &str) -> bool {
        matches!(self.slots.get(key), Some(Slot::Loading))
    }

    pub fn set_progress(&mut self, key: String, received: u64, total: Option<u64>) {
        self.progress.insert(key, (received, total));
    }

    pub fn progress_of(&self, key: &str) -> Option<(u64, Option<u64>)> {
        self.progress.get(key).copied()
    }

    /// True when the ready clip is a range-fetched preview still waiting on
    /// the full download — the player shows a buffering badge.
    pub fn is_preview(&self, key: &str) -> bool {
        matches!(self.slots.get(key), Some(Slot::Ready(clip, _)) if clip.preview)
    }

    /// Zero-timestamp containers (yscv reports all zeros) break pacing —
    /// anything under ~1ms/frame is bogus, so synthesize pacing across the
    /// whole clip. Prefers the container total (even spread over the true
    /// duration); falls back to 30fps. Called once by the progressive
    /// forwarder after the final batch lands (mirrors `decode_file`).
    pub fn normalize_timestamps(&mut self, key: &str) {
        if let Some(Slot::Ready(clip, stamps)) = self.slots.get_mut(key) {
            if stamps.last().copied().unwrap_or(0) < stamps.len() as u64 * 1_000 {
                let n = stamps.len();
                if n > 0 {
                    if let (Some(t), Some(d)) = (clip.total_frames, clip.total_duration_us) {
                        if t == n && d > 0 {
                            *stamps = (0..n)
                                .map(|i| {
                                    if n <= 1 {
                                        0
                                    } else {
                                        i as u64 * d / (n - 1) as u64
                                    }
                                })
                                .collect();
                            clip.duration_us = d;
                            clip.fps = n as f32 / (d as f32 / 1_000_000.0);
                            clip.fps = clip.fps.clamp(1.0, 120.0);
                            return;
                        }
                    }
                    *stamps = (0..n).map(|i| i as u64 * 33_333).collect();
                    clip.duration_us = stamps.last().copied().unwrap_or(0).max(1);
                    clip.fps = 30.0;
                }
            }
        }
    }

    /// Record that `key` (a probed remote URL) is not video. Future
    /// `begin` calls return `false` so the probe runs once.
    pub fn mark_not_video(&mut self, key: String) {
        self.slots.insert(key, Slot::NotVideo);
    }

    /// True once the probe concluded the URL is not video.
    pub fn is_not_video(&self, key: &str) -> bool {
        matches!(self.slots.get(key), Some(Slot::NotVideo))
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
    /// then 30fps). Captured once when playback starts. Prefers the stable
    /// container totals so pacing doesn't drift as batches stream in.
    pub fn frame_dt(&self, key: &str) -> Duration {
        match self.slots.get(key) {
            Some(Slot::Ready(clip, _)) => {
                if let (Some(t), Some(d)) = (clip.total_frames, clip.total_duration_us) {
                    if t > 1 && d > 0 {
                        return Duration::from_micros(d / t as u64).clamp(
                            Duration::from_millis(8),
                            Duration::from_secs(1),
                        );
                    }
                }
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

    /// Advance one frame iff still playing under `gen`. Returns `true` to
    /// keep the timer loop alive, `false` to stop it (paused, superseded,
    /// or end of a finished clip — which also pauses). At the end of a
    /// still-decoding (`preview`) buffer it stalls in place and returns
    /// `true`, so playback resumes as new batches land instead of stopping
    /// at the first chunk boundary.
    pub fn advance_if_active(&mut self, key: &str, gen: u64) -> bool {
        let (n, preview) = match self.slots.get(key) {
            Some(Slot::Ready(clip, _)) => (clip.frames.len(), clip.preview),
            _ => return false,
        };
        let p = self.plays.entry(key.to_string()).or_default();
        if !p.playing || p.generation != gen {
            return false;
        }
        if p.frame + 1 >= n {
            if preview {
                // Buffer exhausted but decode still running — stall one tick.
                return true;
            }
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

/// One progressive decode unit sent from the decode thread to the UI
/// forwarder. `finished` marks the last batch (full clip — clears the
/// `preview` flag); `truncated` mirrors the `MAX_FRAMES` cap.
/// `total_frames` / `total_duration_us` are the stable container totals
/// (sample-table count + moov duration), identical on every batch — the UI
/// uses them as the timeline denominator so the scrub bar doesn't grow per
/// chunk.
pub struct DecodeBatch {
    pub frames: Vec<Arc<RenderImage>>,
    pub stamps: Vec<u64>,
    pub width: u32,
    pub height: u32,
    pub duration_us: u64,
    pub fps: f32,
    pub finished: bool,
    pub truncated: bool,
    pub total_frames: Option<usize>,
    pub total_duration_us: Option<u64>,
}

/// Ship one progressive batch. Returns `false` when the UI went away
/// (stop decoding). `sent` tracks already-shipped frames for the running
/// fps estimate and is advanced by the shipped count.
fn ship_batch(
    tx: &std::sync::mpsc::SyncSender<DecodeBatch>,
    frames: &mut Vec<Arc<RenderImage>>,
    stamps: &mut Vec<u64>,
    sent: &mut usize,
    width: u32,
    height: u32,
    last_us: u64,
    finished: bool,
    truncated: bool,
    total_frames: Option<usize>,
    total_duration_us: Option<u64>,
) -> bool {
    if frames.is_empty() && !finished {
        return true;
    }
    let n = *sent + frames.len();
    let duration_us = stamps.last().copied().unwrap_or(last_us).max(1);
    let secs = duration_us as f32 / 1_000_000.0;
    let fps = if secs > 0.0 { n as f32 / secs } else { 30.0 }.clamp(1.0, 120.0);
    *sent += frames.len();
    tx.send(DecodeBatch {
        frames: std::mem::take(frames),
        stamps: std::mem::take(stamps),
        width,
        height,
        duration_us,
        fps,
        finished,
        truncated,
        total_frames,
        total_duration_us,
    })
    .is_ok()
}
/// Blocking progressive decode: MP4 → `DECODE_BATCH`-sized batches over
/// `tx`. The UI publishes each batch as it arrives, so the first frames
/// paint/play while the (slow pure-Rust software) decode of the remaining
/// ~300 frames still runs. Returns the final `(duration_us, fps,
/// truncated)` on success. A mid-stream error after ≥1 batch still returns
/// `Ok` — the partial clip stays playable; only a zero-frame decode
/// returns `Err`.
pub fn decode_file_batches(
    path: &Path,
    tx: std::sync::mpsc::SyncSender<DecodeBatch>,
) -> Result<(u64, f32, bool), String> {
    let mut reader =
        yscv_video::Mp4VideoReader::open(path).map_err(|e| format!("open failed: {e}"))?;
    // Stable timeline totals, known before a single frame decodes:
    // sample-table count (≈ frame count) + moov media duration.
    let container_samples = reader.nal_count();
    let container_duration = parse_mp4_duration(path);
    // Only advertise totals when the playable run covers the whole video.
    // Truncated (>MAX_FRAMES) clips play a capped preview — the UI keeps
    // the denominator on the decoded run with a "preview of first N" note.
    let total_frames = (container_samples > 0 && container_samples <= MAX_FRAMES)
        .then_some(container_samples);
    let total_duration_us = match (total_frames, container_duration) {
        (Some(_), Some(d)) if d > 0 => Some(d),
        _ => None,
    };
    let mut frames: Vec<Arc<RenderImage>> = Vec::new();
    let mut stamps: Vec<u64> = Vec::new();
    let mut dw = 0u32;
    let mut dh = 0u32;
    let mut truncated = false;
    // Frames already shipped in earlier batches.
    let mut sent = 0usize;
    // Best-effort pacing for the batches still in flight.
    let mut last_us = 0u64;
    loop {
        if sent + frames.len() >= MAX_FRAMES {
            truncated = true;
            break;
        }
        let next = reader.next_frame().map_err(|e| {
            // Mid-decode errors (corrupt tail) shouldn't nuke decoded
            // frames — the forwarder keeps the partial clip playable. Only
            // surface when nothing decoded at all.
            if sent + frames.len() == 0 {
                format!("decode: {e}")
            } else {
                truncated = true;
                String::new()
            }
        });
        let next = match next {
            Ok(n) => n,
            Err(e) if e.is_empty() => break,
            Err(e) => return Err(e),
        };
        let Some(f) = next else { break };
        if f.width == 0 || f.height == 0 || f.rgb8_data.is_empty() {
            continue;
        }
        last_us = f.timestamp_us;
        let (rgb, nw, nh) = downscale_rgb8(&f.rgb8_data, f.width, f.height, MAX_EDGE);
        let (w, h) = (nw as u32, nh as u32);
        dw = w;
        dh = h;
        if let Some(img) = rgb8_to_image(&rgb, w, h) {
            frames.push(img);
            stamps.push(f.timestamp_us);
        }
        if frames.len() >= DECODE_BATCH {
            if !ship_batch(
                &tx,
                &mut frames,
                &mut stamps,
                &mut sent,
                dw,
                dh,
                last_us,
                false,
                truncated,
                total_frames,
                total_duration_us,
            ) {
                return Ok((last_us.max(1), 30.0, true));
            }
        }
    }
    if sent + frames.len() == 0 {
        return Err("no frames decoded".to_string());
    }
    // Zero-timestamp containers (yscv reports all zeros) get fixed up by
    // the forwarder via `normalize_timestamps` after the final batch, so
    // per-batch stamps stay raw here.
    let total = sent + frames.len();
    let duration_us = stamps.last().copied().unwrap_or(last_us).max(1);
    let secs = duration_us as f32 / 1_000_000.0;
    let fps = if secs > 0.0 { total as f32 / secs } else { 30.0 }.clamp(1.0, 120.0);
    let _ = ship_batch(
        &tx,
        &mut frames,
        &mut stamps,
        &mut sent,
        dw,
        dh,
        last_us,
        true,
        truncated,
        total_frames,
        total_duration_us,
    );
    Ok((duration_us, fps, truncated))
}

/// MP4 container duration in microseconds, parsed from the `moov` box
/// (`trak/mdia/mdhd` of the video track, falling back to `mvhd`). Reads
/// only box headers + `moov` (capped), never the media payload — O(1) next
/// to the decode itself. `None` when the layout is unexpected.
pub fn parse_mp4_duration(path: &Path) -> Option<u64> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    // Locate `moov` by walking top-level box headers (skip `mdat` payload).
    let mut moov: Vec<u8> = Vec::new();
    let mut pos = 0u64;
    let mut header = [0u8; 8];
    while pos < file_size {
        file.seek(SeekFrom::Start(pos)).ok()?;
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
        let ty = &header[4..8];
        let mut hdr_len = 8u64;
        let real = if size == 1 {
            let mut ext = [0u8; 8];
            file.read_exact(&mut ext).ok()?;
            hdr_len = 16;
            u64::from_be_bytes(ext)
        } else if size == 0 {
            file_size - pos
        } else {
            size
        };
        if ty == b"moov" {
            let content = real.saturating_sub(hdr_len) as usize;
            // `moov` is metadata; cap to avoid pathological reads.
            if content == 0 || content > 64 * 1024 * 1024 {
                return None;
            }
            moov.resize(content, 0);
            file.read_exact(&mut moov).ok()?;
            break;
        }
        pos += real.max(hdr_len);
    }
    if moov.is_empty() {
        return None;
    }
    // Top-level boxes inside `moov`.
    let mut mvhd_us: Option<u64> = None;
    let mut video_mdhd_us: Option<u64> = None;
    let mut off = 0usize;
    while off + 8 <= moov.len() {
        let sz =
            u32::from_be_bytes([moov[off], moov[off + 1], moov[off + 2], moov[off + 3]]) as usize;
        let ty = &moov[off + 4..off + 8];
        if sz < 8 || off + sz > moov.len() {
            break;
        }
        let body = &moov[off + 8..off + sz];
        if ty == b"mvhd" {
            mvhd_us = parse_mvhd_or_mdhd(body);
        } else if ty == b"trak" && video_mdhd_us.is_none() {
            // Video track = contains `vmhd`, or a video sample entry.
            let is_video = body.windows(4).any(|w| {
                w == b"vmhd" || w == b"avc1" || w == b"hvc1" || w == b"hev1" || w == b"av01"
            });
            if is_video {
                video_mdhd_us = find_mdhd(body);
            }
        }
        off += sz.max(8);
    }
    // Prefer the video track's media duration; fall back to movie header.
    match (video_mdhd_us, mvhd_us) {
        (Some(d), _) if d > 0 => Some(d),
        (_, Some(d)) if d > 0 => Some(d),
        _ => None,
    }
}

/// `mvhd` / `mdhd` body → duration in microseconds.
fn parse_mvhd_or_mdhd(body: &[u8]) -> Option<u64> {
    if body.len() < 4 {
        return None;
    }
    let version = body[0];
    if version == 1 {
        if body.len() < 28 {
            return None;
        }
        let timescale = u32::from_be_bytes([body[20], body[21], body[22], body[23]]) as u64;
        let duration =
            u64::from_be_bytes(body[24..32].try_into().ok()?);
        timescale_to_us(timescale, duration)
    } else {
        if body.len() < 20 {
            return None;
        }
        let timescale = u32::from_be_bytes([body[12], body[13], body[14], body[15]]) as u64;
        let duration = u32::from_be_bytes([body[16], body[17], body[18], body[19]]) as u64;
        timescale_to_us(timescale, duration)
    }
}

/// `trak` body → its `mdia/mdhd` duration in microseconds, if present.
fn find_mdhd(trak: &[u8]) -> Option<u64> {
    // Find `mdia` child, then `mdhd` inside it.
    let mdia = find_child(trak, b"mdia")?;
    let mdhd = find_child(mdia, b"mdhd")?;
    parse_mvhd_or_mdhd(mdhd)
}

/// First direct child box with `tag` inside `parent` body.
fn find_child<'a>(parent: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= parent.len() {
        let sz = u32::from_be_bytes([
            parent[off],
            parent[off + 1],
            parent[off + 2],
            parent[off + 3],
        ]) as usize;
        if sz < 8 || off + sz > parent.len() {
            return None;
        }
        if &parent[off + 4..off + 8] == tag {
            return Some(&parent[off + 8..off + sz]);
        }
        off += sz.max(8);
    }
    None
}

fn timescale_to_us(timescale: u64, duration: u64) -> Option<u64> {
    if timescale == 0 || duration == 0 {
        return None;
    }
    duration.checked_mul(1_000_000)?.checked_div(timescale)
}

/// Blocking decode: MP4 → preview clip. Runs inside `background_spawn`.
/// Thin wrapper over `decode_file_batches` that collects every batch and
/// applies the zero-timestamp fixup across the whole clip.
pub fn decode_file(path: &Path) -> Result<(VideoClip, Vec<u64>), String> {
    let mut reader =
        yscv_video::Mp4VideoReader::open(path).map_err(|e| format!("open failed: {e}"))?;
    let container_samples = reader.nal_count();
    let container_duration = parse_mp4_duration(path);
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
            preview: false,
            total_frames: (container_samples > 0).then_some(container_samples),
            total_duration_us: container_duration.filter(|d| *d > 0),
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

/// Growing download target: the editor polls this path's size for the live
/// buffering bar while curl writes, then it is atomically renamed to the
/// final cache path. Keeps partial files from ever looking like a warm
/// cache on restart.
pub fn remote_part_path(url: &str) -> PathBuf {
    let mut p = remote_cache_path(url);
    let name = format!("{}.part", p.file_name().unwrap_or_default().to_string_lossy());
    p.set_file_name(name);
    p
}

/// Blocking HEAD (redirects followed) → Content-Length, if the host sends
/// one. Feeds the progress bar's total; `None` just shows received bytes.
pub fn fetch_remote_total(url: &str) -> Option<u64> {
    let out = std::process::Command::new("curl")
        .args(["--silent", "--show-error", "--location", "--head", "--max-time", "15", url])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&out.stdout);
    head.lines().rev().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if k.trim().eq_ignore_ascii_case("content-length") {
            v.trim().parse::<u64>().ok()
        } else {
            None
        }
    })
}

/// Streaming download: writes to the `.part` path (so the UI can poll its
/// size for a live buffering bar) and calls `on_progress(received, total)`
/// roughly every 100ms, then renames to the final cache path. Skips the
/// fetch when a non-empty file is cached. Deliberately no HEAD here — the
/// editor's poller already fetches the total in parallel; a blocking HEAD
/// on this path would sit on the download's critical path.
pub fn download_remote_streaming(
    url: &str,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<PathBuf, String> {
    let tmp = remote_cache_path(url);
    if tmp.exists() && tmp.metadata().map(|m| m.len()).unwrap_or(0) > 0 {
        return Ok(tmp);
    }
    let part = remote_part_path(url);
    if let Some(dir) = part.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    on_progress(0, None);
    let mut child = std::process::Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--max-time",
            "90",
            "--silent",
            "--show-error",
            url,
            "-o",
            &part.to_string_lossy(),
        ])
        .spawn()
        .map_err(|e| format!("download failed ({e}); is curl installed?"))?;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let received = part.metadata().map(|m| m.len()).unwrap_or(0);
        on_progress(received, None);
        match child.try_wait() {
            Ok(Some(status)) => {
                let received = part.metadata().map(|m| m.len()).unwrap_or(0);
                on_progress(received, None);
                if !status.success() {
                    // Best-effort stderr is gone with `spawn`; report status.
                    return Err(format!("download failed ({status})"));
                }
                break;
            }
            Ok(None) => {}
            Err(e) => return Err(format!("download wait failed ({e})")),
        }
    }
    // Atomic publish: partials never masquerade as a warm cache.
    if let Err(e) = std::fs::rename(&part, &tmp) {
        // Cross-device fallback: copy + remove.
        std::fs::copy(&part, &tmp).map_err(|e2| format!("cache publish failed ({e2}) after rename ({e})"))?;
        let _ = std::fs::remove_file(&part);
    }
    Ok(tmp)
}

/// `1.4 MB` / `820 KB` labels for the buffering row.
pub fn fmt_bytes(n: u64) -> String {
    const MB: u64 = 1024 * 1024;
    const KB: u64 = 1024;
    if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{} KB", n / KB)
    } else {
        format!("{n} B")
    }
}

/// Magic-byte sniff: MP4/MOV/M4V (`ftyp`), WebM/Matroska (EBML), Ogg
/// (`OggS`), AVI (`RIFF…AVI `). Extensionless URLs (GitHub assets) need
/// this — the tag/extension can't tell video from image.
pub fn is_video_bytes(b: &[u8]) -> bool {
    if b.len() < 12 {
        return false;
    }
    // MP4/MOV/M4V: `....ftyp`
    if &b[4..8] == b"ftyp" {
        return true;
    }
    // WebM/Matroska: EBML header `1A 45 DF A3`
    if b.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return true;
    }
    // Ogg (ogv): `OggS`
    if b.starts_with(b"OggS") {
        return true;
    }
    // AVI: `RIFF....AVI `
    if b.starts_with(b"RIFF") && &b[8..12] == b"AVI " {
        return true;
    }
    false
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
        assert!(store.is_loading("a"));
        store.finish("a".to_string(), Err("boom".to_string()));
        assert_eq!(store.error("a").as_deref(), Some("boom"));
        assert!(!store.is_loading("a"));
    }

    #[test]
    fn stream_progress_and_preview_flags() {
        let mut store = VideoStore::default();
        store.begin("s".to_string());
        assert!(store.progress_of("s").is_none());
        store.set_progress("s".to_string(), 512, Some(1024));
        assert_eq!(store.progress_of("s"), Some((512, Some(1024))));
        assert!(!store.is_preview("s"));
    }

    #[test]
    fn stream_cache_paths_stay_distinct() {
        let url = "https://example.com/assets/12345";
        let final_p = remote_cache_path(url);
        let part = remote_part_path(url);
        assert_ne!(final_p, part);
        // Partials never masquerade as the final cache name.
        assert!(part.extension().and_then(|e| e.to_str()).unwrap_or("").contains("part"));
    }

    #[test]
    fn progressive_batches_assemble_full_clip() {
        let path = std::path::Path::new("examples/random_small.mp4");
        if !path.exists() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        let (duration_us, _fps, truncated) =
            decode_file_batches(path, tx).expect("batched decode");
        let mut batches = 0usize;
        let mut frames = 0usize;
        let mut finished = false;
        while let Ok(b) = rx.try_recv() {
            batches += 1;
            frames += b.frames.len();
            assert_eq!(b.frames.len(), b.stamps.len());
            if b.finished {
                finished = true;
            }
        }
        assert!(finished, "terminal batch must set finished");
        assert!(frames > 0);
        // More than one batch for any clip longer than DECODE_BATCH, and
        // the assembled frame count matches the one-shot decode.
        let (clip, _) = decode_file(path).expect("one-shot decode");
        assert_eq!(frames, clip.frames.len(), "batches must sum to the full clip");
        assert!(batches >= 1);
        if clip.frames.len() > DECODE_BATCH {
            assert!(batches > 1, "long clips must arrive progressively");
        }
        assert_eq!(truncated, clip.truncated);
        assert!(duration_us > 0);
    }

    #[test]
    fn append_batch_grows_preview_then_completes() {
        let path = std::path::Path::new("examples/random_small.mp4");
        if !path.exists() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        decode_file_batches(path, tx).expect("batched decode");
        let mut store = VideoStore::default();
        store.begin("g".to_string());
        let mut saw_preview = false;
        // Timeline denominator must stay pinned from the first batch —
        // later chunks extend the buffered run, never the total.
        let mut first_total: Option<usize> = None;
        let mut first_dur: Option<u64> = None;
        while let Ok(b) = rx.try_recv() {
            let finished = b.finished;
            assert!(store.append_batch("g", b));
            let (clip, stamps) = store.get("g").expect("grown clip");
            assert_eq!(clip.frames.len(), stamps.len());
            if first_total.is_none() {
                first_total = clip.total_frames;
                first_dur = clip.total_duration_us;
            } else {
                assert_eq!(clip.total_frames, first_total, "total frames must not drift");
                assert_eq!(clip.total_duration_us, first_dur, "total duration must not drift");
            }
            // Buffered fraction grows monotonically toward 1.0.
            if let Some(bf) = clip.buffered_frac() {
                assert!((0.0..=1.0).contains(&bf));
            }
            if !finished {
                assert!(store.is_preview("g"), "in-flight clip stays preview");
                saw_preview = true;
            }
        }
        assert!(saw_preview || store.get("g").is_some());
        assert!(!store.is_preview("g"), "final batch clears preview");
        store.normalize_timestamps("g");
        let (clip, stamps) = store.get("g").expect("grown clip");
        assert_eq!(clip.frames.len(), stamps.len());
        assert!(clip.duration_us > 0);
        // Stable timeline: denominator equals the decoded run on completion.
        assert_eq!(clip.timeline_frames(), clip.frames.len());
    }

    #[test]
    fn container_probe_gives_stable_timeline() {
        let path = std::path::Path::new("examples/random_small.mp4");
        if !path.exists() {
            return;
        }
        let reader = yscv_video::Mp4VideoReader::open(path).expect("open sample");
        let samples = reader.nal_count();
        assert!(samples > 0);
        let dur = parse_mp4_duration(path).expect("moov duration");
        assert!(dur > 0);
        let (clip, _) = decode_file(path).expect("one-shot decode");
        // One-shot clips carry the same totals the progressive path uses.
        assert_eq!(clip.total_frames, Some(samples));
        assert_eq!(clip.total_duration_us, Some(dur));
        assert_eq!(clip.timeline_frames(), clip.frames.len());
        assert_eq!(clip.timeline_duration_us(), dur);
        // Buffered fraction is complete once decoded.
        assert!((clip.buffered_frac().unwrap_or(0.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn playback_stalls_at_buffer_end_while_preview() {
        let mut store = VideoStore::default();
        store.begin("p".to_string());
        // Fake a 2-batch progressive clip: first batch preview, 12 frames.
        let mk = |n: usize, finished: bool| DecodeBatch {
            frames: vec![rgb8_to_image(&vec![1u8; 4 * 2 * 3], 4, 2).unwrap(); n],
            stamps: (0..n).map(|i| i as u64 * 33_333).collect(),
            width: 4,
            height: 2,
            duration_us: n as u64 * 33_333,
            fps: 30.0,
            finished,
            truncated: false,
            total_frames: Some(24),
            total_duration_us: Some(24 * 33_333),
        };
        assert!(store.append_batch("p", mk(12, false)));
        assert!(store.is_preview("p"));
        // Play to the buffer end: the loop must stall (true), not stop.
        let (_, gen) = store.toggle("p");
        for _ in 0..11 {
            assert!(store.advance_if_active("p", gen));
        }
        // Frame 11 is the last buffered; next tick stalls, keeps playing.
        assert!(store.advance_if_active("p", gen));
        let (playing, frame) = store.play_state("p");
        assert!(playing, "stall must not pause");
        assert_eq!(frame, 11);
        // Second batch completes the clip → playback runs to the end, then pauses.
        assert!(store.append_batch("p", mk(12, true)));
        assert!(!store.is_preview("p"));
        // From 11, twelve advances reach 23 (last); the next tick pauses.
        for _ in 0..12 {
            assert!(store.advance_if_active("p", gen));
        }
        let (playing, frame) = store.play_state("p");
        assert!(playing && frame == 23);
        assert!(!store.advance_if_active("p", gen));
        let (playing, _) = store.play_state("p");
        assert!(!playing, "finished clip pauses at the end");
    }

    #[test]
    fn fmt_bytes_labels() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(2048), "2 KB");
        assert_eq!(fmt_bytes(1_470_000), "1.4 MB");
    }

    #[test]
    fn fmt_time_labels() {
        assert_eq!(fmt_time(3_500_000), "0:03");
        assert_eq!(fmt_time(75_000_000), "1:15");
    }

    #[test]
    fn video_magic_sniff() {
        // MP4 ftyp
        let mut mp4 = vec![0u8; 16];
        mp4[4..8].copy_from_slice(b"ftyp");
        assert!(is_video_bytes(&mp4));
        // WebM EBML
        assert!(is_video_bytes(&[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0, 0, 0, 0, 0]));
        // Ogg
        assert!(is_video_bytes(b"OggS012345678"));
        // AVI
        assert!(is_video_bytes(b"RIFF1234AVI "));
        // PNG / HTML / tiny inputs are not video
        assert!(!is_video_bytes(b"\x89PNG\r\n\x1a\n0000"));
        assert!(!is_video_bytes(b"<!doctype html......"));
        assert!(!is_video_bytes(b"short"));
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
