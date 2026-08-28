// Public so `tests/` can exercise the ffmpeg/ffprobe layer end to end.
pub mod applog;
pub mod error;
pub mod keyframes;
pub mod preview;
pub mod proc;
pub mod probe;
pub mod trim;

use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use error::AppError;
use keyframes::ScanGeneration;
use probe::VideoMetadata;
use trim::ExportReport;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    ffmpeg: Option<String>,
    ffprobe: Option<String>,
}

/// A freshly opened file: everything the timeline needs, plus how to address it
/// on the loopback preview server.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedFile {
    #[serde(flatten)]
    pub meta: VideoMetadata,
    pub preview_token: u64,
    /// Origin of the loopback server streaming the file for preview; the
    /// frontend appends `/{token}` to build the `<video>` URL.
    pub preview_base_url: String,
}

fn version_of(tool: &'static str) -> Option<String> {
    proc::run(tool, &["-version".to_string()])
        .ok()
        .as_deref()
        .and_then(proc::parse_version)
}

/// Probe `$PATH` for the tools. Never fails — a missing tool is a *result*,
/// because the UI needs to render setup instructions rather than an error.
#[tauri::command]
async fn check_tools() -> ToolStatus {
    blocking(|| ToolStatus {
        ffmpeg: version_of("ffmpeg"),
        ffprobe: version_of("ffprobe"),
    })
    .await
    .unwrap_or(ToolStatus {
        ffmpeg: None,
        ffprobe: None,
    })
}

#[tauri::command]
async fn probe_file(app: AppHandle, path: String) -> Result<OpenedFile, AppError> {
    let meta = {
        let path = path.clone();
        blocking(move || -> Result<VideoMetadata, AppError> {
            if !std::path::Path::new(&path).is_file() {
                return Err(AppError::invalid(format!("No such file: {path}")));
            }
            let json = proc::run("ffprobe", &probe::metadata_args(&path))?;
            probe::parse_metadata(&path, &json)
        })
        .await
        .map_err(|e| AppError::Io(e.to_string()))??
    };

    // Offer the file for preview under its real container type, streamed over
    // the loopback HTTP server — WebKitGTK cannot source media from a custom
    // URI scheme, so the bytes travel as plain HTTP; see `preview`.
    let server = app.state::<preview::PreviewServer>();
    let mime = preview::mime_for(&meta.container, &meta.video_codec);
    let preview_token = server.offer(std::path::PathBuf::from(&meta.path), mime);

    applog::line(
        "probe",
        &format!(
            "opened {} container={:?} video={} audio={:?} {}x{} {:.3}s -> mime={} token={} base={}",
            meta.path,
            meta.container,
            meta.video_codec,
            meta.audio_codec,
            meta.width,
            meta.height,
            meta.duration,
            mime,
            preview_token,
            server.base_url(),
        ),
    );

    Ok(OpenedFile {
        meta,
        preview_token,
        preview_base_url: server.base_url().to_string(),
    })
}

/// Start a background keyframe scan. Results stream back as
/// `keyframes-batch` / `keyframes-done` events tagged with `scan_id`.
#[tauri::command]
fn start_keyframe_scan(
    app: AppHandle,
    generation: State<'_, ScanGeneration>,
    path: String,
    scan_id: u64,
    start_time: f64,
) {
    keyframes::spawn_scan(app, generation.0.clone(), path, scan_id, start_time);
}

/// Supersede `scan_id` so its worker kills its ffprobe child and stops.
#[tauri::command]
fn cancel_keyframe_scan(generation: State<'_, ScanGeneration>, scan_id: u64) {
    generation.0.fetch_max(scan_id.saturating_add(1), Ordering::SeqCst);
}

#[tauri::command]
async fn export_trim(
    input: String,
    start: f64,
    end: f64,
    start_time: f64,
    dest: String,
) -> Result<ExportReport, AppError> {
    blocking(move || trim::export(&input, start, end, start_time, &dest))
        .await
        .map_err(|e| AppError::Io(e.to_string()))?
}

/// Write a line from the webview into the same log as the Rust side, so the
/// media element's behaviour and the requests it makes read as one sequence.
#[tauri::command]
fn log_ui(scope: String, message: String) {
    applog::line(&scope, &message);
}

/// Run blocking work off the UI thread.
async fn blocking<T, F>(f: F) -> Result<T, tauri::Error>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    applog::init();

    let server = preview::PreviewServer::start().expect("failed to start the preview server");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(ScanGeneration::default())
        .manage(server)
        .invoke_handler(tauri::generate_handler![
            check_tools,
            probe_file,
            start_keyframe_scan,
            cancel_keyframe_scan,
            export_trim,
            log_ui
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Whittle");
}
