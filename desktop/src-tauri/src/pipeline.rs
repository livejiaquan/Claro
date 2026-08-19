//! 單次聽寫 session 的編排（SPEC §3）。
//! 結構移植 prototype：熱鍵事件由單一 dispatcher 執行緒序列化餵狀態機
//! （keyDown/keyUp 亂序會卡死錄音——prototype 踩過的坑）；
//! 錄音、轉錄各自在背景執行緒；每個處理 session 有自己的取消旗標。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::audio::{self, CaptureHandle};
use crate::history::{self, NewEntry};
use crate::hotkey::{Ctl, HotkeyMsg};
use crate::inject::TextInjector;
use crate::overlay_client::OverlayClient;
use crate::polish;
use crate::settings::{PolishMode, Settings};
use crate::state_machine::{DictationStateMachine, SmAction, State};
use crate::stt::registry::ModelSpec;
use crate::stt::SttEngine;
use crate::textproc;

/// 送進 initial_prompt 的候選詞上限。真正的天花板是 `build_initial_prompt`
/// 的 token 預算（Whisper 硬限 223 token）；這個數字只是避免無謂地掃過長的
/// 畫面文字，取得比預算略寬即可。
const PROMPT_TERM_LIMIT: usize = 48;

pub enum Msg {
    Hotkey(HotkeyMsg),
    ForceStop(u64),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingResult {
    pub raw: String,
    pub text: String,
    pub reason: &'static str,
}

/// 單次聽寫的即時 UI 事件。文字本身不放進 event；需要救援時由 UI
/// 透過既有的 `get_pending_result` 取回，避免把完整 transcript 複製到多條通道。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DictationEvent {
    pub session: u64,
    pub phase: &'static str,
    pub outcome: Option<&'static str>,
    pub recovery_available: bool,
}

pub type DictationEventSink = Arc<dyn Fn(DictationEvent) + Send + Sync>;

/// 成功貼上只累加成功次數，不得清掉較早尚未由使用者處理的救援文字。
fn record_success(successful_pastes: &AtomicU64) {
    successful_pastes.fetch_add(1, Ordering::SeqCst);
}

fn enqueue_pending_result(queue: &Mutex<VecDeque<PendingResult>>, result: PendingResult) {
    // Recovery UI must show the failure that just happened. Keep older unhandled
    // results behind it instead of letting a stale item masquerade as this session.
    queue.lock().unwrap().push_front(result);
}

fn codex_adoption_policy_is_current(metadata: &polish::PolishMetadata) -> Result<(), String> {
    let current = Settings::load();
    if current.llm_provider() != "codex"
        || current.polish_mode() != PolishMode::Correct
        || polish::blocked_reason(&current).is_some()
        || (metadata.codex_context_used
            && (!current.context_enabled()
                || !current.codex_share_context_terms()
                || !current.codex_context_consent_valid()))
    {
        Err("codex_policy_changed".into())
    } else {
        Ok(())
    }
}

/// Codex 結果在真正貼上前若因取消或焦點切換而要提早持久化，history／pending
/// 也屬於「採用」行為。必須與撤銷共用 policy gate；若已失效，只保存本機
/// deterministic base text，不能留下晚到的雲端改字。
#[allow(clippy::too_many_arguments)]
fn persist_pre_paste_result(
    pending_results: &Mutex<VecDeque<PendingResult>>,
    raw: &str,
    base_text: &str,
    candidate_text: &str,
    duration_s: f64,
    status: &'static str,
    timings: &serde_json::Value,
    metadata: &polish::PolishMetadata,
    pending_reason: Option<&'static str>,
) {
    let persist = |text: &str, polish_metadata: &polish::PolishMetadata| {
        let _ = history::append_entry(
            NewEntry {
                raw,
                text,
                duration_s,
                status,
                timings: Some(timings.clone()),
                polish: Some(polish_metadata.clone()),
            },
            &history::history_path(),
        );
        if let Some(reason) = pending_reason {
            enqueue_pending_result(
                pending_results,
                PendingResult {
                    raw: raw.to_string(),
                    text: text.to_string(),
                    reason,
                },
            );
        }
    };

    if let Some(epoch) = metadata.codex_policy_epoch {
        let adopted = crate::codex::with_policy_permit(epoch, || {
            codex_adoption_policy_is_current(metadata)?;
            persist(candidate_text, metadata);
            Ok(())
        });
        if adopted.is_ok() {
            return;
        }

        let fallback_text = textproc::normalize_cjk_punct(&textproc::to_traditional(base_text));
        let mut fallback_metadata = metadata.clone();
        fallback_metadata.changed = false;
        fallback_metadata.outcome = polish::PolishOutcome::Fallback;
        fallback_metadata.fallback_reason = Some(polish::PolishFallbackReason::CodexCancelled);
        fallback_metadata.codex_policy_epoch = None;
        fallback_metadata.codex_context_used = false;
        persist(&fallback_text, &fallback_metadata);
        return;
    }

    persist(candidate_text, metadata);
}

/// 共享槽只交給建立它的 session。舊 processing thread 不可先 take 再比較，
/// 否則 Esc 後立刻開始的新 session 會被舊 thread 偷走 target/context。
fn take_session_slot<T>(slot: &Mutex<Option<(u64, T)>>, session: u64) -> Option<T> {
    let mut slot = slot.lock().unwrap();
    if slot
        .as_ref()
        .is_some_and(|(captured_session, _)| *captured_session == session)
    {
        slot.take().map(|(_, value)| value)
    } else {
        None
    }
}

fn recv_before<T>(rx: crossbeam_channel::Receiver<T>, deadline: Instant) -> Option<T> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        rx.try_recv().ok()
    } else {
        rx.recv_timeout(remaining).ok()
    }
}

fn force_stop_session(sm: &mut DictationStateMachine, session: u64) -> SmAction {
    if sm.session() == session {
        sm.force_stop()
    } else {
        SmAction::None
    }
}

pub struct Core {
    pub sm: Mutex<DictationStateMachine>,
    pub engine: Mutex<Box<dyn SttEngine>>,
    /// UI 顯示與模型管理用的「使用中模型」（與 engine 分開，避免轉錄中卡 get_status）
    pub active_model: Mutex<&'static ModelSpec>,
    /// 將模型載入、設定寫回與 active_model 更新序列化；避免兩次切換互相覆蓋。
    model_switch_gate: Mutex<()>,
    /// 載入模型可能數秒；dispatcher 看到 true 時丟棄熱鍵事件，不能等載入後再
    /// 用舊 timestamp 開麥或誤判成免持。
    model_switching: AtomicBool,
    pub overlay: OverlayClient,
    pub injector: Box<dyn TextInjector>,
    /// 正式錄音前停止 onboarding mic test，避免同一 input device 同時開兩條 stream。
    stop_mic_test: Box<dyn Fn() + Send + Sync>,
    /// mic test 與正式錄音共用的 start lease；避免兩邊各自先看 Idle 再同時開 stream。
    pub(crate) audio_start_gate: Mutex<()>,
    /// 個人字典（設定 UI 可即時更新）
    pub dict: Mutex<Vec<(String, String)>>,
    /// 熱鍵服務可能在授權後重試重建，所以是 Mutex
    pub esc_ctl: Mutex<crossbeam_channel::Sender<Ctl>>,
    pub msg_tx: crossbeam_channel::Sender<Msg>,
    /// 使用者指定的輸入裝置（None = 系統預設）；設定 UI 可即時更新
    pub input_device: Mutex<Option<String>>,
    /// 本次 app 啟動後真正完成 paste 的次數；onboarding 只接受實際成功，
    /// 不讓單純按按鈕把首次設定標成完成。
    pub successful_pastes: AtomicU64,
    /// keyDown 時建立的 session-bound context；放開後不再重新抓別的 App。
    pub(crate) context_capture: Mutex<
        Option<(
            u64,
            crossbeam_channel::Receiver<Option<crate::context::ContextSnapshot>>,
        )>,
    >,
    /// keyDown 當下先用 NSWorkspace 固定 App id、開啟麥克風，再做有界指紋擷取。
    /// 與 context 文字分開：Context 仍綁定錄音當下的目標；真正貼上前會重新
    /// 取得目前焦點，讓使用者處理期間切到另一個輸入框時，結果仍能直接貼上。
    target_capture: Mutex<Option<(u64, Option<crate::context::PasteTarget>)>>,
    /// 只存在記憶體，供設定頁稽核；永不寫入 history/config。
    pub last_context: Mutex<Option<crate::context::ContextSnapshot>>,
    /// History 關閉或落盤失敗時的救援文字；只存在本次程序記憶體。
    pub pending_results: Mutex<VecDeque<PendingResult>>,
    /// 由 Tauri runtime 接上的即時聽寫 UI event sink；pipeline 本身不依賴 AppHandle。
    pub dictation_event_sink: DictationEventSink,

    capture: Mutex<Option<CaptureHandle>>,
    recording_flag: Arc<AtomicBool>,
    cancel: Mutex<Arc<AtomicBool>>,
    /// STT 最後使用時間——閒置看門狗據此卸載（精準度模型常駐可達數 GB，
    /// 是待機記憶體的大頭；SPEC §12 待機 <300MB）
    stt_last_used: Mutex<Instant>,
}

#[derive(Debug)]
pub enum ModelSwapError {
    Busy,
    Failed(anyhow::Error),
}

impl ModelSwapError {
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Busy)
    }
}

impl std::fmt::Display for ModelSwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => write!(f, "聽寫進行中，稍後再切換"),
            Self::Failed(error) => write!(f, "{error:#}"),
        }
    }
}

impl std::error::Error for ModelSwapError {}

impl Core {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        engine: Box<dyn SttEngine>,
        active_model: &'static ModelSpec,
        overlay: OverlayClient,
        injector: Box<dyn TextInjector>,
        stop_mic_test: Box<dyn Fn() + Send + Sync>,
        esc_ctl: crossbeam_channel::Sender<Ctl>,
        msg_tx: crossbeam_channel::Sender<Msg>,
        input_device: Option<String>,
        dictation_event_sink: DictationEventSink,
    ) -> Self {
        Self {
            sm: Mutex::new(DictationStateMachine::new()),
            engine: Mutex::new(engine),
            active_model: Mutex::new(active_model),
            model_switch_gate: Mutex::new(()),
            model_switching: AtomicBool::new(false),
            overlay,
            injector,
            stop_mic_test,
            audio_start_gate: Mutex::new(()),
            dict: Mutex::new(Settings::load().dictionary()),
            esc_ctl: Mutex::new(esc_ctl),
            msg_tx,
            input_device: Mutex::new(input_device),
            successful_pastes: AtomicU64::new(0),
            context_capture: Mutex::new(None),
            target_capture: Mutex::new(None),
            last_context: Mutex::new(None),
            pending_results: Mutex::new(VecDeque::new()),
            dictation_event_sink,
            capture: Mutex::new(None),
            recording_flag: Arc::new(AtomicBool::new(false)),
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            stt_last_used: Mutex::new(Instant::now()),
        }
    }

    fn touch_stt(&self) {
        *self.stt_last_used.lock().unwrap() = Instant::now();
    }

    /// 模型檔刪除等外部操作用：切換交易進行中時回 None（review 發現：
    /// set_model 正在載入 B 時 delete_model(B) 只看 active_model 擋不住，
    /// 會刪掉正在 commit 的模型檔）。拿到 guard 期間切換也進不來。
    pub fn try_model_switch_gate(&self) -> Option<std::sync::MutexGuard<'_, ()>> {
        self.model_switch_gate.try_lock().ok()
    }

    /// 確保 STT 引擎已載入（閒置卸載後的回載）。錄音一開始就在背景先跑，
    /// 載入時間被使用者說話的時間蓋掉。
    pub fn ensure_stt_loaded(&self) {
        // 收尾中不准回載——exit handler 卸載後又載回會重現 atexit ggml_abort
        if crate::SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let mut eng = self.engine.lock().unwrap();
        if !eng.is_loaded() {
            let t = Instant::now();
            match eng.load() {
                Ok(()) => {
                    tracing::info!("stt engine reloaded in {:.1}s", t.elapsed().as_secs_f32())
                }
                Err(e) => tracing::warn!("stt reload failed: {e}"),
            }
        }
    }

    /// 依狀態機現況同步 Esc 的攔截（非 IDLE 才攔，對應 prototype 條件式攔截）
    fn sync_esc(&self) {
        let state = self.sm.lock().unwrap().state();
        let _ = self.esc_ctl.lock().unwrap().send(if state == State::Idle {
            Ctl::DisarmEsc
        } else {
            Ctl::ArmEsc
        });
    }

    /// 原子切換 STT 模型：舊引擎先卸載以避免兩份大型 Metal 模型同時駐留，
    /// 但只有候選模型成功載入、設定也成功寫回後，才更新 active_model。
    /// 任一步失敗都嘗試把原引擎回載，讓失敗不會把下一次聽寫弄壞。
    pub fn swap_model<F>(&self, spec: &'static ModelSpec, commit: F) -> Result<(), ModelSwapError>
    where
        F: FnOnce() -> anyhow::Result<()>,
    {
        let _switch = self.model_switch_gate.lock().unwrap();
        let _switching = begin_model_switch(&self.sm, &self.model_switching)?;

        let mut engine = self.engine.lock().unwrap();
        let candidate: Box<dyn SttEngine> =
            Box::new(crate::stt::transcribe::TranscribeEngine::new(
                spec,
                crate::stt::registry::model_path(spec),
            ));
        replace_engine_atomically(&mut engine, candidate, commit)
            .map_err(ModelSwapError::Failed)?;
        *self.active_model.lock().unwrap() = spec;
        self.touch_stt();
        Ok(())
    }
}

struct ModelSwitchingGuard<'a> {
    flag: &'a AtomicBool,
}

impl<'a> ModelSwitchingGuard<'a> {
    fn new(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self { flag }
    }
}

impl Drop for ModelSwitchingGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

/// state check 與 switching=true 必須在同一把 sm lock 內線性化。若先抬旗標再
/// 發現正在錄音，dispatcher 可能吞掉用來停止錄音的 Up／Esc。
fn begin_model_switch<'a>(
    sm: &Mutex<DictationStateMachine>,
    flag: &'a AtomicBool,
) -> Result<ModelSwitchingGuard<'a>, ModelSwapError> {
    let sm = sm.lock().unwrap();
    if sm.state() != State::Idle {
        return Err(ModelSwapError::Busy);
    }
    let guard = ModelSwitchingGuard::new(flag);
    drop(sm);
    Ok(guard)
}

/// 低記憶體安全的 engine replacement。候選模型失敗或 commit 失敗時，
/// `current` 仍是原引擎；若切換前已載入，就在回傳錯誤前嘗試回載。
fn replace_engine_atomically<F>(
    current: &mut Box<dyn SttEngine>,
    mut candidate: Box<dyn SttEngine>,
    commit: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    let previous_was_loaded = current.is_loaded();
    current.unload();

    if let Err(candidate_error) = candidate.load() {
        let rollback = previous_was_loaded.then(|| current.load()).transpose();
        return match rollback {
            Ok(_) => Err(candidate_error.context("新模型載入失敗，已保留原模型")),
            Err(rollback_error) => Err(anyhow::anyhow!(
                "新模型載入失敗：{candidate_error:#}；原模型回載也失敗：{rollback_error:#}"
            )),
        };
    }

    if let Err(commit_error) = commit() {
        drop(candidate);
        let rollback = previous_was_loaded.then(|| current.load()).transpose();
        return match rollback {
            Ok(_) => Err(commit_error.context("模型設定寫入失敗，已保留原模型")),
            Err(rollback_error) => Err(anyhow::anyhow!(
                "模型設定寫入失敗：{commit_error:#}；原模型回載也失敗：{rollback_error:#}"
            )),
        };
    }

    *current = candidate;
    Ok(())
}

/// STT 閒置卸載門檻（秒）。與 llm.rs 同款政策（閒置 5 分鐘）；
/// 驗證時可用環境變數縮短，不必等真的五分鐘。
fn stt_idle_secs() -> u64 {
    std::env::var("CLARO_STT_IDLE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300)
}

/// STT 閒置看門狗：每 10s 檢查，IDLE 且閒置超過門檻就卸載引擎。
/// 只在狀態機 IDLE 時動手；engine 用 try_lock，轉錄中直接跳過這一輪。
/// 回載由 start_recording 的背景預載與 process_session 的同步補載負責。
pub fn spawn_stt_idle_watcher(core: Arc<Core>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(10));
        if core.sm.lock().unwrap().state() != State::Idle {
            continue;
        }
        let idle = core.stt_last_used.lock().unwrap().elapsed();
        if idle.as_secs() < stt_idle_secs() {
            continue;
        }
        if let Ok(mut eng) = core.engine.try_lock() {
            if eng.is_loaded() {
                eng.unload();
                tracing::info!("stt engine unloaded after {}s idle", idle.as_secs());
            }
        }
    });
}

fn apply_hotkey_transition(
    sm: &Mutex<DictationStateMachine>,
    model_switching: &AtomicBool,
    message: HotkeyMsg,
) -> SmAction {
    let mut sm = sm.lock().unwrap();
    if model_switching.load(Ordering::SeqCst) {
        return SmAction::None;
    }
    match message {
        HotkeyMsg::Down(ts) => sm.hotkey_down(ts),
        HotkeyMsg::Up(ts) => sm.hotkey_up(ts),
        HotkeyMsg::Esc => sm.esc(),
    }
}

/// dispatcher 主迴圈：在專屬執行緒上跑，直到 msg channel 關閉。
pub fn run_dispatcher(core: Arc<Core>, rx: crossbeam_channel::Receiver<Msg>) {
    for msg in rx {
        match msg {
            Msg::Hotkey(HotkeyMsg::Down(ts)) => {
                let action =
                    apply_hotkey_transition(&core.sm, &core.model_switching, HotkeyMsg::Down(ts));
                match action {
                    SmAction::StartRecording => start_recording(&core),
                    SmAction::StopAndProcess => start_processing(&core),
                    _ => {}
                }
                core.sync_esc();
            }
            Msg::Hotkey(HotkeyMsg::Up(ts)) => {
                let action =
                    apply_hotkey_transition(&core.sm, &core.model_switching, HotkeyMsg::Up(ts));
                match action {
                    SmAction::EnterHandsfree => {
                        core.overlay.send("handsfree");
                        tracing::info!("hands-free mode (press hotkey again to stop)");
                    }
                    SmAction::StopAndProcess => start_processing(&core),
                    _ => {}
                }
                core.sync_esc();
            }
            Msg::Hotkey(HotkeyMsg::Esc) => {
                let action =
                    apply_hotkey_transition(&core.sm, &core.model_switching, HotkeyMsg::Esc);
                match action {
                    SmAction::CancelRecording => cancel_recording(&core),
                    SmAction::CancelProcessing => {
                        core.cancel.lock().unwrap().store(true, Ordering::SeqCst);
                        // Codex writer 另以 policy gate 保護 stdin；同步推進 generation，
                        // 避免 Esc 正好落在 session cancel check 與第一個 byte 之間。
                        crate::codex::cancel_active();
                        core.overlay.send("cancel");
                        tracing::info!("cancelled (processing result goes to history only)");
                    }
                    _ => {}
                }
                core.sync_esc();
            }
            Msg::ForceStop(session) => {
                let action = force_stop_session(&mut core.sm.lock().unwrap(), session);
                if action == SmAction::StopAndProcess {
                    start_processing(&core);
                }
                core.sync_esc();
            }
        }
    }
}

fn start_recording(core: &Arc<Core>) {
    let audio_gate = core.audio_start_gate.lock().unwrap();
    (core.stop_mic_test)();
    let session = core.sm.lock().unwrap().session();
    let device = core.input_device.lock().unwrap().clone();
    // 麥克風裝置初始化與最小 AX target seed 並行。AX 最壞約 50ms，不能讓它
    // 排在開麥前切掉按下即說的第一個音節；seed 仍在 keyDown dispatcher 尚未
    // 處理下一事件時固定，可辨識同一 App 內的欄位切換。
    let mic_requested_at = Instant::now();
    let (capture_tx, capture_rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = capture_tx.send(audio::start_capture(device));
    });
    let target_seed = crate::context::begin_paste_target_capture();
    let capture_result = capture_rx
        .recv_timeout(Duration::from_secs(4))
        .unwrap_or_else(|_| Err(anyhow::anyhow!("audio thread did not start in time")));
    drop(audio_gate);
    match capture_result {
        Ok(handle) => {
            tracing::info!(
                "microphone stream ready in {}ms",
                mic_requested_at.elapsed().as_millis()
            );
            core.recording_flag.store(true, Ordering::SeqCst);
            let level = handle.level_handle();
            let started = Instant::now();
            let flag = core.recording_flag.clone();
            let overlay_core = core.clone();
            let msg_tx = core.msg_tx.clone();
            // level poller 兼 watchdog（錄音上限）
            std::thread::spawn(move || {
                let mut force_sent = false;
                while flag.load(Ordering::SeqCst) {
                    // overlay socket 可能阻塞/重連（500ms timeout）——絕不能在
                    // sm 鎖內做 I/O，否則熱鍵 Up/Esc/ForceStop 全排在後面
                    // （review 發現）。鎖內只判定，送資料在鎖外。
                    let current = {
                        let sm = overlay_core.sm.lock().unwrap();
                        sm.session() == session && flag.load(Ordering::SeqCst)
                    };
                    if !current {
                        break;
                    }
                    overlay_core
                        .overlay
                        .send(&format!("level {:.4}", level.get()));
                    if !force_sent && started.elapsed().as_secs_f64() > audio::MAX_RECORDING_S {
                        let _ = msg_tx.send(Msg::ForceStop(session));
                        force_sent = true;
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
            });
            *core.capture.lock().unwrap() = Some(handle);
            core.overlay.send("recording");
            // 麥克風已開始收音後才做 bounded AX fingerprint；既不漏掉開頭語音，
            // 也把 target 固定在 dispatcher 尚未處理下一個熱鍵事件的時間點。
            let target = target_seed.and_then(crate::context::finish_paste_target_capture);
            *core.target_capture.lock().unwrap() = Some((session, target.clone()));
            // 設定讀檔也放在 audio start 之後，檔案系統壓力不可拖慢開麥。
            let context_enabled = Settings::load().context_enabled();
            let context_tx = if context_enabled {
                let (tx, rx) = crossbeam_channel::bounded(1);
                *core.context_capture.lock().unwrap() = Some((session, rx));
                Some(tx)
            } else {
                *core.context_capture.lock().unwrap() = None;
                *core.last_context.lock().unwrap() = None;
                None
            };
            // Context 針對同一個完整 fingerprint 擷取；同 App 切視窗／欄位時
            // 會 fail closed，而且 worker 自身有約 250ms 的總預算。
            std::thread::spawn(move || {
                if let Some(tx) = context_tx {
                    let snapshot = target
                        .as_ref()
                        .and_then(crate::context::capture_snapshot_for_target);
                    let _ = tx.send(snapshot);
                }
            });
            // 閒置卸載後的回載：邊錄邊載，載入時間被說話時間蓋掉
            core.touch_stt();
            let preload = core.clone();
            std::thread::spawn(move || {
                if crate::hardware::low_memory_mode() {
                    crate::llm::unload_blocking();
                    tracing::info!("low-memory handoff: unloaded builtin LLM before STT preload");
                }
                preload.ensure_stt_loaded();
            });
            tracing::info!("recording... (release to paste, esc to cancel)");
        }
        Err(e) => {
            tracing::error!("audio input failed: {e}");
            core.overlay.send("error");
            let _ = history::append_entry(
                NewEntry {
                    raw: "",
                    text: "",
                    duration_s: 0.0,
                    status: "mic_unavailable",
                    timings: None,
                    polish: None,
                },
                &history::history_path(),
            );
            // 錄不了音：把狀態機拉回 IDLE（沿用 esc 語意）
            let _ = core.sm.lock().unwrap().esc();
        }
    }
}

/// 在 dispatcher thread 同步把本次 handle 從共享槽取走，避免 Esc 後快速開始
/// 下一段時，舊 processing/cancel thread 誤拿新 session 的麥克風。
fn detach_capture(core: &Arc<Core>) -> Option<CaptureHandle> {
    core.recording_flag.store(false, Ordering::SeqCst);
    core.capture.lock().unwrap().take()
}

struct CollectedAudio {
    samples: Vec<f32>,
    /// 正規化前的實際輸入電平；可用來判斷麥克風太小聲，不能用 peak-normalized
    /// samples 回推。
    input_rms: f32,
    clipped_ratio: f32,
}

/// 停止錄音並取回音訊；None 表示不合格（已送 error 態與 history）
fn collect_audio(
    core: &Arc<Core>,
    handle: Option<CaptureHandle>,
    cancelled: bool,
    session: u64,
    cancel: Option<&AtomicBool>,
) -> Option<CollectedAudio> {
    let handle = handle?;
    let stop_immediately = cancelled || cancel.is_some_and(|flag| flag.load(Ordering::SeqCst));
    let samples = match if stop_immediately {
        handle.stop_immediately()
    } else {
        handle.stop()
    } {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("audio capture failed: {e}");
            let was_cancelled = cancelled || cancel.is_some_and(|flag| flag.load(Ordering::SeqCst));
            if !was_cancelled {
                if let Some(flag) = cancel {
                    send_overlay_for_session(core, session, flag, "error");
                }
                let _ = history::append_entry(
                    NewEntry {
                        raw: "",
                        text: "",
                        duration_s: 0.0,
                        status: "mic_unavailable",
                        timings: None,
                        polish: None,
                    },
                    &history::history_path(),
                );
            }
            return None;
        }
    };
    if let Err(guard) = audio::validate(&samples, audio::TARGET_RATE) {
        let dur = samples.len() as f64 / audio::TARGET_RATE as f64;
        let was_cancelled = cancelled || cancel.is_some_and(|flag| flag.load(Ordering::SeqCst));
        let status = if was_cancelled {
            "cancelled"
        } else {
            match guard {
                audio::AudioGuard::TooShort => "too_short",
                audio::AudioGuard::Silent => "silent",
            }
        };
        if !was_cancelled {
            if let Some(flag) = cancel {
                send_overlay_for_session(core, session, flag, "error");
            }
        }
        let _ = history::append_entry(
            NewEntry {
                raw: "",
                text: "",
                duration_s: dur,
                status,
                timings: None,
                polish: None,
            },
            &history::history_path(),
        );
        return None;
    }
    let input_rms = audio::rms(&samples);
    let clipped_ratio =
        samples.iter().filter(|sample| sample.abs() >= 0.99).count() as f32 / samples.len() as f32;
    Some(CollectedAudio {
        samples: audio::normalize(samples),
        input_rms,
        clipped_ratio,
    })
}

fn start_processing(core: &Arc<Core>) {
    // hotkey release 進入 processing 的線性化點。此時計時可涵蓋 post-roll、
    // Context 收尾、STT、整理、焦點重驗與實際貼上，而不是只量模型函式。
    let release_started = Instant::now();
    // 每個 session 配發自己的取消旗標（prototype：不洗掉上一段還沒檢查到的取消）
    let cancel = Arc::new(AtomicBool::new(false));
    *core.cancel.lock().unwrap() = cancel.clone();
    let session = core.sm.lock().unwrap().session();
    emit_dictation_event(core, session, "processing", None, false);
    let capture = detach_capture(core);
    let core = core.clone();

    std::thread::spawn(move || {
        process_session(&core, session, &cancel, capture, release_started);
        core.sm.lock().unwrap().processing_finished(session);
        core.sync_esc();
    });
}

/// 貼上跟隨 Cmd+V 當下的前景 App。這裡刻意不要求 AX 視窗／欄位 metadata：
/// WebView、Electron 與部分原生輸入框常不提供可區分的 identifier/title，若把
/// 完整焦點指紋當成「可否貼上」條件，會把真正可輸入的目前欄位誤判成不存在。
/// Context 仍使用嚴格 PasteTarget 綁定；只有交付文字改採前景 App 的寬容 gate。
fn paste_destination_available(current_app: Option<&str>) -> bool {
    current_app.is_some_and(|app_id| app_id != "dev.claro.desktop")
}

fn context_matches_target(
    expected: Option<&crate::context::PasteTarget>,
    snapshot: &crate::context::ContextSnapshot,
) -> bool {
    expected.is_some_and(|target| target == &snapshot.target)
}

fn async_feedback_allowed(core: &Core, session: u64, cancel: &AtomicBool) -> bool {
    !crate::SHUTTING_DOWN.load(Ordering::SeqCst)
        && !cancel.load(Ordering::SeqCst)
        && core.sm.lock().unwrap().session() == session
}

fn emit_dictation_event(
    core: &Core,
    session: u64,
    phase: &'static str,
    outcome: Option<&'static str>,
    recovery_available: bool,
) {
    (core.dictation_event_sink)(DictationEvent {
        session,
        phase,
        outcome,
        recovery_available,
    });
}

/// 保證即使 processing thread 遇到未預期的 early return 或 panic，UI 也會收到
/// 一個 terminal event，不會永久停在「處理中」或完全沒有回饋。
struct DictationFeedback<'a> {
    core: &'a Core,
    session: u64,
    cancel: &'a AtomicBool,
    finished: bool,
}

impl<'a> DictationFeedback<'a> {
    fn new(core: &'a Core, session: u64, cancel: &'a AtomicBool) -> Self {
        Self {
            core,
            session,
            cancel,
            finished: false,
        }
    }

    fn finish(&mut self, outcome: &'static str, recovery_available: bool) {
        if self.finished {
            return;
        }
        self.finished = true;
        emit_dictation_event(
            self.core,
            self.session,
            "finished",
            Some(outcome),
            recovery_available,
        );
    }
}

impl Drop for DictationFeedback<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let outcome = if self.cancel.load(Ordering::SeqCst) {
            "cancelled"
        } else {
            "error"
        };
        self.finished = true;
        emit_dictation_event(self.core, self.session, "finished", Some(outcome), false);
    }
}

fn commit_paste_for_session(
    sm: &Mutex<DictationStateMachine>,
    cancel: &AtomicBool,
    session: u64,
    paste: &dyn Fn() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let sm = sm.lock().unwrap();
    if crate::SHUTTING_DOWN.load(Ordering::SeqCst)
        || cancel.load(Ordering::SeqCst)
        || sm.session() != session
        || sm.state() != State::Processing
    {
        anyhow::bail!("paste session changed before Cmd+V");
    }
    paste()
}

/// 將 session check 與 overlay write 放在同一個 state-machine lock 內，建立明確順序：
/// 舊 session 若先送，新 recording 必在其後覆蓋；新 session 若先建立，舊送出會被拒絕。
fn send_overlay_for_session(core: &Core, session: u64, cancel: &AtomicBool, message: &str) {
    if cancel.load(Ordering::SeqCst) {
        return;
    }
    let sm = core.sm.lock().unwrap();
    if !cancel.load(Ordering::SeqCst) && sm.session() == session {
        core.overlay.send(message);
    }
}

fn process_session(
    core: &Arc<Core>,
    session: u64,
    cancel: &AtomicBool,
    capture: Option<CaptureHandle>,
    release_started: Instant,
) {
    let mut feedback = DictationFeedback::new(core, session, cancel);
    let Some(captured_audio) = collect_audio(core, capture, false, session, Some(cancel)) else {
        return;
    };
    let CollectedAudio {
        samples,
        input_rms,
        clipped_ratio,
    } = captured_audio;
    let dur = samples.len() as f64 / audio::TARGET_RATE as f64;
    send_overlay_for_session(core, session, cancel, "processing");
    // 單一 session 以這份設定決定本機管線；任何可能外送的 Codex 路徑會在
    // stdin 前另取 policy snapshot，若 provider／mode／同意已變便直接取消，
    // 不沿用這份 STT 前快照。
    let settings = Settings::load();

    // 若錄音太短、背景 preload 尚未完成，這裡會同步補載 STT；低記憶體機
    // 仍先清掉上一輪內建 LLM，確保不會在補載路徑重新造成雙駐留。
    if crate::hardware::low_memory_mode() {
        crate::llm::unload_blocking();
    }

    // 貼上目標與 context 文字分離。前者在 context 關閉、敏感 App 或 AX 失敗時
    // 仍然存在，避免處理期間切 App 後誤貼。
    let target_app_id = take_session_slot(&core.target_capture, session).flatten();
    if target_app_id.is_none() {
        tracing::warn!("recording target unavailable — this result will not auto-paste");
    }

    // Context 已在 keyDown 背景抓取；短句若還沒完成，放開後最多只等 250ms。
    let capture_deadline = Instant::now() + Duration::from_millis(250);
    let snapshot = if settings.context_enabled() {
        let receiver = take_session_slot(&core.context_capture, session);
        receiver
            .and_then(|rx| recv_before(rx, capture_deadline))
            .flatten()
    } else {
        None
    };
    // 只接受和本次 target 完整一致的 snapshot（App + window + focused element）。
    // 若背景執行緒排到時使用者已切同 App 的文件／欄位，Context 也必須丟棄。
    let snapshot = snapshot.filter(|captured| {
        let same_target = context_matches_target(target_app_id.as_ref(), captured);
        if !same_target {
            tracing::warn!("captured context does not match recording target — discarding context");
        }
        same_target
    });
    if core.sm.lock().unwrap().session() == session {
        *core.last_context.lock().unwrap() = snapshot.clone();
    }
    let screen_ctx = snapshot
        .as_ref()
        .map(|captured| captured.text.clone())
        .unwrap_or_default();

    let t0 = Instant::now();
    let dict_pairs = core.dict.lock().unwrap().clone();
    // 偏置詞來自兩處：只做偏置的詞彙表，以及字典的 canonical 值（字典另外負責
    // 事後替換）。實際能塞多少由 build_initial_prompt 的 token 預算決定，這裡
    // 的上限只是避免無謂地掃過長的畫面文字。
    let mut bias_terms = settings.vocabulary();
    bias_terms.extend(dict_pairs.iter().map(|(_, right)| right.clone()));
    // Codex 的雲端資料界線需要能區分「使用者明確建立的本機正確詞」與
    // 「當前畫面臨時萃取詞」。STT 仍使用合併後清單，但雲端路徑分欄傳遞，
    // 畫面詞只有第二層同意有效時才會送出。
    let local_terms = crate::context::context_terms(&bias_terms, "", PROMPT_TERM_LIMIT);
    // Codex 的畫面詞彙同意不涵蓋 raw App 名或視窗標題；雲端候選必須走
    // 保留 provenance 的專用 extractor。本機 Whisper prompt 仍可使用完整 context。
    let screen_terms = snapshot
        .as_ref()
        .map(|captured| crate::context::codex_screen_terms(captured, PROMPT_TERM_LIMIT))
        .unwrap_or_default();
    let terms = crate::context::context_terms(&bias_terms, &screen_ctx, PROMPT_TERM_LIMIT);
    let (stt_model, stt_family, prompt_term_count) = {
        let model = *core.active_model.lock().unwrap();
        let prompt_term_count = matches!(model.family, crate::stt::registry::ModelFamily::Whisper)
            .then_some(terms.len())
            .unwrap_or(0);
        (model.id, format!("{:?}", model.family), prompt_term_count)
    };
    let raw = {
        let req = crate::stt::SttRequest {
            audio: &samples,
            // 台灣中文是目前產品預設。auto 與 zh 的合成 smoke 無差異，
            // 在真人短句／混語 corpus 完成前不把未證實的 auto 當準確度修正。
            language: Some("zh"),
            initial_prompt: crate::stt::build_initial_prompt(&terms),
        };
        let res = {
            let mut eng = core.engine.lock().unwrap();
            // 背景預載可能還沒排到或失敗——同步補載，聽寫不能因卸載而失敗
            if eng.is_loaded() { Ok(()) } else { eng.load() }.and_then(|()| eng.transcribe(&req))
        };
        core.touch_stt();
        match res {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("transcription failed: {e}");
                let cancelled = cancel.load(Ordering::SeqCst);
                send_overlay_for_session(core, session, cancel, "error");
                let _ = history::append_entry(
                    NewEntry {
                        raw: "",
                        text: "",
                        duration_s: dur,
                        status: if cancelled { "cancelled" } else { "stt_failed" },
                        timings: None,
                        polish: None,
                    },
                    &history::history_path(),
                );
                feedback.finish(if cancelled { "cancelled" } else { "stt_failed" }, false);
                return;
            }
        }
    };
    let stt_ms = t0.elapsed().as_millis() as u64;

    let raw =
        textproc::normalize_cjk_punct(&textproc::to_traditional(&textproc::clean_transcript(&raw)));
    if raw.is_empty() {
        let cancelled = cancel.load(Ordering::SeqCst);
        send_overlay_for_session(core, session, cancel, "error");
        let _ = history::append_entry(
            NewEntry {
                raw: "",
                text: "",
                duration_s: dur,
                status: if cancelled { "cancelled" } else { "silent" },
                timings: None,
                polish: None,
            },
            &history::history_path(),
        );
        feedback.finish(if cancelled { "cancelled" } else { "silent" }, false);
        return;
    }

    let base_text = textproc::apply_dict(&raw, &dict_pairs);

    // 8–12 GB 與 Intel Mac 不讓 Whisper 與 4B LLM 同時駐留。
    // 下一次錄音開始會邊錄邊重新載入 STT，因此不增加後續首字延遲。
    let low_memory_builtin = crate::hardware::low_memory_mode()
        && settings.llm_provider() == "builtin"
        && polish::effective_mode(&settings) != PolishMode::Raw;
    if low_memory_builtin {
        let mut engine = core.engine.lock().unwrap();
        if engine.is_loaded() {
            engine.unload();
            tracing::info!("low-memory handoff: unloaded STT before builtin LLM");
        }
    }

    // RAW / CLEAN / CORRECT / ORGANIZE 共用同一入口。provider 不可用、未同意或 guard
    // 拒絕時 transform 一律回 deterministic base text，並留下可稽核 metadata。
    let t1 = Instant::now();
    let polish::PolishResult {
        text: output,
        metadata: mut polish_metadata,
    } = polish::transform_with_cancel_and_terms(
        &settings,
        &base_text,
        &screen_ctx,
        &local_terms,
        &screen_terms,
        cancel,
    );
    if low_memory_builtin {
        crate::llm::unload_blocking();
        tracing::info!("low-memory handoff: unloaded builtin LLM after polishing");
    }
    let polish_ms =
        (settings.polish_mode() != PolishMode::Raw).then(|| t1.elapsed().as_millis() as u64);
    let mut text = textproc::normalize_cjk_punct(&textproc::to_traditional(&output));

    // 只記錄除錯所需的模型／解碼 profile 與聚合音量，不保存 Context 詞彙內容。
    // 讓未來的準確率回報能分辨「哪個模型」與「是否有詞彙偏置」，不再盲猜。
    let mut timings = json!({
        "stt_ms": stt_ms,
        "polish_ms": polish_ms,
        "stt_model": stt_model,
        "stt_family": stt_family,
        "stt_language": "zh",
        "prompt_term_count": prompt_term_count,
        "context_term_count": terms.len(),
        "audio_input_rms": (input_rms * 10_000.0).round() / 10_000.0,
        "audio_clipped_ratio": (clipped_ratio * 10_000.0).round() / 10_000.0,
    });

    if !async_feedback_allowed(core, session, cancel) {
        persist_pre_paste_result(
            &core.pending_results,
            &raw,
            &base_text,
            &text,
            dur,
            "cancelled",
            &timings,
            &polish_metadata,
            None,
        );
        feedback.finish("cancelled", false);
        return;
    }

    let focus_guard_started = Instant::now();
    // Context remains session-bound, but Cmd+V follows the foreground App's current
    // focus. Switching from Mail to Slack (or to another field) is intentional;
    // only an unavailable external App should enter recovery.
    let current_app = crate::context::current_app_id();
    // Esc 或新 session 可能在查詢前景 App 期間發生；貼上前必須再檢查一次。
    if !async_feedback_allowed(core, session, cancel) {
        persist_pre_paste_result(
            &core.pending_results,
            &raw,
            &base_text,
            &text,
            dur,
            "cancelled",
            &timings,
            &polish_metadata,
            None,
        );
        feedback.finish("cancelled", false);
        return;
    }

    if !paste_destination_available(current_app.as_deref()) {
        tracing::warn!("paste destination unavailable — preserving result in history");
        send_overlay_for_session(core, session, cancel, "error");
        // 一律保留於 process-memory queue：處理途中切換 history 開關、磁碟滿或
        // 寫入失敗都不能形成沒有落盤、也沒有救援副本的資料遺失窗口。
        persist_pre_paste_result(
            &core.pending_results,
            &raw,
            &base_text,
            &text,
            dur,
            "focus_changed",
            &timings,
            &polish_metadata,
            Some("focus_changed"),
        );
        feedback.finish("focus_changed", true);
        return;
    }

    let focus_guard_ms = focus_guard_started.elapsed().as_millis();
    let codex_policy_epoch = polish_metadata.codex_policy_epoch;
    let pre_paste_commit = |paste: &dyn Fn() -> anyhow::Result<()>| -> anyhow::Result<()> {
        let current_app = crate::context::current_app_id();
        if !paste_destination_available(current_app.as_deref()) {
            anyhow::bail!("paste destination unavailable before Cmd+V");
        }
        // 持有 state-machine lock 到 Cmd+V 完成；Esc／新 session 只能在線性化點
        // 之前先讓本次失敗，或在本次貼上完成後才生效，不能插入兩者之間。
        commit_paste_for_session(&core.sm, cancel, session, paste)
    };
    let inject_started = Instant::now();
    let inject_result = if let Some(epoch) = codex_policy_epoch {
        match crate::codex::with_policy_permit(epoch, || {
            codex_adoption_policy_is_current(&polish_metadata)?;
            core.injector
                .inject(&text, &pre_paste_commit)
                .map_err(|error| error.to_string())
        }) {
            Ok(()) => Ok(()),
            Err(reason)
                if reason == "codex_policy_changed"
                    && crate::SHUTTING_DOWN.load(Ordering::SeqCst) =>
            {
                text = textproc::normalize_cjk_punct(&textproc::to_traditional(&base_text));
                polish_metadata.changed = false;
                polish_metadata.outcome = polish::PolishOutcome::Fallback;
                polish_metadata.fallback_reason =
                    Some(polish::PolishFallbackReason::CodexCancelled);
                polish_metadata.codex_policy_epoch = None;
                polish_metadata.codex_context_used = false;
                Err(anyhow::Error::msg("app_shutting_down"))
            }
            Err(reason) if reason == "codex_policy_changed" => {
                // 撤銷先線性化：Codex text 尚未進剪貼簿，改貼 deterministic
                // 本機 base text；history/pending 也不得保留已撤銷的晚到結果。
                text = textproc::normalize_cjk_punct(&textproc::to_traditional(&base_text));
                polish_metadata.changed = false;
                polish_metadata.outcome = polish::PolishOutcome::Fallback;
                polish_metadata.fallback_reason =
                    Some(polish::PolishFallbackReason::CodexCancelled);
                polish_metadata.codex_policy_epoch = None;
                polish_metadata.codex_context_used = false;
                core.injector.inject(&text, &pre_paste_commit)
            }
            Err(reason) => Err(anyhow::Error::msg(reason)),
        }
    } else {
        core.injector.inject(&text, &pre_paste_commit)
    };
    timings["focus_guard_ms"] = json!(focus_guard_ms.min(u64::MAX as u128) as u64);
    timings["inject_ms"] = json!(inject_started.elapsed().as_millis().min(u64::MAX as u128) as u64);
    if let Err(e) = inject_result {
        tracing::error!("paste failed: {e}");
        if !async_feedback_allowed(core, session, cancel) {
            persist_pre_paste_result(
                &core.pending_results,
                &raw,
                &base_text,
                &text,
                dur,
                "cancelled",
                &timings,
                &polish_metadata,
                None,
            );
            feedback.finish("cancelled", false);
            return;
        }
        send_overlay_for_session(core, session, cancel, "error");
        persist_pre_paste_result(
            &core.pending_results,
            &raw,
            &base_text,
            &text,
            dur,
            "paste_failed",
            &timings,
            &polish_metadata,
            Some("paste_failed"),
        );
        feedback.finish("paste_failed", true);
        return;
    }
    send_overlay_for_session(core, session, cancel, "success");
    record_success(&core.successful_pastes);
    timings["release_to_paste_ms"] =
        json!(release_started.elapsed().as_millis().min(u64::MAX as u128) as u64);
    let _ = history::append_entry(
        NewEntry {
            raw: &raw,
            text: &text,
            duration_s: dur,
            status: "pasted",
            timings: Some(timings),
            polish: Some(polish_metadata),
        },
        &history::history_path(),
    );
    feedback.finish("pasted", false);
}

/// Esc 取消錄音：立即收 UI，背景仍轉錄一份進歷史（可救回，prototype 語意）
fn cancel_recording(core: &Arc<Core>) {
    core.overlay.send("cancel");
    let session = core.sm.lock().unwrap().session();
    let capture = detach_capture(core);
    let core = core.clone();
    std::thread::spawn(move || {
        let Some(captured_audio) = collect_audio(&core, capture, true, session, None) else {
            return;
        };
        let samples = captured_audio.samples;
        let dur = samples.len() as f64 / audio::TARGET_RATE as f64;
        let req = crate::stt::SttRequest {
            audio: &samples,
            language: Some("zh"),
            initial_prompt: crate::stt::build_initial_prompt(&[]),
        };
        let raw = core
            .engine
            .lock()
            .unwrap()
            .transcribe(&req)
            .map(|t| {
                textproc::normalize_cjk_punct(&textproc::to_traditional(
                    &textproc::clean_transcript(&t),
                ))
            })
            .unwrap_or_default();
        let _ = history::append_entry(
            NewEntry {
                raw: &raw,
                text: &raw,
                duration_s: dur,
                status: "cancelled",
                timings: None,
                polish: None,
            },
            &history::history_path(),
        );
    });
}

#[cfg(test)]
mod target_tests {
    use super::{
        apply_hotkey_transition, begin_model_switch, commit_paste_for_session,
        context_matches_target, enqueue_pending_result, force_stop_session,
        paste_destination_available, record_success, replace_engine_atomically, take_session_slot,
        PendingResult,
    };
    use crate::context::{ContextSnapshot, PasteTarget};
    use crate::state_machine::{DictationStateMachine, SmAction, State};
    use crate::stt::{SttEngine, SttRequest};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct FakeEngine {
        id: &'static str,
        loaded: bool,
        fail_load: bool,
        loads: Arc<AtomicUsize>,
    }

    impl FakeEngine {
        fn boxed(
            id: &'static str,
            loaded: bool,
            fail_load: bool,
            loads: Arc<AtomicUsize>,
        ) -> Box<dyn SttEngine> {
            Box::new(Self {
                id,
                loaded,
                fail_load,
                loads,
            })
        }
    }

    impl SttEngine for FakeEngine {
        fn id(&self) -> &str {
            self.id
        }

        fn is_loaded(&self) -> bool {
            self.loaded
        }

        fn load(&mut self) -> anyhow::Result<()> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            if self.fail_load {
                anyhow::bail!("fake load failed");
            }
            self.loaded = true;
            Ok(())
        }

        fn unload(&mut self) {
            self.loaded = false;
        }

        fn transcribe(&mut self, _req: &SttRequest<'_>) -> anyhow::Result<String> {
            unreachable!("model-swap tests do not transcribe")
        }
    }

    #[test]
    fn hotkey_pair_during_model_load_is_dropped_instead_of_replayed() {
        let sm = Mutex::new(DictationStateMachine::new());
        let switching = AtomicBool::new(true);

        assert_eq!(
            apply_hotkey_transition(&sm, &switching, crate::hotkey::HotkeyMsg::Down(1.0)),
            SmAction::None
        );
        assert_eq!(
            apply_hotkey_transition(&sm, &switching, crate::hotkey::HotkeyMsg::Up(1.1)),
            SmAction::None
        );
        assert_eq!(sm.lock().unwrap().state(), State::Idle);

        switching.store(false, Ordering::SeqCst);
        assert_eq!(
            apply_hotkey_transition(&sm, &switching, crate::hotkey::HotkeyMsg::Down(2.0)),
            SmAction::StartRecording
        );
    }

    #[test]
    fn failed_switch_while_recording_never_raises_drop_hotkeys_flag() {
        let mut machine = DictationStateMachine::new();
        assert_eq!(machine.hotkey_down(1.0), SmAction::StartRecording);
        let sm = Mutex::new(machine);
        let switching = AtomicBool::new(false);

        assert!(begin_model_switch(&sm, &switching).is_err());
        assert!(!switching.load(Ordering::SeqCst));
        assert_eq!(
            apply_hotkey_transition(&sm, &switching, crate::hotkey::HotkeyMsg::Up(2.0)),
            SmAction::StopAndProcess
        );
    }

    #[test]
    fn model_swap_commits_only_after_candidate_loads() {
        let old_loads = Arc::new(AtomicUsize::new(0));
        let new_loads = Arc::new(AtomicUsize::new(0));
        let committed = AtomicBool::new(false);
        let mut current = FakeEngine::boxed("old", true, false, old_loads.clone());
        let candidate = FakeEngine::boxed("new", false, false, new_loads.clone());

        replace_engine_atomically(&mut current, candidate, || {
            committed.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();

        assert_eq!(current.id(), "new");
        assert!(current.is_loaded());
        assert!(committed.load(Ordering::SeqCst));
        assert_eq!(new_loads.load(Ordering::SeqCst), 1);
        assert_eq!(old_loads.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn model_swap_rolls_back_when_candidate_load_fails() {
        let old_loads = Arc::new(AtomicUsize::new(0));
        let mut current = FakeEngine::boxed("old", true, false, old_loads.clone());
        let candidate = FakeEngine::boxed("new", false, true, Arc::new(AtomicUsize::new(0)));

        let error = replace_engine_atomically(&mut current, candidate, || Ok(())).unwrap_err();

        assert!(error.to_string().contains("新模型載入失敗"));
        assert_eq!(current.id(), "old");
        assert!(current.is_loaded());
        assert_eq!(old_loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn model_swap_rolls_back_when_config_commit_fails() {
        let old_loads = Arc::new(AtomicUsize::new(0));
        let mut current = FakeEngine::boxed("old", true, false, old_loads.clone());
        let candidate = FakeEngine::boxed("new", false, false, Arc::new(AtomicUsize::new(0)));

        let error =
            replace_engine_atomically(&mut current, candidate, || anyhow::bail!("disk full"))
                .unwrap_err();

        assert!(error.to_string().contains("模型設定寫入失敗"));
        assert_eq!(current.id(), "old");
        assert!(current.is_loaded());
        assert_eq!(old_loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn paste_follows_any_current_external_app_without_ax_metadata() {
        assert!(paste_destination_available(Some("com.apple.TextEdit")));
        assert!(paste_destination_available(Some("com.google.Chrome")));
        assert!(paste_destination_available(Some(
            "com.tinyspeck.slackmacgap"
        )));
        assert!(!paste_destination_available(Some("dev.claro.desktop")));
        assert!(!paste_destination_available(None));
    }

    #[test]
    fn context_requires_same_window_and_focus_fingerprint() {
        let expected = PasteTarget {
            app_id: "com.tinyspeck.slackmacgap".into(),
            window_hash: [4; 32],
            focus_hash: [5; 32],
        };
        let mut snapshot = ContextSnapshot {
            text: "Visible: 私密頻道".into(),
            app_id: expected.app_id.clone(),
            app_name: "Slack".into(),
            surface: "message",
            content_text: "私密頻道".into(),
            target: expected.clone(),
        };
        assert!(context_matches_target(Some(&expected), &snapshot));
        snapshot.target.focus_hash = [6; 32];
        assert!(!context_matches_target(Some(&expected), &snapshot));
        assert!(!context_matches_target(None, &snapshot));
    }

    #[test]
    fn successful_paste_keeps_older_pending_result() {
        let pending = Mutex::new(VecDeque::from([PendingResult {
            raw: "原始救援文字".into(),
            text: "尚未取回的救援文字。".into(),
            reason: "paste_failed",
        }]));
        let successful_pastes = AtomicU64::new(0);

        record_success(&successful_pastes);

        assert_eq!(successful_pastes.load(Ordering::SeqCst), 1);
        let saved = pending.lock().unwrap();
        assert_eq!(saved.front().unwrap().text, "尚未取回的救援文字。");
    }

    #[test]
    fn failed_results_show_newest_recovery_without_dropping_older_text() {
        let queue = Mutex::new(VecDeque::new());
        for text in ["第一段", "第二段"] {
            enqueue_pending_result(
                &queue,
                PendingResult {
                    raw: text.into(),
                    text: text.into(),
                    reason: "focus_changed",
                },
            );
        }
        let saved = queue.lock().unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved.front().unwrap().text, "第二段");
        assert_eq!(saved.back().unwrap().text, "第一段");
    }

    #[test]
    fn stale_session_cannot_take_new_session_slot() {
        let slot = Mutex::new(Some((2_u64, "session-2")));
        assert_eq!(take_session_slot(&slot, 1), None);
        assert_eq!(take_session_slot(&slot, 2), Some("session-2"));
        assert!(slot.lock().unwrap().is_none());
    }

    #[test]
    fn stale_watchdog_cannot_stop_new_recording() {
        let mut sm = DictationStateMachine::new();
        assert_eq!(sm.hotkey_down(1.0), SmAction::StartRecording);
        let old_session = sm.session();
        assert_eq!(sm.esc(), SmAction::CancelRecording);
        assert_eq!(sm.hotkey_down(2.0), SmAction::StartRecording);
        assert_ne!(sm.session(), old_session);

        assert_eq!(force_stop_session(&mut sm, old_session), SmAction::None);
        assert_eq!(sm.state(), State::Hold);
    }

    #[test]
    fn esc_cannot_interleave_after_commit_gate_before_paste_returns() {
        let mut machine = DictationStateMachine::new();
        assert_eq!(machine.hotkey_down(1.0), SmAction::StartRecording);
        assert_eq!(machine.hotkey_up(2.0), SmAction::StopAndProcess);
        let session = machine.session();
        let sm = Arc::new(Mutex::new(machine));
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();

        let paste_sm = sm.clone();
        let paste_cancel = cancel.clone();
        let paste_thread = std::thread::spawn(move || {
            commit_paste_for_session(&paste_sm, &paste_cancel, session, &|| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let esc_sm = sm.clone();
        let (esc_attempt_tx, esc_attempt_rx) = std::sync::mpsc::channel();
        let (esc_done_tx, esc_done_rx) = std::sync::mpsc::channel();
        let esc_thread = std::thread::spawn(move || {
            esc_attempt_tx.send(()).unwrap();
            let action = esc_sm.lock().unwrap().esc();
            esc_done_tx.send(action).unwrap();
        });
        esc_attempt_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(esc_done_rx.recv_timeout(Duration::from_millis(25)).is_err());

        release_tx.send(()).unwrap();
        paste_thread.join().unwrap().unwrap();
        assert_eq!(
            esc_done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            SmAction::CancelProcessing
        );
        esc_thread.join().unwrap();
    }
}
