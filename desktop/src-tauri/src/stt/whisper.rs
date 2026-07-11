//! Whisper provider：transcribe-cpp（whisper.cpp，macOS Metal）。SPEC D5。

use std::path::PathBuf;
use std::sync::Once;

use anyhow::{Context, Result};
use transcribe_cpp::{Model, RunExtension, RunOptions, Session, Task, WhisperRunOptions};

use super::{SttEngine, SttRequest};

fn init_backends_once() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if let Err(e) = transcribe_cpp::init_backends_default() {
            tracing::warn!("transcribe-cpp backend init failed: {e}");
        }
    });
}

pub struct WhisperEngine {
    id: String,
    model_path: PathBuf,
    session: Option<Session>,
}

impl WhisperEngine {
    pub fn new(id: &str, model_path: PathBuf) -> Self {
        Self {
            id: id.to_string(),
            model_path,
            session: None,
        }
    }
}

impl SttEngine for WhisperEngine {
    fn id(&self) -> &str {
        &self.id
    }

    fn is_loaded(&self) -> bool {
        self.session.is_some()
    }

    fn load(&mut self) -> Result<()> {
        if self.session.is_some() {
            return Ok(());
        }
        let spec = super::registry::find(&self.id)
            .with_context(|| format!("unknown whisper model id '{}'", self.id))?;
        crate::models::verify_model_file(&self.model_path, spec.sha256).with_context(|| {
            format!(
                "verify whisper model {} ({})",
                spec.label,
                self.model_path.display()
            )
        })?;
        init_backends_once();
        let t0 = std::time::Instant::now();
        let model = Model::load(&self.model_path)
            .with_context(|| format!("load whisper model {}", self.model_path.display()))?;
        let session = model.session().context("create whisper session")?;
        tracing::info!(
            "whisper '{}' loaded in {:.1}s (backend: {})",
            self.id,
            t0.elapsed().as_secs_f32(),
            model.backend(),
        );
        self.session = Some(session);
        Ok(())
    }

    fn unload(&mut self) {
        self.session = None;
    }

    fn transcribe(&mut self, req: &SttRequest) -> Result<String> {
        self.load()?;
        let session = self.session.as_mut().expect("loaded above");

        let options = RunOptions {
            task: Task::Transcribe,
            language: req.language.map(str::to_string),
            family: Some(RunExtension::Whisper(WhisperRunOptions {
                initial_prompt: req.initial_prompt.clone(),
                // prototype 經驗：condition_on_previous_text=False 降低幻覺循環
                condition_on_prev_tokens: Some(false),
                ..Default::default()
            })),
            ..Default::default()
        };

        let transcript = session.run(req.audio, &options).context("whisper run")?;
        Ok(transcript.text.trim().to_string())
    }
}
