pub mod audio;
pub mod context;
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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::audio::CaptureHandle;
use crate::state_machine::State;
use crate::stt::registry;

struct AppState {
    core: Arc<pipeline::Core>,
    downloading: Arc<Mutex<Option<&'static str>>>,
    llm_downloading: Arc<Mutex<Option<&'static str>>>,
    mic_test: Arc<Mutex<Option<CaptureHandle>>>,
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

#[tauri::command]
fn set_input_device(name: String, state: tauri::State<AppState>) -> Result<(), String> {
    let selection = if name.is_empty() {
        None
    } else {
        Some(name.clone())
    };
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
                break;
            }
            if started.elapsed().as_secs() >= 30 {
                if let Some(h) = mic_test.lock().unwrap().take() {
                    let _ = h.stop();
                }
                break;
            }
            let _ = app.emit(
                "mic-level",
                MicLevel {
                    level: level.get(),
                    active: true,
                },
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = app.emit(
            "mic-level",
            MicLevel {
                level: 0.0,
                active: false,
            },
        );
    });
    Ok(())
}

#[tauri::command]
fn mic_test_stop(state: tauri::State<AppState>) {
    if let Some(h) = state.mic_test.lock().unwrap().take() {
        let _ = h.stop();
    }
}

// ─── 模型管理 ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ModelInfo {
    id: &'static str,
    label: &'static str,
    desc: &'static str,
    size_mb: u64,
    recommended: bool,
    downloaded: bool,
    active: bool,
    downloading: bool,
}

#[tauri::command]
fn list_models(state: tauri::State<AppState>) -> Vec<ModelInfo> {
    let active = state.core.active_model.lock().unwrap().id;
    let downloading = *state.downloading.lock().unwrap();
    registry::MODELS
        .iter()
        .map(|m| ModelInfo {
            id: m.id,
            label: m.label,
            desc: m.desc,
            size_mb: m.approx_bytes / 1_048_576,
            recommended: m.recommended,
            downloaded: registry::model_is_verified(m),
            active: m.id == active,
            downloading: downloading == Some(m.id),
        })
        .collect()
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    model_id: &'static str,
    downloaded_mb: u64,
    total_mb: Option<u64>,
    done: bool,
    error: Option<String>,
}

/// 使用者在 UI 明確點擊後才會呼叫（絕不自動下載，SPEC §6）
#[tauri::command]
fn download_model(
    id: String,
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let spec = registry::find(&id).ok_or("未知的模型")?;
    {
        let mut guard = state.downloading.lock().unwrap();
        if guard.is_some() {
            return Err("已有模型在下載中".into());
        }
        *guard = Some(spec.id);
    }
    let downloading = state.downloading.clone();
    std::thread::spawn(move || {
        let dest = registry::model_path(spec);
        let result = models::download(spec.url, &dest, spec.sha256, |p| {
            let _ = app.emit(
                "model-download",
                DownloadProgress {
                    model_id: spec.id,
                    downloaded_mb: p.downloaded / 1_048_576,
                    total_mb: p.total.map(|t| t / 1_048_576),
                    done: false,
                    error: None,
                },
            );
        });
        let payload = match result {
            Ok(()) => DownloadProgress {
                model_id: spec.id,
                downloaded_mb: 0,
                total_mb: None,
                done: true,
                error: None,
            },
            Err(e) => DownloadProgress {
                model_id: spec.id,
                downloaded_mb: 0,
                total_mb: None,
                done: false,
                error: Some(e.to_string()),
            },
        };
        let _ = app.emit("model-download", payload);
        *downloading.lock().unwrap() = None;
    });
    Ok(())
}

/// 切換使用中的模型（需已下載）；引擎熱換、背景預載、寫回 config。
#[tauri::command]
fn set_model(id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let spec = registry::find(&id).ok_or("未知的模型")?;
    models::verify_model_file(&registry::model_path(spec), spec.sha256)
        .map_err(|e| format!("請先下載完整且驗證通過的模型：{e}"))?;
    if state.core.sm.lock().unwrap().state() != State::Idle {
        return Err("聽寫進行中，稍後再切換".into());
    }
    state.core.swap_model(spec);
    settings::update_config_key(
        &settings::config_path(),
        "whisper_model",
        serde_json::Value::String(spec.id.into()),
    )
    .map_err(|e| e.to_string())
}

/// 刪除已下載的模型檔（使用中的不可刪）。
#[tauri::command]
fn delete_model(id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let spec = registry::find(&id).ok_or("未知的模型")?;
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
    cloud_consent_valid: bool,
    execution_location: polish::ExecutionLocation,
    endpoint_origin: Option<String>,
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
        cloud_consent_valid: s.cloud_consent_valid(),
        execution_location: polish::execution_location(s),
        endpoint_origin: polish::endpoint_origin(s),
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
    {
        let mut guard = state.llm_downloading.lock().unwrap();
        if guard.is_some() {
            return Err("已有模型在下載中".into());
        }
        *guard = Some(spec.id);
    }
    let downloading = state.llm_downloading.clone();
    std::thread::spawn(move || {
        let dest = llm::model_path(spec);
        let result = models::download(spec.url, &dest, spec.sha256, |p| {
            let _ = app.emit(
                "llm-model-download",
                DownloadProgress {
                    model_id: spec.id,
                    downloaded_mb: p.downloaded / 1_048_576,
                    total_mb: p.total.map(|t| t / 1_048_576),
                    done: false,
                    error: None,
                },
            );
        });
        let payload = match result {
            Ok(()) => DownloadProgress {
                model_id: spec.id,
                downloaded_mb: 0,
                total_mb: None,
                done: true,
                error: None,
            },
            Err(e) => DownloadProgress {
                model_id: spec.id,
                downloaded_mb: 0,
                total_mb: None,
                done: false,
                error: Some(e.to_string()),
            },
        };
        let _ = app.emit("llm-model-download", payload);
        *downloading.lock().unwrap() = None;
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
    if !["off", "apple", "builtin", "ollama", "lmstudio", "custom"].contains(&provider.as_str()) {
        return Err("未知的處理引擎".into());
    }
    // confirmed 保留在 command contract，讓 UI 能明確表達敏感操作；雲端權限
    // 只允許由 set_local_only 寫入，單純改 provider/URL 絕不順便解除隱私 gate。
    let _ = confirmed;
    settings::update_config_keys(
        &settings::config_path(),
        vec![
            ("llm_provider".into(), provider.into()),
            ("llm_polish_model".into(), model.into()),
            ("llm_base_url".into(), base_url.into()),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(get_llm_config())
}

#[tauri::command]
fn set_polish_mode(mode: String, confirmed: bool) -> Result<LlmConfig, String> {
    let mode = mode.parse::<settings::PolishMode>()?;
    let current = settings::Settings::load();
    if mode == settings::PolishMode::Organize && !current.organize_consent_valid() && !confirmed {
        return Err("啟用 ORGANIZE 前必須明確確認它會重排句序與段落".into());
    }
    let mut pairs = vec![("polish_mode".into(), mode.as_str().into())];
    if mode == settings::PolishMode::Organize && confirmed {
        pairs.push((
            "organize_consent_version".into(),
            settings::ORGANIZE_CONSENT_VERSION.into(),
        ));
    }
    settings::update_config_keys(&settings::config_path(), pairs).map_err(|e| e.to_string())?;
    Ok(get_llm_config())
}

#[tauri::command]
fn set_local_only(enabled: bool, confirmed: bool) -> Result<LlmConfig, String> {
    let current = settings::Settings::load();
    if !enabled && current.local_only() && !confirmed {
        return Err("關閉「僅限本機」前必須明確確認雲端資料傳送".into());
    }
    let mut pairs = vec![("local_only".into(), enabled.into())];
    if !enabled && confirmed {
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
    }
    settings::update_config_keys(&settings::config_path(), pairs).map_err(|e| e.to_string())?;
    Ok(get_llm_config())
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
            "嗯我們用 hyTorch，那個，跑訓練",
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
    settings::update_config_key(
        &settings::config_path(),
        "dictionary",
        serde_json::Value::Object(map),
    )
    .map_err(|e| e.to_string())?;
    *state.core.dict.lock().unwrap() = settings::Settings::load().dictionary();
    Ok(())
}

#[tauri::command]
fn set_context_enabled(enabled: bool) -> Result<(), String> {
    settings::update_config_key(
        &settings::config_path(),
        "context_enabled",
        serde_json::Value::Bool(enabled),
    )
    .map_err(|e| e.to_string())
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
    settings::update_config_key(
        &settings::config_path(),
        "setup_completed",
        serde_json::Value::Bool(true),
    )
    .map_err(|e| e.to_string())
}

// ─── 歷史與剪貼簿 ────────────────────────────────────────────────────────────

#[tauri::command]
fn get_history(n: usize) -> Vec<serde_json::Value> {
    history::read_recent(n.min(500), &history::history_path())
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
    let _ = state.core.msg_tx.send(pipeline::Msg::ForceStop);
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

/// 所有重初始化（引擎、overlay、熱鍵、預載）都在這裡——
/// 必須在 single-instance 檢查之後才跑，第二個實例才不會 spawn 任何東西。
fn init_core(app: &tauri::AppHandle) {
    let cfg = settings::Settings::load();
    let spec = registry::resolve(&cfg.whisper_model());
    tracing::info!("Claro starting — whisper model: {}", spec.id);

    #[cfg(target_os = "macos")]
    let engine: Box<dyn stt::SttEngine> = Box::new(stt::whisper::WhisperEngine::new(
        spec.id,
        registry::model_path(spec),
    ));
    #[cfg(not(target_os = "macos"))]
    let engine: Box<dyn stt::SttEngine> = unimplemented!("M1 is macOS-only");

    let overlay = overlay_client::OverlayClient::start();
    let (msg_tx, msg_rx) = crossbeam_channel::unbounded::<pipeline::Msg>();
    let (dummy_esc, _keep) = crossbeam_channel::unbounded();
    std::mem::forget(_keep); // 佔位 receiver；wire_hotkey 成功後會被真的 sender 換掉

    let core = Arc::new(pipeline::Core::new(
        engine,
        spec,
        overlay,
        Box::new(inject::MacPasteInjector),
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
        downloading: Arc::new(Mutex::new(None)),
        llm_downloading: Arc::new(Mutex::new(None)),
        mic_test: Arc::new(Mutex::new(None)),
    });
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
            set_input_device,
            mic_test_start,
            mic_test_stop,
            list_models,
            download_model,
            set_model,
            delete_model,
            get_llm_config,
            set_llm_config,
            set_polish_mode,
            set_local_only,
            set_llm_key,
            list_provider_models,
            test_polish,
            get_dictionary,
            set_dictionary,
            set_context_enabled,
            complete_setup,
            list_builtin_llms,
            download_builtin_llm,
            delete_builtin_llm,
            set_hotkey,
            open_accessibility_settings,
            reset_accessibility,
            get_history,
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
