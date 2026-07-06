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
pub enum Ctl {
    ArmEsc,
    DisarmEsc,
    /// 換主熱鍵（設定頁自訂）：先註冊新鍵成功才卸舊鍵，失敗維持原狀
    SetMain(Hotkey),
}

/// 預設主熱鍵；config `hotkey` 鍵可覆寫（見 settings::hotkey_combo）
pub const DEFAULT_COMBO: &str = "Opt+Shift+C";

/// 解析使用者設定的熱鍵字串；無效時退回預設（絕不讓 app 沒有熱鍵）
pub fn parse_combo(combo: &str) -> Hotkey {
    combo
        .parse::<Hotkey>()
        .unwrap_or_else(|_| DEFAULT_COMBO.parse().expect("default combo valid"))
}

/// 單調時鐘秒數（給狀態機的 tap 判定）
pub fn now_ts() -> f64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}

pub struct HotkeyService {
    pub events: Receiver<HotkeyMsg>,
    pub esc_ctl: Sender<Ctl>,
}

/// 啟動熱鍵執行緒。Accessibility 未授權時 HotkeyManager::new_with_blocking 會失敗。
pub fn start(main: Hotkey) -> Result<HotkeyService> {
    let (event_tx, event_rx) = unbounded::<HotkeyMsg>();
    let (ctl_tx, ctl_rx) = unbounded::<Ctl>();
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<()>>(1);

    std::thread::spawn(move || {
        let manager = match HotkeyManager::new_with_blocking() {
            Ok(m) => m,
            Err(e) => {
                let _ = ready_tx.send(Err(anyhow::anyhow!("hotkey manager: {e}")));
                return;
            }
        };
        let mut main_hotkey = match manager.register(main).context("register main hotkey") {
            Ok(id) => id,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        let _ = ready_tx.send(Ok(()));

        let mut esc_id = None;
        loop {
            // 控制命令：Esc 攔截開關／換主熱鍵
            while let Ok(cmd) = ctl_rx.try_recv() {
                match cmd {
                    Ctl::ArmEsc if esc_id.is_none() => {
                        match Hotkey::new(Modifiers::empty(), Key::Escape)
                            .map_err(anyhow::Error::from)
                            .and_then(|hk| manager.register(hk).map_err(anyhow::Error::from))
                        {
                            Ok(id) => esc_id = Some(id),
                            Err(e) => tracing::warn!("arm esc failed: {e}"),
                        }
                    }
                    Ctl::DisarmEsc => {
                        if let Some(id) = esc_id.take() {
                            let _ = manager.unregister(id);
                        }
                    }
                    Ctl::SetMain(hk) => match manager.register(hk) {
                        Ok(new_id) => {
                            let _ = manager.unregister(main_hotkey);
                            main_hotkey = new_id;
                            tracing::info!("main hotkey changed to {hk}");
                        }
                        Err(e) => tracing::warn!("set main hotkey failed, keeping old: {e}"),
                    },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_presets_all_parse() {
        // 設定頁提供的每個 preset 都必須能解析（與 Settings.tsx 同步維護）
        for combo in ["Opt+Shift+C", "CmdRight", "OptRight", "Fn", "F5"] {
            assert!(combo.parse::<Hotkey>().is_ok(), "preset {combo} must parse");
        }
    }

    #[test]
    fn invalid_combo_falls_back_to_default() {
        let hk = parse_combo("NotAKey+Whatever");
        assert_eq!(hk, DEFAULT_COMBO.parse::<Hotkey>().unwrap());
    }
}
