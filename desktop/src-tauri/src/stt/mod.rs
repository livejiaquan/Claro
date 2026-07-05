//! STT 抽象層（SPEC §5–6）。新增 provider = 實作 `SttEngine` + 在 registry 註冊。

pub mod registry;
#[cfg(target_os = "macos")]
pub mod whisper;

use anyhow::Result;

pub struct SttRequest<'a> {
    /// 16kHz mono f32
    pub audio: &'a [f32],
    /// ISO 語言碼 hint（None = 自動偵測）
    pub language: Option<&'a str>,
    /// Whisper initial_prompt（繁中模板＋詞彙偏置；M3 起帶上下文詞彙）
    pub initial_prompt: Option<String>,
}

pub trait SttEngine: Send {
    fn id(&self) -> &str;
    fn is_loaded(&self) -> bool;
    fn load(&mut self) -> Result<()>;
    fn unload(&mut self);
    fn transcribe(&mut self, req: &SttRequest) -> Result<String>;
}

/// 預設的繁中 initial_prompt（移植 prototype `_transcribe`；
/// terms 為詞彙偏置，M1 只有個人字典值，M3 加上下文詞彙）
pub fn build_initial_prompt(terms: &[String]) -> String {
    let hint = if terms.is_empty() {
        String::new()
    } else {
        format!("，可能提到：{}", terms.join("、"))
    };
    format!("以下是繁體中文為主、夾雜英文技術術語的口語內容{hint}。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_without_terms() {
        assert_eq!(
            build_initial_prompt(&[]),
            "以下是繁體中文為主、夾雜英文技術術語的口語內容。"
        );
    }

    #[test]
    fn prompt_with_terms() {
        let p = build_initial_prompt(&["PyTorch".into(), "GPT".into()]);
        assert!(p.contains("可能提到：PyTorch、GPT"));
    }
}
