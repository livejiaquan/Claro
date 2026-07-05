//! 沿用 prototype 的 Swift `mic_indicator` overlay（SPEC D10）：
//! Rust spawn 該二進位（argv[1] = 本進程 pid），以 Unix socket
//! `~/.claro/indicator.sock` 送單行文字命令，協議與 prototype 完全相同：
//! recording / handsfree / level <rms> / processing / success / cancel / error / quit

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub fn socket_path() -> PathBuf {
    dirs::home_dir().expect("no home dir").join(".claro").join("indicator.sock")
}

/// 依序尋找 mic_indicator 二進位：
/// 1. CLARO_INDICATOR_PATH 環境變數
/// 2. 執行檔同目錄
/// 3. repo 內 prototype/（開發模式）
/// 4. /usr/local/bin/mic_indicator
fn find_indicator() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CLARO_INDICATOR_PATH") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("mic_indicator");
            if p.exists() {
                return Some(p);
            }
        }
    }
    // 開發模式：desktop/src-tauri → repo 根 → prototype/
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../prototype/mic_indicator");
    if dev.exists() {
        return Some(dev);
    }
    let usr = PathBuf::from("/usr/local/bin/mic_indicator");
    usr.exists().then_some(usr)
}

pub struct OverlayClient {
    child: std::sync::Mutex<Option<Child>>,
}

impl OverlayClient {
    /// spawn overlay；找不到二進位時靜默降級（無 overlay 仍可聽寫）。
    pub fn start() -> Self {
        let child = match find_indicator() {
            Some(path) => Command::new(&path)
                .arg(std::process::id().to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| tracing::warn!("mic_indicator spawn failed: {e}"))
                .ok(),
            None => {
                tracing::warn!("mic_indicator not found, skipping floating overlay");
                None
            }
        };
        Self { child: std::sync::Mutex::new(child) }
    }

    /// 送命令；任何錯誤都吞掉（overlay 掛了不影響聽寫）。
    pub fn send(&self, command: &str) {
        let _ = Self::send_raw(command);
    }

    fn send_raw(command: &str) -> std::io::Result<()> {
        let stream = UnixStream::connect(socket_path())?;
        stream.set_write_timeout(Some(Duration::from_millis(500)))?;
        let mut stream = stream;
        stream.write_all(format!("{command}\n").as_bytes())
    }

    pub fn stop(&self) {
        let taken = self.child.lock().ok().and_then(|mut c| c.take());
        if let Some(mut child) = taken {
            let _ = Self::send_raw("quit");
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    _ => {
                        let _ = child.kill();
                        break;
                    }
                }
            }
        }
    }
}

impl Drop for OverlayClient {
    fn drop(&mut self) {
        OverlayClient::stop(self);
    }
}
