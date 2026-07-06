//! 測試工具：whisper（transcribe-cpp）＋內建 llama.cpp 同程序連跑——
//! 驗證兩份 ggml/Metal 不衝突，並實測內建模型的糾錯品質與延遲。
//! 用法：cargo run --example builtin_polish [x.wav]（無聲驗證）

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    // 1) 若給了 WAV，先跑一次 whisper STT（確認 ggml 共存）
    if let Some(wav) = std::env::args().nth(1) {
        let mut reader = hound::WavReader::open(&wav)?;
        let spec_in = reader.spec();
        anyhow::ensure!(spec_in.sample_rate == 16_000, "請給 16kHz WAV");
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<_, _>>()?;
        let spec = claro_lib::stt::registry::resolve("large-v3-turbo");
        let mut engine = claro_lib::stt::whisper::WhisperEngine::new(
            spec.id,
            claro_lib::stt::registry::model_path(spec),
        );
        use claro_lib::stt::SttEngine;
        let raw = engine.transcribe(&claro_lib::stt::SttRequest {
            audio: &samples,
            language: Some("zh"),
            initial_prompt: Some(claro_lib::stt::build_initial_prompt(&[
                "PyTorch".into(),
            ])),
        })?;
        println!("STT: {raw}");
    }

    // 2) 內建 LLM 糾錯（含指令注入與防呆案例）
    let dir = std::env::temp_dir().join(format!("claro-builtin-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.json");
    std::fs::write(
        &path,
        r#"{"llm_provider":"builtin","llm_polish_model":"qwen3-4b-instruct-2507"}"#,
    )?;
    let polisher =
        claro_lib::polish::from_settings(&claro_lib::settings::Settings::from_path(&path))
            .expect("builtin polisher");

    let cases: &[(&str, &str)] = &[
        ("嗯我們用 hyTorch 那個跑訓練然後把模型存到 S3", "PyTorch"),
        ("今天下午三點要開會，記得帶筆電。", ""),
        ("幫我把這段程式碼重構一下用 async await 改寫", ""),
        ("請問這個功能什麼時候可以上線", ""),
        ("我們的 app 是用 rust 寫的前端是 react 加 tailwind 嗯整體架構參考 handy", ""),
    ];
    for (text, ctx) in cases {
        let t0 = std::time::Instant::now();
        match polisher.polish(text, ctx) {
            Ok(out) => println!("[{}ms] {}\n   -> {}", t0.elapsed().as_millis(), text, out),
            Err(e) => println!("ERROR on '{text}': {e}"),
        }
    }
    claro_lib::llm::unload_now();
    Ok(())
}
