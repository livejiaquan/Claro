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
///
/// 底層上限是 223 個 text token，且超限時**從左邊截斷**（保留尾端）；
/// attention 也讓尾端 token 權重最高。兩者都指向同一件事：最重要的詞要排在
/// 最後。因此這裡超出預算時同樣只從前面丟，呼叫端負責把高價值詞排在尾端。
pub fn build_initial_prompt(terms: &[String]) -> Option<String> {
    /// 留在 223 硬上限之下的保守預算，吸收估算誤差。
    const TOKEN_BUDGET: usize = 180;

    let mut used = 1; // 結尾的「。」
    let mut start = terms.len();
    for (i, term) in terms.iter().enumerate().rev() {
        let cost = approx_tokens(term) + 1; // 詞本身 + 分隔用的「、」
        if used + cost > TOKEN_BUDGET {
            break;
        }
        used += cost;
        start = i;
    }
    let kept = &terms[start..];
    (!kept.is_empty()).then(|| format!("{}。", kept.join("、")))
}

/// 保守估 Whisper token 數。
///
/// 係數取自實測（mlx_whisper multilingual tokenizer）：中文每字 0.84–0.94
/// token，而本函式面對的「頓號分隔詞表」格式最差為 1.21；ASCII 實測約 5.2
/// 字元一個 token。這裡取 CJK 1.5、ASCII 3，兩者都留有餘裕。
/// 寧可高估而少放幾個詞，也不要低估而讓 prompt 實際超過 223 硬限。
fn approx_tokens(text: &str) -> usize {
    let (mut cjk, mut ascii) = (0usize, 0usize);
    for c in text.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            cjk += 1;
        }
    }
    (cjk * 3).div_ceil(2) + ascii.div_ceil(3)
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

    /// 超出預算時只能從前面丟：底層也是左截斷，尾端才是高權重位置。
    #[test]
    fn prompt_drops_from_the_front_when_over_budget() {
        let mut terms: Vec<String> = (0..300).map(|i| format!("詞彙{i}")).collect();
        terms.push("MostImportant".into());
        let prompt = build_initial_prompt(&terms).expect("應該產生 prompt");
        assert!(prompt.ends_with("MostImportant。"), "尾端必須保留最重要的詞");
        assert!(!prompt.contains("詞彙0、"), "最前面的詞應該被丟掉");
        assert!(approx_tokens(&prompt) <= 223, "不可超過 Whisper 硬上限");
    }

    /// 預算內的詞一個都不能少。
    #[test]
    fn prompt_keeps_every_term_within_budget() {
        let terms: Vec<String> = (0..20).map(|i| format!("Term{i}")).collect();
        let prompt = build_initial_prompt(&terms).expect("應該產生 prompt");
        for t in &terms {
            assert!(prompt.contains(t.as_str()), "{t} 不該被丟掉");
        }
    }
}
