//! 單次聽寫 session 的編排（SPEC §3）。
//! 結構移植 prototype：熱鍵事件由單一 dispatcher 執行緒序列化餵狀態機
//! （keyDown/keyUp 亂序會卡死錄音——prototype 踩過的坑）；
//! 錄音、轉錄各自在背景執行緒；每個處理 session 有自己的取消旗標。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::audio::{self, CaptureHandle};
use crate::history::{self, NewEntry};
use crate::hotkey::{EscControl, HotkeyMsg};
use crate::inject::TextInjector;
use crate::overlay_client::OverlayClient;
use crate::polish;
use crate::settings::Settings;
use crate::state_machine::{DictationStateMachine, SmAction, State};
use crate::stt::registry::ModelSpec;
use crate::stt::SttEngine;
use crate::textproc;

pub enum Msg {
    Hotkey(HotkeyMsg),
    ForceStop,
}

pub struct Core {
    pub sm: Mutex<DictationStateMachine>,
    pub engine: Mutex<Box<dyn SttEngine>>,
    /// UI 顯示與模型管理用的「使用中模型」（與 engine 分開，避免轉錄中卡 get_status）
    pub active_model: Mutex<&'static ModelSpec>,
    pub overlay: OverlayClient,
    pub injector: Box<dyn TextInjector>,
    /// 個人字典（設定 UI 可即時更新）
    pub dict: Mutex<Vec<(String, String)>>,
    /// 熱鍵服務可能在授權後重試重建，所以是 Mutex
    pub esc_ctl: Mutex<crossbeam_channel::Sender<EscControl>>,
    pub msg_tx: crossbeam_channel::Sender<Msg>,
    /// 使用者指定的輸入裝置（None = 系統預設）；設定 UI 可即時更新
    pub input_device: Mutex<Option<String>>,

    capture: Mutex<Option<CaptureHandle>>,
    recording_flag: Arc<AtomicBool>,
    cancel: Mutex<Arc<AtomicBool>>,
}

impl Core {
    pub fn new(
        engine: Box<dyn SttEngine>,
        active_model: &'static ModelSpec,
        overlay: OverlayClient,
        injector: Box<dyn TextInjector>,
        esc_ctl: crossbeam_channel::Sender<EscControl>,
        msg_tx: crossbeam_channel::Sender<Msg>,
        input_device: Option<String>,
    ) -> Self {
        Self {
            sm: Mutex::new(DictationStateMachine::new()),
            engine: Mutex::new(engine),
            active_model: Mutex::new(active_model),
            overlay,
            injector,
            dict: Mutex::new(Settings::load().dictionary()),
            esc_ctl: Mutex::new(esc_ctl),
            msg_tx,
            input_device: Mutex::new(input_device),
            capture: Mutex::new(None),
            recording_flag: Arc::new(AtomicBool::new(false)),
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
        }
    }

    /// 依狀態機現況同步 Esc 的攔截（非 IDLE 才攔，對應 prototype 條件式攔截）
    fn sync_esc(&self) {
        let state = self.sm.lock().unwrap().state();
        let _ = self.esc_ctl.lock().unwrap().send(if state == State::Idle {
            EscControl::Disarm
        } else {
            EscControl::Arm
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
            Msg::ForceStop => {
                let action = core.sm.lock().unwrap().force_stop();
                if action == SmAction::StopAndProcess {
                    start_processing(&core);
                }
                core.sync_esc();
            }
        }
    }
}

fn start_recording(core: &Arc<Core>) {
    let device = core.input_device.lock().unwrap().clone();
    match audio::start_capture(device) {
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
                    overlay_core.overlay.send(&format!("level {:.4}", level.get()));
                    if !force_sent && started.elapsed().as_secs_f64() > audio::MAX_RECORDING_S {
                        let _ = msg_tx.send(Msg::ForceStop);
                        force_sent = true;
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
            });
            *core.capture.lock().unwrap() = Some(handle);
            core.overlay.send("recording");
            tracing::info!("recording... (release to paste, esc to cancel)");
        }
        Err(e) => {
            tracing::error!("audio input failed: {e}");
            core.overlay.send("error");
            // 錄不了音：把狀態機拉回 IDLE（沿用 esc 語意）
            let _ = core.sm.lock().unwrap().esc();
        }
    }
}

/// 停止錄音並取回音訊；None 表示不合格（已送 error 態與 history）
fn collect_audio(core: &Arc<Core>, cancelled: bool) -> Option<Vec<f32>> {
    core.recording_flag.store(false, Ordering::SeqCst);
    let handle = core.capture.lock().unwrap().take()?;
    let samples = match handle.stop() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("audio capture failed: {e}");
            if !cancelled {
                core.overlay.send("error");
                let _ = history::append_entry(
                    NewEntry { raw: "", text: "", duration_s: 0.0, status: "error", timings: None },
                    &history::history_path(),
                );
            }
            return None;
        }
    };
    if let Err(guard) = audio::validate(&samples, audio::TARGET_RATE) {
        let dur = samples.len() as f64 / audio::TARGET_RATE as f64;
        let status = if cancelled {
            "cancelled"
        } else {
            match guard {
                audio::AudioGuard::TooShort => "too_short",
                audio::AudioGuard::Silent => "silent",
            }
        };
        if !cancelled {
            core.overlay.send("error");
        }
        let _ = history::append_entry(
            NewEntry { raw: "", text: "", duration_s: dur, status, timings: None },
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
    let core = core.clone();

    std::thread::spawn(move || {
        process_session(&core, session, &cancel);
        core.sm.lock().unwrap().processing_finished(session);
        core.sync_esc();
    });
}

fn process_session(core: &Arc<Core>, _session: u64, cancel: &AtomicBool) {
    let Some(samples) = collect_audio(core, false) else {
        return;
    };
    let dur = samples.len() as f64 / audio::TARGET_RATE as f64;
    core.overlay.send("processing");

    // 螢幕上下文（M3 前移）：AX 抓前景視窗詞彙，餵辨識偏置＋潤飾參考。
    // 只在記憶體、永不落盤；使用者可在設定關閉。
    let screen_ctx = if Settings::load().context_enabled() {
        let t = Instant::now();
        let ctx = crate::context::capture().unwrap_or_default();
        if !ctx.is_empty() {
            tracing::info!(
                "🖥️ context: {} chars in {}ms",
                ctx.chars().count(),
                t.elapsed().as_millis()
            );
        }
        ctx
    } else {
        String::new()
    };

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
        match core.engine.lock().unwrap().transcribe(&req) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("transcription failed: {e}");
                core.overlay.send("error");
                let _ = history::append_entry(
                    NewEntry { raw: "", text: "", duration_s: dur, status: "error", timings: None },
                    &history::history_path(),
                );
                return;
            }
        }
    };
    let stt_ms = t0.elapsed().as_millis() as u64;

    let raw = textproc::normalize_cjk_punct(&textproc::to_traditional(&textproc::clean_transcript(&raw)));
    if raw.is_empty() {
        core.overlay.send("error");
        let _ = history::append_entry(
            NewEntry { raw: "", text: "", duration_s: dur, status: "silent", timings: None },
            &history::history_path(),
        );
        return;
    }

    let mut text = textproc::apply_dict(&raw, &dict_pairs);

    // LLM 潤飾（使用者在設定中選擇；失敗一律退回原文，聽寫不因 LLM 而失敗）
    let mut polish_ms: Option<u64> = None;
    if let Some(polisher) = polish::from_settings(&Settings::load()) {
        let t1 = Instant::now();
        match polisher.polish(&text, &screen_ctx) {
            Ok(p) => {
                text = textproc::normalize_cjk_punct(&textproc::to_traditional(&p));
                polish_ms = Some(t1.elapsed().as_millis() as u64);
            }
            Err(e) => tracing::warn!("polish failed, using raw transcript: {e}"),
        }
        // 潤飾期間使用者可能按了 Esc
        if cancel.load(Ordering::SeqCst) {
            let _ = history::append_entry(
                NewEntry {
                    raw: &raw,
                    text: &text,
                    duration_s: dur,
                    status: "cancelled",
                    timings: Some(json!({ "stt_ms": stt_ms, "polish_ms": polish_ms })),
                },
                &history::history_path(),
            );
            return;
        }
    }

    let timings = Some(json!({ "stt_ms": stt_ms, "polish_ms": polish_ms }));

    if cancel.load(Ordering::SeqCst) {
        let _ = history::append_entry(
            NewEntry { raw: &raw, text: &text, duration_s: dur, status: "cancelled", timings },
            &history::history_path(),
        );
        return;
    }

    tracing::info!("📝 {text}");
    if let Err(e) = core.injector.inject(&text) {
        tracing::error!("paste failed: {e}");
        core.overlay.send("error");
        let _ = history::append_entry(
            NewEntry { raw: &raw, text: &text, duration_s: dur, status: "error", timings },
            &history::history_path(),
        );
        return;
    }
    core.overlay.send("success");
    let _ = history::append_entry(
        NewEntry { raw: &raw, text: &text, duration_s: dur, status: "pasted", timings },
        &history::history_path(),
    );
}

/// Esc 取消錄音：立即收 UI，背景仍轉錄一份進歷史（可救回，prototype 語意）
fn cancel_recording(core: &Arc<Core>) {
    core.overlay.send("cancel");
    let core = core.clone();
    std::thread::spawn(move || {
        let Some(samples) = collect_audio(&core, true) else {
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
            .map(|t| textproc::normalize_cjk_punct(&textproc::to_traditional(&textproc::clean_transcript(&t))))
            .unwrap_or_default();
        let _ = history::append_entry(
            NewEntry { raw: &raw, text: &raw, duration_s: dur, status: "cancelled", timings: None },
            &history::history_path(),
        );
    });
}
