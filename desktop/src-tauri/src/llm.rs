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
    pub sha256: &'static str,
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
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/a06e946bb6b655725eafa393f4a9745d460374c9/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
        approx_bytes: 2_497_281_120,
        recommended: true,
    },
    LlmModelSpec {
        id: "gemma-3-4b-it",
        label: "Gemma 3 4B",
        desc: "Google 多語模型，另一種校正風格（Q4 量化）",
        file: "google_gemma-3-4b-it-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/71506238f970075ca85125cd749c28b1b0eee84e/google_gemma-3-4b-it-Q4_K_M.gguf",
        sha256: "4996030242583a40aa151ff93f49ed787ac8c25e4120c3ae4588b2e2a7d1ae94",
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

/// 只有內容通過 registry 內固定的 SHA-256 才能視為已下載。
pub fn model_is_verified(spec: &LlmModelSpec) -> bool {
    crate::models::model_file_is_verified(&model_path(spec), spec.sha256)
}

fn require_complete_generation(reached_eog: bool) -> Result<()> {
    if !reached_eog {
        bail!("內建模型輸出達 token 上限，拒絕使用可能截斷的內容");
    }
    Ok(())
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn builtin_models_are_pinned_and_hashed() {
        for model in LLM_MODELS {
            assert!(model.url.starts_with("https://huggingface.co/"));
            assert!(model.url.ends_with(model.file));
            assert!(!model.url.contains("/resolve/main/"));
            assert_eq!(model.sha256.len(), 64);
            assert!(model.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }
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
    const MAX_GENERATION_TIME: Duration = Duration::from_secs(6);
    const MAX_TOTAL_TIME: Duration = Duration::from_secs(8);

    struct Loaded {
        id: String,
        model: LlamaModel,
        last_used: Instant,
    }

    static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
    static STATE: Mutex<Option<Loaded>> = Mutex::new(None);
    static WATCHER: Once = Once::new();

    fn ensure_total_budget(
        cancel: &std::sync::atomic::AtomicBool,
        total_deadline: Instant,
    ) -> Result<()> {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            bail!("內建整理已取消");
        }
        if Instant::now() >= total_deadline {
            bail!("內建整理準備逾時；模型已保留供下一次使用");
        }
        Ok(())
    }

    /// 生成最多六秒，但不可超過整次內建整理的總額度。
    fn start_generation_deadline(
        cancel: &std::sync::atomic::AtomicBool,
        timeout: Duration,
        total_deadline: Instant,
    ) -> Result<Instant> {
        ensure_total_budget(cancel, total_deadline)?;
        Ok((Instant::now() + timeout).min(total_deadline))
    }

    fn backend() -> Result<&'static LlamaBackend> {
        if let Some(b) = BACKEND.get() {
            return Ok(b);
        }
        // 初始化鎖：首次並發呼叫（聽寫潤飾＋設定頁測試同時）不能重複 init
        static INIT_LOCK: Mutex<()> = Mutex::new(());
        let _g = INIT_LOCK.lock().unwrap();
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

    /// 立即卸載（app 退出路徑用：Metal 資源不能留給 atexit teardown）。
    /// 回傳 false = 鎖被占（生成中），呼叫端要重試或走 _exit 跳過 atexit。
    pub fn unload_now() -> bool {
        match STATE.try_lock() {
            Ok(mut guard) => {
                *guard = None;
                true
            }
            Err(_) => false,
        }
    }

    /// 低記憶體模型交接用。只從 pipeline 的背景執行緒呼叫；等待正在收尾的
    /// watcher／生成鎖後完整 drop LLM，再載入 STT，避免兩個 Metal 模型重疊。
    pub fn unload_blocking() {
        let mut guard = STATE.lock().unwrap();
        *guard = None;
    }

    /// 用內建模型跑一次 chat completion（greedy，最保守）。
    pub fn generate_with_cancel(
        model_id: &str,
        system: &str,
        user: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<String> {
        let total_deadline = Instant::now() + MAX_TOTAL_TIME;
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            bail!("內建整理已取消");
        }
        let spec = find(model_id).with_context(|| format!("未知的內建模型 {model_id}"))?;
        let path = model_path(spec);
        crate::models::verify_model_file(&path, spec.sha256)
            .with_context(|| format!("模型尚未下載或完整性驗證失敗：{}", spec.label))?;
        ensure_total_budget(cancel, total_deadline)?;

        let backend = backend()?;
        ensure_total_budget(cancel, total_deadline)?;
        let mut guard = STATE
            .try_lock()
            .map_err(|_| anyhow::anyhow!("內建整理正在處理另一個要求，安全退回原文"))?;

        // 換模型先 drop 舊的（避免雙駐留峰值），再載新的
        if guard.as_ref().map(|l| l.id.as_str()) != Some(model_id) {
            *guard = None;
            // 收尾旗標要在拿到 STATE 鎖之後檢查：unload_now/exit 只觀察這把鎖，
            // 冷生成在 hash/初始化階段不持鎖，先前會在卸載判定後又把模型載回
            if crate::SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("app shutting down — refusing to load builtin model");
            }
            let t0 = Instant::now();
            let params = LlamaModelParams::default().with_n_gpu_layers(u32::MAX); // 全層 Metal
            let model = LlamaModel::load_from_file(backend, &path, &params)
                .with_context(|| format!("load {}", path.display()))?;
            tracing::info!(
                "builtin llm '{}' loaded in {:.1}s",
                spec.id,
                t0.elapsed().as_secs_f32()
            );
            *guard = Some(Loaded {
                id: model_id.to_string(),
                model,
                last_used: Instant::now(),
            });
            spawn_watcher();
        }

        // Metal 冷載本身不可安全中斷；若已吃完總額度，模型仍保留在 STATE，
        // 這一段立即 fallback，下一段 warm call 不再支付冷載成本。
        let deadline = start_generation_deadline(cancel, MAX_GENERATION_TIME, total_deadline)?;
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
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            bail!("內建整理已取消");
        }
        if Instant::now() >= deadline {
            bail!("內建整理逾時");
        }

        // greedy：糾錯任務要最保守、可重現的輸出。
        // 注意：CJK 一個字常拆成多個 token（半個 UTF-8 序列），
        // 必須整串收完再一次轉字串——逐 token 轉會把拆開的字靜默丟掉（實測踩過）。
        let mut sampler = LlamaSampler::greedy();
        let mut generated: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();
        let mut n_cur = tokens.len() as i32;
        let mut reached_eog = false;
        for _ in 0..MAX_OUTPUT_TOKENS {
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                bail!("內建整理已取消");
            }
            if Instant::now() >= deadline {
                bail!("內建整理逾時");
            }
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                reached_eog = true;
                break;
            }
            generated.push(token);
            batch.clear();
            batch.add(token, n_cur, &[0], true)?;
            n_cur += 1;
            ctx.decode(&mut batch).context("decode step")?;
        }
        require_complete_generation(reached_eog)?;
        // llama-cpp-2 的 tokens_to_str 對每個 token 只給固定 8 bytes 緩衝且不重試，
        // 只要有一個 token 超過 8 bytes 就整串失敗。Qwen 詞彙表常把多個中文字併成
        // 一個 token（3 個中文字就是 9 bytes），因此中文輸出幾乎必然踩雷——實測
        // 使用者的聽寫穩定回報 `detokenize output`。改用會自動擴充緩衝的
        // token_to_bytes 逐 token 取「位元組」，全部累積完再一次轉字串：
        // 仍是整串收完才 decode，不會把拆成多 token 的 CJK 字丟掉。
        let mut bytes: Vec<u8> = Vec::with_capacity(generated.len() * 4);
        for token in &generated {
            bytes.extend_from_slice(
                &model
                    .token_to_bytes(*token, Special::Plaintext)
                    .context("detokenize output")?,
            );
        }
        let out = String::from_utf8(bytes).context("detokenize output")?;
        Ok(out.trim().to_string())
    }

    pub fn generate(model_id: &str, system: &str, user: &str) -> Result<String> {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        generate_with_cancel(model_id, system, user, &cancel)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicBool, Ordering};

        #[test]
        fn generation_budget_is_capped_by_total_deadline() {
            let cancel = AtomicBool::new(false);
            let timeout = Duration::from_millis(50);
            let total_deadline = Instant::now() + Duration::from_millis(20);

            let deadline = start_generation_deadline(&cancel, timeout, total_deadline)
                .expect("total budget is still available");

            assert!(deadline <= total_deadline);
        }

        #[test]
        fn cancellation_after_model_preparation_stops_before_generation() {
            let cancel = AtomicBool::new(false);
            cancel.store(true, Ordering::SeqCst);
            let error = start_generation_deadline(
                &cancel,
                MAX_GENERATION_TIME,
                Instant::now() + MAX_TOTAL_TIME,
            )
            .expect_err("cancellation after preparation must stop generation");

            assert!(error.to_string().contains("取消"));
        }

        #[test]
        fn expired_total_budget_stops_before_generation() {
            let cancel = AtomicBool::new(false);
            let error = start_generation_deadline(
                &cancel,
                MAX_GENERATION_TIME,
                Instant::now() - Duration::from_millis(1),
            )
            .expect_err("expired preparation budget must fallback");

            assert!(error.to_string().contains("準備逾時"));
        }
    }
}

#[cfg(target_os = "macos")]
pub use engine::{generate, generate_with_cancel, unload_blocking, unload_now};

#[cfg(not(target_os = "macos"))]
pub fn generate(_model_id: &str, _system: &str, _user: &str) -> Result<String> {
    bail!("內建模型目前只支援 macOS")
}

#[cfg(not(target_os = "macos"))]
pub fn generate_with_cancel(
    _model_id: &str,
    _system: &str,
    _user: &str,
    _cancel: &std::sync::atomic::AtomicBool,
) -> Result<String> {
    bail!("內建模型目前只支援 macOS")
}

#[cfg(not(target_os = "macos"))]
pub fn unload_now() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn unload_blocking() {}

#[cfg(test)]
mod generation_stop_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn token_limit_without_eog_is_rejected() {
        assert!(require_complete_generation(false).is_err());
        assert!(require_complete_generation(true).is_ok());
    }

    #[test]
    fn cancelled_generation_stops_before_model_lookup_or_load() {
        let cancel = AtomicBool::new(true);
        let error = generate_with_cancel("missing-model", "system", "user", &cancel)
            .expect_err("cancelled generation must stop");
        assert!(error.to_string().contains("取消"));
    }
}
