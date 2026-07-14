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

/// 成功貼上只累加成功次數，不得清掉較早尚未由使用者處理的救援文字。
fn record_success(successful_pastes: &AtomicU64) {
    successful_pastes.fetch_add(1, Ordering::SeqCst);
}

fn enqueue_pending_result(queue: &Mutex<VecDeque<PendingResult>>, result: PendingResult) {
    queue.lock().unwrap().push_back(result);
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
    /// 與 context 文字分開：即使 context 關閉或位於敏感 App，
    /// 仍不可把完成文字貼到後來切換的目標。
    target_capture: Mutex<Option<(u64, Option<crate::context::PasteTarget>)>>,
    /// 只存在記憶體，供設定頁稽核；永不寫入 history/config。
    pub last_context: Mutex<Option<crate::context::ContextSnapshot>>,
    /// History 關閉或落盤失敗時的救援文字；只存在本次程序記憶體。
    pub pending_results: Mutex<VecDeque<PendingResult>>,

    capture: Mutex<Option<CaptureHandle>>,
    recording_flag: Arc<AtomicBool>,
    cancel: Mutex<Arc<AtomicBool>>,
    /// STT 最後使用時間——閒置看門狗據此卸載（large-v3-turbo 常駐 1.6GB，
    /// 是待機記憶體的大頭；SPEC §12 待機 <300MB）
    stt_last_used: Mutex<Instant>,
}

impl Core {
    pub fn new(
        engine: Box<dyn SttEngine>,
        active_model: &'static ModelSpec,
        overlay: OverlayClient,
        injector: Box<dyn TextInjector>,
        stop_mic_test: Box<dyn Fn() + Send + Sync>,
        esc_ctl: crossbeam_channel::Sender<Ctl>,
        msg_tx: crossbeam_channel::Sender<Msg>,
        input_device: Option<String>,
    ) -> Self {
        Self {
            sm: Mutex::new(DictationStateMachine::new()),
            engine: Mutex::new(engine),
            active_model: Mutex::new(active_model),
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
            capture: Mutex::new(None),
            recording_flag: Arc::new(AtomicBool::new(false)),
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            stt_last_used: Mutex::new(Instant::now()),
        }
    }

    fn touch_stt(&self) {
        *self.stt_last_used.lock().unwrap() = Instant::now();
    }

    /// 確保 STT 引擎已載入（閒置卸載後的回載）。錄音一開始就在背景先跑，
    /// 載入時間（large-v3-turbo 約 1.4s）被使用者說話的時間蓋掉。
    pub fn ensure_stt_loaded(&self) {
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

    /// 熱換 STT 模型：先丟舊引擎（避免雙駐留，Handy 實證），再背景載入新引擎。
    pub fn swap_model(self: &Arc<Self>, spec: &'static ModelSpec) {
        *self.active_model.lock().unwrap() = spec;
        {
            let mut engine = self.engine.lock().unwrap();
            *engine = Box::new(crate::stt::whisper::WhisperEngine::new(
                spec.id,
                crate::stt::registry::model_path(spec),
            ));
        }
        let core = self.clone();
        std::thread::spawn(move || {
            if let Err(e) = core.engine.lock().unwrap().load() {
                tracing::warn!("model preload after swap failed: {e}");
            }
        });
    }
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

/// dispatcher 主迴圈：在專屬執行緒上跑，直到 msg channel 關閉。
pub fn run_dispatcher(core: Arc<Core>, rx: crossbeam_channel::Receiver<Msg>) {
    for msg in rx {
        match msg {
            Msg::Hotkey(HotkeyMsg::Down(ts)) => {
                let action = core.sm.lock().unwrap().hotkey_down(ts);
                match action {
                    SmAction::StartRecording => start_recording(&core),
                    SmAction::StopAndProcess => start_processing(&core),
                    _ => {}
                }
                core.sync_esc();
            }
            Msg::Hotkey(HotkeyMsg::Up(ts)) => {
                let action = core.sm.lock().unwrap().hotkey_up(ts);
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
                let action = core.sm.lock().unwrap().esc();
                match action {
                    SmAction::CancelRecording => cancel_recording(&core),
                    SmAction::CancelProcessing => {
                        core.cancel.lock().unwrap().store(true, Ordering::SeqCst);
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
    // keyDown 只先固定 focused AX reference（不讀文字／metadata，最長 40ms），
    // 讓同 App A→B 也可辨識；完整 hash 在麥克風開始收音後才做。
    let session = core.sm.lock().unwrap().session();
    let target_seed = crate::context::begin_paste_target_capture();
    let device = core.input_device.lock().unwrap().clone();
    let capture_result = audio::start_capture(device);
    drop(audio_gate);
    match capture_result {
        Ok(handle) => {
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
                    let current = {
                        let sm = overlay_core.sm.lock().unwrap();
                        if sm.session() == session && flag.load(Ordering::SeqCst) {
                            overlay_core
                                .overlay
                                .send(&format!("level {:.4}", level.get()));
                            true
                        } else {
                            false
                        }
                    };
                    if !current {
                        break;
                    }
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

/// 停止錄音並取回音訊；None 表示不合格（已送 error 態與 history）
fn collect_audio(
    core: &Arc<Core>,
    handle: Option<CaptureHandle>,
    cancelled: bool,
    session: u64,
    cancel: Option<&AtomicBool>,
) -> Option<Vec<f32>> {
    let handle = handle?;
    let samples = match handle.stop() {
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
    Some(audio::normalize(samples))
}

fn start_processing(core: &Arc<Core>) {
    // 每個 session 配發自己的取消旗標（prototype：不洗掉上一段還沒檢查到的取消）
    let cancel = Arc::new(AtomicBool::new(false));
    *core.cancel.lock().unwrap() = cancel.clone();
    let session = core.sm.lock().unwrap().session();
    let capture = detach_capture(core);
    let core = core.clone();

    std::thread::spawn(move || {
        process_session(&core, session, &cancel, capture);
        core.sm.lock().unwrap().processing_finished(session);
        core.sync_esc();
    });
}

fn paste_target_matches(
    expected: Option<&crate::context::PasteTarget>,
    current: Option<&crate::context::PasteTarget>,
) -> bool {
    matches!((expected, current), (Some(expected), Some(current)) if expected == current)
}

fn context_matches_target(
    expected: Option<&crate::context::PasteTarget>,
    snapshot: &crate::context::ContextSnapshot,
) -> bool {
    expected.is_some_and(|target| target == &snapshot.target)
}

fn async_feedback_allowed(core: &Core, session: u64, cancel: &AtomicBool) -> bool {
    !cancel.load(Ordering::SeqCst) && core.sm.lock().unwrap().session() == session
}

fn commit_paste_for_session(
    sm: &Mutex<DictationStateMachine>,
    cancel: &AtomicBool,
    session: u64,
    paste: &dyn Fn() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let sm = sm.lock().unwrap();
    if cancel.load(Ordering::SeqCst) || sm.session() != session || sm.state() != State::Processing {
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
) {
    let Some(samples) = collect_audio(core, capture, false, session, Some(cancel)) else {
        return;
    };
    let dur = samples.len() as f64 / audio::TARGET_RATE as f64;
    send_overlay_for_session(core, session, cancel, "processing");
    // 單一 session 只讀一次設定，避免使用者在轉錄途中切模式造成 provider、
    // consent 與 history metadata 彼此不一致。
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
    let raw = {
        let dict_terms: Vec<String> = dict_pairs.iter().map(|(_, right)| right.clone()).collect();
        let terms = crate::context::context_terms(&dict_terms, &screen_ctx, 15);
        let req = crate::stt::SttRequest {
            audio: &samples,
            language: Some("zh"),
            initial_prompt: Some(crate::stt::build_initial_prompt(&terms)),
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

    // RAW / CLEAN / ORGANIZE 共用同一入口。provider 不可用、未同意或 guard
    // 拒絕時 transform 一律回 deterministic base text，並留下可稽核 metadata。
    let t1 = Instant::now();
    let polish::PolishResult {
        text: output,
        metadata: polish_metadata,
    } = polish::transform_with_cancel(&settings, &base_text, &screen_ctx, cancel);
    if low_memory_builtin {
        crate::llm::unload_blocking();
        tracing::info!("low-memory handoff: unloaded builtin LLM after polishing");
    }
    let polish_ms =
        (settings.polish_mode() != PolishMode::Raw).then(|| t1.elapsed().as_millis() as u64);
    let text = textproc::normalize_cjk_punct(&textproc::to_traditional(&output));

    let timings = Some(json!({ "stt_ms": stt_ms, "polish_ms": polish_ms }));

    if !async_feedback_allowed(core, session, cancel) {
        let _ = history::append_entry(
            NewEntry {
                raw: &raw,
                text: &text,
                duration_s: dur,
                status: "cancelled",
                timings,
                polish: Some(polish_metadata),
            },
            &history::history_path(),
        );
        return;
    }

    let current_target = crate::context::capture_paste_target();
    // Esc 或新 session 可能在 bounded AX 驗證期間發生；貼上前必須再檢查一次。
    if !async_feedback_allowed(core, session, cancel) {
        let _ = history::append_entry(
            NewEntry {
                raw: &raw,
                text: &text,
                duration_s: dur,
                status: "cancelled",
                timings,
                polish: Some(polish_metadata),
            },
            &history::history_path(),
        );
        return;
    }

    if !paste_target_matches(target_app_id.as_ref(), current_target.as_ref()) {
        tracing::warn!("paste target changed or is unavailable — preserving result in history");
        send_overlay_for_session(core, session, cancel, "error");
        let _ = history::append_entry(
            NewEntry {
                raw: &raw,
                text: &text,
                duration_s: dur,
                status: "focus_changed",
                timings,
                polish: Some(polish_metadata),
            },
            &history::history_path(),
        );
        // 一律保留於 process-memory queue：處理途中切換 history 開關、磁碟滿或
        // 寫入失敗都不能形成沒有落盤、也沒有救援副本的資料遺失窗口。
        enqueue_pending_result(
            &core.pending_results,
            PendingResult {
                raw: raw.clone(),
                text: text.clone(),
                reason: "focus_changed",
            },
        );
        return;
    }

    let pre_paste_commit = |paste: &dyn Fn() -> anyhow::Result<()>| -> anyhow::Result<()> {
        let current = crate::context::capture_paste_target();
        if !paste_target_matches(target_app_id.as_ref(), current.as_ref()) {
            anyhow::bail!("paste target changed before Cmd+V");
        }
        // 持有 state-machine lock 到 Cmd+V 完成；Esc／新 session 只能在線性化點
        // 之前先讓本次失敗，或在本次貼上完成後才生效，不能插入兩者之間。
        commit_paste_for_session(&core.sm, cancel, session, paste)
    };
    if let Err(e) = core.injector.inject(&text, &pre_paste_commit) {
        tracing::error!("paste failed: {e}");
        if !async_feedback_allowed(core, session, cancel) {
            let _ = history::append_entry(
                NewEntry {
                    raw: &raw,
                    text: &text,
                    duration_s: dur,
                    status: "cancelled",
                    timings,
                    polish: Some(polish_metadata),
                },
                &history::history_path(),
            );
            return;
        }
        send_overlay_for_session(core, session, cancel, "error");
        let _ = history::append_entry(
            NewEntry {
                raw: &raw,
                text: &text,
                duration_s: dur,
                status: "paste_failed",
                timings,
                polish: Some(polish_metadata),
            },
            &history::history_path(),
        );
        enqueue_pending_result(
            &core.pending_results,
            PendingResult {
                raw: raw.clone(),
                text: text.clone(),
                reason: "paste_failed",
            },
        );
        return;
    }
    send_overlay_for_session(core, session, cancel, "success");
    record_success(&core.successful_pastes);
    let _ = history::append_entry(
        NewEntry {
            raw: &raw,
            text: &text,
            duration_s: dur,
            status: "pasted",
            timings,
            polish: Some(polish_metadata),
        },
        &history::history_path(),
    );
}

/// Esc 取消錄音：立即收 UI，背景仍轉錄一份進歷史（可救回，prototype 語意）
fn cancel_recording(core: &Arc<Core>) {
    core.overlay.send("cancel");
    let session = core.sm.lock().unwrap().session();
    let capture = detach_capture(core);
    let core = core.clone();
    std::thread::spawn(move || {
        let Some(samples) = collect_audio(&core, capture, true, session, None) else {
            return;
        };
        let dur = samples.len() as f64 / audio::TARGET_RATE as f64;
        let req = crate::stt::SttRequest {
            audio: &samples,
            language: Some("zh"),
            initial_prompt: Some(crate::stt::build_initial_prompt(&[])),
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
        commit_paste_for_session, context_matches_target, enqueue_pending_result,
        force_stop_session, paste_target_matches, record_success, take_session_slot, PendingResult,
    };
    use crate::context::{ContextSnapshot, PasteTarget};
    use crate::state_machine::{DictationStateMachine, SmAction, State};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn paste_requires_same_known_target() {
        let target = PasteTarget {
            app_id: "com.apple.TextEdit".into(),
            window_hash: [1; 32],
            focus_hash: [2; 32],
        };
        let same = target.clone();
        let other_app = PasteTarget {
            app_id: "com.apple.Safari".into(),
            ..target.clone()
        };
        let other_focus = PasteTarget {
            focus_hash: [3; 32],
            ..target.clone()
        };
        assert!(paste_target_matches(Some(&target), Some(&same)));
        assert!(!paste_target_matches(Some(&target), Some(&other_app)));
        assert!(!paste_target_matches(Some(&target), Some(&other_focus)));
        assert!(!paste_target_matches(Some(&target), None));
        assert!(!paste_target_matches(None, Some(&target)));
        assert!(!paste_target_matches(None, None));
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
    fn failed_results_queue_without_overwriting_older_recovery() {
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
        assert_eq!(saved.front().unwrap().text, "第一段");
        assert_eq!(saved.back().unwrap().text, "第二段");
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
