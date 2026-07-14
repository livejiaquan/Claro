//! 聽寫歷史（~/.claro/history.jsonl），自 prototype/history.py 移植。
//! 核心格式不變：{"ts","duration_s","raw","text","status"}；新增欄位一律可選，
//! 舊讀取器可直接忽略。檔案 0600、目錄 0700。

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::polish::PolishMetadata;

static HISTORY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn history_path() -> PathBuf {
    dirs::home_dir()
        .expect("no home dir")
        .join(".claro")
        .join("history.jsonl")
}

pub struct NewEntry<'a> {
    pub raw: &'a str,
    pub text: &'a str,
    pub duration_s: f64,
    pub status: &'a str,
    /// 各階段耗時（ms），例如 {"stt": 812, "polish": 1430}
    pub timings: Option<Value>,
    /// 可選的後處理稽核資訊；舊 JSONL reader 會忽略這個新增欄位。
    pub polish: Option<PolishMetadata>,
}

pub fn append_entry(entry: NewEntry, path: &Path) -> anyhow::Result<Value> {
    let _guard = HISTORY_LOCK.lock().unwrap();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }

    let ts = chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();
    let mut record = json!({
        "ts": ts,
        "duration_s": (entry.duration_s * 10.0).round() / 10.0,
        "raw": entry.raw,
        "text": entry.text,
        "status": entry.status,
    });
    if let Some(t) = entry.timings {
        record["timings"] = t;
    }
    if let Some(metadata) = entry.polish {
        // 扁平欄位方便 History UI 與外部 JSONL 工具取用；完整物件保留
        // fallback reason / changed 等稽核資訊。兩者都不含螢幕上下文。
        record["polish_mode"] = serde_json::to_value(metadata.mode)?;
        record["polish_outcome"] = serde_json::to_value(metadata.outcome)?;
        record["polish_changed"] = Value::Bool(metadata.changed);
        if let Some(provider) = &metadata.provider {
            record["polish_provider"] = Value::String(provider.clone());
        }
        record["polish"] = serde_json::to_value(metadata)?;
    }

    // 隱私開關只影響正式 history；測試傳入的獨立 temp path 仍可驗證序列化。
    if path == history_path() && !crate::settings::Settings::load().history_enabled() {
        return Ok(record);
    }

    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    f.set_permissions(fs::Permissions::from_mode(0o600))?;
    let mut line = serde_json::to_string(&record)?;
    line.push('\n');
    f.write_all(line.as_bytes())?;
    Ok(record)
}

pub fn clear(path: &Path) -> anyhow::Result<()> {
    let _guard = HISTORY_LOCK.lock().unwrap();
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// 讀最近 n 筆（保序）；檔案不存在 → 空；壞行跳過。
pub fn read_recent(n: usize, path: &Path) -> Vec<Value> {
    let _guard = HISTORY_LOCK.lock().unwrap();
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..]
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let d = std::env::temp_dir()
            .join(format!(
                "claro-hist-{}-{:?}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                std::thread::current().id()
            ))
            .with_extension("")
            .join("history.jsonl");
        d
    }

    // 對應 prototype/tests/test_history.py::test_append_creates_file_and_returns_entry
    #[test]
    fn append_creates_file_and_returns_entry() {
        let path = temp_path();
        let e = append_entry(
            NewEntry {
                raw: "原始",
                text: "整理後",
                duration_s: 3.14,
                status: "pasted",
                timings: None,
                polish: None,
            },
            &path,
        )
        .unwrap();
        assert_eq!(e["raw"], "原始");
        assert_eq!(e["text"], "整理後");
        assert_eq!(e["duration_s"], 3.1);
        assert_eq!(e["status"], "pasted");
        assert!(path.exists());
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // 對應 test_read_recent_returns_last_n_in_order
    #[test]
    fn read_recent_returns_last_n_in_order() {
        let path = temp_path();
        for i in 0..5 {
            append_entry(
                NewEntry {
                    raw: "",
                    text: &format!("t{i}"),
                    duration_s: 1.0,
                    status: "pasted",
                    timings: None,
                    polish: None,
                },
                &path,
            )
            .unwrap();
        }
        let out = read_recent(3, &path);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["text"], "t2");
        assert_eq!(out[2]["text"], "t4");
    }

    // 對應 test_read_recent_missing_file_returns_empty
    #[test]
    fn read_recent_missing_file_returns_empty() {
        assert!(read_recent(10, Path::new("/nonexistent/claro/history.jsonl")).is_empty());
    }

    #[test]
    fn clear_is_idempotent_and_removes_history() {
        let path = temp_path();
        append_entry(
            NewEntry {
                raw: "原文",
                text: "原文",
                duration_s: 1.0,
                status: "pasted",
                timings: None,
                polish: None,
            },
            &path,
        )
        .unwrap();
        clear(&path).unwrap();
        clear(&path).unwrap();
        assert!(!path.exists());
    }

    // 對應 test_read_recent_skips_corrupt_lines
    #[test]
    fn read_recent_skips_corrupt_lines() {
        let path = temp_path();
        append_entry(
            NewEntry {
                raw: "",
                text: "good",
                duration_s: 1.0,
                status: "pasted",
                timings: None,
                polish: None,
            },
            &path,
        )
        .unwrap();
        {
            let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"{corrupt\n").unwrap();
        }
        let out = read_recent(10, &path);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["text"], "good");
    }

    // timings 欄位寫入且不影響其他欄位
    #[test]
    fn timings_field_is_recorded_when_present() {
        let path = temp_path();
        append_entry(
            NewEntry {
                raw: "r",
                text: "t",
                duration_s: 2.0,
                status: "pasted",
                timings: Some(json!({"stt": 812})),
                polish: None,
            },
            &path,
        )
        .unwrap();
        let out = read_recent(1, &path);
        assert_eq!(out[0]["timings"]["stt"], 812);
    }

    #[test]
    fn optional_polish_metadata_is_written_and_old_shape_stays_valid() {
        use crate::polish::{PolishMetadata, PolishOutcome};
        use crate::settings::PolishMode;

        let path = temp_path();
        append_entry(
            NewEntry {
                raw: "先講結論，再講原因",
                text: "原因。結論。",
                duration_s: 2.0,
                status: "pasted",
                timings: None,
                polish: Some(PolishMetadata {
                    mode: PolishMode::Organize,
                    provider: Some("builtin".into()),
                    changed: true,
                    outcome: PolishOutcome::Changed,
                    fallback_reason: None,
                }),
            },
            &path,
        )
        .unwrap();
        let out = read_recent(1, &path);
        assert_eq!(out[0]["polish"]["mode"], "organize");
        assert_eq!(out[0]["polish"]["provider"], "builtin");
        assert_eq!(out[0]["polish"]["changed"], true);
        assert_eq!(out[0]["polish"]["outcome"], "changed");
        assert_eq!(out[0]["polish_mode"], "organize");
        assert_eq!(out[0]["polish_provider"], "builtin");
        assert_eq!(out[0]["polish_outcome"], "changed");
    }
}
