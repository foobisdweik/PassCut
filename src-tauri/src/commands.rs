// src-tauri/src/commands.rs
// Tauri IPC commands exposed to the Svelte frontend.

use crate::engine::{detect_fps, export_sequence};
use crate::models::{ExportSettings, Timeline, WaveformData};
use crate::preview::extract_frame;
use crate::probe::{ffprobe_bin, make_cmd, probe_streams};
use crate::waveform::{extract_waveform, samples_for_duration};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Metadata returned to the frontend when a file is opened.
#[derive(Debug, Serialize, Deserialize)]
pub struct MediaInfo {
    pub path: String,
    pub duration: f64,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub has_audio: bool,
}

/// Probe a media file and return its metadata for the UI.
#[tauri::command]
pub async fn probe_media(path: String) -> Result<MediaInfo, String> {
    let streams = probe_streams(&path).map_err(|e| e.to_string())?;
    let fps = detect_fps(&path).unwrap_or(30.0);

    let video = streams.iter().find(|s| s.codec_type == "video");
    let has_audio = streams.iter().any(|s| s.codec_type == "audio");

    // Width/height are now included in ProbeStream (via -show_entries in
    // probe_streams), so no second ffprobe call is needed for resolution.
    let width  = video.and_then(|v| v.width).unwrap_or(0);
    let height = video.and_then(|v| v.height).unwrap_or(0);
    let duration = probe_duration(&path).unwrap_or(0.0);

    let info = MediaInfo {
        path: path.clone(),
        duration,
        fps,
        width,
        height,
        codec: video
            .map(|v| v.codec_name.clone())
            .unwrap_or_else(|| "unknown".into()),
        has_audio,
    };

    Ok(info)
}

/// Export the timeline to a single output file.
/// Emits "export-progress" events during processing.
#[tauri::command]
pub async fn export_timeline(
    app: AppHandle,
    timeline: Timeline,
    settings: ExportSettings,
) -> Result<String, String> {
    export_sequence(app, timeline, settings).map_err(|e| e.to_string())
}

/// Return the list of keyframe timestamps in a source file within a time window.
/// Used by the frontend to snap trim handles to I-frames.
#[tauri::command]
pub async fn get_keyframe_times(
    path: String,
    range_start: f64,
    range_end: f64,
) -> Result<Vec<f64>, String> {
    crate::probe::get_keyframes(&path, range_start, range_end).map_err(|e| e.to_string())
}

/// Generate a downsampled waveform for the given clip.
/// Returns normalised peak amplitudes (one per pixel at base zoom).
/// `clip_id` is a frontend-only UUID echoed back so the store can cache by id.
/// `duration` is the full source duration (not the trimmed clip duration) so the
/// entire file is represented; the frontend trims the visible region via CSS.
#[tauri::command]
pub async fn generate_waveform(
    clip_id: String,
    path: String,
    duration: f64,
) -> Result<WaveformData, String> {
    let num_samples = samples_for_duration(duration);
    let samples = extract_waveform(&path, num_samples).map_err(|e| e.to_string())?;
    Ok(WaveformData { clip_id, samples })
}

/// Extract a single JPEG frame at `timestamp` from `path` using hardware decode.
/// Returns a base64-encoded JPEG string for direct use as an img src.
#[tauri::command]
pub async fn preview_frame(path: String, timestamp: f64) -> Result<String, String> {
    let bytes = extract_frame(&path, timestamp).map_err(|e| e.to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", STANDARD.encode(&bytes)))
}

/// AI-assisted loop diagnostic: find the best frame at the end of the video
/// that matches the beginning of the video for a seamless loop.
#[tauri::command]
pub async fn suggest_loop_point(path: String, search_duration: f64) -> Result<f64, String> {
    crate::looping::find_loop_point(&path, search_duration).map_err(|e| e.to_string())
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

pub fn probe_duration(source: &str) -> Result<f64> {
    let out = make_cmd(&ffprobe_bin())
        .args([
            "-v", "quiet",
            "-show_entries", "format=duration",
            "-of", "csv=p=0",
            source,
        ])
        .output()?;

    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.trim().parse().unwrap_or(0.0))
}
