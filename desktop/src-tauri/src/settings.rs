//! 設定檔讀寫，自 prototype/config.py 移植。
//! M1 沿用 prototype 的扁平 schema（whisper_model / llm_model / llm_enabled），
//! 未知欄位一律保留（merge over defaults）；schema 分節化與遷移在 M2（SPEC §14）。

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const ORGANIZE_CONSENT_VERSION: u64 = 1;
pub const CLOUD_CONSENT_VERSION: u64 = 1;

/// 聽寫後處理模式。模式與 LLM provider 分離：即使預設意圖是 Clean，
/// provider=off 時 pipeline 仍會安全退回 deterministic base text。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolishMode {
    Raw,
    Clean,
    Organize,
}

impl PolishMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Clean => "clean",
            Self::Organize => "organize",
        }
    }
}

impl Default for PolishMode {
    fn default() -> Self {
        Self::Clean
    }
}

impl FromStr for PolishMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "raw" => Ok(Self::Raw),
            "clean" => Ok(Self::Clean),
            "organize" => Ok(Self::Organize),
            _ => Err(format!("未知的潤飾模式：{value}")),
        }
    }
}

pub fn default_config() -> Map<String, Value> {
    let Value::Object(m) = json!({
        "whisper_model": "large-v3-turbo",
        "llm_model": "mlx-community/Qwen2.5-7B-Instruct-4bit",
        "llm_enabled": true,
        // 輸出意圖預設 CLEAN；provider 仍預設 off，所以未選引擎時安全退回原文。
        "polish_mode": "clean",
        // 預設硬斷自訂雲端潤飾；只允許使用者經明確同意解除。
        "local_only": true,
        "organize_consent_version": 0,
        "cloud_consent_version": 0,
        // 雲端同意綁定到當時確認的 scheme + authority；改端點後必須重新確認。
        "cloud_consent_origin": null,
        // 必須完成權限、本次麥克風測試與模型檢查才由 UI 寫入。
        "setup_completed": false,
        // 本地聽寫歷史預設開啟；可由使用者關閉並清除。
        "history_enabled": true,
        // 主熱鍵（handy-keys 字串格式，如 "Opt+Shift+C"、"CmdRight"）
        "hotkey": "Opt+Shift+C",
        // 個人字典（誤認詞 → 正確詞）：貼上前替換＋餵給語音模型做詞彙偏置
        "dictionary": { "GBT": "GPT", "My Torch": "PyTorch" },
        // 螢幕上下文（AX）：抓前景視窗詞彙給辨識與潤飾；內容永不落盤
        "context_enabled": true,
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
        Self {
            raw: load_config(&config_path()),
        }
    }

    pub fn from_path(path: &Path) -> Self {
        Self {
            raw: load_config(path),
        }
    }

    pub fn whisper_model(&self) -> String {
        self.raw
            .get("whisper_model")
            .and_then(Value::as_str)
            .unwrap_or("large-v3-turbo")
            .to_string()
    }

    pub fn llm_enabled(&self) -> bool {
        self.raw
            .get("llm_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    /// 使用者指定的輸入裝置（缺省 = 系統預設）
    pub fn input_device(&self) -> Option<String> {
        self.raw
            .get("input_device")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// 潤飾 provider："off"（預設）| "ollama" | "custom"。
    /// 注意：prototype 的 llm_enabled/llm_model 是 MLX 語意，desktop 不沿用，
    /// 未明確設定 llm_provider 一律視為 off（不悄悄外連）。
    pub fn llm_provider(&self) -> String {
        self.raw
            .get("llm_provider")
            .and_then(Value::as_str)
            .unwrap_or("off")
            .to_string()
    }

    pub fn llm_model(&self) -> String {
        self.raw
            .get("llm_polish_model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    pub fn llm_base_url(&self) -> String {
        self.raw
            .get("llm_base_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    /// RAW / CLEAN / ORGANIZE。未知或損壞值一律安全退回 RAW；缺值（舊設定）
    /// 採用新的產品預設 CLEAN。
    pub fn polish_mode(&self) -> PolishMode {
        match self.raw.get("polish_mode").and_then(Value::as_str) {
            None => PolishMode::Clean,
            Some(value) => PolishMode::from_str(value).unwrap_or_else(|_| {
                tracing::warn!("unknown polish_mode '{value}', falling back to raw");
                PolishMode::Raw
            }),
        }
    }

    /// 隱私預設：禁止自訂雲端 provider 發送任何請求。
    pub fn local_only(&self) -> bool {
        self.raw
            .get("local_only")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn organize_consent_valid(&self) -> bool {
        self.raw
            .get("organize_consent_version")
            .and_then(Value::as_u64)
            == Some(ORGANIZE_CONSENT_VERSION)
    }

    pub fn cloud_consent_valid(&self) -> bool {
        let version_valid = self
            .raw
            .get("cloud_consent_version")
            .and_then(Value::as_u64)
            == Some(CLOUD_CONSENT_VERSION);
        if !version_valid || self.llm_provider() != "custom" {
            return version_valid;
        }

        // loopback 不是雲端，本來就不需要 cloud consent。遠端自訂 API
        // 必須與當時同意的 origin 完全一致；更換 host/port/scheme 即失效。
        if matches!(
            crate::polish::custom_endpoint_is_loopback(&self.llm_base_url()),
            Ok(true)
        ) {
            return true;
        }
        let current_origin = crate::polish::custom_endpoint_origin(&self.llm_base_url());
        let consented_origin = self.raw.get("cloud_consent_origin").and_then(Value::as_str);
        current_origin
            .as_deref()
            .is_some_and(|origin| Some(origin) == consented_origin)
    }

    /// 個人字典（誤認詞 → 正確詞）。config 缺鍵時回預設字典。
    pub fn dictionary(&self) -> Vec<(String, String)> {
        self.raw
            .get("dictionary")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| {
                        let to = v.as_str()?.trim();
                        let from = k.trim();
                        (!from.is_empty() && !to.is_empty())
                            .then(|| (from.to_string(), to.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_else(crate::textproc::default_dict)
    }

    /// 螢幕上下文擷取開關（預設開；內容只在記憶體，永不落盤）
    pub fn context_enabled(&self) -> bool {
        self.raw
            .get("context_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn setup_completed(&self) -> bool {
        self.raw
            .get("setup_completed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn history_enabled(&self) -> bool {
        self.raw
            .get("history_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    /// 主熱鍵組合字串（handy-keys 格式）；缺省用預設
    pub fn hotkey_combo(&self) -> String {
        self.raw
            .get("hotkey")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(crate::hotkey::DEFAULT_COMBO)
            .to_string()
    }
}

/// 更新單一設定鍵並寫回（保留未知欄位、0600）。
pub fn update_config_key(path: &Path, key: &str, value: Value) -> anyhow::Result<()> {
    update_config_keys(path, vec![(key.to_string(), value)])
}

/// 全域寫入鎖：多個 Tauri command 併發 read-modify-write 會互相蓋寫
/// （改輸入裝置後馬上切上下文開關，後寫者可能帶著舊快照覆蓋前者）
static CONFIG_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 一次更新多個設定鍵、單次寫回——多鍵設定（如 LLM provider/model/base_url）
/// 必須原子成組寫入，分次寫會在連續操作時留下混搭的中間狀態。
pub fn update_config_keys(path: &Path, pairs: Vec<(String, Value)>) -> anyhow::Result<()> {
    let _g = CONFIG_WRITE_LOCK.lock().unwrap();
    let mut cfg = load_config(path);
    for (key, value) in pairs {
        cfg.insert(key, value);
    }
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
    let mut s = serde_json::to_string_pretty(&Value::Object(cfg))?;
    s.push('\n');
    f.write_all(s.as_bytes())?;
    Ok(())
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
        assert_eq!(
            cfg.get("setup_completed").and_then(Value::as_bool),
            Some(false)
        );
        assert!(path.exists());
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
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
        assert!(cfg
            .get("llm_model")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("Qwen"));
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

    // update_config_key：改一鍵、留其餘（含未知鍵）、權限 0600
    #[test]
    fn update_config_key_preserves_other_fields() {
        let dir = tempdir();
        let path = dir.join("config.json");
        fs::write(&path, r#"{"future_field": 42, "llm_enabled": false}"#).unwrap();
        update_config_key(
            &path,
            "input_device",
            serde_json::json!("MacBook Pro麥克風"),
        )
        .unwrap();
        let cfg = load_config(&path);
        assert_eq!(cfg.get("input_device").unwrap(), "MacBook Pro麥克風");
        assert_eq!(cfg.get("future_field").unwrap(), 42);
        assert_eq!(cfg.get("llm_enabled").unwrap(), false);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
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

    #[test]
    fn polish_mode_defaults_clean_but_unknown_is_safe_raw() {
        let dir = tempdir();
        let missing = dir.join("missing.json");
        let defaults = Settings::from_path(&missing);
        assert_eq!(defaults.polish_mode(), PolishMode::Clean);
        assert!(defaults.local_only());

        let invalid = dir.join("invalid-mode.json");
        fs::write(&invalid, r#"{"polish_mode":"surprise"}"#).unwrap();
        assert_eq!(Settings::from_path(&invalid).polish_mode(), PolishMode::Raw);
    }

    #[test]
    fn all_polish_modes_round_trip_and_consent_versions_are_exact() {
        for (raw, expected) in [
            ("raw", PolishMode::Raw),
            ("clean", PolishMode::Clean),
            ("organize", PolishMode::Organize),
        ] {
            assert_eq!(PolishMode::from_str(raw).unwrap(), expected);
            assert_eq!(expected.as_str(), raw);
        }

        let dir = tempdir();
        let path = dir.join("consent.json");
        fs::write(
            &path,
            format!(
                r#"{{"organize_consent_version":{},"cloud_consent_version":{}}}"#,
                ORGANIZE_CONSENT_VERSION, CLOUD_CONSENT_VERSION
            ),
        )
        .unwrap();
        let settings = Settings::from_path(&path);
        assert!(settings.organize_consent_valid());
        assert!(settings.cloud_consent_valid());
    }

    #[test]
    fn remote_cloud_consent_is_bound_to_the_exact_origin() {
        let dir = tempdir();
        let path = dir.join("cloud-consent.json");
        fs::write(
            &path,
            format!(
                r#"{{"llm_provider":"custom","llm_base_url":"https://api.example.com/v1","cloud_consent_version":{},"cloud_consent_origin":"https://api.example.com"}}"#,
                CLOUD_CONSENT_VERSION
            ),
        )
        .unwrap();
        assert!(Settings::from_path(&path).cloud_consent_valid());

        update_config_key(
            &path,
            "llm_base_url",
            Value::String("https://other.example.com/v1".into()),
        )
        .unwrap();
        assert!(!Settings::from_path(&path).cloud_consent_valid());
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
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::thread::current().id()
        )
        .replace(['(', ')', ' '], "")
    }
}
