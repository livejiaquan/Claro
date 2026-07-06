//! 文字注入：剪貼簿備份 → 寫入 → 合成 Cmd+V → 還原（SPEC §9）。
//! 自 prototype/main.py::_paste_text 移植。
//! 已知限制（沿用並文件化）：備份/還原只保留純文字，富文本/圖片會遺失。
//! 還原延遲 300ms（SPEC D8：Handy 的 50ms 會 race 慢的目標 app）。
//! CJK IME 輸入源防護在 M4（SPEC D9）。

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

pub trait TextInjector: Send + Sync {
    fn inject(&self, text: &str) -> Result<()>;
}

pub struct MacPasteInjector;

/// macOS kVK_ANSI_V；用虛擬鍵碼而非字元，避免鍵盤配置差異
const VK_V: u32 = 9;

impl TextInjector for MacPasteInjector {
    fn inject(&self, text: &str) -> Result<()> {
        let mut clipboard = arboard::Clipboard::new().context("open clipboard")?;
        // 備份失敗（例如剪貼簿是圖片）不阻擋貼上，只是不還原
        let backup = clipboard.get_text().ok();

        clipboard.set_text(text.to_string()).context("set clipboard")?;
        thread::sleep(Duration::from_millis(50));

        // 貼上失敗也要還原剪貼簿——不能讓使用者原本複製的東西被聽寫文字蓋掉
        let paste_result = send_cmd_v().context("synthesize Cmd+V");

        if let Some(old) = backup {
            thread::sleep(Duration::from_millis(300));
            let _ = clipboard.set_text(old);
        }
        paste_result
    }
}

fn send_cmd_v() -> Result<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).context("init enigo")?;
    enigo.key(Key::Meta, Direction::Press)?;
    enigo.key(Key::Other(VK_V), Direction::Click)?;
    enigo.key(Key::Meta, Direction::Release)?;
    Ok(())
}
