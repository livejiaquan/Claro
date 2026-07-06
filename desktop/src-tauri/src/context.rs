//! 螢幕上下文擷取（SPEC §7，移植 prototype `_get_window_context`）。
//!
//! AX API 抓前景 app／視窗標題／焦點元件游標前後文／選取文字／可見文字 BFS。
//! 隱私鐵律：AXSecureTextField 永不讀；內容只在記憶體、永不落盤；
//! 設定 `context_enabled=false` 時整個模組零 AX 呼叫（呼叫端把關）。
//! 預算沿用 prototype：游標前後 500 字、可見文字 1200 字、400 節點。

const CTX_CURSOR_CHARS: usize = 500;
const CTX_VISIBLE_CHARS: usize = 1200;
const CTX_MAX_NODES: usize = 400;

/// 從螢幕上下文抽英文/技術詞彙，連同個人字典餵給 Whisper 做詞彙偏置。
/// （Typeless 準確度的關鍵之一：上下文不只給 LLM 事後修，
/// 在 ASR 解碼階段就先偏置——NVIDIA、PyTorch、bounding box 這類詞。）
pub fn context_terms(dict_terms: &[String], context: &str, limit: usize) -> Vec<String> {
    let stop = ["app", "window", "visible", "selected", "editing", "around", "cursor"];
    let mut seen: Vec<String> = stop.iter().map(|s| s.to_string()).collect();
    let mut out: Vec<String> = Vec::new();

    let push = |t: &str, seen: &mut Vec<String>, out: &mut Vec<String>| {
        let k = t.to_lowercase();
        if !seen.contains(&k) {
            seen.push(k);
            out.push(t.to_string());
        }
    };

    for t in dict_terms {
        if out.len() >= limit {
            return out;
        }
        push(t, &mut seen, &mut out);
    }
    for t in ascii_tokens(context) {
        if out.len() >= limit {
            break;
        }
        push(&t, &mut seen, &mut out);
    }
    out
}

/// 等價 prototype 的 `[A-Za-z][A-Za-z0-9_.+-]{2,}`（免拉 regex 依賴）
fn ascii_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        let cont = c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-');
        if cur.is_empty() {
            if c.is_ascii_alphabetic() {
                cur.push(c);
            }
        } else if cont {
            cur.push(c);
        } else {
            if cur.len() >= 3 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            if c.is_ascii_alphabetic() {
                cur.push(c);
            }
        }
    }
    if cur.len() >= 3 {
        out.push(cur);
    }
    out
}

fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(target_os = "macos")]
pub use macos::capture;

#[cfg(not(target_os = "macos"))]
pub fn capture() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{truncate_chars, CTX_CURSOR_CHARS, CTX_MAX_NODES, CTX_VISIBLE_CHARS};
    use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};
    use core_foundation::base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use std::collections::VecDeque;
    use std::ffi::c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
        fn AXUIElementCopyAttributeValue(
            element: CFTypeRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXValueGetValue(value: CFTypeRef, the_type: u32, out: *mut c_void) -> bool;
    }

    /// 前景 app 的 (pid, 名稱)。走 NSWorkspace（prototype 原路）——
    /// system-wide AX 元素的 AXFocusedApplication 在部分執行情境回 -25204。
    fn frontmost_app() -> Option<(i32, String)> {
        use objc2_app_kit::NSWorkspace;
        let ws = unsafe { NSWorkspace::sharedWorkspace() };
        let front = unsafe { ws.frontmostApplication() }?;
        let pid = front.processIdentifier();
        let name = front
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_default();
        Some((pid, name))
    }

    const K_AX_VALUE_CFRANGE: u32 = 4; // kAXValueTypeCFRange

    #[repr(C)]
    struct CFRange {
        location: isize,
        length: isize,
    }

    /// 持有一個 retained CFTypeRef，drop 時 CFRelease（AX 回傳皆 create rule）
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    fn copy_attr(el: &Owned, name: &str) -> Option<Owned> {
        let attr = CFString::new(name);
        let mut out: CFTypeRef = std::ptr::null();
        let err = unsafe {
            AXUIElementCopyAttributeValue(el.0, attr.as_concrete_TypeRef(), &mut out)
        };
        (err == 0 && !out.is_null()).then_some(Owned(out))
    }

    fn attr_string(el: &Owned, name: &str) -> Option<String> {
        let v = copy_attr(el, name)?;
        unsafe {
            if CFGetTypeID(v.0) != CFString::type_id() {
                return None;
            }
            // wrap_under_get_rule：所有權仍在 Owned（drop 釋放），wrapper 借用＋retain
            let s = CFString::wrap_under_get_rule(v.0 as CFStringRef);
            Some(s.to_string())
        }
    }

    fn attr_children(el: &Owned) -> Vec<Owned> {
        let Some(v) = copy_attr(el, "AXChildren") else {
            return Vec::new();
        };
        unsafe {
            if CFGetTypeID(v.0) != CFArrayGetTypeID() {
                return Vec::new();
            }
            let arr = v.0 as core_foundation::array::CFArrayRef;
            let n = CFArrayGetCount(arr);
            (0..n)
                .filter_map(|i| {
                    let item = CFArrayGetValueAtIndex(arr, i) as CFTypeRef;
                    if item.is_null() {
                        None
                    } else {
                        // 陣列持有元素；要活過陣列就得自己 retain
                        Some(Owned(CFRetain(item)))
                    }
                })
                .collect()
        }
    }

    fn attr_range(el: &Owned, name: &str) -> Option<(usize, usize)> {
        let v = copy_attr(el, name)?;
        let mut range = CFRange { location: 0, length: 0 };
        let ok = unsafe {
            AXValueGetValue(v.0, K_AX_VALUE_CFRANGE, &mut range as *mut CFRange as *mut c_void)
        };
        (ok && range.location >= 0).then_some((range.location as usize, range.length as usize))
    }

    /// 取游標前後文。拿不到游標位置時退而取尾段（聽寫時游標通常在文末）。
    fn text_around_cursor(focused: &Owned, value: &str) -> String {
        let chars: Vec<char> = value.chars().collect();
        if let Some((pos, _)) = attr_range(focused, "AXSelectedTextRange") {
            // AX 的 location 是 UTF-16 單位，這裡以字元近似（CJK 情境差距可忽略），夾住邊界
            let pos = pos.min(chars.len());
            let lo = pos.saturating_sub(CTX_CURSOR_CHARS);
            let hi = (pos + CTX_CURSOR_CHARS).min(chars.len());
            return chars[lo..hi].iter().collect();
        }
        let lo = chars.len().saturating_sub(CTX_CURSOR_CHARS * 2);
        chars[lo..].iter().collect()
    }

    /// 廣度優先走訪焦點視窗的 AX 樹，收集可見文字（預算上限）。
    /// 只收內容性角色；按鈕/連結（書籤列、工具列）噪音多，排除。
    /// AXSecureTextField 不在允收角色內，天然跳過。
    fn collect_visible_text(window: &Owned) -> String {
        let mut chunks: Vec<String> = Vec::new();
        let mut total = 0usize;
        let mut visited = 0usize;
        let mut queue: VecDeque<Owned> = VecDeque::new();
        queue.push_back(Owned(unsafe { CFRetain(window.0) }));

        while let Some(el) = queue.pop_front() {
            if visited >= CTX_MAX_NODES || total >= CTX_VISIBLE_CHARS {
                break;
            }
            visited += 1;
            if let Some(role) = attr_string(&el, "AXRole") {
                if matches!(role.as_str(), "AXStaticText" | "AXTextField" | "AXTextArea" | "AXHeading")
                {
                    for attr in ["AXValue", "AXTitle"] {
                        if let Some(t) = attr_string(&el, attr) {
                            let t = t.trim();
                            if t.chars().count() >= 3 {
                                let t = truncate_chars(t, CTX_VISIBLE_CHARS - total);
                                if !chunks.contains(&t) {
                                    total += t.chars().count();
                                    chunks.push(t);
                                }
                                break;
                            }
                        }
                    }
                }
            }
            for child in attr_children(&el) {
                queue.push_back(child);
            }
        }
        chunks.join(" | ")
    }

    /// 讀前景 app／視窗／游標周邊／可見文字。任何一步失敗都靜默略過該部分。
    pub fn capture() -> Option<String> {
        let (pid, app_name) = frontmost_app()?;
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        if app.0.is_null() {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        if !app_name.is_empty() {
            parts.push(format!("App: {app_name}"));
        }

        let window = copy_attr(&app, "AXFocusedWindow");
        if let Some(w) = &window {
            if let Some(title) = attr_string(w, "AXTitle") {
                if !title.is_empty() {
                    parts.push(format!("Window: {title}"));
                }
            }
        }

        if let Some(focused) = copy_attr(&app, "AXFocusedUIElement") {
            // 隱私鐵律：密碼欄（role 或 subrole 是 AXSecureTextField）整塊跳過
            let role = attr_string(&focused, "AXRole").unwrap_or_default();
            let subrole = attr_string(&focused, "AXSubrole").unwrap_or_default();
            if role != "AXSecureTextField" && subrole != "AXSecureTextField" {
                if let Some(value) = attr_string(&focused, "AXValue") {
                    if !value.trim().is_empty() {
                        let around = text_around_cursor(&focused, &value);
                        let around = around.trim();
                        if !around.is_empty() {
                            parts.push(format!("Editing(around cursor): {around}"));
                        }
                    }
                }
                if let Some(sel) = attr_string(&focused, "AXSelectedText") {
                    let sel = sel.trim();
                    if !sel.is_empty() {
                        parts.push(format!("Selected: {}", truncate_chars(sel, 200)));
                    }
                }
            }
        }

        if let Some(w) = &window {
            let visible = collect_visible_text(w);
            if !visible.is_empty() {
                parts.push(format!("Visible: {visible}"));
            }
        }

        (!parts.is_empty()).then(|| parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_prefer_dict_then_context_dedup_and_cap() {
        let dict = vec!["PyTorch".to_string(), "GPT".to_string()];
        let ctx = "App: Zed\nWindow: main.rs — claro\nVisible: pytorch tokenizer | whisper.cpp | GPT";
        let terms = context_terms(&dict, ctx, 15);
        assert_eq!(terms[0], "PyTorch");
        assert_eq!(terms[1], "GPT");
        assert!(terms.contains(&"Zed".to_string()));
        assert!(terms.contains(&"whisper.cpp".to_string()));
        // pytorch（小寫）與 GPT 重複 → 去重
        assert_eq!(terms.iter().filter(|t| t.to_lowercase() == "pytorch").count(), 1);
        assert_eq!(terms.iter().filter(|t| t.to_lowercase() == "gpt").count(), 1);
    }

    #[test]
    fn terms_skip_stopwords_and_respect_limit() {
        let ctx = "App: Safari\nWindow: docs\nEditing(around cursor): alpha beta gamma delta";
        let terms = context_terms(&[], ctx, 3);
        assert_eq!(terms.len(), 3);
        assert!(!terms.iter().any(|t| t.to_lowercase() == "app"));
        assert!(!terms.iter().any(|t| t.to_lowercase() == "editing"));
    }

    #[test]
    fn ascii_tokens_match_prototype_regex_semantics() {
        let toks = ascii_tokens("用 whisper.cpp 跑 STT，b2 版本 x_1 ok 1abc");
        assert!(toks.contains(&"whisper.cpp".to_string()));
        assert!(toks.contains(&"STT".to_string()));
        assert!(toks.contains(&"x_1".to_string()));
        assert!(!toks.contains(&"b2".to_string())); // 長度 <3
        assert!(!toks.contains(&"ok".to_string())); // 長度 <3
        assert!(!toks.contains(&"1abc".to_string())); // 需字母開頭
    }
}
