// src-tauri/src/lib.rs
mod commands;
mod engine;
mod looping;
mod models;
mod preview;
mod probe;
mod waveform;

use commands::{
    export_timeline, generate_waveform, get_keyframe_times, preview_frame, probe_media,
    suggest_loop_point,
};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .register_uri_scheme_protocol("preview", |app, request| {
            // preview://frame?path=/path/to/file.mp4&ts=1.234
            let uri = request.uri().clone();
            
            // This runs in a background thread, we parse the query params manually.
            let query = uri.query().unwrap_or("");
            let mut path = String::new();
            let mut ts = 0.0;
            
            for pair in query.split('&') {
                let mut parts = pair.split('=');
                if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                    let decoded = urlencoding::decode(v).unwrap_or(std::borrow::Cow::Borrowed(v));
                    if k == "path" {
                        path = decoded.to_string();
                    } else if k == "ts" {
                        ts = decoded.parse().unwrap_or(0.0);
                    }
                }
            }

            if path.is_empty() {
                return tauri::http::Response::builder()
                    .status(400)
                    .body(Vec::new())
                    .unwrap();
            }

            match preview::extract_frame(&path, ts) {
                Ok(bytes) => {
                    tauri::http::Response::builder()
                        .header("Access-Control-Allow-Origin", "*")
                        .header("Content-Type", "image/jpeg")
                        .status(200)
                        .body(bytes)
                        .unwrap()
                },
                Err(e) => {
                    log::error!("Preview extraction failed: {}", e);
                    tauri::http::Response::builder()
                        .status(500)
                        .body(Vec::new())
                        .unwrap()
                }
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            // Resolve bundled sidecar paths. On Windows the binaries carry .exe
            // extensions; on Linux/macOS they do not.
            let ffmpeg_name  = if cfg!(windows) { "bin/ffmpeg.exe"  } else { "bin/ffmpeg"  };
            let ffprobe_name = if cfg!(windows) { "bin/ffprobe.exe" } else { "bin/ffprobe" };

            if let Ok(p) = app.path().resolve(ffmpeg_name, tauri::path::BaseDirectory::Resource) {
                // to_str() on Windows may produce UNC paths (\\?\...); convert to
                // a plain extended path so ffmpeg accepts it without escaping.
                let s = dunce_path(&p);
                std::env::set_var("FFMPEG_BIN", &s);
                log::info!("FFMPEG_BIN = {s}");
            }
            if let Ok(p) = app.path().resolve(ffprobe_name, tauri::path::BaseDirectory::Resource) {
                let s = dunce_path(&p);
                std::env::set_var("FFPROBE_BIN", &s);
                log::info!("FFPROBE_BIN = {s}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            probe_media,
            export_timeline,
            get_keyframe_times,
            generate_waveform,
            preview_frame,
            suggest_loop_point,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PassThrough-Cut");
}

/// Strip Windows UNC prefix (\\?\) that PathBuf sometimes produces.
/// This gives us a plain absolute path that both FFmpeg and the shell accept.
fn dunce_path(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    // \\?\ prefix is UNC extended-length path — FFmpeg doesn't handle it.
    if s.starts_with(r"\\?\") {
        s[4..].to_string()
    } else {
        s.into_owned()
    }
}
