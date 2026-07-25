/// 程序收尾旗標：RunEvent::Exit 第一步就設起。所有「背景載入模型」的路徑
/// （STT 預載/回載、LLM 冷生成載入）都必須先看它——否則 exit handler 卸載
/// 完成後，還在等鎖的背景執行緒會把 Metal 模型又載回來，atexit teardown
/// 再度 ggml_abort（review 抓到的競態）。
pub static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub mod audio;
pub mod codex;
pub mod context;
pub mod hardware;
pub mod history;
pub mod hotkey;
pub mod inject;
pub mod llm;
pub mod models;
pub mod overlay_client;
pub mod pipeline;
pub mod polish;
pub mod settings;
pub mod state_machine;
pub mod stt;
pub mod textproc;

use std::collections::{hash_map::Entry, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::audio::CaptureHandle;
use crate::state_machine::State;
use crate::stt::registry;

struct AppState {
    core: Arc<pipeline::Core>,
    /// STT 與 LLM 共用同一下載閘門，避免兩個大型檔案同時通過各自的磁碟預檢。
    download_gate: Arc<Mutex<Option<&'static str>>>,
    downloading: Arc<Mutex<Option<&'static str>>>,
    llm_downloading: Arc<Mutex<Option<&'static str>>>,
    /// 使用者按下取消時設起；下載迴圈在每個 chunk 前檢查。STT 與 LLM 共用一支，
    /// 因為 download_gate 已保證同時只有一個下載在跑。
    download_cancel: Arc<AtomicBool>,
    mic_test: Arc<Mutex<Option<CaptureHandle>>>,
    mic_test_passed: Arc<AtomicBool>,
    mic_test_generation: Arc<AtomicU64>,
    /// Codex 自我測試使用 request-scoped cancel handle；不可碰正式聽寫的全域
    /// policy generation 或 active process group。
    codex_test_cancellations: Arc<Mutex<CodexTestCancellationRegistry>>,
}

const MAX_CODEX_TEST_CANCEL_TOMBSTONES: usize = 128;

enum CodexTestCancellation {
    Active(Arc<AtomicBool>),
    CancelledBeforeStart,
}

#[derive(Default)]
struct CodexTestCancellationRegistry {
    entries: HashMap<u64, CodexTestCancellation>,
    tombstone_order: VecDeque<u64>,
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

// ─── 狀態 ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Status {
    model_id: String,
    model_label: String,
    model_present: bool,
    model_approx_mb: u64,
    accessibility: bool,
    hotkey_active: bool,
    input_device: Option<String>,
    default_input: Option<String>,
    input_devices: Vec<String>,
    dictation_state: &'static str,
    context_enabled: bool,
    hotkey: String,
    setup_completed: bool,
    successful_pastes_this_launch: u64,
    history_enabled: bool,
    mic_test_passed_this_launch: bool,
    /// 以下欄位讓首頁不必從 provider 名稱猜測隱私狀態；全部由 runtime gate 推導。
    polish_mode: settings::PolishMode,
    effective_mode: settings::PolishMode,
    llm_provider: String,
    local_only: bool,
    execution_location: polish::ExecutionLocation,
    endpoint_origin: Option<String>,
    blocked_reason: Option<String>,
}

#[tauri::command]
fn get_status(state: tauri::State<AppState>) -> Status {
    let spec = *state.core.active_model.lock().unwrap();
    let dictation_state = match state.core.sm.lock().unwrap().state() {
        State::Idle => "idle",
        State::Hold | State::Handsfree => "recording",
        State::Processing => "processing",
    };
    let settings = settings::Settings::load();
    Status {
        model_id: spec.id.to_string(),
        model_label: spec.label.to_string(),
        model_present: registry::model_is_verified(spec),
        model_approx_mb: spec.approx_bytes / 1_048_576,
        accessibility: accessibility_trusted(),
        hotkey_active: HOTKEY_ACTIVE.load(Ordering::SeqCst),
        input_device: state.core.input_device.lock().unwrap().clone(),
        default_input: audio::default_input_name(),
        input_devices: audio::list_input_devices(),
        dictation_state,
        context_enabled: settings.context_enabled(),
        hotkey: settings.hotkey_combo(),
        setup_completed: settings.setup_completed(),
        successful_pastes_this_launch: state.core.successful_pastes.load(Ordering::SeqCst),
        history_enabled: settings.history_enabled(),
        mic_test_passed_this_launch: state.mic_test_passed.load(Ordering::SeqCst),
        polish_mode: settings.polish_mode(),
        effective_mode: polish::effective_mode(&settings),
        llm_provider: settings.llm_provider(),
        local_only: settings.local_only(),
        execution_location: polish::execution_location(&settings),
        endpoint_origin: polish::endpoint_origin(&settings),
        blocked_reason: polish::blocked_reason(&settings).map(str::to_string),
    }
}

// ─── 輸入裝置與麥克風測試 ────────────────────────────────────────────────────

fn emit_mic_inactive(app: &tauri::AppHandle, generation: u64, passed: bool) {
    let _ = app.emit(
        "mic-level",
        MicLevel {
            level: 0.0,
            active: false,
            generation,
            passed,
            timed_out: false,
        },
    );
}

#[tauri::command]
fn set_input_device(
    name: String,
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let _audio_gate = state.core.audio_start_gate.lock().unwrap();
    let selection = if name.is_empty() {
        None
    } else {
        Some(name.clone())
    };
    let generation = state.mic_test_generation.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(handle) = state.mic_test.lock().unwrap().take() {
        let _ = handle.stop_immediately();
    }
    *state.core.input_device.lock().unwrap() = selection;
    state.mic_test_passed.store(false, Ordering::SeqCst);
    emit_mic_inactive(&app, generation, false);
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
    generation: u64,
    passed: bool,
    timed_out: bool,
}

#[tauri::command]
fn mic_test_start(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    let _audio_gate = state.core.audio_start_gate.lock().unwrap();
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
    let mic_test_passed = state.mic_test_passed.clone();
    mic_test_passed.store(false, Ordering::SeqCst);
    let mic_test_generation = state.mic_test_generation.clone();
    let generation = mic_test_generation.fetch_add(1, Ordering::SeqCst) + 1;
    *guard = Some(handle);
    drop(guard);

    let mic_test = state.mic_test.clone();
    let core = state.core.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut passed = false;
        let mut timed_out = false;
        loop {
            if mic_test_generation.load(Ordering::SeqCst) != generation {
                break;
            }
            if mic_test.lock().unwrap().is_none() {
                break;
            }
            if started.elapsed().as_secs() >= 30 {
                let _audio_gate = core.audio_start_gate.lock().unwrap();
                if let Some(h) = mic_test.lock().unwrap().take() {
                    let _ = h.stop_immediately();
                }
                timed_out = true;
                break;
            }
            let current_level = level.get();
            if current_level > 0.01 && mic_test_generation.load(Ordering::SeqCst) == generation {
                mic_test_passed.store(true, Ordering::SeqCst);
                passed = true;
                if mic_test_generation.load(Ordering::SeqCst) != generation {
                    mic_test_passed.store(false, Ordering::SeqCst);
                    passed = false;
                }
            }
            if mic_test_generation.load(Ordering::SeqCst) != generation {
                break;
            }
            let _ = app.emit(
                "mic-level",
                MicLevel {
                    level: current_level,
                    active: true,
                    generation,
                    passed,
                    timed_out: false,
                },
            );
            // 一確認收音就自動釋放裝置，避免使用者切到 TextEdit 做首次聽寫時
            // mic test 與正式錄音同時佔用 USB／Bluetooth input stream。
            if passed {
                let _audio_gate = core.audio_start_gate.lock().unwrap();
                if let Some(h) = mic_test.lock().unwrap().take() {
                    let _ = h.stop_immediately();
                }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = app.emit(
            "mic-level",
            MicLevel {
                level: 0.0,
                active: false,
                generation,
                passed,
                timed_out,
            },
        );
    });
    Ok(())
}

#[tauri::command]
fn mic_test_stop(app: tauri::AppHandle, state: tauri::State<AppState>) {
    let _audio_gate = state.core.audio_start_gate.lock().unwrap();
    let generation = state.mic_test_generation.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(h) = state.mic_test.lock().unwrap().take() {
        let _ = h.stop_immediately();
    }
    emit_mic_inactive(
        &app,
        generation,
        state.mic_test_passed.load(Ordering::SeqCst),
    );
}

// ─── 模型管理 ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ModelInfo {
    id: &'static str,
    label: &'static str,
    desc: &'static str,
    size_mb: u64,
    recommended: bool,
    preview: bool,
    available: bool,
    downloaded: bool,
    active: bool,
    downloading: bool,
}

#[tauri::command]
fn list_models(state: tauri::State<AppState>) -> Vec<ModelInfo> {
    let active = state.core.active_model.lock().unwrap().id;
    let downloading = *state.downloading.lock().unwrap();
    let recommended = hardware::profile(polish::apple_status()).recommended_stt;
    registry::MODELS
        .iter()
        .map(|m| ModelInfo {
            id: m.id,
            label: m.label,
            desc: m.desc,
            size_mb: m.approx_bytes / 1_048_576,
            recommended: m.id == recommended,
            preview: m.preview,
            available: m.available,
            downloaded: registry::model_is_verified(m),
            active: m.id == active,
            downloading: downloading == Some(m.id),
        })
        .collect()
}

#[tauri::command]
fn get_hardware_profile() -> hardware::HardwareProfile {
    hardware::profile(polish::apple_status())
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    model_id: &'static str,
    downloaded_mb: u64,
    total_mb: Option<u64>,
    done: bool,
    downloaded: bool,
    activation_status: &'static str,
    error: Option<String>,
}

fn reserve_download(gate: &Mutex<Option<&'static str>>, id: &'static str) -> Result<(), String> {
    let mut active = gate.lock().unwrap();
    if active.is_some() {
        return Err("已有另一個模型在下載中，完成後再試".into());
    }
    *active = Some(id);
    Ok(())
}

fn release_download(gate: &Mutex<Option<&'static str>>) {
    *gate.lock().unwrap() = None;
}

/// 下載進行中。聽寫仍須可用（STT 會照常重載），但內建 LLM 潤飾會讓路——
/// 兩個模型加上下載寫入與整檔 sha256，在 16GB 機器上足以讓 WebKit renderer
/// 被記憶體壓力殺掉（主進程存活，使用者看到的就是白屏）。
pub static DOWNLOAD_ACTIVE: AtomicBool = AtomicBool::new(false);

const DOWNLOAD_PREPARE_RETRY: std::time::Duration = std::time::Duration::from_millis(25);

/// 等待模型資源讓出時仍要能回應「取消」。`Mutex::lock()` 無法被 AtomicBool
/// 中斷，因此用有上限的 park 週期搭配 try_lock；沒有持續佔用 CPU 的 busy spin。
fn wait_for_download_resource(
    cancel: &AtomicBool,
    mut try_prepare: impl FnMut() -> Result<bool, String>,
) -> Result<(), String> {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(models::DOWNLOAD_CANCELLED.into());
        }
        if try_prepare()? {
            return Ok(());
        }
        std::thread::park_timeout(DOWNLOAD_PREPARE_RETRY);
    }
}

/// 下載期間的狀態持有者。用 Drop 收尾，背景執行緒即使 panic（例如 mutex
/// poisoned）也不會把 downloading／gate 永久卡住而必須重開 app。
struct DownloadSession {
    slot: Arc<Mutex<Option<&'static str>>>,
    gate: Arc<Mutex<Option<&'static str>>>,
    active: &'static AtomicBool,
}

impl DownloadSession {
    fn new(slot: Arc<Mutex<Option<&'static str>>>, gate: Arc<Mutex<Option<&'static str>>>) -> Self {
        Self::with_active(slot, gate, &DOWNLOAD_ACTIVE)
    }

    fn with_active(
        slot: Arc<Mutex<Option<&'static str>>>,
        gate: Arc<Mutex<Option<&'static str>>>,
        active: &'static AtomicBool,
    ) -> Self {
        active.store(true, Ordering::SeqCst);
        Self { slot, gate, active }
    }

    /// 下載前卸載大型模型。等待推論／轉錄釋放鎖的期間可以取消；一旦拿到鎖，
    /// 實際的 Metal drop/unload 仍須完整做完，不能在資源析構中途硬切。
    fn prepare(&self, core: &Arc<pipeline::Core>, cancel: &AtomicBool) -> Result<(), String> {
        wait_for_download_resource(cancel, || {
            llm::try_unload_for_download().map_err(|error| error.to_string())
        })?;
        wait_for_download_resource(cancel, || match core.engine.try_lock() {
            Ok(mut engine) => {
                if engine.is_loaded() {
                    engine.unload();
                    tracing::info!("unloaded STT before model download to reduce memory pressure");
                }
                Ok(true)
            }
            Err(std::sync::TryLockError::WouldBlock) => Ok(false),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                Err("語音模型狀態異常，請重新啟動 Claro".into())
            }
        })?;
        if cancel.load(Ordering::SeqCst) {
            return Err(models::DOWNLOAD_CANCELLED.into());
        }
        Ok(())
    }

    /// final event 必須在 Drop 清除 downloading、共用 gate 與
    /// DOWNLOAD_ACTIVE 之後送出，讓事件處理器立即 refetch 時讀到權威狀態。
    fn finish(self, emit: impl FnOnce()) {
        drop(self);
        emit();
    }
}

impl Drop for DownloadSession {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = None;
        self.slot.clear_poison();
        drop(slot);

        // gate 最後釋放：下一個 session 只能在這個 session 的 active/slot
        // 都清理完成後開始，避免舊 Drop 把新 session 的狀態覆寫掉。
        let mut gate = self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *gate = None;
        self.gate.clear_poison();
    }
}

/// 取消進行中的下載。`.partial` 會保留，下次點下載自動續傳。
#[tauri::command]
fn cancel_download(state: tauri::State<AppState>) -> Result<(), String> {
    state.download_cancel.store(true, Ordering::SeqCst);
    Ok(())
}

/// 使用者在 UI 明確點擊後才會呼叫（絕不自動下載，SPEC §6）
#[tauri::command]
fn download_model(
    id: String,
    activate: Option<bool>,
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let spec = registry::find(&id).ok_or("未知的模型")?;
    if !spec.available {
        return Err("此模型仍在做長音訊與台灣語料驗證，目前僅供離線評測".into());
    }
    reserve_download(&state.download_gate, spec.id)?;
    if let Err(error) =
        models::ensure_download_capacity(&registry::model_path(spec), spec.approx_bytes)
    {
        release_download(&state.download_gate);
        return Err(error.to_string());
    }
    {
        let mut guard = state.downloading.lock().unwrap();
        if guard.is_some() {
            drop(guard);
            release_download(&state.download_gate);
            return Err("已有模型在下載中".into());
        }
        *guard = Some(spec.id);
    }
    let downloading = state.downloading.clone();
    let download_gate = state.download_gate.clone();
    let core = state.core.clone();
    let cancel = state.download_cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    let activate_after_download = activate.unwrap_or(false);
    std::thread::spawn(move || {
        // 下載會寫入數 GB 並在結尾整檔 hash；常駐的 STT/LLM 若同時佔著數 GB，
        // WebKit renderer 會被系統以記憶體壓力殺掉（使用者實測踩過：白屏）。
        // 先讓出記憶體，下載結束後照原本的用時載入路徑自然回來。
        let session = DownloadSession::new(downloading, download_gate);
        let dest = registry::model_path(spec);
        let result = session.prepare(&core, &cancel).and_then(|()| {
            models::download(spec.url, &dest, spec.sha256, &cancel, |p| {
                let _ = app.emit(
                    "model-download",
                    DownloadProgress {
                        model_id: spec.id,
                        downloaded_mb: p.downloaded / 1_048_576,
                        total_mb: p.total.map(|t| t / 1_048_576),
                        done: false,
                        downloaded: false,
                        activation_status: "none",
                        error: None,
                    },
                );
            })
            .map_err(|error| error.to_string())
        });
        let payload = match result {
            Ok(()) if activate_after_download => match core.swap_model(spec, || {
                settings::update_config_key(
                    &settings::config_path(),
                    "whisper_model",
                    serde_json::Value::String(spec.id.to_string()),
                )
            }) {
                Ok(()) => DownloadProgress {
                    model_id: spec.id,
                    downloaded_mb: 0,
                    total_mb: None,
                    done: true,
                    downloaded: true,
                    activation_status: "none",
                    error: None,
                },
                Err(error) => {
                    let activation_status = if error.is_busy() {
                        "waiting_for_idle"
                    } else {
                        "retry_required"
                    };
                    let reason = error.to_string();
                    DownloadProgress {
                        model_id: spec.id,
                        downloaded_mb: 0,
                        total_mb: None,
                        done: true,
                        downloaded: true,
                        activation_status,
                        error: Some(reason),
                    }
                }
            },
            Ok(()) => DownloadProgress {
                model_id: spec.id,
                downloaded_mb: 0,
                total_mb: None,
                done: true,
                downloaded: true,
                activation_status: "none",
                error: None,
            },
            Err(e) => DownloadProgress {
                model_id: spec.id,
                downloaded_mb: 0,
                total_mb: None,
                done: false,
                downloaded: false,
                activation_status: "none",
                error: Some(e.to_string()),
            },
        };
        session.finish(|| {
            let _ = app.emit("model-download", payload);
        });
    });
    Ok(())
}

/// 切換使用中的模型（需已下載）；引擎熱換、背景預載、寫回 config。
#[tauri::command]
fn set_model(id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let spec = registry::find(&id).ok_or("未知的模型")?;
    if !spec.available {
        return Err("此模型仍在做長音訊與台灣語料驗證，目前不可啟用".into());
    }
    models::verify_model_file(&registry::model_path(spec), spec.sha256)
        .map_err(|e| format!("請先下載完整且驗證通過的模型：{e}"))?;
    state
        .core
        .swap_model(spec, || {
            settings::update_config_key(
                &settings::config_path(),
                "whisper_model",
                serde_json::Value::String(spec.id.into()),
            )
        })
        .map_err(|e| e.to_string())
}

/// 刪除已下載的模型檔（使用中的不可刪）。
#[tauri::command]
fn delete_model(id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let spec = registry::find(&id).ok_or("未知的模型")?;
    // 與 set_model 的切換交易互斥：切換中 active_model 還沒 commit，
    // 只看它會誤放行「刪掉正在載入的模型檔」
    let Some(_switch) = state.core.try_model_switch_gate() else {
        return Err("模型切換進行中，稍後再刪除".into());
    };
    if state.core.active_model.lock().unwrap().id == spec.id {
        return Err("使用中的模型不能刪除，先切換到其他模型".into());
    }
    std::fs::remove_file(registry::model_path(spec)).map_err(|e| e.to_string())
}

// ─── LLM 潤飾設定 ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct LlmConfig {
    provider: String,
    model: String,
    base_url: String,
    has_key: bool,
    /// Apple Intelligence 可用性：0=可用 1=裝置不支援 2=未開啟 3=模型下載中 4=系統過舊 5=其他
    apple_status: i32,
    polish_mode: settings::PolishMode,
    effective_mode: settings::PolishMode,
    local_only: bool,
    organize_consent_valid: bool,
    correct_consent_valid: bool,
    cloud_consent_valid: bool,
    execution_location: polish::ExecutionLocation,
    endpoint_origin: Option<String>,
    destination_label: Option<String>,
    codex_consent_valid: bool,
    codex_context_consent_valid: bool,
    codex_share_context_terms: bool,
    codex_correction_preferences: String,
    blocked_reason: Option<String>,
}

fn llm_config_from_settings(s: &settings::Settings) -> LlmConfig {
    LlmConfig {
        provider: s.llm_provider(),
        model: s.llm_model(),
        base_url: s.llm_base_url(),
        has_key: polish::has_api_key(),
        apple_status: polish::apple_status(),
        polish_mode: s.polish_mode(),
        effective_mode: polish::effective_mode(s),
        local_only: s.local_only(),
        organize_consent_valid: s.organize_consent_valid(),
        correct_consent_valid: s.correct_consent_valid(),
        cloud_consent_valid: s.cloud_consent_valid(),
        execution_location: polish::execution_location(s),
        endpoint_origin: polish::endpoint_origin(s),
        destination_label: polish::destination_label(s),
        codex_consent_valid: s.llm_provider() == "codex" && s.cloud_consent_valid(),
        codex_context_consent_valid: s.codex_context_consent_valid(),
        codex_share_context_terms: s.codex_share_context_terms(),
        codex_correction_preferences: s.codex_correction_preferences(),
        blocked_reason: polish::blocked_reason(s).map(str::to_string),
    }
}

#[tauri::command]
fn get_llm_config() -> LlmConfig {
    llm_config_from_settings(&settings::Settings::load())
}

/// 本機 LLM 服務（Ollama/LM Studio）目前可用的模型清單；連不上回 Err。
#[tauri::command]
async fn list_provider_models(provider: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        polish::list_local_models(&provider).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ─── 內建 LLM 模型管理 ───────────────────────────────────────────────────────

#[derive(Serialize)]
struct BuiltinLlmInfo {
    id: &'static str,
    label: &'static str,
    desc: &'static str,
    size_mb: u64,
    recommended: bool,
    preview: bool,
    available: bool,
    downloaded: bool,
    active: bool,
    downloading: bool,
}

#[tauri::command]
fn list_builtin_llms(state: tauri::State<AppState>) -> Vec<BuiltinLlmInfo> {
    let s = settings::Settings::load();
    let active = if s.llm_provider() == "builtin" {
        let m = s.llm_model();
        if m.is_empty() {
            "qwen3-4b-instruct-2507".to_string()
        } else {
            m
        }
    } else {
        String::new()
    };
    let downloading = *state.llm_downloading.lock().unwrap();
    llm::LLM_MODELS
        .iter()
        .map(|m| BuiltinLlmInfo {
            id: m.id,
            label: m.label,
            desc: m.desc,
            size_mb: m.approx_bytes / 1_048_576,
            recommended: m.recommended,
            preview: false,
            available: true,
            downloaded: llm::model_is_verified(m),
            active: m.id == active,
            downloading: downloading == Some(m.id),
        })
        .collect()
}

/// 使用者在 UI 明確點擊後才會呼叫（絕不自動下載，SPEC §6）。
/// 進度走 "llm-model-download" 事件（與 whisper 的 "model-download" 分流）。
#[tauri::command]
fn download_builtin_llm(
    id: String,
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let spec = llm::find(&id).ok_or("未知的內建模型")?;
    reserve_download(&state.download_gate, spec.id)?;
    if let Err(error) = models::ensure_download_capacity(&llm::model_path(spec), spec.approx_bytes)
    {
        release_download(&state.download_gate);
        return Err(error.to_string());
    }
    {
        let mut guard = state.llm_downloading.lock().unwrap();
        if guard.is_some() {
            drop(guard);
            release_download(&state.download_gate);
            return Err("已有模型在下載中".into());
        }
        *guard = Some(spec.id);
    }
    let downloading = state.llm_downloading.clone();
    let download_gate = state.download_gate.clone();
    let core = state.core.clone();
    let cancel = state.download_cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let session = DownloadSession::new(downloading, download_gate);
        let dest = llm::model_path(spec);
        let result = session.prepare(&core, &cancel).and_then(|()| {
            models::download(spec.url, &dest, spec.sha256, &cancel, |p| {
                let _ = app.emit(
                    "llm-model-download",
                    DownloadProgress {
                        model_id: spec.id,
                        downloaded_mb: p.downloaded / 1_048_576,
                        total_mb: p.total.map(|t| t / 1_048_576),
                        done: false,
                        downloaded: false,
                        activation_status: "none",
                        error: None,
                    },
                );
            })
            .map_err(|error| error.to_string())
        });
        let payload = match result {
            Ok(()) => DownloadProgress {
                model_id: spec.id,
                downloaded_mb: 0,
                total_mb: None,
                done: true,
                downloaded: true,
                activation_status: "none",
                error: None,
            },
            Err(e) => DownloadProgress {
                model_id: spec.id,
                downloaded_mb: 0,
                total_mb: None,
                done: false,
                downloaded: false,
                activation_status: "none",
                error: Some(e.to_string()),
            },
        };
        session.finish(|| {
            let _ = app.emit("llm-model-download", payload);
        });
    });
    Ok(())
}

#[tauri::command]
fn delete_builtin_llm(id: String) -> Result<(), String> {
    let spec = llm::find(&id).ok_or("未知的內建模型")?;
    let s = settings::Settings::load();
    if s.llm_provider() == "builtin" {
        // 與 from_settings 同語意：model 未設定時實際使用的是預設模型
        let active = {
            let m = s.llm_model();
            if m.is_empty() {
                "qwen3-4b-instruct-2507".to_string()
            } else {
                m
            }
        };
        if active == spec.id {
            return Err("使用中的模型不能刪除，先切換潤飾引擎或模型".into());
        }
    }
    llm::unload_now();
    std::fs::remove_file(llm::model_path(spec)).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_llm_config(
    provider: String,
    model: String,
    base_url: String,
    confirmed: bool,
) -> Result<LlmConfig, String> {
    if ![
        "off", "apple", "builtin", "ollama", "lmstudio", "codex", "custom",
    ]
    .contains(&provider.as_str())
    {
        return Err("未知的處理引擎".into());
    }
    // confirmed 保留在 command contract，讓 UI 能明確表達敏感操作；雲端權限
    // 只允許由 set_local_only 寫入，單純改 provider/URL 絕不順便解除隱私 gate。
    let _ = confirmed;
    codex::with_policy_change(|| {
        // provider 必須在 policy gate 內重讀；否則延遲中的 Codex enable
        // 可能超車 gate 外快照，讓這次設定變更漏掉 epoch invalidation。
        let current = settings::Settings::load();
        let mut pairs = vec![
            ("llm_provider".into(), provider.clone().into()),
            ("llm_polish_model".into(), model.into()),
            ("llm_base_url".into(), base_url.into()),
        ];
        // 下拉選到 Codex 只表示「想設定」；真正雲端同意只能由專用 panel寫入。
        if provider == "codex" && current.llm_provider() != "codex" {
            pairs.extend([
                ("local_only".into(), true.into()),
                ("cloud_consent_version".into(), 0.into()),
                ("cloud_consent_target".into(), serde_json::Value::Null),
                ("codex_share_context_terms".into(), false.into()),
                ("codex_context_consent_version".into(), 0.into()),
                (
                    "codex_context_consent_target".into(),
                    serde_json::Value::Null,
                ),
            ]);
        } else if current.llm_provider() == "codex" && provider != "codex" {
            pairs.extend([
                ("codex_share_context_terms".into(), false.into()),
                ("codex_context_consent_version".into(), 0.into()),
                (
                    "codex_context_consent_target".into(),
                    serde_json::Value::Null,
                ),
            ]);
            if current.polish_mode() == settings::PolishMode::Correct {
                // CORRECT 目前需要 Codex 的結構化 proposal。離開 Codex 時
                // 原子地回到 CLEAN，避免普通 provider 被 UI 誤標成專業校字。
                pairs.push(("polish_mode".into(), "clean".into()));
            }
        }
        settings::update_config_keys(&settings::config_path(), pairs).map_err(|e| e.to_string())
    })?;
    Ok(get_llm_config())
}

#[tauri::command]
fn set_polish_mode(mode: String, confirmed: bool) -> Result<LlmConfig, String> {
    let mode = mode.parse::<settings::PolishMode>()?;
    codex::with_policy_change(|| {
        let current = settings::Settings::load();
        validate_polish_mode_provider(mode, &current.llm_provider())?;
        validate_polish_mode_consent(
            mode,
            current.correct_consent_valid(),
            current.organize_consent_valid(),
            confirmed,
        )?;
        let mut pairs = vec![("polish_mode".into(), mode.as_str().into())];
        if mode == settings::PolishMode::Correct && confirmed {
            pairs.push((
                "correct_consent_version".into(),
                settings::CORRECT_CONSENT_VERSION.into(),
            ));
        }
        if mode == settings::PolishMode::Organize && confirmed {
            pairs.push((
                "organize_consent_version".into(),
                settings::ORGANIZE_CONSENT_VERSION.into(),
            ));
        }
        settings::update_config_keys(&settings::config_path(), pairs).map_err(|e| e.to_string())
    })?;
    Ok(get_llm_config())
}

fn validate_polish_mode_provider(mode: settings::PolishMode, provider: &str) -> Result<(), String> {
    match (mode, provider) {
        (settings::PolishMode::Raw, _) => Ok(()),
        (settings::PolishMode::Correct, "codex") => Ok(()),
        (settings::PolishMode::Correct, _) => {
            Err("專業校字目前只支援 Codex；請先選擇 Codex".into())
        }
        (settings::PolishMode::Clean | settings::PolishMode::Organize, "codex") => {
            Err("Codex 目前只支援專業校字；請先選擇其他整理引擎".into())
        }
        _ => Ok(()),
    }
}

fn validate_polish_mode_consent(
    mode: settings::PolishMode,
    correct_consent_valid: bool,
    organize_consent_valid: bool,
    confirmed: bool,
) -> Result<(), String> {
    if mode == settings::PolishMode::Correct && !correct_consent_valid && !confirmed {
        Err("啟用 CORRECT 前必須明確確認它會調整授權詞彙的拼法格式".into())
    } else if mode == settings::PolishMode::Organize && !organize_consent_valid && !confirmed {
        Err("啟用 ORGANIZE 前必須明確確認它會重排句序與段落".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
fn set_local_only(enabled: bool, confirmed: bool) -> Result<LlmConfig, String> {
    codex::with_policy_change(|| {
        let current = settings::Settings::load();
        if !enabled && current.llm_provider() == "codex" {
            return Err("Codex 的雲端同意必須從「Codex 專業拼法」面板重新確認".into());
        }
        if !enabled && current.local_only() && !confirmed {
            return Err("關閉「僅限本機」前必須明確確認雲端資料傳送".into());
        }
        let mut pairs = vec![("local_only".into(), enabled.into())];
        if enabled {
            pairs.extend([
                ("cloud_consent_version".into(), 0.into()),
                ("cloud_consent_origin".into(), serde_json::Value::Null),
                ("cloud_consent_target".into(), serde_json::Value::Null),
                ("codex_share_context_terms".into(), false.into()),
                ("codex_context_consent_version".into(), 0.into()),
                (
                    "codex_context_consent_target".into(),
                    serde_json::Value::Null,
                ),
            ]);
        }
        if !enabled && confirmed {
            let target = polish::cloud_consent_target(&current);
            pairs.push((
                "cloud_consent_version".into(),
                settings::CLOUD_CONSENT_VERSION.into(),
            ));
            pairs.push((
                "cloud_consent_origin".into(),
                polish::endpoint_origin(&current)
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
            ));
            pairs.push((
                "cloud_consent_target".into(),
                target
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
            ));
        }
        settings::update_config_keys(&settings::config_path(), pairs).map_err(|e| e.to_string())
    })?;
    Ok(get_llm_config())
}

#[tauri::command]
async fn get_codex_status() -> codex::CodexStatus {
    tauri::async_runtime::spawn_blocking(|| {
        let (current, policy_epoch) = codex::policy_snapshot(settings::Settings::load);
        let mut result = codex::probe(current.codex_cli_path().as_deref());
        if current.llm_provider() == "codex" {
            let auth_changed = result.availability == codex::CodexAvailability::Ready
                && result.auth_mode != current.codex_auth_mode();
            let contract_changed = result.availability == codex::CodexAvailability::Ready
                && result
                    .contract
                    .as_ref()
                    .is_none_or(|contract| !current.codex_contract_matches(contract));
            let explicitly_unusable = matches!(
                result.availability,
                codex::CodexAvailability::NotInstalled
                    | codex::CodexAvailability::NotAuthenticated
                    | codex::CodexAvailability::Unsupported
                    | codex::CodexAvailability::MissingCapability
            );
            if auth_changed || contract_changed || explicitly_unusable {
                let auth_value = result
                    .auth_mode
                    .map(|auth_mode| serde_json::Value::String(auth_mode.as_str().into()))
                    .unwrap_or(serde_json::Value::Null);
                let version_value = result
                    .version
                    .clone()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null);
                let executable_path_value = result
                    .executable_path
                    .clone()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null);
                let raw_version_value = result
                    .contract
                    .as_ref()
                    .map(|contract| serde_json::Value::String(contract.raw_version.clone()))
                    .unwrap_or(serde_json::Value::Null);
                let executable_sha256_value = result
                    .contract
                    .as_ref()
                    .map(|contract| serde_json::Value::String(contract.executable_sha256.clone()))
                    .unwrap_or(serde_json::Value::Null);
                let capability_fingerprint_value = result
                    .contract
                    .as_ref()
                    .map(|contract| {
                        serde_json::Value::String(contract.capability_fingerprint.clone())
                    })
                    .unwrap_or(serde_json::Value::Null);
                if let Err(error) = codex::with_policy_change_if(policy_epoch, || {
                    if settings::Settings::load().llm_provider() != "codex" {
                        return Err("codex_request_stale".into());
                    }
                    settings::update_config_keys(
                        &settings::config_path(),
                        vec![
                            ("local_only".into(), true.into()),
                            ("codex_auth_kind".into(), auth_value),
                            ("codex_cli_path".into(), executable_path_value),
                            ("codex_cli_version".into(), version_value),
                            ("codex_cli_raw_version".into(), raw_version_value),
                            ("codex_executable_sha256".into(), executable_sha256_value),
                            (
                                "codex_capability_fingerprint".into(),
                                capability_fingerprint_value,
                            ),
                            ("cloud_consent_version".into(), 0.into()),
                            ("cloud_consent_origin".into(), serde_json::Value::Null),
                            ("cloud_consent_target".into(), serde_json::Value::Null),
                            ("codex_share_context_terms".into(), false.into()),
                            ("codex_context_consent_version".into(), 0.into()),
                            (
                                "codex_context_consent_target".into(),
                                serde_json::Value::Null,
                            ),
                        ],
                    )
                    .map_err(|error| error.to_string())
                }) {
                    tracing::error!("failed to persist Codex safety revocation: {error}");
                    result.availability = codex::CodexAvailability::ProbeFailed;
                    result.error_code = Some("policy_update_failed".into());
                }
            }
        }
        result
    })
    .await
    .unwrap_or_else(|_| codex::CodexStatus {
        availability: codex::CodexAvailability::ProbeFailed,
        version: None,
        auth_mode: None,
        executable_path: None,
        error_code: Some("probe_join_failed".into()),
        contract: None,
    })
}

#[tauri::command]
async fn enable_codex_provider(
    share_context_terms: bool,
    confirmed: bool,
) -> Result<LlmConfig, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !confirmed {
            return Err("啟用 Codex 前必須明確確認校字與雲端資料傳送".into());
        }
        let (current, policy_epoch) = codex::policy_snapshot(settings::Settings::load);
        if current.llm_provider() != "codex" {
            return Err("codex_request_stale".into());
        }
        if share_context_terms && !current.context_enabled() {
            return Err("螢幕上下文已關閉，無法啟用畫面詞彙分享".into());
        }
        let probe = codex::probe(current.codex_cli_path().as_deref());
        if probe.availability != codex::CodexAvailability::Ready {
            return Err(match probe.availability {
                codex::CodexAvailability::NotInstalled => "codex_not_installed",
                codex::CodexAvailability::NotAuthenticated => "codex_auth_required",
                codex::CodexAvailability::Unsupported
                | codex::CodexAvailability::MissingCapability => "codex_unsupported",
                _ => "codex_unavailable",
            }
            .into());
        }
        let auth_mode = probe.auth_mode.ok_or("codex_auth_required")?;
        let contract = probe.contract.as_ref().ok_or("codex_unsupported")?;
        let target = codex::contract_consent_target(auth_mode, contract);
        let context_target = format!("{target}:context-terms-v1");
        let cli_version = contract.version.clone();
        let raw_version = contract.raw_version.clone();
        let executable_sha256 = contract.executable_sha256.clone();
        let capability_fingerprint = contract.capability_fingerprint.clone();
        codex::with_policy_change_if(policy_epoch, || {
            let latest = settings::Settings::load();
            if latest.llm_provider() != "codex" {
                return Err("codex_request_stale".into());
            }
            settings::update_config_keys(
                &settings::config_path(),
                vec![
                    ("llm_provider".into(), "codex".into()),
                    ("polish_mode".into(), "correct".into()),
                    ("local_only".into(), false.into()),
                    (
                        "correct_consent_version".into(),
                        settings::CORRECT_CONSENT_VERSION.into(),
                    ),
                    (
                        "codex_auth_kind".into(),
                        serde_json::Value::String(auth_mode.as_str().into()),
                    ),
                    (
                        "codex_cli_path".into(),
                        probe
                            .executable_path
                            .map(serde_json::Value::String)
                            .unwrap_or(serde_json::Value::Null),
                    ),
                    (
                        "codex_cli_version".into(),
                        serde_json::Value::String(cli_version),
                    ),
                    (
                        "codex_cli_raw_version".into(),
                        serde_json::Value::String(raw_version),
                    ),
                    (
                        "codex_executable_sha256".into(),
                        serde_json::Value::String(executable_sha256),
                    ),
                    (
                        "codex_capability_fingerprint".into(),
                        serde_json::Value::String(capability_fingerprint),
                    ),
                    (
                        "cloud_consent_version".into(),
                        settings::CLOUD_CONSENT_VERSION.into(),
                    ),
                    (
                        "cloud_consent_target".into(),
                        serde_json::Value::String(target),
                    ),
                    ("cloud_consent_origin".into(), serde_json::Value::Null),
                    (
                        "codex_share_context_terms".into(),
                        share_context_terms.into(),
                    ),
                    (
                        "codex_context_consent_version".into(),
                        if share_context_terms {
                            settings::CODEX_CONTEXT_CONSENT_VERSION.into()
                        } else {
                            0.into()
                        },
                    ),
                    (
                        "codex_context_consent_target".into(),
                        if share_context_terms {
                            serde_json::Value::String(context_target)
                        } else {
                            serde_json::Value::Null
                        },
                    ),
                ],
            )
            .map_err(|error| error.to_string())
        })?;
        Ok(get_llm_config())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn set_codex_correction_preferences(correction_preferences: String) -> Result<LlmConfig, String> {
    let (current, policy_epoch) = codex::policy_snapshot(settings::Settings::load);
    if current.llm_provider() != "codex" {
        return Err("目前未選擇 Codex".into());
    }
    let correction_preferences = correction_preferences.trim().to_string();
    if correction_preferences.chars().count() > settings::CODEX_CORRECTION_PREFERENCES_MAX_CHARS {
        return Err("校字偏好最多 1000 字".into());
    }
    codex::with_policy_change_if(policy_epoch, || {
        if settings::Settings::load().llm_provider() != "codex" {
            return Err("codex_request_stale".into());
        }
        settings::update_config_key(
            &settings::config_path(),
            "codex_correction_preferences",
            correction_preferences.into(),
        )
        .map_err(|error| error.to_string())
    })?;
    Ok(get_llm_config())
}

#[tauri::command]
fn set_codex_context_terms_enabled(enabled: bool, confirmed: bool) -> Result<LlmConfig, String> {
    let (current, policy_epoch) = codex::policy_snapshot(settings::Settings::load);
    if current.llm_provider() != "codex" {
        return Err("目前未選擇 Codex".into());
    }
    let share_context_terms = enabled;
    if share_context_terms && !current.context_enabled() {
        return Err("螢幕上下文已關閉，無法啟用畫面詞彙分享".into());
    }
    if share_context_terms && !current.codex_context_consent_valid() && !confirmed {
        return Err("啟用畫面詞彙分享前必須明確確認資料傳送".into());
    }
    let context_target = format!(
        "{}:context-terms-v1",
        current
            .codex_consent_target()
            .ok_or("codex_consent_required")?
    );
    codex::with_policy_change_if(policy_epoch, || {
        if settings::Settings::load().llm_provider() != "codex" {
            return Err("codex_request_stale".into());
        }
        settings::update_config_keys(
            &settings::config_path(),
            vec![
                (
                    "codex_share_context_terms".into(),
                    share_context_terms.into(),
                ),
                (
                    "codex_context_consent_version".into(),
                    if share_context_terms {
                        settings::CODEX_CONTEXT_CONSENT_VERSION.into()
                    } else {
                        0.into()
                    },
                ),
                (
                    "codex_context_consent_target".into(),
                    if share_context_terms {
                        serde_json::Value::String(context_target)
                    } else {
                        serde_json::Value::Null
                    },
                ),
            ],
        )
        .map_err(|error| error.to_string())
    })?;
    Ok(get_llm_config())
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CodexTestFailureReason {
    Timeout,
    RateLimited,
    AuthRequired,
    ConsentChanged,
    Unavailable,
    OutputRejected,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum CodexTestResult {
    Success { input: String, output: String },
    Failed { reason: CodexTestFailureReason },
    Cancelled,
}

fn codex_test_failure(reason: Option<polish::PolishFallbackReason>) -> CodexTestResult {
    use polish::PolishFallbackReason;
    match reason {
        Some(PolishFallbackReason::CodexCancelled) => CodexTestResult::Cancelled,
        Some(PolishFallbackReason::CodexTimeout) => CodexTestResult::Failed {
            reason: CodexTestFailureReason::Timeout,
        },
        Some(PolishFallbackReason::CodexUsageLimited) => CodexTestResult::Failed {
            reason: CodexTestFailureReason::RateLimited,
        },
        Some(PolishFallbackReason::CodexAuthRequired) => CodexTestResult::Failed {
            reason: CodexTestFailureReason::AuthRequired,
        },
        Some(PolishFallbackReason::CloudConsentRequired) => CodexTestResult::Failed {
            reason: CodexTestFailureReason::ConsentChanged,
        },
        Some(PolishFallbackReason::CodexInvalidOutput)
        | Some(PolishFallbackReason::CodexOutputRejected) => CodexTestResult::Failed {
            reason: CodexTestFailureReason::OutputRejected,
        },
        _ => CodexTestResult::Failed {
            reason: CodexTestFailureReason::Unavailable,
        },
    }
}

#[tauri::command]
async fn test_codex_polish(
    request_id: u64,
    state: tauri::State<'_, AppState>,
) -> Result<CodexTestResult, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    register_codex_test_request(&state.codex_test_cancellations, request_id, cancel.clone())?;
    let worker_cancel = cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if worker_cancel.load(Ordering::SeqCst) {
            return CodexTestResult::Cancelled;
        }
        let settings = settings::Settings::load();
        if settings.llm_provider() != "codex" {
            return CodexTestResult::Failed {
                reason: CodexTestFailureReason::Unavailable,
            };
        }
        let input = "今天用 Clau-de 跑 training，版本是 2.7.1，不要升級到 3.0。".to_string();
        let local_terms = vec!["Claude".to_string()];
        let result = polish::transform_with_cancel_and_terms(
            &settings,
            &input,
            "",
            &local_terms,
            &[],
            &worker_cancel,
        );
        if result.metadata.outcome == polish::PolishOutcome::Fallback {
            return codex_test_failure(result.metadata.fallback_reason);
        }
        if !result.text.contains("Claude")
            || result.text.contains("Clau-de")
            || !result.text.contains("2.7.1")
            || !result.text.contains("不要")
            || !result.text.contains("3.0")
        {
            return CodexTestResult::Failed {
                reason: CodexTestFailureReason::OutputRejected,
            };
        }
        CodexTestResult::Success {
            input,
            output: result.text,
        }
    })
    .await;
    finish_codex_test_request(&state.codex_test_cancellations, request_id, &cancel);
    result.map_err(|error| error.to_string())
}

#[tauri::command]
fn cancel_codex_test(request_id: u64, state: tauri::State<'_, AppState>) -> bool {
    cancel_codex_test_request(&state.codex_test_cancellations, request_id)
}

fn cancel_codex_test_request(
    registry: &Mutex<CodexTestCancellationRegistry>,
    request_id: u64,
) -> bool {
    let mut registry = registry.lock().unwrap();
    match registry.entries.entry(request_id) {
        Entry::Occupied(entry) => {
            if let CodexTestCancellation::Active(cancel) = entry.get() {
                codex::cancel_request(cancel);
            }
            true
        }
        Entry::Vacant(entry) => {
            entry.insert(CodexTestCancellation::CancelledBeforeStart);
            registry.tombstone_order.push_back(request_id);
            while registry.tombstone_order.len() > MAX_CODEX_TEST_CANCEL_TOMBSTONES {
                if let Some(expired) = registry.tombstone_order.pop_front() {
                    if matches!(
                        registry.entries.get(&expired),
                        Some(CodexTestCancellation::CancelledBeforeStart)
                    ) {
                        registry.entries.remove(&expired);
                    }
                }
            }
            true
        }
    }
}

fn register_codex_test_request(
    registry: &Mutex<CodexTestCancellationRegistry>,
    request_id: u64,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut registry = registry.lock().unwrap();
    match registry.entries.entry(request_id) {
        Entry::Occupied(mut entry) => match entry.get() {
            CodexTestCancellation::Active(_) => Err("codex_test_duplicate".into()),
            CodexTestCancellation::CancelledBeforeStart => {
                cancel.store(true, Ordering::SeqCst);
                entry.insert(CodexTestCancellation::Active(cancel));
                registry
                    .tombstone_order
                    .retain(|existing| *existing != request_id);
                Ok(())
            }
        },
        Entry::Vacant(entry) => {
            entry.insert(CodexTestCancellation::Active(cancel));
            Ok(())
        }
    }
}

fn finish_codex_test_request(
    registry: &Mutex<CodexTestCancellationRegistry>,
    request_id: u64,
    cancel: &Arc<AtomicBool>,
) {
    let mut registry = registry.lock().unwrap();
    let owns_entry = matches!(
        registry.entries.get(&request_id),
        Some(CodexTestCancellation::Active(active)) if Arc::ptr_eq(active, cancel)
    );
    if owns_entry {
        registry.entries.remove(&request_id);
    }
}

#[tauri::command]
fn set_llm_key(key: String) -> Result<(), String> {
    polish::set_api_key(&key).map_err(|e| e.to_string())
}

/// 用固定樣例測潤飾設定；回傳整理後文字。
#[tauri::command]
async fn test_polish() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let settings = settings::Settings::load();
        if settings.polish_mode() == settings::PolishMode::Raw {
            return Err("RAW 模式不會呼叫整理引擎；請先選擇 CLEAN 或 ORGANIZE".into());
        }
        if let Some(reason) = polish::blocked_reason(&settings) {
            return Err(format!("目前安全退回 RAW：{reason}"));
        }
        let result = polish::transform(
            &settings,
            "嗯我們用 PyTorch，那個，跑訓練",
            "PyTorch training pipeline",
        );
        if result.metadata.outcome == polish::PolishOutcome::Fallback {
            return Err(format!(
                "整理失敗，已安全退回原文：{:?}",
                result.metadata.fallback_reason
            ));
        }
        Ok(result.text)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ─── 字典與上下文 ────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct DictEntry {
    from: String,
    to: String,
}

#[tauri::command]
fn get_dictionary() -> Vec<DictEntry> {
    settings::Settings::load()
        .dictionary()
        .into_iter()
        .map(|(from, to)| DictEntry { from, to })
        .collect()
}

/// 整份覆寫字典（UI 是小清單，整存最單純）；同步熱更新 pipeline。
#[tauri::command]
fn set_dictionary(entries: Vec<DictEntry>, state: tauri::State<AppState>) -> Result<(), String> {
    let mut map = serde_json::Map::new();
    for e in &entries {
        let from = e.from.trim();
        let to = e.to.trim();
        if !from.is_empty() && !to.is_empty() {
            map.insert(from.to_string(), serde_json::Value::String(to.to_string()));
        }
    }
    let update = || {
        settings::update_config_key(
            &settings::config_path(),
            "dictionary",
            serde_json::Value::Object(map),
        )
        .map_err(|e| e.to_string())?;
        *state.core.dict.lock().unwrap() = settings::Settings::load().dictionary();
        Ok(())
    };
    codex::with_policy_change(update)
}

#[tauri::command]
fn set_context_enabled(enabled: bool, state: tauri::State<AppState>) -> Result<(), String> {
    let update = || {
        if !enabled {
            *state.core.last_context.lock().unwrap() = None;
            *state.core.context_capture.lock().unwrap() = None;
            settings::update_config_keys(
                &settings::config_path(),
                vec![
                    ("context_enabled".into(), false.into()),
                    ("codex_share_context_terms".into(), false.into()),
                    ("codex_context_consent_version".into(), 0.into()),
                    (
                        "codex_context_consent_target".into(),
                        serde_json::Value::Null,
                    ),
                ],
            )
            .map_err(|e| e.to_string())
        } else {
            settings::update_config_key(
                &settings::config_path(),
                "context_enabled",
                serde_json::Value::Bool(true),
            )
            .map_err(|e| e.to_string())
        }
    };
    codex::with_policy_change(update)
}

#[tauri::command]
fn get_context_audit(state: tauri::State<AppState>) -> Option<context::ContextSnapshot> {
    state.core.last_context.lock().unwrap().clone()
}

#[tauri::command]
fn clear_context_audit(state: tauri::State<AppState>) {
    *state.core.last_context.lock().unwrap() = None;
}

/// 首次設定的持久化完成點。前端只會在本次麥克風測試通過後呼叫；
/// 後端再獨立驗證輔助使用／熱鍵與模型完整性，不接受單純的 UI 旗標。
#[tauri::command]
fn complete_setup(state: tauri::State<AppState>) -> Result<(), String> {
    if !accessibility_trusted() || !HOTKEY_ACTIVE.load(Ordering::SeqCst) {
        return Err("輔助使用權限或快捷鍵尚未就緒".into());
    }
    let spec = *state.core.active_model.lock().unwrap();
    if !registry::model_is_verified(spec) {
        return Err("語音模型尚未下載或完整性驗證失敗".into());
    }
    let current = settings::Settings::load();
    if let Err(reason) = validate_setup_polish(&current) {
        return Err(format!(
            "文字整理尚未就緒（{reason}）；請選擇本機整理或明確使用 RAW"
        ));
    }
    if !current.setup_completed() && !state.mic_test_passed.load(Ordering::SeqCst) {
        return Err("本次尚未完成麥克風收音測試".into());
    }
    if !current.setup_completed() && state.core.successful_pastes.load(Ordering::SeqCst) == 0 {
        return Err("請先在任意輸入框完成一次真正的聽寫與貼上".into());
    }
    settings::update_config_key(
        &settings::config_path(),
        "setup_completed",
        serde_json::Value::Bool(true),
    )
    .map_err(|e| e.to_string())
}

fn validate_setup_polish(current: &settings::Settings) -> Result<(), &'static str> {
    if current.polish_mode() == settings::PolishMode::Raw {
        Ok(())
    } else if let Some(reason) = polish::blocked_reason(current) {
        Err(reason)
    } else {
        Ok(())
    }
}

// ─── 歷史與剪貼簿 ────────────────────────────────────────────────────────────

#[tauri::command]
fn get_history(n: usize) -> Vec<serde_json::Value> {
    history::read_recent(n.min(500), &history::history_path())
}

#[tauri::command]
fn get_pending_result(state: tauri::State<AppState>) -> Option<pipeline::PendingResult> {
    state.core.pending_results.lock().unwrap().front().cloned()
}

#[tauri::command]
fn clear_pending_result(state: tauri::State<AppState>) {
    state.core.pending_results.lock().unwrap().pop_front();
}

#[tauri::command]
fn clear_history(state: tauri::State<AppState>) -> Result<(), String> {
    history::clear(&history::history_path()).map_err(|error| error.to_string())?;
    state.core.pending_results.lock().unwrap().clear();
    Ok(())
}

#[tauri::command]
fn set_history_enabled(enabled: bool) -> Result<(), String> {
    settings::update_config_key(
        &settings::config_path(),
        "history_enabled",
        serde_json::Value::Bool(enabled),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| e.to_string())
}

// ─── 熱鍵 ─────────────────────────────────────────────────────────────────────

static HOTKEY_ACTIVE: AtomicBool = AtomicBool::new(false);

fn wire_hotkey(core: &Arc<pipeline::Core>) -> Result<(), String> {
    let combo = settings::Settings::load().hotkey_combo();
    let service = hotkey::start(hotkey::parse_combo(&combo)).map_err(|e| e.to_string())?;
    HOTKEY_ACTIVE.store(true, Ordering::SeqCst);
    let tx = core.msg_tx.clone();
    let events = service.events;
    std::thread::spawn(move || {
        for ev in events {
            if tx.send(pipeline::Msg::Hotkey(ev)).is_err() {
                break;
            }
        }
    });
    *core.esc_ctl.lock().unwrap() = service.esc_ctl;
    Ok(())
}

/// 換主熱鍵：驗證 → 寫 config → 熱鍵執行緒動態換註冊（免重啟）。
#[tauri::command]
fn set_hotkey(combo: String, state: tauri::State<AppState>) -> Result<(), String> {
    let hk: handy_keys::Hotkey = combo.parse().map_err(|e| format!("無效的快捷鍵：{e}"))?;
    settings::update_config_key(
        &settings::config_path(),
        "hotkey",
        serde_json::Value::String(combo),
    )
    .map_err(|e| e.to_string())?;
    // 若正按著舊熱鍵錄音，先強制收尾——換鍵後舊鍵的 release 事件
    // 會因 id 不符被忽略，不收尾會卡在錄音狀態（review 抓到的情境）
    let session = state.core.sm.lock().unwrap().session();
    let _ = state.core.msg_tx.send(pipeline::Msg::ForceStop(session));
    let _ = state
        .core
        .esc_ctl
        .lock()
        .unwrap()
        .send(hotkey::Ctl::SetMain(hk));
    Ok(())
}

/// 打開 系統設定 → 隱私與安全性 → 輔助使用（授權引導用）
#[tauri::command]
fn open_accessibility_settings(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            None::<String>,
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_microphone_settings(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
            None::<String>,
        )
        .map_err(|error| error.to_string())
}

/// 「已勾選卻仍顯示未啟用」的解法：無正式簽章的 app 每次重建簽章雜湊都變，
/// macOS 的輔助使用授權綁簽章——舊條目看似勾選、實際對新版無效，重新勾選
/// 也救不回來。tccutil 清掉本 app 的舊條目後重新觸發系統詢問；授權後
/// init_core 的輪詢執行緒會自動接上熱鍵，不用重開 app。
#[tauri::command]
fn reset_accessibility(app: tauri::AppHandle) -> Result<(), String> {
    let id = app.config().identifier.clone();
    let out = std::process::Command::new("tccutil")
        .args(["reset", "Accessibility", &id])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    #[cfg(target_os = "macos")]
    let _ = macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
    Ok(())
}

/// 使用者授予輔助使用權限後，不用重啟就能啟用熱鍵。
#[tauri::command]
fn retry_hotkey(state: tauri::State<AppState>) -> Result<bool, String> {
    if HOTKEY_ACTIVE.load(Ordering::SeqCst) {
        return Ok(true);
    }
    if !accessibility_trusted() {
        return Ok(false);
    }
    wire_hotkey(&state.core)?;
    Ok(true)
}

// ─── 進入點 ───────────────────────────────────────────────────────────────────

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn refresh_codex_contract_on_startup() {
    let (current, policy_epoch) = codex::policy_snapshot(settings::Settings::load);
    if current.llm_provider() != "codex" {
        return;
    }
    let status = codex::probe(current.codex_cli_path().as_deref());
    let ready_contract_changed = status.availability == codex::CodexAvailability::Ready
        && (status.auth_mode != current.codex_auth_mode()
            || status
                .contract
                .as_ref()
                .is_none_or(|contract| !current.codex_contract_matches(contract)));
    let explicitly_unusable = matches!(
        status.availability,
        codex::CodexAvailability::NotInstalled
            | codex::CodexAvailability::NotAuthenticated
            | codex::CodexAvailability::Unsupported
            | codex::CodexAvailability::MissingCapability
    );
    if !ready_contract_changed && !explicitly_unusable {
        return;
    }
    if let Err(error) = codex::with_policy_change_if(policy_epoch, || {
        if settings::Settings::load().llm_provider() != "codex" {
            return Err("codex_request_stale".into());
        }
        settings::update_config_keys(
            &settings::config_path(),
            vec![
                ("local_only".into(), true.into()),
                ("cloud_consent_version".into(), 0.into()),
                ("cloud_consent_origin".into(), serde_json::Value::Null),
                ("cloud_consent_target".into(), serde_json::Value::Null),
                ("codex_share_context_terms".into(), false.into()),
                ("codex_context_consent_version".into(), 0.into()),
                (
                    "codex_context_consent_target".into(),
                    serde_json::Value::Null,
                ),
            ],
        )
        .map_err(|error| error.to_string())
    }) {
        tracing::error!("failed to revoke stale Codex consent during startup: {error}");
    }
}

/// 所有重初始化（引擎、overlay、熱鍵、預載）都在這裡——
/// 必須在 single-instance 檢查之後才跑，第二個實例才不會 spawn 任何東西。
fn init_core(app: &tauri::AppHandle) {
    let cfg = settings::Settings::load();
    let spec = registry::resolve(&cfg.whisper_model());
    tracing::info!("Claro starting — STT model: {}", spec.id);

    // 正式聽寫 hot path 只接受本次 process 已完成的 capability contract。
    // 啟動時背景重建快取；CLI path、raw version、bytes 或 features 漂移時
    // 先撤銷同意，絕不把第一段 transcript 當 capability probe。
    std::thread::spawn(refresh_codex_contract_on_startup);

    #[cfg(target_os = "macos")]
    let engine: Box<dyn stt::SttEngine> = Box::new(stt::transcribe::TranscribeEngine::new(
        spec,
        registry::model_path(spec),
    ));
    #[cfg(not(target_os = "macos"))]
    let engine: Box<dyn stt::SttEngine> = unimplemented!("M1 is macOS-only");

    let overlay = overlay_client::OverlayClient::start();
    let (msg_tx, msg_rx) = crossbeam_channel::unbounded::<pipeline::Msg>();
    let (dummy_esc, _keep) = crossbeam_channel::unbounded();
    std::mem::forget(_keep); // 佔位 receiver；wire_hotkey 成功後會被真的 sender 換掉

    let mic_test: Arc<Mutex<Option<CaptureHandle>>> = Arc::new(Mutex::new(None));
    let mic_test_passed = Arc::new(AtomicBool::new(false));
    let mic_test_generation = Arc::new(AtomicU64::new(0));
    let stop_mic_test = {
        let mic_test = mic_test.clone();
        let passed = mic_test_passed.clone();
        let generation = mic_test_generation.clone();
        let app = app.clone();
        Box::new(move || {
            let handle = mic_test.lock().unwrap().take();
            if let Some(handle) = handle {
                let current_generation = generation.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = handle.stop_immediately();
                emit_mic_inactive(&app, current_generation, passed.load(Ordering::SeqCst));
            }
        }) as Box<dyn Fn() + Send + Sync>
    };

    let core = Arc::new(pipeline::Core::new(
        engine,
        spec,
        overlay,
        Box::new(inject::MacPasteInjector),
        stop_mic_test,
        dummy_esc,
        msg_tx.clone(),
        cfg.input_device(),
    ));

    if let Err(e) = wire_hotkey(&core) {
        tracing::warn!("hotkey unavailable ({e}) — prompting for Accessibility");
        // 明確告訴使用者發生什麼：跳系統授權提示（只在未授權時出現一次）
        #[cfg(target_os = "macos")]
        if !accessibility_trusted() {
            let _ = macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
        }
        // 授權後自動啟用，不需要使用者回設定頁按「重試」
        let core = core.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if HOTKEY_ACTIVE.load(Ordering::SeqCst) {
                break;
            }
            if accessibility_trusted() {
                match wire_hotkey(&core) {
                    Ok(()) => {
                        tracing::info!("accessibility granted — hotkey now active");
                        break;
                    }
                    Err(e) => tracing::warn!("hotkey retry failed: {e}"),
                }
            }
        });
    }

    {
        let core = core.clone();
        std::thread::spawn(move || pipeline::run_dispatcher(core, msg_rx));
    }

    // 模型預載（啟動即背景載入，首次聽寫不用等）；
    // 之後交給閒置看門狗——5 分鐘沒聽寫就卸載（待機記憶體預算，SPEC §12），
    // 下次錄音開始時邊錄邊回載
    {
        let core = core.clone();
        std::thread::spawn(move || {
            if registry::model_is_verified(spec) {
                // 校驗（整檔 hash 可能數秒）期間 app 可能已進入收尾——
                // 這時載入會在 exit handler 卸載後把 Metal 模型又載回來
                if SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                if let Err(e) = core.engine.lock().unwrap().load() {
                    tracing::warn!("model preload failed: {e}");
                }
            } else {
                tracing::info!(
                    "model missing or failed integrity verification — open the Claro window to download"
                );
            }
        });
    }
    pipeline::spawn_stt_idle_watcher(core.clone());

    // 模型檔背景預校驗：還沒有持久 marker 的既有模型檔（升級後第一次啟動、
    // 或校驗器版本變更）第一次 verify 要整檔 SHA-256，數 GB 會卡住第一個
    // 碰到它的路徑——聽寫中載入內建 LLM、設定頁模型列表都會扛到。
    // 啟動時先在背景把所有已下載模型的 marker 補齊；之後的 verify 都是
    // 讀 marker 的 O(1)。上面 whisper 預載已順路校驗使用中模型，這裡補
    // 內建 LLM 與其餘已下載的 whisper 變體。
    std::thread::spawn(|| {
        for spec in llm::LLM_MODELS {
            let path = llm::model_path(spec);
            if path.exists() && !models::model_file_is_verified(&path, spec.sha256) {
                tracing::warn!("llm model failed integrity pre-verification: {}", spec.id);
            }
        }
        for spec in registry::MODELS {
            let path = registry::model_path(spec);
            if path.exists() && !models::model_file_is_verified(&path, spec.sha256) {
                tracing::warn!("stt model failed integrity pre-verification: {}", spec.id);
            }
        }
    });

    // SIGTERM/SIGINT：走 AppHandle::exit 優雅收場（直接 exit 會觸發 crash reporter）
    {
        let core = core.clone();
        let handle = app.clone();
        if let Err(e) = ctrlc::set_handler(move || {
            core.overlay.stop();
            handle.exit(0);
        }) {
            tracing::warn!("signal handler setup failed: {e}");
        }
    }

    app.manage(AppState {
        core,
        download_gate: Arc::new(Mutex::new(None)),
        downloading: Arc::new(Mutex::new(None)),
        llm_downloading: Arc::new(Mutex::new(None)),
        download_cancel: Arc::new(AtomicBool::new(false)),
        mic_test,
        mic_test_passed,
        mic_test_generation,
        codex_test_cancellations: Arc::new(Mutex::new(CodexTestCancellationRegistry::default())),
    });
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod product_gate_tests {
    use super::*;

    fn settings_with(values: &[(&str, serde_json::Value)]) -> settings::Settings {
        let mut raw = settings::default_config();
        for (key, value) in values {
            raw.insert((*key).to_string(), value.clone());
        }
        settings::Settings { raw }
    }

    #[test]
    fn fresh_user_can_choose_raw_or_clean_before_provider_is_ready() {
        assert!(
            validate_polish_mode_consent(settings::PolishMode::Raw, false, false, false).is_ok()
        );
        assert!(
            validate_polish_mode_consent(settings::PolishMode::Clean, false, false, false).is_ok()
        );
        assert!(
            validate_polish_mode_consent(settings::PolishMode::Correct, false, false, false)
                .is_err()
        );
        assert!(
            validate_polish_mode_consent(settings::PolishMode::Organize, false, false, false)
                .is_err()
        );
    }

    #[test]
    fn polish_mode_provider_contract_rejects_incompatible_combinations() {
        assert!(validate_polish_mode_provider(settings::PolishMode::Raw, "codex").is_ok());
        assert!(validate_polish_mode_provider(settings::PolishMode::Correct, "codex").is_ok());
        assert!(validate_polish_mode_provider(settings::PolishMode::Correct, "apple").is_err());
        assert!(validate_polish_mode_provider(settings::PolishMode::Clean, "codex").is_err());
        assert!(validate_polish_mode_provider(settings::PolishMode::Organize, "codex").is_err());
        assert!(validate_polish_mode_provider(settings::PolishMode::Clean, "off").is_ok());
    }

    #[test]
    fn setup_requires_explicit_raw_or_a_ready_provider() {
        let fresh = settings_with(&[]);
        assert_eq!(validate_setup_polish(&fresh), Err("provider_missing"));
        let raw = settings_with(&[("polish_mode", serde_json::json!("raw"))]);
        assert_eq!(validate_setup_polish(&raw), Ok(()));
    }

    #[test]
    fn model_downloads_share_one_global_gate() {
        let gate = Mutex::new(None);
        assert!(reserve_download(&gate, "stt").is_ok());
        assert!(reserve_download(&gate, "llm").is_err());
        release_download(&gate);
        assert!(reserve_download(&gate, "llm").is_ok());
    }

    #[test]
    fn final_download_event_observes_authoritative_idle_state() {
        static TEST_DOWNLOAD_ACTIVE: AtomicBool = AtomicBool::new(false);

        let downloading = Arc::new(Mutex::new(Some("stt")));
        let gate = Arc::new(Mutex::new(Some("stt")));
        let session =
            DownloadSession::with_active(downloading.clone(), gate.clone(), &TEST_DOWNLOAD_ACTIVE);
        assert!(TEST_DOWNLOAD_ACTIVE.load(Ordering::SeqCst));

        let mut emitted = false;
        session.finish(|| {
            assert_eq!(*downloading.lock().unwrap(), None);
            assert_eq!(*gate.lock().unwrap(), None);
            assert!(!TEST_DOWNLOAD_ACTIVE.load(Ordering::SeqCst));
            emitted = true;
        });

        assert!(emitted);
    }

    #[test]
    fn cancelling_download_interrupts_resource_wait() {
        use std::sync::mpsc;

        let resource = Arc::new(Mutex::new(()));
        let held = resource.lock().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let waiter_resource = resource.clone();
        let waiter_cancel = cancel.clone();
        let (attempt_tx, attempt_rx) = mpsc::channel();

        let waiter = std::thread::spawn(move || {
            let mut announced = false;
            wait_for_download_resource(&waiter_cancel, || {
                if !announced {
                    attempt_tx.send(()).unwrap();
                    announced = true;
                }
                Ok(waiter_resource.try_lock().is_ok())
            })
        });

        attempt_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("waiter should try the held resource");
        cancel.store(true, Ordering::SeqCst);
        let result = waiter.join().expect("waiter should not panic");
        assert_eq!(result, Err(models::DOWNLOAD_CANCELLED.into()));
        drop(held);
    }

    #[test]
    fn codex_test_cancel_is_request_scoped_and_does_not_touch_dictation() {
        let active = Mutex::new(CodexTestCancellationRegistry::default());
        let first_test = Arc::new(AtomicBool::new(false));
        let second_test = Arc::new(AtomicBool::new(false));
        let formal_dictation = AtomicBool::new(false);
        register_codex_test_request(&active, 41, first_test.clone()).unwrap();
        register_codex_test_request(&active, 42, second_test.clone()).unwrap();

        assert!(cancel_codex_test_request(&active, 41));
        assert!(first_test.load(Ordering::SeqCst));
        assert!(!second_test.load(Ordering::SeqCst));
        assert!(!formal_dictation.load(Ordering::SeqCst));
        assert!(cancel_codex_test_request(&active, 99));
    }

    #[test]
    fn codex_test_duplicate_does_not_overwrite_the_original_cancel_handle() {
        let registry = Mutex::new(CodexTestCancellationRegistry::default());
        let original = Arc::new(AtomicBool::new(false));
        let duplicate = Arc::new(AtomicBool::new(false));
        register_codex_test_request(&registry, 7, original.clone()).unwrap();

        assert_eq!(
            register_codex_test_request(&registry, 7, duplicate.clone()),
            Err("codex_test_duplicate".into())
        );
        assert!(cancel_codex_test_request(&registry, 7));
        assert!(original.load(Ordering::SeqCst));
        assert!(!duplicate.load(Ordering::SeqCst));
    }

    #[test]
    fn codex_test_cancel_before_start_is_consumed_and_tombstones_are_bounded() {
        let registry = Mutex::new(CodexTestCancellationRegistry::default());
        assert!(cancel_codex_test_request(&registry, 55));
        let late = Arc::new(AtomicBool::new(false));
        register_codex_test_request(&registry, 55, late.clone()).unwrap();
        assert!(late.load(Ordering::SeqCst));
        finish_codex_test_request(&registry, 55, &late);

        for request_id in 1_000..(1_000 + MAX_CODEX_TEST_CANCEL_TOMBSTONES as u64 + 5) {
            assert!(cancel_codex_test_request(&registry, request_id));
        }
        let registry = registry.lock().unwrap();
        assert_eq!(
            registry.tombstone_order.len(),
            MAX_CODEX_TEST_CANCEL_TOMBSTONES
        );
        assert_eq!(
            registry
                .entries
                .values()
                .filter(|value| matches!(value, CodexTestCancellation::CancelledBeforeStart))
                .count(),
            MAX_CODEX_TEST_CANCEL_TOMBSTONES
        );
    }

    #[test]
    fn codex_test_failures_use_the_typed_ui_contract() {
        assert_eq!(
            codex_test_failure(Some(polish::PolishFallbackReason::CodexTimeout)),
            CodexTestResult::Failed {
                reason: CodexTestFailureReason::Timeout,
            }
        );
        assert_eq!(
            codex_test_failure(Some(polish::PolishFallbackReason::CodexOutputRejected)),
            CodexTestResult::Failed {
                reason: CodexTestFailureReason::OutputRejected,
            }
        );
        assert_eq!(
            codex_test_failure(Some(polish::PolishFallbackReason::CodexCancelled)),
            CodexTestResult::Cancelled
        );
        assert_eq!(
            codex_test_failure(Some(polish::PolishFallbackReason::CloudConsentRequired)),
            CodexTestResult::Failed {
                reason: CodexTestFailureReason::ConsentChanged,
            }
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    tauri::Builder::default()
        // 防雙開（雙實例會雙重貼上）；二度啟動時喚出既有視窗。必須是第一個 plugin。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_hardware_profile,
            set_input_device,
            mic_test_start,
            mic_test_stop,
            list_models,
            download_model,
            cancel_download,
            set_model,
            delete_model,
            get_llm_config,
            set_llm_config,
            set_polish_mode,
            set_local_only,
            get_codex_status,
            enable_codex_provider,
            set_codex_correction_preferences,
            set_codex_context_terms_enabled,
            test_codex_polish,
            cancel_codex_test,
            set_llm_key,
            list_provider_models,
            test_polish,
            get_dictionary,
            set_dictionary,
            set_context_enabled,
            get_context_audit,
            clear_context_audit,
            complete_setup,
            list_builtin_llms,
            download_builtin_llm,
            delete_builtin_llm,
            set_hotkey,
            open_accessibility_settings,
            open_microphone_settings,
            reset_accessibility,
            get_history,
            get_pending_result,
            clear_pending_result,
            clear_history,
            set_history_enabled,
            copy_text,
            retry_hotkey
        ])
        .setup(|app| {
            init_core(app.handle());
            if std::env::var("CLARO_DEVTOOLS").as_deref() == Ok("1") {
                if let Some(w) = app.get_webview_window("main") {
                    w.open_devtools();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let _ = window.hide();
                    api.prevent_close();
                }
                tauri::WindowEvent::Focused(false) => {
                    // 首次設定會請使用者切到 TextEdit 試聽寫；視窗失焦時先釋放
                    // mic test，避免正式錄音與測試同時佔用 USB／Bluetooth 裝置。
                    if let Some(state) = window.app_handle().try_state::<AppState>() {
                        let _audio_gate = state.core.audio_start_gate.lock().unwrap();
                        let handle = state.mic_test.lock().unwrap().take();
                        if let Some(handle) = handle {
                            let generation =
                                state.mic_test_generation.fetch_add(1, Ordering::SeqCst) + 1;
                            let _ = handle.stop_immediately();
                            emit_mic_inactive(
                                window.app_handle(),
                                generation,
                                state.mic_test_passed.load(Ordering::SeqCst),
                            );
                        }
                    }
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app, event| match event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_main_window(app),
            tauri::RunEvent::Exit => {
                // 先立旗再卸載：擋住所有還在排隊的背景載入（見 SHUTTING_DOWN 註解）
                crate::SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = codex::cancel_active_and_wait();
                if let Some(state) = app.try_state::<AppState>() {
                    state.core.overlay.stop();
                    // 主動釋放 whisper/llama 的 Metal 資源：留給 atexit teardown 會
                    // ggml_abort（SIGABRT + crash report）。背景載入/轉錄/潤飾可能
                    // 占著鎖，最多等 3s；還是拿不到就跳過 atexit 直接收場（_exit），
                    // 寧可少跑 teardown 也不要給使用者一個 crash 對話框。
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                    let mut whisper_done = false;
                    loop {
                        if !whisper_done {
                            if let Ok(mut engine) = state.core.engine.try_lock() {
                                engine.unload();
                                whisper_done = true;
                            }
                        }
                        let llm_done = whisper_done && llm::unload_now();
                        if llm_done {
                            break;
                        }
                        if std::time::Instant::now() >= deadline {
                            tracing::warn!(
                                "engine busy at exit — fast-exit to skip Metal teardown"
                            );
                            unsafe { libc::_exit(0) };
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            }
            _ => {}
        });
}
