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
pub const CORRECT_CONSENT_VERSION: u64 = 1;
// v2：同意由單純 custom origin 擴充成「provider + destination + auth + contract」
// 目標。舊版同意保留在檔案中但不自動套用到 Codex。
pub const CLOUD_CONSENT_VERSION: u64 = 2;
pub const CODEX_CONTEXT_CONSENT_VERSION: u64 = 1;
pub const CODEX_CORRECTION_PREFERENCES_MAX_CHARS: usize = 1_000;
const DICTIONARY_DEFAULTS_VERSION: u64 = 1;

fn default_stt_model() -> &'static str {
    crate::hardware::recommended_stt()
}

/// 聽寫後處理模式。模式與 LLM provider 分離：即使預設意圖是 Clean，
/// provider=off 時 pipeline 仍會安全退回 deterministic base text。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolishMode {
    Raw,
    #[default]
    Clean,
    Correct,
    Organize,
}

impl PolishMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Clean => "clean",
            Self::Correct => "correct",
            Self::Organize => "organize",
        }
    }
}

impl FromStr for PolishMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "raw" => Ok(Self::Raw),
            "clean" => Ok(Self::Clean),
            "correct" => Ok(Self::Correct),
            "organize" => Ok(Self::Organize),
            _ => Err(format!("未知的潤飾模式：{value}")),
        }
    }
}

pub fn default_config() -> Map<String, Value> {
    let stt_model = default_stt_model();
    let Value::Object(m) = json!({
        "whisper_model": stt_model,
        "llm_model": "mlx-community/Qwen2.5-7B-Instruct-4bit",
        "llm_enabled": true,
        // 輸出意圖預設 CLEAN；provider 仍預設 off，所以未選引擎時安全退回原文。
        "polish_mode": "clean",
        // 預設硬斷自訂雲端潤飾；只允許使用者經明確同意解除。
        "local_only": true,
        // CORRECT 可替換使用者授權的英文字詞，須獨立於格式整理與雲端傳送同意。
        "correct_consent_version": 0,
        "organize_consent_version": 0,
        "cloud_consent_version": 0,
        // 雲端同意綁定到當時確認的 scheme + authority；改端點後必須重新確認。
        "cloud_consent_origin": null,
        // v2 統一同意目標；Codex 還會綁登入類型與 runner contract 版本。
        "cloud_consent_target": null,
        // 必須完成權限、本次麥克風測試與模型檢查才由 UI 寫入。
        "setup_completed": false,
        // 本地聽寫歷史預設開啟；可由使用者關閉並清除。
        "history_enabled": true,
        // 主熱鍵（handy-keys 字串格式，如 "Opt+Shift+C"、"CmdRight"）
        "hotkey": "Opt+Shift+C",
        // 個人字典必須由使用者明確建立；通用預設替換可能改變合法縮寫的意思。
        "dictionary": {},
        "dictionary_defaults_version": DICTIONARY_DEFAULTS_VERSION,
        // 螢幕上下文（AX）：抓前景視窗詞彙給辨識與潤飾；內容永不落盤
        "context_enabled": true,
        // Codex CORRECT 的使用者偏好與額外螢幕詞彙分享皆預設關閉／空白。
        "codex_correction_preferences": "",
        "codex_share_context_terms": false,
        "codex_context_consent_version": 0,
        "codex_context_consent_target": null,
        "codex_auth_kind": null,
        // contract-v2 同時綁 raw version、launcher bytes 與完整 feature
        // capability；任一項更新都必須重新驗證與同意。
        "codex_cli_version": "",
        "codex_cli_raw_version": "",
        "codex_executable_sha256": "",
        "codex_capability_fingerprint": "",
        // 進階救援欄位；UI 不要求一般使用者自行找路徑。
        "codex_cli_path": "",
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
        Some(mut data) => {
            migrate_legacy_dictionary(&mut data);
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

/// 早期版本未經同意內建 `GBT→GPT`、`My Torch→PyTorch`。只有 dictionary
/// **完全等於**這組舊預設且尚無 migration marker 時才移除；任何新增、刪除或
/// 修改都視為使用者資料，原樣保留。回傳 map 會在下一次正常設定寫入時持久化。
fn migrate_legacy_dictionary(data: &mut Map<String, Value>) {
    if data
        .get("dictionary_defaults_version")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        >= DICTIONARY_DEFAULTS_VERSION
    {
        return;
    }
    let legacy = json!({ "GBT": "GPT", "My Torch": "PyTorch" });
    if data.get("dictionary") == Some(&legacy) {
        data.insert("dictionary".into(), json!({}));
        tracing::info!("removed unsafe legacy built-in dictionary defaults");
    }
    data.insert(
        "dictionary_defaults_version".into(),
        Value::from(DICTIONARY_DEFAULTS_VERSION),
    );
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
            .map(str::to_string)
            .unwrap_or_else(|| default_stt_model().to_string())
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

    /// RAW / CLEAN / CORRECT / ORGANIZE。未知或損壞值一律安全退回 RAW；
    /// 缺值（舊設定）採用新的產品預設 CLEAN。
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

    pub fn correct_consent_valid(&self) -> bool {
        self.raw
            .get("correct_consent_version")
            .and_then(Value::as_u64)
            == Some(CORRECT_CONSENT_VERSION)
    }

    pub fn cloud_consent_valid(&self) -> bool {
        let version_valid = self
            .raw
            .get("cloud_consent_version")
            .and_then(Value::as_u64)
            == Some(CLOUD_CONSENT_VERSION);
        if !version_valid {
            return false;
        }

        if self.llm_provider() == "codex" {
            return self.codex_consent_target().is_some_and(|current_target| {
                self.raw
                    .get("cloud_consent_target")
                    .and_then(Value::as_str)
                    .is_some_and(|target| target == current_target)
            });
        }

        let Some(current_target) = crate::polish::cloud_consent_target(self) else {
            return true;
        };
        if self
            .raw
            .get("cloud_consent_target")
            .and_then(Value::as_str)
            .is_some_and(|target| target == current_target)
        {
            return true;
        }

        // custom origin 是 v1 已有的精確目標，v2 過渡期仍接受「使用目前
        // CLOUD_CONSENT_VERSION 寫入的 exact origin」；Codex 沒有此後門。
        if self.llm_provider() == "custom" {
            let current_origin = crate::polish::custom_endpoint_origin(&self.llm_base_url());
            let consented_origin = self.raw.get("cloud_consent_origin").and_then(Value::as_str);
            return current_origin
                .as_deref()
                .is_some_and(|origin| Some(origin) == consented_origin);
        }
        false
    }

    /// 詞彙表：只餵給 STT 做解碼期偏置，不做任何字面替換。
    ///
    /// 之所以和 `dictionary` 分開，是因為兩者的安全條件相反。「Claude」適合當
    /// 偏置詞讓 Whisper 在聲學階段就認對；但把「Cloud → Claude」寫成替換規則，
    /// 會把使用者真的在講 cloud storage 的句子改壞。偏置無副作用、替換有。
    pub fn vocabulary(&self) -> Vec<String> {
        self.raw
            .get("vocabulary")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| {
                        let term = v.as_str()?.trim();
                        (!term.is_empty()).then(|| term.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// CORRECT 的使用者偏好。讀取時以 Unicode scalar value 截到固定上限，
    /// 避免舊版或手動修改的 config 把任意長文字送進後續 provider。
    pub fn codex_correction_preferences(&self) -> String {
        self.raw
            .get("codex_correction_preferences")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(CODEX_CORRECTION_PREFERENCES_MAX_CHARS)
            .collect()
    }

    /// 是否允許把本機抽取、清理且有界的候選詞另行提供給 Codex。
    /// 這只是資料範圍選項，不取代獨立的同意 gate。
    pub fn codex_share_context_terms(&self) -> bool {
        self.raw
            .get("codex_share_context_terms")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn codex_auth_mode(&self) -> Option<crate::codex::CodexAuthMode> {
        match self.raw.get("codex_auth_kind").and_then(Value::as_str) {
            Some("chat_gpt") => Some(crate::codex::CodexAuthMode::ChatGpt),
            Some("api_key") => Some(crate::codex::CodexAuthMode::ApiKey),
            _ => None,
        }
    }

    pub fn codex_cli_path(&self) -> Option<String> {
        self.raw
            .get("codex_cli_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    pub fn codex_cli_version(&self) -> Option<String> {
        self.raw
            .get("codex_cli_version")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    pub fn codex_cli_raw_version(&self) -> Option<String> {
        self.nonempty_string("codex_cli_raw_version")
    }

    pub fn codex_executable_sha256(&self) -> Option<String> {
        self.nonempty_string("codex_executable_sha256")
    }

    pub fn codex_capability_fingerprint(&self) -> Option<String> {
        self.nonempty_string("codex_capability_fingerprint")
    }

    fn nonempty_string(&self, key: &str) -> Option<String> {
        self.raw
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    pub fn codex_consent_target(&self) -> Option<String> {
        Some(crate::codex::contract_consent_target_values(
            self.codex_auth_mode()?,
            &self.codex_cli_version()?,
            &self.codex_cli_raw_version()?,
            &self.codex_cli_path()?,
            &self.codex_executable_sha256()?,
            &self.codex_capability_fingerprint()?,
        ))
    }

    pub fn codex_contract_matches(&self, contract: &crate::codex::CodexCapabilityContract) -> bool {
        self.codex_cli_path().as_deref() == Some(contract.executable_path.as_str())
            && self.codex_cli_version().as_deref() == Some(contract.version.as_str())
            && self.codex_cli_raw_version().as_deref() == Some(contract.raw_version.as_str())
            && self.codex_executable_sha256().as_deref()
                == Some(contract.executable_sha256.as_str())
            && self.codex_capability_fingerprint().as_deref()
                == Some(contract.capability_fingerprint.as_str())
    }

    pub fn codex_context_consent_valid(&self) -> bool {
        let Some(target) = self.codex_consent_target() else {
            return false;
        };
        let target = format!("{target}:context-terms-v1");
        self.raw
            .get("codex_context_consent_version")
            .and_then(Value::as_u64)
            == Some(CODEX_CONTEXT_CONSENT_VERSION)
            && self
                .raw
                .get("codex_context_consent_target")
                .and_then(Value::as_str)
                .is_some_and(|consented| consented == target)
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
///
/// 落盤走 temp＋rename（review 發現：原本直接 truncate 正式檔，讀者不持
/// 寫入鎖，truncate 到寫完之間讀到空檔/半份 JSON 會退回預設值——
/// context_enabled 之類的隱私選項會因此短暫「重新打開」；程序在該窗口
/// 被 _exit/斷電更會把損壞留到下次啟動）。rename 同目錄是原子的，
/// 讀者永遠只看到舊完整檔或新完整檔。
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
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        let mut s = serde_json::to_string_pretty(&Value::Object(cfg))?;
        s.push('\n');
        f.write_all(s.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
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
        assert_eq!(cfg.get("whisper_model").unwrap(), default_stt_model());
        assert_eq!(
            cfg.get("setup_completed").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(cfg.get("dictionary"), Some(&json!({})));
        assert_eq!(
            cfg.get("correct_consent_version").and_then(Value::as_u64),
            Some(0)
        );
        assert_eq!(
            cfg.get("codex_correction_preferences")
                .and_then(Value::as_str),
            Some("")
        );
        assert_eq!(
            cfg.get("codex_share_context_terms")
                .and_then(Value::as_bool),
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

    #[test]
    fn exact_legacy_builtin_dictionary_is_removed_but_custom_data_is_preserved() {
        let dir = tempdir();
        let legacy_path = dir.join("legacy.json");
        fs::write(
            &legacy_path,
            r#"{"dictionary":{"GBT":"GPT","My Torch":"PyTorch"}}"#,
        )
        .unwrap();
        let migrated = load_config(&legacy_path);
        assert_eq!(migrated.get("dictionary"), Some(&json!({})));
        assert_eq!(
            migrated
                .get("dictionary_defaults_version")
                .and_then(Value::as_u64),
            Some(DICTIONARY_DEFAULTS_VERSION)
        );

        let custom_path = dir.join("custom.json");
        fs::write(
            &custom_path,
            r#"{"dictionary":{"GBT":"GPT","My Torch":"PyTorch","克拉洛":"Claro"}}"#,
        )
        .unwrap();
        let preserved = load_config(&custom_path);
        assert_eq!(preserved["dictionary"]["克拉洛"], "Claro");
        assert_eq!(preserved["dictionary"]["GBT"], "GPT");
    }

    // 對應 test_corrupt_json_returns_defaults_without_exception
    #[test]
    fn corrupt_json_returns_defaults_without_exception() {
        let dir = tempdir();
        let path = dir.join("config.json");
        fs::write(&path, "{not json!!").unwrap();
        let cfg = load_config(&path);
        assert_eq!(cfg.get("whisper_model").unwrap(), default_stt_model());
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
        assert_eq!(cfg.get("whisper_model").unwrap(), default_stt_model());
    }

    // JSON root 不是 object → 退預設值
    #[test]
    fn non_object_root_returns_defaults() {
        let dir = tempdir();
        let path = dir.join("config.json");
        fs::write(&path, "[1,2,3]").unwrap();
        let cfg = load_config(&path);
        assert_eq!(cfg.get("whisper_model").unwrap(), default_stt_model());
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
            ("correct", PolishMode::Correct),
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
                r#"{{"correct_consent_version":{},"organize_consent_version":{},"cloud_consent_version":{}}}"#,
                CORRECT_CONSENT_VERSION, ORGANIZE_CONSENT_VERSION, CLOUD_CONSENT_VERSION
            ),
        )
        .unwrap();
        let settings = Settings::from_path(&path);
        assert!(settings.correct_consent_valid());
        assert!(settings.organize_consent_valid());
        assert!(settings.cloud_consent_valid());

        update_config_key(
            &path,
            "correct_consent_version",
            Value::from(CORRECT_CONSENT_VERSION + 1),
        )
        .unwrap();
        assert!(!Settings::from_path(&path).correct_consent_valid());
    }

    #[test]
    fn codex_correction_preferences_are_bounded_and_context_terms_default_off() {
        let dir = tempdir();
        let defaults = Settings::from_path(&dir.join("defaults.json"));
        assert_eq!(defaults.codex_correction_preferences(), "");
        assert!(!defaults.codex_share_context_terms());
        assert!(!defaults.correct_consent_valid());

        let path = dir.join("codex-preferences.json");
        let preferences = format!("{}🧪不應讀到", "a".repeat(999));
        fs::write(
            &path,
            json!({
                "codex_correction_preferences": preferences,
                "codex_share_context_terms": true,
            })
            .to_string(),
        )
        .unwrap();
        let settings = Settings::from_path(&path);
        let bounded = settings.codex_correction_preferences();
        assert_eq!(
            bounded.chars().count(),
            CODEX_CORRECTION_PREFERENCES_MAX_CHARS
        );
        assert!(bounded.ends_with('🧪'));
        assert!(!bounded.contains("不應讀到"));
        assert!(settings.codex_share_context_terms());
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

    #[test]
    fn codex_consent_is_bound_to_auth_contract_and_separate_context_scope() {
        let dir = tempdir();
        let path = dir.join("codex-consent.json");
        let chat_target = crate::codex::contract_consent_target_values(
            crate::codex::CodexAuthMode::ChatGpt,
            "0.145.0",
            "codex-cli 0.145.0",
            "/test/codex",
            "executable-hash",
            "capability-hash",
        );
        fs::write(
            &path,
            json!({
                "llm_provider": "codex",
                "local_only": false,
                "codex_auth_kind": "chat_gpt",
                "codex_cli_version": "0.145.0",
                "codex_cli_raw_version": "codex-cli 0.145.0",
                "codex_cli_path": "/test/codex",
                "codex_executable_sha256": "executable-hash",
                "codex_capability_fingerprint": "capability-hash",
                "cloud_consent_version": CLOUD_CONSENT_VERSION,
                "cloud_consent_target": chat_target,
                "codex_context_consent_version": CODEX_CONTEXT_CONSENT_VERSION,
                "codex_context_consent_target": format!("{chat_target}:context-terms-v1"),
            })
            .to_string(),
        )
        .unwrap();
        let settings = Settings::from_path(&path);
        assert!(settings.cloud_consent_valid());
        assert!(settings.codex_context_consent_valid());

        update_config_key(&path, "codex_cli_version", Value::String("0.146.0".into())).unwrap();
        let changed_version = Settings::from_path(&path);
        assert!(!changed_version.cloud_consent_valid());
        assert!(!changed_version.codex_context_consent_valid());

        update_config_key(&path, "codex_cli_version", Value::String("0.145.0".into())).unwrap();
        update_config_key(
            &path,
            "codex_capability_fingerprint",
            Value::String("changed-capability".into()),
        )
        .unwrap();
        let changed_capability = Settings::from_path(&path);
        assert!(!changed_capability.cloud_consent_valid());
        assert!(!changed_capability.codex_context_consent_valid());

        update_config_key(
            &path,
            "codex_capability_fingerprint",
            Value::String("capability-hash".into()),
        )
        .unwrap();
        update_config_key(
            &path,
            "codex_cli_path",
            Value::String("/different/codex".into()),
        )
        .unwrap();
        let changed_path = Settings::from_path(&path);
        assert!(!changed_path.cloud_consent_valid());
        assert!(!changed_path.codex_context_consent_valid());

        update_config_key(&path, "codex_cli_path", Value::String("/test/codex".into())).unwrap();
        update_config_key(&path, "codex_auth_kind", Value::String("api_key".into())).unwrap();
        let changed_auth = Settings::from_path(&path);
        assert!(!changed_auth.cloud_consent_valid());
        assert!(!changed_auth.codex_context_consent_valid());

        // 任何 custom provider 的 version/origin 同意都不可變成 Codex 同意。
        update_config_keys(
            &path,
            vec![
                ("codex_auth_kind".into(), "chat_gpt".into()),
                (
                    "cloud_consent_target".into(),
                    "custom:https://api.example.com".into(),
                ),
                (
                    "cloud_consent_origin".into(),
                    "https://api.example.com".into(),
                ),
            ],
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
