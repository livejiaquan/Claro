//! Codex provider 的固定合成 smoke。
//!
//! 不讀麥克風、畫面或使用者檔案；只送出下方硬編碼測試句。此測試會使用
//! 目前 Codex 登入與少量額度，必須由開發者明確執行，不會隨 app 或測試自動跑。

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use claro_lib::codex::{self, CodexAvailability, CodexRequest};

fn main() -> anyhow::Result<()> {
    let status = codex::probe(None);
    println!(
        "availability={:?} version={} auth={:?}",
        status.availability,
        status.version.as_deref().unwrap_or("unknown"),
        status.auth_mode
    );
    if status.availability != CodexAvailability::Ready {
        anyhow::bail!("Codex provider is not ready");
    }
    let auth_mode = status
        .auth_mode
        .ok_or_else(|| anyhow::anyhow!("Codex auth mode is unavailable"))?;
    let request = CodexRequest {
        transcript: "今天用 Py Torch 跑 training，版本是 2.7.1，不要升級到 3.0。".into(),
        context_terms: Vec::new(),
        vocabulary_terms: vec!["PyTorch".into()],
        canonical_spellings: Vec::new(),
        mode: "correct".into(),
    };
    let cancel = AtomicBool::new(false);
    let (_, policy_epoch) = codex::policy_snapshot(|| ());
    let output = codex::run(
        status.executable_path.as_deref(),
        &request,
        &cancel,
        auth_mode,
        policy_epoch,
        Duration::from_secs(20),
    )?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    if !output.text.contains("PyTorch")
        || output.text.contains("Py Torch")
        || !output.text.contains("2.7.1")
        || !output.text.contains("不要")
        || !output.text.contains("3.0")
    {
        anyhow::bail!("Codex output failed the synthetic anchor check");
    }
    Ok(())
}
