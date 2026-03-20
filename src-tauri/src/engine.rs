// src-tauri/src/engine.rs
// PassThrough-Cut export engine.
//
// Pipeline per clip:
//   1. Probe whether start/end cut points fall on I-frame boundaries.
//   2a. If BOTH are aligned → -c copy segment (zero re-encode).
//   2b. If start is unaligned → re-encode from nearest preceding I-frame to
//       the cut point, then stream-copy the remaining tail.
//   2c. If end is unaligned → stream-copy from start to the nearest following
//       I-frame, re-encode the tail.
//   3. Write each segment to a temp file.
//   4. Concat all segments via FFmpeg concat demuxer (no re-encode).
//   5. Map metadata from clip[0] onto the final container.

use crate::models::{Clip, ExportSettings, ProgressEvent, Timeline};
use crate::probe::{ffmpeg_bin, is_keyframe_aligned, make_cmd, nearest_preceding_keyframe, probe_streams};
use anyhow::{Context, Result};
use log::{debug, info, warn};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use tempfile::TempDir;
use uuid::Uuid;

/// Emits a progress event to the frontend via Tauri event system.
fn emit_progress(app: &AppHandle, stage: &str, percent: f32, msg: &str) {
    let _ = app.emit(
        "export-progress",
        ProgressEvent {
            stage: stage.into(),
            percent,
            message: msg.into(),
        },
    );
}

/// Top-level export entry point.
/// Called by the Tauri command `export_sequence`.
pub fn export_sequence(
    app: AppHandle,
    timeline: Timeline,
    settings: ExportSettings,
) -> Result<String> {
    if timeline.clips.is_empty() {
        anyhow::bail!("Timeline is empty — nothing to export.");
    }

    // Working directory for all intermediate files.
    let tmp = TempDir::new().context("Failed to create temp directory")?;
    let tmp_path = tmp.path().to_path_buf();

    emit_progress(&app, "prepare", 0.0, "Analysing clips…");

    let total = timeline.clips.len() as f32;
    let mut segment_files: Vec<PathBuf> = Vec::new();

    for (i, clip) in timeline.clips.iter().enumerate() {
        let pct_base = (i as f32 / total) * 85.0;
        emit_progress(
            &app,
            "segment",
            pct_base,
            &format!("Processing clip {}/{}", i + 1, timeline.clips.len()),
        );

        let segments = process_clip(clip, &tmp_path, settings.force_smart_cut)
            .with_context(|| format!("Failed to process clip {i}: {}", clip.source_path))?;

        segment_files.extend(segments);
    }

    emit_progress(&app, "concat", 87.0, "Concatenating segments…");

    let concat_list = write_concat_list(&segment_files, &tmp_path)?;
    let output = concat_segments(
        &concat_list,
        &settings.output_path,
        &timeline.clips[0].source_path, // metadata donor
    )?;

    emit_progress(&app, "done", 100.0, "Export complete.");

    // tmp is dropped here, cleaning up all intermediate files.
    info!("Export complete: {output}");
    Ok(output)
}

/// Process a single clip and return a list of segment file paths.
/// Implements smart-cut: re-encode unaligned boundaries, stream-copy the rest.
fn process_clip(clip: &Clip, tmp: &Path, force_smart_cut: bool) -> Result<Vec<PathBuf>> {
    let fps = detect_fps(&clip.source_path).unwrap_or(30.0);
    let start = clip.start_time;
    let end = clip.end_time;

    let start_aligned = !force_smart_cut && is_keyframe_aligned(&clip.source_path, start, fps)?;
    let end_aligned = !force_smart_cut && is_keyframe_aligned(&clip.source_path, end, fps)?;

    debug!(
        "Clip '{}' [{:.3}→{:.3}] start_aligned={start_aligned} end_aligned={end_aligned}",
        clip.source_path, start, end
    );

    if start_aligned && end_aligned {
        // ── Fast path: pure stream copy ──────────────────────────────────────
        let out = tmp_file(tmp, "copy_full");
        stream_copy_segment(&clip.source_path, start, end, &out)?;
        Ok(vec![out])
    } else {
        // ── Smart-cut path ──────────────────────────────────────────────────
        let mut segments = Vec::new();
        let mut current_pos = start;

        // 1. Re-encode the head if start is unaligned.
        // We re-encode from 'start' up to the next following keyframe.
        if !start_aligned {
            if let Some(next_kf) = nearest_following_keyframe(&clip.source_path, start, fps)? {
                if next_kf < end - 0.01 {
                    let out = tmp_file(tmp, "reenc_head");
                    reencode_segment(&clip.source_path, start, next_kf, &out)?;
                    segments.push(out);
                    current_pos = next_kf;
                } else {
                    // Next keyframe is beyond the end, re-encode the entire clip.
                    let out = tmp_file(tmp, "reenc_all");
                    reencode_segment(&clip.source_path, start, end, &out)?;
                    return Ok(vec![out]);
                }
            } else {
                // No following keyframe found, re-encode to the end.
                let out = tmp_file(tmp, "reenc_all");
                reencode_segment(&clip.source_path, start, end, &out)?;
                return Ok(vec![out]);
            }
        }

        // 2. Determine the boundary for the middle copy and the tail re-encode.
        if !end_aligned {
            // Find the keyframe immediately preceding the end point.
            if let Some(prev_kf) = nearest_preceding_keyframe(&clip.source_path, end, 10.0)? {
                if prev_kf > current_pos + 0.01 {
                    // Middle section: stream-copy from current_pos to prev_kf.
                    let copy_out = tmp_file(tmp, "copy_mid");
                    stream_copy_segment(&clip.source_path, current_pos, prev_kf, &copy_out)?;
                    segments.push(copy_out);
                    current_pos = prev_kf;
                }
                
                // Tail section: re-encode from current_pos to end.
                let reenc_tail = tmp_file(tmp, "reenc_tail");
                reencode_segment(&clip.source_path, current_pos, end, &reenc_tail)?;
                segments.push(reenc_tail);
            } else {
                // No preceding keyframe for end; re-encode remaining from current_pos.
                let out = tmp_file(tmp, "reenc_tail");
                reencode_segment(&clip.source_path, current_pos, end, &out)?;
                segments.push(out);
            }
        } else {
            // End is aligned, stream-copy the remainder.
            if end > current_pos + 0.01 {
                let out = tmp_file(tmp, "copy_tail");
                stream_copy_segment(&clip.source_path, current_pos, end, &out)?;
                segments.push(out);
            }
        }

        Ok(segments)
    }
}

/// Stream-copy a time range from source into a temp MP4.
/// -avoid_negative_ts make_zero fixes DTS discontinuities between clips.
fn stream_copy_segment(source: &str, start: f64, end: f64, out: &Path) -> Result<()> {
    let status = make_cmd(&ffmpeg_bin())
        .args([
            "-y",
            "-ss", &format!("{start:.6}"),
            "-to", &format!("{end:.6}"),
            "-i", source,
            "-c", "copy",
            "-map", "0",
            "-vsync", "0",
            "-avoid_negative_ts", "make_zero",
            "-ignore_unknown",
            "-movflags", "+faststart",
            out.to_str().unwrap(),
        ])
        .status()
        .context("Failed to spawn ffmpeg for stream copy")?;

    if !status.success() {
        anyhow::bail!("ffmpeg stream copy failed for segment [{start:.3}→{end:.3}] of {source}");
    }
    Ok(())
}

/// Re-encode a small segment (only the GOP at the cut boundary).
/// Prioritises hardware encoders to prevent CPU bottlenecks and keep VRAM busy.
fn reencode_segment(source: &str, start: f64, end: f64, out: &Path) -> Result<()> {
    // We use very high quality (CRF 14) + no B-frames to stay frame-accurate.
    // This segment is typically < 2 seconds (one GOP), so speed is not a concern.

    // Try to detect the source codec to match it (critical for concat demuxer)
    let is_hevc = probe_streams(source)
        .unwrap_or_default()
        .iter()
        .any(|s| s.codec_name == "hevc");

    // Choose the best encoder for the current platform and codec
    let (vcodec, extra_args) = if cfg!(windows) {
        if is_hevc {
            ("hevc_nvenc", vec!["-cq", "14", "-preset", "p4", "-bf", "0"])
        } else {
            ("h264_nvenc", vec!["-cq", "14", "-preset", "p4", "-bf", "0"])
        }
    } else if cfg!(target_os = "macos") {
        if is_hevc {
            ("hevc_videotoolbox", vec!["-q:v", "60", "-bf", "0"])
        } else {
            ("h264_videotoolbox", vec!["-q:v", "60", "-bf", "0"])
        }
    } else if cfg!(target_os = "linux") {
        if is_hevc {
            ("hevc_vaapi", vec!["-qp", "14", "-bf", "0"])
        } else {
            ("h264_vaapi", vec!["-qp", "14", "-bf", "0"])
        }
    } else {
        if is_hevc {
            ("libx265", vec!["-crf", "14", "-preset", "fast", "-x265-params", "bframes=0:keyint=1"])
        } else {
            ("libx264", vec!["-crf", "14", "-preset", "fast", "-x264-params", "bframes=0:keyint=1"])
        }
    };
    let start_str = format!("{start:.6}");
    let end_str = format!("{end:.6}");
    
    let mut args = vec![
        "-y",
        "-ss", &start_str,
        "-to", &end_str,
        "-i", source,
        "-c:v", vcodec,
    ];
    args.extend(extra_args);
    args.extend([
        "-c:a", "copy",
        "-vsync", "0",
        "-avoid_negative_ts", "make_zero",
        "-movflags", "+faststart",
        out.to_str().unwrap(),
    ]);

    let status = make_cmd(&ffmpeg_bin())
        .args(&args)
        .status()
        .context("Failed to spawn ffmpeg for re-encode")?;

    // If hardware encoding fails, fallback to libx264
    if !status.success() && vcodec != "libx264" {
        warn!("Hardware encode failed ({vcodec}), falling back to libx264");
        let fallback_status = make_cmd(&ffmpeg_bin())
            .args([
                "-y",
                "-ss", &format!("{start:.6}"),
                "-to", &format!("{end:.6}"),
                "-i", source,
                "-c:v", "libx264",
                "-crf", "14",
                "-preset", "fast",
                "-x264-params", "bframes=0:keyint=1",
                "-c:a", "copy",
                "-vsync", "0",
                "-avoid_negative_ts", "make_zero",
                "-movflags", "+faststart",
                out.to_str().unwrap(),
            ])
            .status()
            .context("Failed to spawn fallback ffmpeg for re-encode")?;

        if !fallback_status.success() {
            anyhow::bail!(
                "ffmpeg fallback re-encode failed for GOP segment [{start:.3}→{end:.3}] of {source}"
            );
        }
    } else if !status.success() {
        anyhow::bail!(
            "ffmpeg re-encode failed for GOP segment [{start:.3}→{end:.3}] of {source}"
        );
    }
    
    Ok(())
}

/// Concatenate all segment files using the FFmpeg concat demuxer (no re-encode).
/// Maps metadata from `metadata_source` (first original clip) onto the output.
fn concat_segments(
    concat_list: &Path,
    output_path: &str,
    metadata_source: &str,
) -> Result<String> {
    let status = make_cmd(&ffmpeg_bin())
        .args([
            "-y",
            "-f", "concat",
            "-safe", "0",
            "-i", concat_list.to_str().unwrap(),
            // Second input solely for metadata passthrough.
            "-i", metadata_source,
            "-c", "copy",
            "-vsync", "0",
            "-map_metadata", "1",   // pull global metadata from metadata_source
            "-map", "0",            // video/audio from concat
            "-ignore_unknown",
            "-avoid_negative_ts", "make_zero",
            "-fflags", "+genpts",
            "-write_tmcd", "0",
            "-movflags", "+faststart",
            output_path,
        ])
        .status()
        .context("Failed to spawn ffmpeg for final concat")?;

    if !status.success() {
        anyhow::bail!("ffmpeg concat step failed. Check concat list: {}", concat_list.display());
    }

    Ok(output_path.to_string())
}

/// Write an FFmpeg concat demuxer input list.
///
/// Format:
/// ```
/// ffconcat version 1.0
/// file '/abs/path/to/seg0.mp4'
/// file '/abs/path/to/seg1.mp4'
/// ```
fn write_concat_list(files: &[PathBuf], tmp: &Path) -> Result<PathBuf> {
    let list_path = tmp.join("concat.txt");
    let mut f = fs::File::create(&list_path).context("Failed to create concat list")?;

    writeln!(f, "ffconcat version 1.0")?;
    for seg in files {
        writeln!(f, "file '{}'", seg.to_str().unwrap())?;
    }
    Ok(list_path)
}

/// Generate a unique temp file path with the given tag prefix.
fn tmp_file(tmp: &Path, tag: &str) -> PathBuf {
    tmp.join(format!("{tag}_{}.mp4", Uuid::new_v4()))
}

/// Detect the framerate of a source file's primary video stream.
/// Returns 30.0 as a safe fallback.
pub fn detect_fps(source: &str) -> Result<f64> {
    let streams = probe_streams(source)?;
    let video = streams
        .iter()
        .find(|s| s.codec_type == "video")
        .ok_or_else(|| anyhow::anyhow!("No video stream found in {source}"))?;

    let rate_str = video
        .r_frame_rate
        .as_deref()
        .or(video.avg_frame_rate.as_deref())
        .unwrap_or("30/1");

    parse_rate_fraction(rate_str)
}

/// Parse "num/den" fraction strings produced by ffprobe.
fn parse_rate_fraction(s: &str) -> Result<f64> {
    let mut parts = s.split('/');
    let num: f64 = parts
        .next()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(30.0);
    let den: f64 = parts
        .next()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(1.0);

    if den == 0.0 {
        return Ok(30.0);
    }
    Ok(num / den)
}

/// Return the nearest I-frame at or after `time` within a short search window.
fn nearest_following_keyframe(source: &str, time: f64, fps: f64) -> Result<Option<f64>> {
    use crate::probe::get_keyframes;
    // Search up to one GOP ahead (typically < 2s for normal content).
    let keyframes = get_keyframes(source, time, time + 4.0)?;
    let frame_duration = if fps > 0.0 { 1.0 / fps } else { 0.042 };
    let following = keyframes
        .into_iter()
        .find(|&kf| kf >= time - frame_duration * 0.5);
    Ok(following)
}
