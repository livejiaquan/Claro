//! 內建 LLM（llama.cpp in-process，Metal）——不裝任何外部軟體的高精度潤飾。
//! SPEC D3：macOS 26 以下或未啟用 Apple Intelligence 的「零安裝」路徑，
//! 也是想要比 Apple 端上 3B 模型更高精度的使用者選項。
//!
//! 生命週期沿用 Handy 實證模式（docs/research/llm-polish-runtime.md）：
//! 用時載入 → 閒置 watcher（10s 巡檢，5 分鐘未用卸載）→ 換模型先 drop 再載。
//! 模型檔一律經使用者在 UI 明確點擊下載（絕不自動下載）。

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

pub struct LlmModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub approx_bytes: u64,
    pub recommended: bool,
}

/// 內建模型目錄。4B 級起跳——prototype 實測 1.5B 級會洩漏提示詞、亂加列表。
pub const LLM_MODELS: &[LlmModelSpec] = &[
    LlmModelSpec {
        id: "qwen3-4b-instruct-2507",
        label: "Qwen3 4B Instruct",
        desc: "中英混排最穩，繁中品質佳（Q4 量化）",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        approx_bytes: 2_497_281_120,
        recommended: true,
    },
    LlmModelSpec {
        id: "gemma-3-4b-it",
        label: "Gemma 3 4B",
        desc: "Google 多語模型，另一種校正風格（Q4 量化）",
        file: "google_gemma-3-4b-it-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/main/google_gemma-3-4b-it-Q4_K_M.gguf",
        approx_bytes: 2_489_758_112,
        recommended: false,
    },
];

pub fn find(id: &str) -> Option<&'static LlmModelSpec> {
    LLM_MODELS.iter().find(|m| m.id == id)
}

/// 模型檔目錄：~/Library/Application Support/Claro/models/llm/
pub fn models_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Claro/models/llm")
}

pub fn model_path(spec: &LlmModelSpec) -> PathBuf {
    models_dir().join(spec.file)
}

// ─── 引擎（macOS/Metal） ─────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod engine {
    use super::*;
    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel, Special};
    use llama_cpp_2::sampling::LlamaSampler;

    const IDLE_UNLOAD: Duration = Duration::from_secs(300); // Handy 預設 5 分鐘
    const MAX_OUTPUT_TOKENS: usize = 512;
    const N_CTX: u32 = 4096;

    struct Loaded {
        id: String,
        model: LlamaModel,
        last_used: Instant,
    }

    static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
    static STATE: Mutex<Option<Loaded>> = Mutex::new(None);
    static WATCHER: Once = Once::new();

    fn backend() -> Result<&'static LlamaBackend> {
        if BACKEND.get().is_none() {
            let b = LlamaBackend::init().context("init llama backend")?;
            let _ = BACKEND.set(b);
        }
        Ok(BACKEND.get().expect("set above"))
    }

    fn spawn_watcher() {
        WATCHER.call_once(|| {
            std::thread::spawn(|| loop {
                std::thread::sleep(Duration::from_secs(10));
                let mut guard = STATE.lock().unwrap();
                if let Some(loaded) = guard.as_ref() {
                    if loaded.last_used.elapsed() >= IDLE_UNLOAD {
                        tracing::info!("builtin llm '{}' idle — unloading", loaded.id);
                        *guard = None; // drop 釋放 Metal 資源
                    }
                }
            });
        });
    }

    /// 立即卸載（app 退出路徑用：Metal 資源不能留給 atexit teardown）
    pub fn unload_now() {
        if let Ok(mut guard) = STATE.try_lock() {
            *guard = None;
        }
    }

    /// 用內建模型跑一次 chat completion（greedy，最保守）。
    pub fn generate(model_id: &str, system: &str, user: &str) -> Result<String> {
        let spec = find(model_id).with_context(|| format!("未知的內建模型 {model_id}"))?;
        let path = model_path(spec);
        if !path.exists() {
            bail!("模型尚未下載：{}", spec.label);
        }

        let backend = backend()?;
        let mut guard = STATE.lock().unwrap();

        // 換模型先 drop 舊的（避免雙駐留峰值），再載新的
        if guard.as_ref().map(|l| l.id.as_str()) != Some(model_id) {
            *guard = None;
            let t0 = Instant::now();
            let params = LlamaModelParams::default().with_n_gpu_layers(u32::MAX); // 全層 Metal
            let model = LlamaModel::load_from_file(backend, &path, &params)
                .with_context(|| format!("load {}", path.display()))?;
            tracing::info!(
                "builtin llm '{}' loaded in {:.1}s",
                spec.id,
                t0.elapsed().as_secs_f32()
            );
            *guard = Some(Loaded { id: model_id.to_string(), model, last_used: Instant::now() });
            spawn_watcher();
        }
        let loaded = guard.as_mut().expect("loaded above");
        loaded.last_used = Instant::now();
        let model = &loaded.model;

        // chat template（GGUF 內建；Qwen3/Gemma3 都有）
        let msgs = vec![
            LlamaChatMessage::new("system".into(), system.into())?,
            LlamaChatMessage::new("user".into(), user.into())?,
        ];
        let template = model.chat_template(None).context("model chat template")?;
        let prompt = model
            .apply_chat_template(&template, &msgs, true)
            .context("apply chat template")?;

        let tokens = model
            .str_to_token(&prompt, AddBos::Never) // template 已含 BOS 標記
            .context("tokenize prompt")?;
        if tokens.len() as u32 > N_CTX - MAX_OUTPUT_TOKENS as u32 {
            bail!("prompt 過長（{} tokens）", tokens.len());
        }

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(N_CTX))
            .with_n_batch(N_CTX);
        let mut ctx = model
            .new_context(backend, ctx_params)
            .context("create llama context")?;

        let mut batch = LlamaBatch::new(N_CTX as usize, 1);
        let last = tokens.len() - 1;
        for (i, t) in tokens.iter().enumerate() {
            batch.add(*t, i as i32, &[0], i == last)?;
        }
        ctx.decode(&mut batch).context("decode prompt")?;

        // greedy：糾錯任務要最保守、可重現的輸出。
        // 注意：CJK 一個字常拆成多個 token（半個 UTF-8 序列），
        // 必須整串收完再一次轉字串——逐 token 轉會把拆開的字靜默丟掉（實測踩過）。
        let mut sampler = LlamaSampler::greedy();
        let mut generated: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();
        let mut n_cur = tokens.len() as i32;
        for _ in 0..MAX_OUTPUT_TOKENS {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }
            generated.push(token);
            batch.clear();
            batch.add(token, n_cur, &[0], true)?;
            n_cur += 1;
            ctx.decode(&mut batch).context("decode step")?;
        }
        let out = model
            .tokens_to_str(&generated, Special::Plaintext)
            .context("detokenize output")?;
        Ok(out.trim().to_string())
    }
}

#[cfg(target_os = "macos")]
pub use engine::{generate, unload_now};

#[cfg(not(target_os = "macos"))]
pub fn generate(_model_id: &str, _system: &str, _user: &str) -> Result<String> {
    bail!("內建模型目前只支援 macOS")
}

#[cfg(not(target_os = "macos"))]
pub fn unload_now() {}
