// src-tauri/src/models.rs
// Core data model for PassThrough-Cut.
// Clip is the atomic unit on the timeline.

use serde::{Deserialize, Serialize};

/// A single segment of source media on the timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    /// Absolute path to the source file.
    pub source_path: String,
    /// Trim in-point in seconds (fractional, e.g. 3.541).
    pub start_time: f64,
    /// Trim out-point in seconds.
    pub end_time: f64,
    /// Track layer (0 = primary video track).
    pub z_index: u32,
    /// Optional clip label shown in the UI.
    pub label: Option<String>,
}

impl Clip {
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }
}

/// The ordered, single-track timeline passed from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    /// Clips in display/playback order.
    pub clips: Vec<Clip>,
}

/// User-facing export configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSettings {
    /// Absolute path for the output file (must end in .mp4 or .mkv).
    pub output_path: String,
    /// If true, force re-encode at every cut regardless of keyframe alignment.
    /// Useful for frame-perfect trim on variable GOP sources.
    pub force_smart_cut: bool,
    /// If true, optimizes the export for gapless looping (passthrough FPS).
    pub optimize_for_looping: bool,
    /// If true, uses the AI diagnostic to automatically trim the end point
    /// to the best matching loop frame.
    pub auto_trim_loop: bool,
    /// Override container. Defaults to "mp4".
    pub container: Option<String>,
}

/// Progress update emitted to the frontend during export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub stage: String,
    pub percent: f32,
    pub message: String,
}

/// Waveform data returned to the frontend for a single clip.
/// `samples` contains normalised peak amplitudes in [0.0, 1.0],
/// one value per pixel-column at BASE_ZOOM_PX_PER_SEC (100 px/s).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveformData {
    pub clip_id: String,
    pub samples: Vec<f32>,
}
