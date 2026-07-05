//! 設定檔讀寫，自 prototype/config.py 移植。
//! M1 沿用 prototype 的扁平 schema（whisper_model / llm_model / llm_enabled），
//! 未知欄位一律保留（merge over defaults）；schema 分節化與遷移在 M2（SPEC §14）。

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

pub fn default_config() -> Map<String, Value> {
    let Value::Object(m) = json!({
        "whisper_model": "large-v3-turbo",
        "llm_model": "mlx-community/Qwen2.5-7B-Instruct-4bit",
        "llm_enabled": true,
    }) else {
        unreachable!()
    };
    m
}

pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .expect("no home dir")
        .join(".claro")
        .join("config.json")
}

fn write_defaults(path: &Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.set_permissions(fs::Permissions::from_mode(0o600))?;
    let mut s = serde_json::to_string_pretty(&Value::Object(default_config()))?;
    s.push('\n');
    f.write_all(s.as_bytes())?;
    Ok(())
}

/// 讀設定：檔案不存在 → 以預設值建立（0600）；解析失敗 → 用預設值並警告，不崩潰；
/// 有效檔案 → 蓋在預設值上（未知欄位保留）。
pub fn load_config(path: &Path) -> Map<String, Value> {
    if !path.exists() {
        if let Err(e) = write_defaults(path) {
            tracing::warn!("could not write default config {}: {e}", path.display());
        }
        return default_config();
    }

    let parsed: Option<Map<String, Value>> = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        });

    match parsed {
        Some(data) => {
            let mut cfg = default_config();
            for (k, v) in data {
                cfg.insert(k, v);
            }
            cfg
        }
        None => {
            tracing::warn!("could not parse config {}, using defaults", path.display());
            default_config()
        }
    }
}

/// 型別化存取（讀不到就退預設值）
pub struct Settings {
    pub raw: Map<String, Value>,
}

impl Settings {
    pub fn load() -> Self {
        Self { raw: load_config(&config_path()) }
    }

    pub fn from_path(path: &Path) -> Self {
        Self { raw: load_config(path) }
    }

    pub fn whisper_model(&self) -> String {
        self.raw
            .get("whisper_model")
            .and_then(Value::as_str)
            .unwrap_or("large-v3-turbo")
            .to_string()
    }

    pub fn llm_enabled(&self) -> bool {
        self.raw.get("llm_enabled").and_then(Value::as_bool).unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 對應 prototype/tests/test_config.py::test_missing_file_creates_defaults_with_private_mode
    #[test]
    fn missing_file_creates_defaults_with_private_mode() {
        let dir = tempdir();
        let path = dir.join("cfg").join("config.json");
        let cfg = load_config(&path);
        assert_eq!(cfg.get("whisper_model").unwrap(), "large-v3-turbo");
        assert!(path.exists());
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
    }

    // 對應 test_valid_file_overrides_defaults
    #[test]
    fn valid_file_overrides_defaults() {
        let dir = tempdir();
        let path = dir.join("config.json");
        fs::write(&path, r#"{"whisper_model": "small", "llm_enabled": false}"#).unwrap();
        let cfg = load_config(&path);
        assert_eq!(cfg.get("whisper_model").unwrap(), "small");
        assert_eq!(cfg.get("llm_enabled").unwrap(), false);
        // 未指定的鍵保留預設
        assert!(cfg.get("llm_model").unwrap().as_str().unwrap().contains("Qwen"));
    }

    // 對應 test_corrupt_json_returns_defaults_without_exception
    #[test]
    fn corrupt_json_returns_defaults_without_exception() {
        let dir = tempdir();
        let path = dir.join("config.json");
        fs::write(&path, "{not json!!").unwrap();
        let cfg = load_config(&path);
        assert_eq!(cfg.get("whisper_model").unwrap(), "large-v3-turbo");
    }

    // 對應 test_unknown_keys_are_preserved_in_returned_config
    #[test]
    fn unknown_keys_are_preserved_in_returned_config() {
        let dir = tempdir();
        let path = dir.join("config.json");
        fs::write(&path, r#"{"future_field": {"a": 1}}"#).unwrap();
        let cfg = load_config(&path);
        assert_eq!(cfg.get("future_field").unwrap()["a"], 1);
        assert_eq!(cfg.get("whisper_model").unwrap(), "large-v3-turbo");
    }

    // JSON root 不是 object → 退預設值
    #[test]
    fn non_object_root_returns_defaults() {
        let dir = tempdir();
        let path = dir.join("config.json");
        fs::write(&path, "[1,2,3]").unwrap();
        let cfg = load_config(&path);
        assert_eq!(cfg.get("whisper_model").unwrap(), "large-v3-turbo");
    }

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("claro-test-{}", rand_suffix()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn rand_suffix() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        format!(
            "{}-{:?}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
            std::thread::current().id()
        )
        .replace(['(', ')', ' '], "")
    }
}
