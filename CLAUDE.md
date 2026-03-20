# PassCut: GPU Optimization & QoL

## Project Overview
A lossless video editor focusing on stream-copy (pass-through) by default and "smart-cut" (re-encoding only at GOP boundaries) for frame-accurate exports. Built with Tauri (Rust) and Svelte 5 (TypeScript).

## Build & Development Commands
- **Install Dependencies:** `npm install`
- **Dev Mode (Vite):** `npm run dev`
- **Dev Mode (Tauri):** `npm run tauri dev`
- **Build (Vite):** `npm run build`
- **Build (Tauri):** `npm run tauri build`
- **Rust Checks:** `cargo check` / `cargo fmt` / `cargo clippy`
- **Rust Tests:** `cargo test`

## Project Guidelines
- **GPU Optimization Priority:** Keep frames in VRAM as long as possible. Use hardware-accelerated encoders (`nvenc`, `vaapi`, `videotoolbox`) for both preview extraction and smart-cut re-encoding.
- **Preview Path:** Use the custom `preview://` URI scheme for zero-copy frame delivery to the frontend, avoiding Base64 overhead.
- **Smart-Cut:** Re-encode unaligned GOP boundaries using source-matching codecs (H.264 or HEVC) with high quality (CRF 14/CQ 14) and zero B-frames for accuracy.
- **Frontend Style:** Svelte 5 with TypeScript. Prefer Vanilla CSS for styling. Use `tick()` for DOM-dependent updates.

## Recent Changes (March 2026)
- **Zero-Copy Previews:** Fixed `preview://` custom protocol in `preview.rs` by correcting buggy argument popping and adding `hwdownload` for D3D12 compatibility with software encoders.
- **Smart-Cut Precision:** Fixed `engine.rs` to correctly re-encode only the GOP boundaries at the exact cut points, preventing extra footage "overhang" when start/end points are unaligned.
- **Hardware-Accelerated Smart-Cut:** Updated `engine.rs` to detect and use GPU encoders (`nvenc`, `vaapi`, `videotoolbox`) based on the platform and source codec (H.264/HEVC).
- **Timeline QoL:** Added Ctrl/Cmd/Alt + Scroll wheel zoom to `Timeline.svelte`, centering on the mouse cursor.
- **Export Stability:** Added togglable FFmpeg flags (`-fps_mode passthrough`, `-copytb 1`, etc.) for seamless gapless looping. Default remains standard `-vsync 0` unless "Looping" is checked.
- **AI Loop Diagnostic:** Added `looping.rs` and `suggest_loop_point` command to identify the best matching end-frame using MSE. Integrated as an "AI Trim" toggle in the export UI that auto-adjusts clip duration before processing.
