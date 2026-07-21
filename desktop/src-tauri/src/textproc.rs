//! 確定性文字處理（SPEC §8「不靠 LLM 的部分」），自 prototype/main.py 移植：
//! `_clean_transcript`（CJK 空白清理）、`_apply_dict`（個人字典）、
//! `_to_traditional`（OpenCC s2twp 終盤繁化，ferrous-opencc 純 Rust，SPEC D6）。

use std::sync::OnceLock;

fn is_cjk_numeral(c: char) -> bool {
    matches!(
        c,
        '零' | '〇' | '一' | '二' | '兩' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十'
            | '百' | '千' | '萬' | '億'
    )
}

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
            (Some(p), Some(n)) => (is_cjk(p) && is_cjk(n)) || is_cjk_punct(n) || is_cjk_punct(p),
            _ => true, // 頭尾空白
        };
        if !drop_space {
            out.push(' ');
        }
        i = j;
    }
    out
}

/// 個人字典替換（寬鬆匹配）：忽略大小寫；條目裡的空白可對應文本中零或多個
/// 空白——Whisper 對 CJK 夾雜的英文詞常把 "My Torch" 黏成 "MyTorch"，
/// 字面比對會漏掉（實測踩到）。
///
/// 全程只掃描原始文本一次，避免某條替換結果再次觸發後續條目；同位置有重疊時
/// 優先採用覆蓋較長原文的條目。ASCII 英數邊緣需落在 token boundary，避免
/// `GBT` 誤改 `XGBT2`。替換值永遠使用使用者指定的原始大小寫。
pub fn apply_dict(text: &str, dict: &[(String, String)]) -> String {
    let entries: Vec<(Vec<char>, &str)> = dict
        .iter()
        .filter_map(|(wrong, right)| {
            let key: Vec<char> = wrong.trim().chars().collect();
            (!key.is_empty()).then_some((key, right.as_str()))
        })
        .collect();
    if entries.is_empty() {
        return text.to_string();
    }

    let tc: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < tc.len() {
        let mut best: Option<(usize, &str)> = None;
        for (key, right) in &entries {
            if let Some(end) = match_loose_at(&tc, i, key) {
                if !has_ascii_token_boundaries(&tc, i, end, key) {
                    continue;
                }
                // 較長匹配優先；同長度保留字典原順序，結果可預測。
                if best.map(|(best_end, _)| end > best_end).unwrap_or(true) {
                    best = Some((end, *right));
                }
            }
        }

        match best {
            Some((end, right)) => {
                out.push_str(right);
                i = end;
            }
            None => {
                out.push(tc[i]);
                i += 1;
            }
        }
    }
    out
}

fn is_ascii_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// 純 ASCII 且含英數的 key，對應原文必須位於 ASCII token 邊界。
/// CJK／混合語言條目不加邊界限制，維持既有可在連續中文字串內替換的行為。
fn has_ascii_token_boundaries(text: &[char], start: usize, end: usize, key: &[char]) -> bool {
    let ascii_key =
        key.iter().all(|c| c.is_ascii()) && key.iter().any(|c| c.is_ascii_alphanumeric());
    if !ascii_key {
        return true;
    }

    let left_ok = start == 0 || !is_ascii_token_char(text[start - 1]);
    let right_ok = end == text.len() || !is_ascii_token_char(text[end]);
    left_ok && right_ok
}

/// 從 tc[start] 起嘗試匹配 key：key 的空白 → 文本零或多個空白；其餘字元不分大小寫。
/// 匹配成功回傳結束位置（exclusive）。
fn match_loose_at(tc: &[char], start: usize, kc: &[char]) -> Option<usize> {
    let mut i = start;
    for &k in kc {
        if k.is_whitespace() {
            while i < tc.len() && tc[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        if i >= tc.len() || tc[i].to_lowercase().ne(k.to_lowercase()) {
            return None;
        }
        i += 1;
    }
    Some(i)
}

pub fn default_dict() -> Vec<(String, String)> {
    Vec::new()
}

/// CJK 標點正規化：緊跟在 CJK 字後的半形標點 → 全形（Whisper 常吐半形逗號）。
/// 保守規則：只在「前一字是 CJK，且下一字不是 ASCII 英數」時轉換，
/// 避免誤傷 "3.14"、"foo,bar"、URL 等。
pub fn normalize_cjk_punct(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        let mapped = match c {
            ',' | '.' | '?' | '!' | ';' | ':' => {
                let prev_cjk = i > 0 && is_cjk(chars[i - 1]);
                let next_ascii_alnum = chars
                    .get(i + 1)
                    .map(|n| n.is_ascii_alphanumeric())
                    .unwrap_or(false);
                // CJK 數字間的半形句點是小數點/版本號（review 反例：
                // 「三.五」「二.七.一」），不是句尾——保留半形
                let cjk_decimal = c == '.'
                    && i > 0
                    && is_cjk_numeral(chars[i - 1])
                    && chars.get(i + 1).copied().is_some_and(is_cjk_numeral);
                if prev_cjk && !next_ascii_alnum && !cjk_decimal {
                    match c {
                        ',' => '，',
                        '.' => '。',
                        '?' => '？',
                        '!' => '！',
                        ';' => '；',
                        ':' => '：',
                        _ => unreachable!(),
                    }
                } else {
                    c
                }
            }
            _ => c,
        };
        out.push(mapped);
    }
    out
}

/// OpenCC s2twp：確定性簡轉繁（台灣用語）。初始化失敗時原樣返回（不阻擋聽寫）。
/// 正體與簡體都合法的字：OpenCC `STCharacters` 中一對多、且候選包含自身者
/// （「只」→只／隻／祇、「干」→幹／乾／干、「里」→裏／里、「台」→臺／檯／颱／台…）。
/// 它們單獨出現不足以判定整段是簡體；拿它們當轉換觸發條件會把正體改壞。
const DUAL_SCRIPT_CHARS: &str = "㐹万丑丰了于云亘仆仇价仿伙余佛佣俊修借僵克党冬准凌几凶出划刮制千升卜占卷厂厘只台叶吁吃合吊同后向周咨咸咽哄唇喂噪回困夫夸奸姜娘它家尸局岩岳巨布帘席干幸广庵弦彩征御志念恤愈愿戚才扎托扣折抵拐挂挨挽据搜斗旋昆暗曲朱朴杆杠杯杰松板极柜栗核梁欲沈沾泛注涂涌淀游漓熏玩璇症皂矩确私秋种筑系胄背胜胡腊腌膻致舍芸苔苹范荐蒙蔑虫蚝蜡蝎表谷跖辟适郁酸采里雕面";

/// 是否含「明確簡體字」（不含雙義字）。
///
/// s2twp 的前提是輸入為簡體；對已經是正體的文字硬轉會過度轉換——實測
/// 「不是只會有一個」→「不是隻會有一個」、「干擾」→「幹擾」、「聯繫」→「聯絡」。
/// 因此只有偵測到明確簡體字時才做轉換。逐字比對用不帶詞組表的 s2t，
/// 避免詞組表本身的替換造成誤判。
fn has_simplified(text: &str) -> bool {
    static S2T: OnceLock<Option<ferrous_opencc::OpenCC>> = OnceLock::new();
    let cc = S2T.get_or_init(|| {
        ferrous_opencc::OpenCC::from_config(ferrous_opencc::config::BuiltinConfig::S2t)
            .map_err(|e| tracing::warn!("opencc s2t init failed: {e}"))
            .ok()
    });
    let Some(cc) = cc else { return false };
    let mut buf = [0u8; 4];
    for c in text.chars() {
        if !is_cjk(c) || DUAL_SCRIPT_CHARS.contains(c) {
            continue;
        }
        let s: &str = c.encode_utf8(&mut buf);
        if cc.convert(s) != s {
            return true;
        }
    }
    false
}

pub fn to_traditional(text: &str) -> String {
    if !has_simplified(text) {
        return text.to_string();
    }
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
    fn default_dictionary_is_empty() {
        assert!(default_dict().is_empty());
    }

    #[test]
    fn dict_replaces_both_cases() {
        let d = vec![
            ("GBT".into(), "GPT".into()),
            ("My Torch".into(), "PyTorch".into()),
        ];
        assert_eq!(apply_dict("我用 GBT 寫 code", &d), "我用 GPT 寫 code");
        assert_eq!(apply_dict("gbt 很好用", &d), "GPT 很好用");
        assert_eq!(apply_dict("My Torch 訓練", &d), "PyTorch 訓練");
    }

    // Whisper 把 CJK 夾雜的英文詞黏起來或改大小寫時仍要命中（transcribe_file 實測踩到）
    #[test]
    fn dict_matches_spaceless_and_mixed_case() {
        let d = vec![
            ("GBT".into(), "GPT".into()),
            ("My Torch".into(), "PyTorch".into()),
        ];
        assert_eq!(apply_dict("我們用MyTorch跑訓練", &d), "我們用PyTorch跑訓練");
        assert_eq!(apply_dict("用 my  torch 跑", &d), "用 PyTorch 跑");
        assert_eq!(apply_dict("Gbt 也可以", &d), "GPT 也可以");
    }

    #[test]
    fn dict_preserves_canonical_case_and_ascii_token_boundaries() {
        let d = vec![
            ("gbt".into(), "GPT".into()),
            ("my torch".into(), "PyTorch".into()),
        ];
        assert_eq!(
            apply_dict("gbt GBT2 XGBT gbt_name (GbT)", &d),
            "GPT GBT2 XGBT gbt_name (GPT)"
        );
        assert_eq!(apply_dict("MyTorch NotMyTorch", &d), "PyTorch NotMyTorch");

        let punct = vec![("C++".into(), "cpp".into())];
        assert_eq!(apply_dict("C++ C++20 XC++", &punct), "cpp C++20 XC++");
    }

    #[test]
    fn dict_uses_longest_overlap_and_does_not_cascade() {
        let overlap = vec![
            ("GPT".into(), "short".into()),
            ("GPT-4".into(), "GPT-4o".into()),
        ];
        assert_eq!(apply_dict("GPT-4 and GPT", &overlap), "GPT-4o and short");
        let reversed: Vec<_> = overlap.into_iter().rev().collect();
        assert_eq!(apply_dict("GPT-4 and GPT", &reversed), "GPT-4o and short");

        let cascade = vec![("GBT".into(), "GPT".into()), ("GPT".into(), "wrong".into())];
        assert_eq!(apply_dict("GBT", &cascade), "GPT");
    }

    #[test]
    fn dict_keeps_cjk_substring_matching() {
        let d = vec![("蘋果".into(), "Apple".into())];
        assert_eq!(apply_dict("紅蘋果汁", &d), "紅Apple汁");
    }

    #[test]
    fn punct_normalizes_after_cjk() {
        assert_eq!(
            normalize_cjk_punct("今天天氣很好,我們來測試."),
            "今天天氣很好，我們來測試。"
        );
        assert_eq!(normalize_cjk_punct("真的嗎?太好了!"), "真的嗎？太好了！");
    }

    #[test]
    fn punct_leaves_ascii_context_alone() {
        assert_eq!(normalize_cjk_punct("圓周率是3.14"), "圓周率是3.14");
        // CJK 數字間的小數點/版本號不是句尾（review 反例）
        assert_eq!(normalize_cjk_punct("比例是三.五"), "比例是三.五");
        assert_eq!(normalize_cjk_punct("版本二.七.一"), "版本二.七.一");
        // 但 CJK 數字後真正的句尾仍要轉
        assert_eq!(normalize_cjk_punct("總共有三."), "總共有三。");
        assert_eq!(normalize_cjk_punct("run test,ing now"), "run test,ing now");
        assert_eq!(
            normalize_cjk_punct("網址是 example.com"),
            "網址是 example.com"
        );
        // 前一字非 CJK → 不動
        assert_eq!(
            normalize_cjk_punct("用 PyTorch, 跑訓練"),
            "用 PyTorch, 跑訓練"
        );
    }

    #[test]
    fn punct_converts_at_end_of_cjk_sentence_before_space() {
        assert_eq!(
            normalize_cjk_punct("先做這個, 再做那個"),
            "先做這個， 再做那個"
        );
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

    /// 迴歸：實際使用中出現過「Figma 不是隻會有一個」——輸入已是正體，
    /// s2twp 的詞組表仍把「只會」轉成「隻會」。
    #[test]
    fn opencc_does_not_over_convert_traditional_text() {
        assert_eq!(
            to_traditional("Figma不是只會有一個嗎？"),
            "Figma不是只會有一個嗎？"
        );
        assert_eq!(to_traditional("干擾很嚴重"), "干擾很嚴重");
        assert_eq!(to_traditional("聯繫我"), "聯繫我");
        assert_eq!(to_traditional("這一里路"), "這一里路");
    }

    /// 雙義字不可讓判定器誤以為整段是簡體，但真的有簡體字時仍要轉。
    #[test]
    fn has_simplified_ignores_dual_script_chars() {
        assert!(!has_simplified("只會有一個"));
        assert!(!has_simplified("干擾"));
        assert!(!has_simplified("Figma 是什麼"));
        assert!(has_simplified("这是简体"));
        assert!(has_simplified("请不吝点赞"));
    }

    /// LLM 若吐出簡體，終盤仍必須繁化（SPEC D6 的核心保證不能被閘門破壞）。
    #[test]
    fn opencc_still_converts_when_simplified_present() {
        assert_eq!(to_traditional("重点是当成一个"), "重點是當成一個");
        assert_eq!(to_traditional("这个软件"), "這個軟體");
    }
}
