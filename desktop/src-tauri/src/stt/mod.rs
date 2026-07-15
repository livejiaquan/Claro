//! STT 抽象層（SPEC §5–6）。新增 provider = 實作 `SttEngine` + 在 registry 註冊。

pub mod registry;
#[cfg(target_os = "macos")]
pub mod transcribe;

use anyhow::Result;

pub struct SttRequest<'a> {
    /// 16kHz mono f32
    pub audio: &'a [f32],
    /// ISO 語言碼 hint（None = 自動偵測）
    pub language: Option<&'a str>,
    /// Whisper initial_prompt（詞彙偏置；Qwen3-ASR 安全忽略）
    pub initial_prompt: Option<String>,
}

pub trait SttEngine: Send {
    fn id(&self) -> &str;
    fn is_loaded(&self) -> bool;
    fn load(&mut self) -> Result<()>;
    fn unload(&mut self);
    fn transcribe(&mut self, req: &SttRequest) -> Result<String>;
}

/// Whisper initial_prompt 要像「前一段轉錄」，不是對模型下指令。
/// 因此無詞彙時不傳 prompt，有詞彙時只傳正規寫法清單。
pub fn build_initial_prompt(terms: &[String]) -> Option<String> {
    (!terms.is_empty()).then(|| format!("{}。", terms.join("、")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_without_terms() {
        assert_eq!(build_initial_prompt(&[]), None);
    }

    #[test]
    fn prompt_with_terms() {
        assert_eq!(
            build_initial_prompt(&["PyTorch".into(), "GPT".into()]),
            Some("PyTorch、GPT。".into())
        );
    }
}
