pub mod audio;
pub mod history;
pub mod hotkey;
pub mod inject;
pub mod models;
pub mod overlay_client;
pub mod pipeline;
pub mod settings;
pub mod state_machine;
pub mod stt;
pub mod textproc;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::audio::CaptureHandle;
use crate::state_machine::State;
use crate::stt::registry::{self, ModelSpec};

struct AppState {
    core: Arc<pipeline::Core>,
    model_spec: &'static ModelSpec,
    downloading: Arc<AtomicBool>,
    mic_test: Arc<Mutex<Option<CaptureHandle>>>,
}

#[derive(Serialize)]
struct Status {
    model_id: String,
    model_present: bool,
    model_approx_mb: u64,
    accessibility: bool,
    hotkey_active: bool,
    /// 使用者在設定中選的裝置（null = 跟隨系統預設）
    input_device: Option<String>,
    /// 系統目前的預設輸入裝置
    default_input: Option<String>,
    input_devices: Vec<String>,
    dictation_state: &'static str,
}

fn accessibility_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_accessibility_client::accessibility::application_is_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[tauri::command]
fn get_status(state: tauri::State<AppState>) -> Status {
    let spec = state.model_spec;
    let dictation_state = match state.core.sm.lock().unwrap().state() {
        State::Idle => "idle",
        State::Hold | State::Handsfree => "recording",
        State::Processing => "processing",
    };
    Status {
        model_id: spec.id.to_string(),
        model_present: registry::model_path(spec).exists(),
        model_approx_mb: spec.approx_bytes / 1_048_576,
        accessibility: accessibility_trusted(),
        hotkey_active: HOTKEY_ACTIVE.load(Ordering::SeqCst),
        input_device: state.core.input_device.lock().unwrap().clone(),
        default_input: audio::default_input_name(),
        input_devices: audio::list_input_devices(),
        dictation_state,
    }
}

/// 空字串 = 跟隨系統預設。即時生效並持久化到 config。
#[tauri::command]
fn set_input_device(name: String, state: tauri::State<AppState>) -> Result<(), String> {
    let selection = if name.is_empty() { None } else { Some(name.clone()) };
    *state.core.input_device.lock().unwrap() = selection;
    settings::update_config_key(
        &settings::config_path(),
        "input_device",
        serde_json::Value::String(name),
    )
    .map_err(|e| e.to_string())
}

#[derive(Serialize, Clone)]
struct MicLevel {
    level: f32,
    active: bool,
}

/// 麥克風測試：開始擷取並以 "mic-level" 事件串流電平（50ms 一次，30s 自動停）。
#[tauri::command]
fn mic_test_start(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    if state.core.sm.lock().unwrap().state() != State::Idle {
        return Err("聽寫進行中，無法測試麥克風".into());
    }
    let mut guard = state.mic_test.lock().unwrap();
    if guard.is_some() {
        return Ok(());
    }
    let device = state.core.input_device.lock().unwrap().clone();
    let handle = audio::start_capture(device).map_err(|e| e.to_string())?;
    let level = handle.level_handle();
    *guard = Some(handle);
    drop(guard);

    let mic_test = state.mic_test.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        loop {
            if mic_test.lock().unwrap().is_none() {
                break; // 已被 mic_test_stop 收走
            }
            if started.elapsed().as_secs() >= 30 {
                if let Some(h) = mic_test.lock().unwrap().take() {
                    let _ = h.stop();
                }
                break;
            }
            let _ = app.emit("mic-level", MicLevel { level: level.get(), active: true });
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = app.emit("mic-level", MicLevel { level: 0.0, active: false });
    });
    Ok(())
}

#[tauri::command]
fn mic_test_stop(state: tauri::State<AppState>) {
    if let Some(h) = state.mic_test.lock().unwrap().take() {
        let _ = h.stop();
    }
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    downloaded_mb: u64,
    total_mb: Option<u64>,
    done: bool,
    error: Option<String>,
}

/// 使用者在 UI 按下「同意下載」後才會呼叫（絕不自動下載，SPEC §6）
#[tauri::command]
fn download_model(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    if state.downloading.swap(true, Ordering::SeqCst) {
        return Err("已在下載中".into());
    }
    let spec = state.model_spec;
    let downloading = state.downloading.clone();
    std::thread::spawn(move || {
        let dest = registry::model_path(spec);
        let result = models::download(spec.url, &dest, spec.sha256, |p| {
            let _ = app.emit(
                "model-download",
                DownloadProgress {
                    downloaded_mb: p.downloaded / 1_048_576,
                    total_mb: p.total.map(|t| t / 1_048_576),
                    done: false,
                    error: None,
                },
            );
        });
        let payload = match result {
            Ok(()) => DownloadProgress { downloaded_mb: 0, total_mb: None, done: true, error: None },
            Err(e) => DownloadProgress {
                downloaded_mb: 0,
                total_mb: None,
                done: false,
                error: Some(e.to_string()),
            },
        };
        let _ = app.emit("model-download", payload);
        downloading.store(false, Ordering::SeqCst);
    });
    Ok(())
}

static HOTKEY_ACTIVE: AtomicBool = AtomicBool::new(false);

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let cfg = settings::Settings::load();
    let spec = registry::resolve(&cfg.whisper_model());
    tracing::info!("Claro starting — whisper model: {}", spec.id);

    #[cfg(target_os = "macos")]
    let engine: Box<dyn stt::SttEngine> =
        Box::new(stt::whisper::WhisperEngine::new(spec.id, registry::model_path(spec)));
    #[cfg(not(target_os = "macos"))]
    let engine: Box<dyn stt::SttEngine> = unimplemented!("M1 is macOS-only");

    let overlay = overlay_client::OverlayClient::start();
    let (msg_tx, msg_rx) = crossbeam_channel::unbounded::<pipeline::Msg>();

    // 熱鍵：Accessibility 未授權時降級啟動（UI 會顯示指引，授權後重啟生效）
    let esc_ctl = match hotkey::start() {
        Ok(service) => {
            HOTKEY_ACTIVE.store(true, Ordering::SeqCst);
            let tx = msg_tx.clone();
            let events = service.events;
            std::thread::spawn(move || {
                for ev in events {
                    if tx.send(pipeline::Msg::Hotkey(ev)).is_err() {
                        break;
                    }
                }
            });
            service.esc_ctl
        }
        Err(e) => {
            tracing::warn!("hotkey unavailable ({e}) — grant Accessibility and restart");
            let (dummy_tx, _dummy_rx) = crossbeam_channel::unbounded();
            dummy_tx
        }
    };

    let core = Arc::new(pipeline::Core::new(
        engine,
        overlay,
        Box::new(inject::MacPasteInjector),
        esc_ctl,
        msg_tx.clone(),
        cfg.input_device(),
    ));

    {
        let core = core.clone();
        std::thread::spawn(move || pipeline::run_dispatcher(core, msg_rx));
    }

    // 模型預載（prototype 行為：啟動即背景載入，首次聽寫不用等）
    {
        let core = core.clone();
        let model_path = registry::model_path(spec);
        std::thread::spawn(move || {
            if model_path.exists() {
                if let Err(e) = core.engine.lock().unwrap().load() {
                    tracing::warn!("model preload failed: {e}");
                }
            } else {
                tracing::info!("model not downloaded yet — open the Claro window to download");
            }
        });
    }

    let state = AppState {
        core: core.clone(),
        model_spec: spec,
        downloading: Arc::new(AtomicBool::new(false)),
        mic_test: Arc::new(Mutex::new(None)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_status,
            set_input_device,
            mic_test_start,
            mic_test_stop,
            download_model
        ])
        .setup({
            let signal_core = core.clone();
            move |app| {
                // CLARO_DEVTOOLS=1 時自動打開 inspector（除錯 webview 白屏用）
                if std::env::var("CLARO_DEVTOOLS").as_deref() == Ok("1") {
                    if let Some(w) = app.get_webview_window("main") {
                        w.open_devtools();
                    }
                }
                // SIGTERM/SIGINT（mic_indicator 的「結束 Claro」走 SIGTERM）。
                // 必須走 AppHandle::exit 讓事件迴圈優雅收場——
                // 從 signal 執行緒直接 std::process::exit 會觸發 macOS crash reporter。
                let handle = app.handle().clone();
                if let Err(e) = ctrlc::set_handler(move || {
                    signal_core.overlay.stop();
                    handle.exit(0);
                }) {
                    tracing::warn!("signal handler setup failed: {e}");
                }
                Ok(())
            }
        })
        // 關窗＝隱藏（背景工具不退出）；Dock 點擊（Reopen）再顯示。Cmd+Q 正常退出。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app, event| match event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_main_window(app),
            tauri::RunEvent::Exit => {
                core.overlay.stop();
            }
            _ => {}
        });
}
