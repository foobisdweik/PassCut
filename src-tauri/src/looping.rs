// src-tauri/src/looping.rs
// Diagnostic tools for gapless looping.
// Uses MSE (Mean Squared Error) to find the best match between the first and last frames.

use crate::probe::{ffmpeg_bin, make_cmd};
use anyhow::{Context, Result};
use image::{GenericImageView, ImageReader, Luma, RgbImage};
use std::io::Cursor;
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

/// Find the best looping point by comparing the first frame of the video
/// with a range of frames at the end.
/// Returns the timestamp of the best matching end frame.
pub fn find_loop_point(source: &str, search_duration: f64) -> Result<f64> {
    // 1. Extract the first frame as a reference.
    let ref_frame_bytes = extract_single_frame(source, 0.0)?;
    let ref_img = load_and_preprocess(&ref_frame_bytes)?;

    // 2. Extract frames from the end.
    let tmp = TempDir::new()?;
    let out_pattern = tmp.path().join("frame_%04d.jpg");
    
    // Determine start time for search.
    let duration = crate::commands::probe_duration(source)?;
    let search_start = (duration - search_duration).max(0.0);

    // Extract frames using FFmpeg to a temporary directory.
    let status = make_cmd(&ffmpeg_bin())
        .args([
            "-y",
            "-ss", &format!("{search_start:.6}"),
            "-i", source,
            "-vf", "scale=128:128", // Downscale for faster comparison
            "-q:v", "4",
            out_pattern.to_str().unwrap(),
        ])
        .status()?;

    if !status.success() {
        anyhow::bail!("FFmpeg failed to extract search frames for looping diagnostic.");
    }

    // 3. Compare each extracted frame with the reference frame.
    let mut best_timestamp = duration;
    let mut min_mse = f64::MAX;

    // We need to know the timestamps of the extracted frames.
    // FFmpeg extracts them sequentially starting from search_start.
    // We'll estimate timestamps based on the index and FPS.
    let fps = crate::engine::detect_fps(source).unwrap_or(30.0);
    let frame_duration = 1.0 / fps;

    let mut i = 1;
    loop {
        let frame_path = tmp.path().join(format!("frame_{:04}.jpg", i));
        if !frame_path.exists() {
            break;
        }

        let frame_bytes = std::fs::read(&frame_path)?;
        let frame_img = load_and_preprocess(&frame_bytes)?;
        
        let mse = calculate_mse(&ref_img, &frame_img);
        let current_ts = search_start + (i as f64 - 1.0) * frame_duration;

        if mse < min_mse {
            min_mse = mse;
            best_timestamp = current_ts;
        }

        i += 1;
    }

    Ok(best_timestamp)
}

fn extract_single_frame(source: &str, timestamp: f64) -> Result<Vec<u8>> {
    let out = make_cmd(&ffmpeg_bin())
        .args([
            "-y",
            "-ss", &format!("{timestamp:.6}"),
            "-i", source,
            "-frames:v", "1",
            "-vf", "scale=128:128", // Match the search frame scale
            "-f", "image2",
            "-vcodec", "mjpeg",
            "pipe:1"
        ])
        .output()?;
    Ok(out.stdout)
}

fn load_and_preprocess(bytes: &[u8]) -> Result<RgbImage> {
    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?
        .to_rgb8();
    Ok(img)
}

fn calculate_mse(img1: &RgbImage, img2: &RgbImage) -> f64 {
    let mut total_error = 0.0;
    let (w1, h1) = img1.dimensions();
    let (w2, h2) = img2.dimensions();
    
    // Ensure they are same size (should be due to scale filter)
    let w = w1.min(w2);
    let h = h1.min(h2);

    for y in 0..h {
        for x in 0..w {
            let p1 = img1.get_pixel(x, y);
            let p2 = img2.get_pixel(x, y);
            
            for c in 0..3 {
                let diff = p1[c] as f64 - p2[c] as f64;
                total_error += diff * diff;
            }
        }
    }

    total_error / (w as f64 * h as f64 * 3.0)
}
