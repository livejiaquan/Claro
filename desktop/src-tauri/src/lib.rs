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
use std::sync::Arc;

use serde::Serialize;
use tauri::Emitter;

use crate::stt::registry::{self, ModelSpec};

struct AppState {
    model_spec: &'static ModelSpec,
    downloading: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct Status {
    model_id: String,
    model_present: bool,
    model_approx_mb: u64,
    accessibility: bool,
    hotkey_active: bool,
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
    Status {
        model_id: spec.id.to_string(),
        model_present: registry::model_path(spec).exists(),
        model_approx_mb: spec.approx_bytes / 1_048_576,
        accessibility: accessibility_trusted(),
        hotkey_active: HOTKEY_ACTIVE.load(Ordering::SeqCst),
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

    // SIGTERM/SIGINT（mic_indicator 的「結束 Claro」走 SIGTERM）：清掉 overlay 再退出
    {
        let core = core.clone();
        if let Err(e) = ctrlc::set_handler(move || {
            core.overlay.stop();
            std::process::exit(0);
        }) {
            tracing::warn!("signal handler setup failed: {e}");
        }
    }

    let state = AppState {
        model_spec: spec,
        downloading: Arc::new(AtomicBool::new(false)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![get_status, download_model])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |_app, event| match event {
            // 關掉視窗 ≠ 結束 app（背景聽寫工具）；只有明確 exit（Cmd+Q、SIGTERM、
            // mic_indicator 選單）才真的退出
            tauri::RunEvent::ExitRequested { code: None, api, .. } => {
                api.prevent_exit();
            }
            tauri::RunEvent::Exit => {
                core.overlay.stop();
            }
            _ => {}
        });
}
