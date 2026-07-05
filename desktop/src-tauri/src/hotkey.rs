//! 全域熱鍵（SPEC D7：handy-keys，blocking 模式）。
//! Opt+Shift+C 常駐註冊（吞掉不外漏，同 prototype CGEventTap 行為）；
//! Esc 只在非 IDLE 時動態註冊（錄音/處理中吞掉、閒置時放行——
//! 對應 prototype「state 不是 IDLE 才攔 Esc」的條件式攔截）。
//!
//! HotkeyManager 內含 mpsc::Receiver（!Sync），整個生命週期關在專屬執行緒，
//! 以 try_recv + 短輪詢同時處理鍵盤事件與 arm/disarm 命令。

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use handy_keys::{Hotkey, HotkeyManager, HotkeyState, Key, Modifiers};

#[derive(Debug, Clone, Copy)]
pub enum HotkeyMsg {
    Down(f64),
    Up(f64),
    Esc,
}

#[derive(Debug, Clone, Copy)]
pub enum EscControl {
    Arm,
    Disarm,
}

/// 單調時鐘秒數（給狀態機的 tap 判定）
pub fn now_ts() -> f64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}

pub struct HotkeyService {
    pub events: Receiver<HotkeyMsg>,
    pub esc_ctl: Sender<EscControl>,
}

/// 啟動熱鍵執行緒。Accessibility 未授權時 HotkeyManager::new_with_blocking 會失敗。
pub fn start() -> Result<HotkeyService> {
    let (event_tx, event_rx) = unbounded::<HotkeyMsg>();
    let (ctl_tx, ctl_rx) = unbounded::<EscControl>();
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<()>>(1);

    std::thread::spawn(move || {
        let manager = match HotkeyManager::new_with_blocking() {
            Ok(m) => m,
            Err(e) => {
                let _ = ready_tx.send(Err(anyhow::anyhow!("hotkey manager: {e}")));
                return;
            }
        };
        let main_hotkey = match Hotkey::new(Modifiers::OPT | Modifiers::SHIFT, Key::C)
            .context("build main hotkey")
            .and_then(|hk| manager.register(hk).context("register main hotkey"))
        {
            Ok(id) => id,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        let _ = ready_tx.send(Ok(()));

        let mut esc_id = None;
        loop {
            // esc arm/disarm 命令
            while let Ok(cmd) = ctl_rx.try_recv() {
                match cmd {
                    EscControl::Arm if esc_id.is_none() => {
                        match Hotkey::new(Modifiers::empty(), Key::Escape)
                            .map_err(anyhow::Error::from)
                            .and_then(|hk| manager.register(hk).map_err(anyhow::Error::from))
                        {
                            Ok(id) => esc_id = Some(id),
                            Err(e) => tracing::warn!("arm esc failed: {e}"),
                        }
                    }
                    EscControl::Disarm => {
                        if let Some(id) = esc_id.take() {
                            let _ = manager.unregister(id);
                        }
                    }
                    _ => {}
                }
            }

            // 鍵盤事件
            let mut got = false;
            while let Some(ev) = manager.try_recv() {
                got = true;
                let ts = now_ts();
                let msg = if ev.id == main_hotkey {
                    match ev.state {
                        HotkeyState::Pressed => Some(HotkeyMsg::Down(ts)),
                        HotkeyState::Released => Some(HotkeyMsg::Up(ts)),
                    }
                } else if Some(ev.id) == esc_id {
                    matches!(ev.state, HotkeyState::Pressed).then_some(HotkeyMsg::Esc)
                } else {
                    None
                };
                if let Some(m) = msg {
                    if event_tx.send(m).is_err() {
                        return; // pipeline 收攤，跟著退出
                    }
                }
            }
            if !got {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    });

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(HotkeyService { events: event_rx, esc_ctl: ctl_tx }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow::anyhow!("hotkey thread did not start")),
    }
}
