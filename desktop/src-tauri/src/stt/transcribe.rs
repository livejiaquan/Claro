//! transcribe.cpp provider：Whisper 與 Qwen3-ASR 共用 GGUF/Metal runtime。

use std::path::PathBuf;
use std::sync::Once;

use anyhow::{Context, Result};
use transcribe_cpp::{Model, RunExtension, RunOptions, Session, Task, WhisperRunOptions};

use super::registry::{ModelFamily, ModelSpec};
use super::{SttEngine, SttRequest};

fn init_backends_once() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if let Err(e) = transcribe_cpp::init_backends_default() {
            tracing::warn!("transcribe-cpp backend init failed: {e}");
        }
    });
}

pub struct TranscribeEngine {
    spec: &'static ModelSpec,
    model_path: PathBuf,
    session: Option<Session>,
}

impl TranscribeEngine {
    pub fn new(spec: &'static ModelSpec, model_path: PathBuf) -> Self {
        Self {
            spec,
            model_path,
            session: None,
        }
    }

    fn run_options(&self, req: &SttRequest<'_>) -> RunOptions {
        match self.spec.family {
            ModelFamily::Whisper => RunOptions {
                task: Task::Transcribe,
                language: req.language.map(str::to_string),
                family: Some(RunExtension::Whisper(WhisperRunOptions {
                    initial_prompt: req.initial_prompt.clone(),
                    // prototype 經驗：condition_on_previous_text=False 降低幻覺循環
                    condition_on_prev_tokens: Some(false),
                    ..Default::default()
                })),
                ..Default::default()
            },
            // Claro 鎖定的 transcribe.cpp 0.1.1 可接受 BCP-47 language hint，
            // 但沒有 Whisper initial_prompt extension；預設仍由 pipeline 傳 auto。
            ModelFamily::Qwen3Asr => RunOptions {
                task: Task::Transcribe,
                language: req.language.map(str::to_string),
                family: None,
                ..Default::default()
            },
        }
    }
}

impl SttEngine for TranscribeEngine {
    fn id(&self) -> &str {
        self.spec.id
    }

    fn is_loaded(&self) -> bool {
        self.session.is_some()
    }

    fn load(&mut self) -> Result<()> {
        if self.session.is_some() {
            return Ok(());
        }
        crate::models::verify_model_file(&self.model_path, self.spec.sha256).with_context(
            || {
                format!(
                    "verify STT model {} ({})",
                    self.spec.label,
                    self.model_path.display()
                )
            },
        )?;
        init_backends_once();
        let t0 = std::time::Instant::now();
        let model = Model::load(&self.model_path)
            .with_context(|| format!("load STT model {}", self.model_path.display()))?;
        let backend = model.backend().to_string();
        let session = model.session().context("create STT session")?;
        tracing::info!(
            "STT '{}' ({:?}) loaded in {:.1}s (backend: {})",
            self.spec.id,
            self.spec.family,
            t0.elapsed().as_secs_f32(),
            backend,
        );
        self.session = Some(session);
        Ok(())
    }

    fn unload(&mut self) {
        self.session = None;
    }

    fn transcribe(&mut self, req: &SttRequest<'_>) -> Result<String> {
        self.load()?;
        // 先建 options 再借用 session，避免同時對 self 做 immutable/mutable borrow。
        let options = self.run_options(req);
        let session = self.session.as_mut().expect("loaded above");
        let transcript = session
            .run(req.audio, &options)
            .with_context(|| format!("{} transcription run", self.spec.label))?;
        Ok(transcript.text.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::registry;

    fn request<'a>(audio: &'a [f32]) -> SttRequest<'a> {
        SttRequest {
            audio,
            language: Some("zh"),
            initial_prompt: Some("Claro、PyTorch。".into()),
        }
    }

    #[test]
    fn whisper_keeps_language_and_prompt_extension() {
        let spec = registry::find("large-v3-turbo").unwrap();
        let engine = TranscribeEngine::new(spec, registry::model_path(spec));
        let audio = [];
        let options = engine.run_options(&request(&audio));
        assert_eq!(options.language.as_deref(), Some("zh"));
        match options.family {
            Some(RunExtension::Whisper(options)) => {
                assert_eq!(options.initial_prompt.as_deref(), Some("Claro、PyTorch。"));
                assert_eq!(options.condition_on_prev_tokens, Some(false));
            }
            _ => panic!("Whisper model must use Whisper run extension"),
        }
    }

    #[test]
    fn qwen_keeps_supported_language_hint_but_ignores_whisper_prompt() {
        let spec = registry::find("qwen3-asr-1.7b-q8_0").unwrap();
        let engine = TranscribeEngine::new(spec, registry::model_path(spec));
        let audio = [];
        let options = engine.run_options(&request(&audio));
        assert_eq!(options.language.as_deref(), Some("zh"));
        assert!(options.family.is_none());
    }
}
