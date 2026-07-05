//! 確定性文字處理（SPEC §8「不靠 LLM 的部分」），自 prototype/main.py 移植：
//! `_clean_transcript`（CJK 空白清理）、`_apply_dict`（個人字典）、
//! `_to_traditional`（OpenCC s2twp 終盤繁化，ferrous-opencc 純 Rust，SPEC D6）。

use std::sync::OnceLock;

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

/// CJK 標點與全形符號區（對應 prototype 的 　-〿、＀-￯、 -⁯）
fn is_cjk_punct(c: char) -> bool {
    ('\u{3000}'..='\u{303f}').contains(&c)
        || ('\u{ff00}'..='\u{ffef}').contains(&c)
        || ('\u{2000}'..='\u{206f}').contains(&c)
}

/// 移植 `_clean_transcript` 的四條規則（原版用 lookaround regex，這裡手寫等價邏輯）：
/// 1. 兩個 CJK 字之間的空白 → 刪
/// 2. CJK 標點前的空白 → 刪
/// 3. CJK 標點後的空白 → 刪
/// 4. 連續空白 → 一個；頭尾 trim
pub fn clean_transcript(text: &str) -> String {
    let chars: Vec<char> = text.trim().chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != ' ' {
            out.push(c);
            i += 1;
            continue;
        }
        // 一段連續空白：找前後的非空白字元決定去留
        let mut j = i;
        while j < chars.len() && chars[j] == ' ' {
            j += 1;
        }
        let prev = out.chars().last();
        let next = chars.get(j).copied();
        let drop_space = match (prev, next) {
            (Some(p), Some(n)) => {
                (is_cjk(p) && is_cjk(n)) || is_cjk_punct(n) || is_cjk_punct(p)
            }
            _ => true, // 頭尾空白
        };
        if !drop_space {
            out.push(' ');
        }
        i = j;
    }
    out
}

/// 個人字典替換：原樣與全小寫兩態（移植 `_apply_dict`）。
/// M1 沿用 prototype 的內建預設；設定 UI 與持久化在 M2。
pub fn apply_dict(text: &str, dict: &[(String, String)]) -> String {
    let mut t = text.to_string();
    for (wrong, right) in dict {
        t = t.replace(wrong.as_str(), right);
        t = t.replace(&wrong.to_lowercase(), &right.to_lowercase());
    }
    t
}

pub fn default_dict() -> Vec<(String, String)> {
    vec![
        ("GBT".into(), "GPT".into()),
        ("My Torch".into(), "PyTorch".into()),
    ]
}

/// OpenCC s2twp：確定性簡轉繁（台灣用語）。初始化失敗時原樣返回（不阻擋聽寫）。
pub fn to_traditional(text: &str) -> String {
    static OPENCC: OnceLock<Option<ferrous_opencc::OpenCC>> = OnceLock::new();
    let cc = OPENCC.get_or_init(|| {
        ferrous_opencc::OpenCC::from_config(ferrous_opencc::config::BuiltinConfig::S2twp)
            .map_err(|e| tracing::warn!("opencc init failed: {e}"))
            .ok()
    });
    match cc {
        Some(cc) => cc.convert(text),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_removes_spaces_between_cjk() {
        assert_eq!(clean_transcript("你好 世界"), "你好世界");
        assert_eq!(clean_transcript("你好  世界  再見"), "你好世界再見");
    }

    #[test]
    fn clean_keeps_spaces_around_latin() {
        assert_eq!(clean_transcript("用 PyTorch 跑 訓練"), "用 PyTorch 跑訓練");
        assert_eq!(clean_transcript("run the test"), "run the test");
    }

    #[test]
    fn clean_removes_spaces_around_cjk_punct() {
        assert_eq!(clean_transcript("你好 ，世界"), "你好，世界");
        assert_eq!(clean_transcript("你好， 世界"), "你好，世界");
        assert_eq!(clean_transcript("好。 然後"), "好。然後");
    }

    #[test]
    fn clean_collapses_spaces_and_trims() {
        assert_eq!(clean_transcript("  hello   world  "), "hello world");
    }

    #[test]
    fn dict_replaces_both_cases() {
        let d = default_dict();
        assert_eq!(apply_dict("我用 GBT 寫 code", &d), "我用 GPT 寫 code");
        assert_eq!(apply_dict("gbt 很好用", &d), "gpt 很好用");
        assert_eq!(apply_dict("My Torch 訓練", &d), "PyTorch 訓練");
    }

    #[test]
    fn opencc_converts_simplified_to_taiwan_traditional() {
        let out = to_traditional("软件测试");
        assert_eq!(out, "軟體測試"); // s2twp：軟件→軟體（台灣用語）
    }

    #[test]
    fn opencc_leaves_traditional_untouched() {
        assert_eq!(to_traditional("我們用繁體中文"), "我們用繁體中文");
    }
}
