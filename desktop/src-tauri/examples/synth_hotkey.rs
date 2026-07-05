//! 測試工具：合成 Claro 熱鍵事件（⌥⇧C / Esc），配合 afplay 播 TTS 做全自動 e2e。
//! 用法：cargo run --example synth_hotkey <down|up|tap|esc>

use enigo::{Direction, Enigo, Key, Keyboard, Settings};

const VK_C: u32 = 8;
const VK_ESC: u32 = 53;

fn main() {
    let action = std::env::args().nth(1).unwrap_or_default();
    let mut e = Enigo::new(&Settings::default()).expect("enigo init (needs accessibility)");
    match action.as_str() {
        "down" => {
            e.key(Key::Alt, Direction::Press).unwrap();
            e.key(Key::Shift, Direction::Press).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
            e.key(Key::Other(VK_C), Direction::Press).unwrap();
        }
        "up" => {
            e.key(Key::Other(VK_C), Direction::Release).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
            e.key(Key::Shift, Direction::Release).unwrap();
            e.key(Key::Alt, Direction::Release).unwrap();
        }
        "tap" => {
            e.key(Key::Alt, Direction::Press).unwrap();
            e.key(Key::Shift, Direction::Press).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
            e.key(Key::Other(VK_C), Direction::Press).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(120));
            e.key(Key::Other(VK_C), Direction::Release).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
            e.key(Key::Shift, Direction::Release).unwrap();
            e.key(Key::Alt, Direction::Release).unwrap();
        }
        "esc" => {
            e.key(Key::Other(VK_ESC), Direction::Click).unwrap();
        }
        other => {
            eprintln!("unknown action '{other}' — use down|up|tap|esc");
            std::process::exit(1);
        }
    }
}
