// src-tauri/src/preview.rs
// Hardware-accelerated frame extraction for the preview panel.
//
// Decode strategy (in priority order):
//   Windows: -hwaccel d3d12va  (RTX 3070 / any D3D12 adapter)
//   Linux:   -hwaccel vaapi -vaapi_device /dev/dri/renderD128
//   Fallback: software decode if hwaccel init fails
//
// Returns raw JPEG bytes suitable for base64-encoding in the IPC layer.

use crate::probe::{ffmpeg_bin, make_cmd};
use anyhow::{Context, Result};

/// Extract a single JPEG frame at `timestamp` seconds from `source`.
/// Tries hardware acceleration first, falls back to software on failure.
pub fn extract_frame(source: &str, timestamp: f64) -> Result<Vec<u8>> {
    // Try hwaccel path first.
    match extract_frame_hwaccel(source, timestamp) {
        Ok(bytes) if !bytes.is_empty() => return Ok(bytes),
        _ => {}
    }
    extract_frame_software(source, timestamp)
}

fn hwaccel_args() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["-hwaccel", "d3d12va", "-hwaccel_output_format", "d3d12"]
    } else if cfg!(target_os = "linux") {
        vec!["-hwaccel", "vaapi", "-vaapi_device", "/dev/dri/renderD128",
             "-hwaccel_output_format", "vaapi"]
    } else {
        vec![]
    }
}

fn extract_frame_hwaccel(source: &str, timestamp: f64) -> Result<Vec<u8>> {
    let hw = hwaccel_args();
    if hw.is_empty() {
        anyhow::bail!("no hwaccel on this platform");
    }
    let ts = format!("{timestamp:.6}");
    let mut args: Vec<&str> = hw;
    args.extend(["-ss", &ts, "-i", source, "-frames:v", "1"]);

    // Use hardware encoder if possible, dropping hwdownload entirely.
    if cfg!(windows) {
        // For Windows (D3D12/NVIDIA/Intel), we can usually use standard mjpeg or let it auto-negotiate, 
        // but for true zero-copy out of D3D12 surface, we need the hardware mjpeg encoder if available.
        // As a safe intermediate that still avoids a full CPU format conversion:
        args.extend(["-vf", "hwdownload,format=nv12", "-c:v", "mjpeg", "-f", "image2", "-q:v", "4", "pipe:1"]); 
    } else if cfg!(target_os = "linux") {
        args.extend(["-c:v", "mjpeg_vaapi", "-f", "image2", "pipe:1"]);
    }

    let out = make_cmd(&ffmpeg_bin())
        .args(&args)
        .output()
        .context("hwaccel frame extract spawn failed")?;

    if out.stdout.is_empty() { anyhow::bail!("empty stdout"); }
    Ok(out.stdout)
}

fn extract_frame_software(source: &str, timestamp: f64) -> Result<Vec<u8>> {
    let ts = format!("{timestamp:.6}");
    let out = make_cmd(&ffmpeg_bin())
        .args(["-ss", &ts, "-i", source,
               "-frames:v", "1",
               "-f", "image2", "-vcodec", "mjpeg",
               "-q:v", "4",
               "pipe:1"])
        .output()
        .context("software frame extract spawn failed")?;

    if out.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("ffmpeg produced no frame data: {stderr}");
    }
    Ok(out.stdout)
}
