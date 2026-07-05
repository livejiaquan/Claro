//! 測試工具：對指定 OpenAI-compatible 端點跑一次保守糾錯。
//! 用法：CLARO_LLM_URL=http://127.0.0.1:8123/v1 CLARO_LLM_MODEL=mock cargo run --example test_polish

fn main() -> anyhow::Result<()> {
    let base = std::env::var("CLARO_LLM_URL").unwrap_or_else(|_| "http://localhost:11434/v1".into());
    let model = std::env::var("CLARO_LLM_MODEL").unwrap_or_else(|_| "qwen3:4b".into());

    // 直接組 Polisher（繞過 config/keychain），驗證 HTTP 與防呆路徑
    let cfg_json = serde_json::json!({
        "llm_provider": "custom",
        "llm_base_url": base,
        "llm_polish_model": model,
    });
    let dir = std::env::temp_dir().join(format!("claro-polish-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.json");
    std::fs::write(&path, cfg_json.to_string())?;

    let settings = claro_lib::settings::Settings::from_path(&path);
    let polisher = claro_lib::polish::from_settings(&settings).expect("polisher config");

    let input = "嗯我們用 hyTorch，那個，跑訓練";
    let out = polisher.polish(input, "PyTorch training")?;
    println!("in : {input}");
    println!("out: {out}");
    Ok(())
}
