//! 螢幕上下文擷取（SPEC §7，移植 prototype `_get_window_context`）。
//!
//! AX API 抓前景 app／視窗標題／焦點元件游標前後文／選取文字／可見文字 BFS。
//! 隱私鐵律：AXSecureTextField 永不讀；內容只在記憶體、永不落盤；
//! 設定 `context_enabled=false` 時零 AX 文字擷取；貼上安全仍會讀取並立即 hash
//! App／視窗／焦點 metadata，但不讀欄位值、不顯示、不落盤。
//! 預算沿用 prototype：游標前後 500 字、可見文字 1200 字、400 節點。

const CTX_CURSOR_CHARS: usize = 500;
const CTX_VISIBLE_CHARS: usize = 1200;
const CTX_MAX_NODES: usize = 400;
const CLARO_BUNDLE_ID: &str = "dev.claro.desktop";

/// Claro 的 WKWebView accessibility 必須留在 WebKit 主執行緒處理；背景內容擷取
/// 遇到自己的 bundle 時一律 fail closed。刻意用 exact match，避免誤傷相似 bundle。
fn should_skip_background_context(bundle_id: &str) -> bool {
    bundle_id == CLARO_BUNDLE_ID
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextSnapshot {
    pub text: String,
    pub app_id: String,
    pub app_name: String,
    pub surface: &'static str,
    /// 只供 pipeline 驗證本次 Context 是否來自同一個視窗／焦點；不送到 UI。
    #[serde(skip_serializing)]
    pub(crate) target: PasteTarget,
}

/// 貼上安全用的焦點指紋。只保留 App id 與不可逆的視窗／焦點 metadata hash，
/// 不含欄位內容；即使 context 關閉也能防止處理期間切到同 App 的另一個目標。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteTarget {
    pub app_id: String,
    pub(crate) window_hash: [u8; 32],
    pub(crate) focus_hash: [u8; 32],
}

/// 密碼管理器整個 app 預設不讀；AXSecureTextField 仍是欄位級第二道防線。
pub fn is_sensitive_app(app_name: &str, bundle_id: &str) -> bool {
    let name = app_name.to_lowercase();
    let bundle = bundle_id.to_lowercase();
    // review H2：needle 用單數/字根，複數與品牌全名靠 substring 涵蓋
    // （"password" 涵蓋 Passwords/Keeper Password Manager；"keychain" 涵蓋
    // Keychain Access/com.apple.keychainaccess；"鑰匙圈" 涵蓋鑰匙圈存取）
    [
        "1password",
        "bitwarden",
        "keepass",
        "dashlane",
        "lastpass",
        "enpass",
        "strongbox",
        "keeper",
        "nordpass",
        "roboform",
        "proton pass",
        "proton.pass",
        "password",
        "密碼",
        "keychain",
        "鑰匙圈",
    ]
    .iter()
    .any(|needle| name.contains(needle) || bundle.contains(needle))
}

/// 可解釋的 App 類型，不做黑箱猜測；未知 App 永遠回 neutral。
pub fn classify_surface(app_name: &str, bundle_id: &str, window_title: &str) -> &'static str {
    let haystack = format!("{app_name} {bundle_id} {window_title}").to_lowercase();
    if ["mail", "outlook", "gmail", "spark", "mimestream"]
        .iter()
        .any(|needle| haystack.contains(needle))
    {
        "email"
    } else if [
        "slack",
        "teams",
        "discord",
        "line",
        "messages",
        "whatsapp",
        "telegram",
        "messenger",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
    {
        "message"
    } else if [
        "xcode",
        "visual studio code",
        "vscode",
        "cursor",
        "zed",
        "terminal",
        "iterm",
        "warp",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
    {
        "technical"
    } else if [
        "notes",
        "notion",
        "obsidian",
        "pages",
        "word",
        "google docs",
        "docs",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
    {
        "document"
    } else {
        "neutral"
    }
}

pub fn surface_guidance(context: &str) -> &'static str {
    let lower = context.to_lowercase();
    let app = lower
        .lines()
        .find_map(|line| line.strip_prefix("app: "))
        .unwrap_or_default();
    let window = lower
        .lines()
        .find_map(|line| line.strip_prefix("window: "))
        .unwrap_or_default();
    match classify_surface(app, "", window) {
        "email" => "email：只在原文已有稱呼或結尾時保留；使用自然段落，不新增收件人、主旨或署名",
        "message" => "message：維持簡短口語；除非原文明確列舉，否則不用標題或長段落",
        "technical" => {
            "technical：保留程式碼、路徑、指令與識別字；不新增 Markdown code fence 或解釋"
        }
        "document" => "document：依原文明確的段落或列舉意圖排版；不得新增標題或分類",
        _ => "neutral：只依 transcript 的明確意圖排版，不推測使用情境",
    }
}

/// 從螢幕上下文抽英文／技術／CJK 詞彙，連同個人字典餵給 Whisper。
/// 這是 Claro 自己可驗證的解碼期偏置，不推測 Typeless 的內部管線。
pub fn context_terms(dict_terms: &[String], context: &str, limit: usize) -> Vec<String> {
    let stop = [
        "app", "window", "visible", "selected", "editing", "around", "cursor",
    ];
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
    for t in cjk_tokens(context) {
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

/// 保守抽取 2–8 字 CJK 詞組；只拿內容欄位，不把 `App:` 等標籤餵回模型。
fn cjk_tokens(text: &str) -> Vec<String> {
    let stop = [
        "目前", "設定", "使用", "內容", "這個", "可以", "我們", "今天", "明天", "因為", "所以",
        "如果", "需要", "完成", "正在", "沒有", "顯示", "視窗", "選取", "游標",
    ];
    let mut out = Vec::new();
    for line in text.lines() {
        let value = line.split_once(':').map(|(_, value)| value).unwrap_or(line);
        let mut token = String::new();
        let flush = |token: &mut String, out: &mut Vec<String>| {
            let len = token.chars().count();
            if (2..=8).contains(&len) && !stop.contains(&token.as_str()) && !out.contains(token) {
                out.push(std::mem::take(token));
            } else {
                token.clear();
            }
        };
        for c in value.chars() {
            if matches!(c as u32, 0x3400..=0x9FFF | 0xF900..=0xFAFF) {
                token.push(c);
            } else {
                flush(&mut token, &mut out);
            }
        }
        flush(&mut token, &mut out);
    }
    out
}

fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(target_os = "macos")]
pub use macos::{
    begin_paste_target_capture, capture, capture_paste_target, capture_snapshot,
    capture_snapshot_for_target, current_app_id, finish_paste_target_capture, PasteTargetSeed,
};

#[cfg(not(target_os = "macos"))]
pub fn capture() -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn capture_snapshot() -> Option<ContextSnapshot> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn capture_snapshot_for_target(_target: &PasteTarget) -> Option<ContextSnapshot> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn current_app_id() -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn capture_paste_target() -> Option<PasteTarget> {
    None
}

#[cfg(not(target_os = "macos"))]
pub struct PasteTargetSeed;

#[cfg(not(target_os = "macos"))]
pub fn begin_paste_target_capture() -> Option<PasteTargetSeed> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn finish_paste_target_capture(_seed: PasteTargetSeed) -> Option<PasteTarget> {
    None
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{
        classify_surface, is_sensitive_app, should_skip_background_context, truncate_chars,
        ContextSnapshot, PasteTarget, CTX_CURSOR_CHARS, CTX_MAX_NODES, CTX_VISIBLE_CHARS,
    };
    use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};
    use core_foundation::base::{CFEqual, CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use std::collections::VecDeque;
    use std::ffi::c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
        fn AXUIElementCreateSystemWide() -> CFTypeRef;
        fn AXUIElementCopyAttributeValue(
            element: CFTypeRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: CFTypeRef, timeout_seconds: f32) -> i32;
        fn AXValueGetValue(value: CFTypeRef, the_type: u32, out: *mut c_void) -> bool;
    }

    /// 對 system-wide 元素設 messaging timeout ＝ 全程序 AX 呼叫的預設逾時。
    /// 沒有這個，目標 app 的 AX server 卡住會 wedge 整個聽寫 session（review 抓到的坑）。
    fn set_global_ax_timeout_once() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe {
            let sys = AXUIElementCreateSystemWide();
            if !sys.is_null() {
                let _ = AXUIElementSetMessagingTimeout(sys, 0.25);
                CFRelease(sys);
            }
        });
    }

    /// Paste safety 在熱鍵後的背景工作執行，但仍需有硬上限，避免卡住的 AX server
    /// 拖慢 Esc／短句處理。單次 40ms、整批（含最後 focus recheck）預算 240ms。
    const PASTE_TARGET_CALL_TIMEOUT_S: f32 = 0.04;
    const PASTE_TARGET_BUDGET_MS: u64 = 240;

    fn set_paste_target_timeout(element: &Owned) {
        unsafe {
            let _ = AXUIElementSetMessagingTimeout(element.0, PASTE_TARGET_CALL_TIMEOUT_S);
        }
    }

    /// 前景 app 的 (pid, 名稱, bundle id)。走 NSWorkspace（prototype 原路）——
    /// system-wide AX 元素的 AXFocusedApplication 在部分執行情境回 -25204。
    fn frontmost_app() -> Option<(i32, String, String)> {
        use objc2_app_kit::NSWorkspace;
        let ws = NSWorkspace::sharedWorkspace();
        let front = ws.frontmostApplication()?;
        let pid = front.processIdentifier();
        let name = front
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_default();
        let bundle_id = front
            .bundleIdentifier()
            .map(|identifier| identifier.to_string())
            .unwrap_or_default();
        Some((pid, name, bundle_id))
    }

    pub fn current_app_id() -> Option<String> {
        let (pid, name, bundle_id) = frontmost_app()?;
        Some(if bundle_id.is_empty() {
            format!("pid:{pid}:{}", name.to_lowercase())
        } else {
            bundle_id
        })
    }

    const K_AX_VALUE_CFRANGE: u32 = 4; // kAXValueTypeCFRange

    #[repr(C)]
    struct CFRange {
        location: isize,
        length: isize,
    }

    #[repr(C)]
    #[derive(Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct CGSize {
        width: f64,
        height: f64,
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

    struct TargetElements {
        app_name: String,
        bundle_id: String,
        app_id: String,
        app: Owned,
        window: Owned,
        focused: Owned,
    }

    /// keyDown 當下只固定 focused AX element reference；不讀文字或 metadata。
    /// 一般 App 幾毫秒完成，卡住的 AX server 最多等待 40ms。
    pub struct PasteTargetSeed {
        app_name: String,
        bundle_id: String,
        app_id: String,
        app: Owned,
        focused: Owned,
    }

    fn attr_point(el: &Owned, name: &str) -> Option<CGPoint> {
        let value = copy_attr(el, name)?;
        let mut point = CGPoint::default();
        unsafe {
            AXValueGetValue(value.0, 1, &mut point as *mut CGPoint as *mut c_void).then_some(point)
        }
    }

    fn attr_size(el: &Owned, name: &str) -> Option<CGSize> {
        let value = copy_attr(el, name)?;
        let mut size = CGSize::default();
        unsafe {
            AXValueGetValue(value.0, 2, &mut size as *mut CGSize as *mut c_void).then_some(size)
        }
    }

    fn metadata_hash(parts: &[String]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        static PROCESS_SALT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let salt = PROCESS_SALT.get_or_init(|| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("{}:{now}", std::process::id())
        });
        let mut hasher = Sha256::new();
        hasher.update(salt.as_bytes());
        for part in parts {
            hasher.update((part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
        hasher.finalize().into()
    }

    fn capture_target_elements_for_app(
        pid: i32,
        app_name: String,
        bundle_id: String,
        deadline: std::time::Instant,
    ) -> Option<TargetElements> {
        let app_id = if bundle_id.is_empty() {
            format!("pid:{pid}:{}", app_name.to_lowercase())
        } else {
            bundle_id.clone()
        };
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        if app.0.is_null() {
            return None;
        }
        set_paste_target_timeout(&app);

        let window = copy_attr_before(&app, "AXFocusedWindow", deadline)?;
        set_paste_target_timeout(&window);
        let focused = copy_attr_before(&app, "AXFocusedUIElement", deadline)?;
        set_paste_target_timeout(&focused);

        Some(TargetElements {
            app_name,
            bundle_id,
            app_id,
            app,
            window,
            focused,
        })
    }

    /// 內容擷取專用入口。先以 NSWorkspace 辨識前景 bundle，再建立任何 AX 元素，
    /// 避免背景執行緒碰觸 Claro 自己的 WKWebView accessibility tree。
    fn capture_context_target_elements(deadline: std::time::Instant) -> Option<TargetElements> {
        let (pid, app_name, bundle_id) = frontmost_app()?;
        if should_skip_background_context(&bundle_id) {
            tracing::info!("context skipped for Claro's own WebView");
            return None;
        }
        capture_target_elements_for_app(pid, app_name, bundle_id, deadline)
    }

    fn paste_target_from_elements(
        elements: &TargetElements,
        deadline: std::time::Instant,
    ) -> Option<PasteTarget> {
        let mut window_parts = Vec::new();
        let mut window_discriminating = false;
        for attr in ["AXIdentifier", "AXDocument", "AXTitle"] {
            if let Some(value) =
                attr_string_before(&elements.window, attr, deadline).filter(|s| !s.is_empty())
            {
                window_parts.push(format!("{attr}={value}"));
                window_discriminating = true;
            }
        }
        if std::time::Instant::now() < deadline {
            if let Some(point) = attr_point(&elements.window, "AXPosition") {
                window_parts.push(format!("AXPosition={:.1},{:.1}", point.x, point.y));
                window_discriminating = true;
            }
        }
        if std::time::Instant::now() < deadline {
            if let Some(size) = attr_size(&elements.window, "AXSize") {
                window_parts.push(format!("AXSize={:.1},{:.1}", size.width, size.height));
                window_discriminating = true;
            }
        }
        for attr in ["AXRole", "AXSubrole"] {
            if let Some(value) =
                attr_string_before(&elements.window, attr, deadline).filter(|s| !s.is_empty())
            {
                window_parts.push(format!("{attr}={value}"));
            }
        }

        let mut focus_parts = Vec::new();
        let mut focus_discriminating = false;
        for attr in ["AXIdentifier", "AXDOMIdentifier"] {
            if let Some(value) =
                attr_string_before(&elements.focused, attr, deadline).filter(|s| !s.is_empty())
            {
                focus_parts.push(format!("{attr}={value}"));
                focus_discriminating = true;
            }
        }
        if std::time::Instant::now() < deadline {
            if let Some(point) = attr_point(&elements.focused, "AXPosition") {
                focus_parts.push(format!("AXPosition={:.1},{:.1}", point.x, point.y));
                focus_discriminating = true;
            }
        }
        if std::time::Instant::now() < deadline {
            if let Some(size) = attr_size(&elements.focused, "AXSize") {
                focus_parts.push(format!("AXSize={:.1},{:.1}", size.width, size.height));
                focus_discriminating = true;
            }
        }
        for attr in ["AXTitle", "AXDescription"] {
            if let Some(value) =
                attr_string_before(&elements.focused, attr, deadline).filter(|s| !s.is_empty())
            {
                focus_parts.push(format!("{attr}={value}"));
                focus_discriminating = true;
            }
        }
        for attr in ["AXRole", "AXSubrole"] {
            if let Some(value) =
                attr_string_before(&elements.focused, attr, deadline).filter(|s| !s.is_empty())
            {
                focus_parts.push(format!("{attr}={value}"));
            }
        }

        // unknown→unknown 不可視為相同；只有 Role/Subrole 不足以區分兩個輸入框。
        if !window_discriminating || !focus_discriminating {
            return None;
        }

        Some(PasteTarget {
            app_id: elements.app_id.clone(),
            window_hash: metadata_hash(&window_parts),
            focus_hash: metadata_hash(&focus_parts),
        })
    }

    fn elements_still_focused(elements: &TargetElements, deadline: std::time::Instant) -> bool {
        if current_app_id().as_deref() != Some(elements.app_id.as_str()) {
            return false;
        }
        let Some(window) = copy_attr_before(&elements.app, "AXFocusedWindow", deadline) else {
            return false;
        };
        let Some(focused) = copy_attr_before(&elements.app, "AXFocusedUIElement", deadline) else {
            return false;
        };
        unsafe {
            CFEqual(window.0, elements.window.0) != 0 && CFEqual(focused.0, elements.focused.0) != 0
        }
    }

    pub fn begin_paste_target_capture() -> Option<PasteTargetSeed> {
        set_global_ax_timeout_once();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
        let (pid, app_name, bundle_id) = frontmost_app()?;
        if should_skip_background_context(&bundle_id) {
            tracing::info!("AX target capture skipped for Claro's own WebView");
            return None;
        }
        let app_id = if bundle_id.is_empty() {
            format!("pid:{pid}:{}", app_name.to_lowercase())
        } else {
            bundle_id.clone()
        };
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        if app.0.is_null() {
            return None;
        }
        set_paste_target_timeout(&app);
        let focused = copy_attr_before(&app, "AXFocusedUIElement", deadline)?;
        set_paste_target_timeout(&focused);
        Some(PasteTargetSeed {
            app_name,
            bundle_id,
            app_id,
            app,
            focused,
        })
    }

    pub fn finish_paste_target_capture(seed: PasteTargetSeed) -> Option<PasteTarget> {
        if should_skip_background_context(&seed.bundle_id) {
            tracing::info!("AX target capture skipped for Claro's own WebView");
            return None;
        }
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_millis(PASTE_TARGET_BUDGET_MS);
        let metadata_deadline = deadline
            .checked_sub(std::time::Duration::from_millis(120))
            .unwrap_or(started);
        if current_app_id().as_deref() != Some(seed.app_id.as_str()) {
            return None;
        }
        let focused = copy_attr_before(&seed.app, "AXFocusedUIElement", metadata_deadline)?;
        if unsafe { CFEqual(focused.0, seed.focused.0) } == 0 {
            return None;
        }
        set_paste_target_timeout(&focused);
        let window = copy_attr_before(&seed.app, "AXFocusedWindow", metadata_deadline)?;
        set_paste_target_timeout(&window);
        let elements = TargetElements {
            app_name: seed.app_name,
            bundle_id: seed.bundle_id,
            app_id: seed.app_id,
            app: seed.app,
            window,
            focused,
        };
        let target = paste_target_from_elements(&elements, metadata_deadline)?;
        elements_still_focused(&elements, deadline).then_some(target)
    }

    /// 不讀 AXValue／選取文字／可見文字，只把焦點 metadata 立即 hash。
    /// 這是貼上安全機制，不受「讀取畫面詞彙」開關控制。
    pub fn capture_paste_target() -> Option<PasteTarget> {
        set_global_ax_timeout_once();
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_millis(PASTE_TARGET_BUDGET_MS);
        // 預留最後兩次 focus recheck 的時間；超時一律 fail closed。
        let metadata_deadline = deadline
            .checked_sub(std::time::Duration::from_millis(120))
            .unwrap_or(started);
        let elements = capture_context_target_elements(metadata_deadline)?;
        let target = paste_target_from_elements(&elements, metadata_deadline)?;
        elements_still_focused(&elements, deadline).then_some(target)
    }

    fn copy_attr(el: &Owned, name: &str) -> Option<Owned> {
        let attr = CFString::new(name);
        let mut out: CFTypeRef = std::ptr::null();
        let err =
            unsafe { AXUIElementCopyAttributeValue(el.0, attr.as_concrete_TypeRef(), &mut out) };
        (err == 0 && !out.is_null()).then_some(Owned(out))
    }

    fn copy_attr_before(el: &Owned, name: &str, deadline: std::time::Instant) -> Option<Owned> {
        (std::time::Instant::now() < deadline)
            .then(|| copy_attr(el, name))
            .flatten()
    }

    fn attr_string_before(el: &Owned, name: &str, deadline: std::time::Instant) -> Option<String> {
        (std::time::Instant::now() < deadline)
            .then(|| attr_string(el, name))
            .flatten()
    }

    fn attr_string(el: &Owned, name: &str) -> Option<String> {
        attr_string_capped(el, name, usize::MAX)
    }

    /// 讀字串屬性，但先用 CFStringGetLength 檢查長度——超過上限直接放棄，
    /// 不為巨大文件（AXValue 可能是整份內容）做整串複製。
    /// 安全欄位判定（fail-closed，review 發現）：只有 subrole「確定沒有值」
    /// （NoValue／AttributeUnsupported——一般元素的常態）才視為非安全；
    /// 讀到 AXSecureTextField 當然是安全欄位；逾時、CannotComplete、無效
    /// 元素等其他錯誤一律**當成安全欄位**——密碼欄的 subrole 暫時讀不到時
    /// 絕不能 fail-open 去讀 AXValue。
    fn secure_or_unknown(el: &Owned, deadline: std::time::Instant) -> bool {
        if std::time::Instant::now() >= deadline {
            return true; // 時間耗盡＝未知＝當安全欄位跳過
        }
        const AX_ERROR_NO_VALUE: i32 = -25212;
        const AX_ERROR_ATTRIBUTE_UNSUPPORTED: i32 = -25205;
        let attr = CFString::new("AXSubrole");
        let mut out: CFTypeRef = std::ptr::null();
        let err =
            unsafe { AXUIElementCopyAttributeValue(el.0, attr.as_concrete_TypeRef(), &mut out) };
        match err {
            0 => {
                if out.is_null() {
                    return false;
                }
                let owned = Owned(out);
                // 非字串或轉換失敗＝未知＝當安全欄位
                attr_owned_string(&owned).is_none_or(|s| s == "AXSecureTextField")
            }
            AX_ERROR_NO_VALUE | AX_ERROR_ATTRIBUTE_UNSUPPORTED => false,
            _ => true,
        }
    }

    fn attr_owned_string(v: &Owned) -> Option<String> {
        unsafe {
            if CFGetTypeID(v.0) != CFString::type_id() {
                return None;
            }
            let s = CFString::wrap_under_get_rule(v.0 as CFStringRef);
            Some(s.to_string())
        }
    }

    fn attr_string_capped(el: &Owned, name: &str, max_chars: usize) -> Option<String> {
        let v = copy_attr(el, name)?;
        unsafe {
            if CFGetTypeID(v.0) != CFString::type_id() {
                return None;
            }
            // wrap_under_get_rule：所有權仍在 Owned（drop 釋放），wrapper 借用＋retain
            let s = CFString::wrap_under_get_rule(v.0 as CFStringRef);
            if s.char_len() as usize > max_chars {
                return None;
            }
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
            // 單一 container 可能回報數萬 children（瀏覽器/大型文件），全 retain
            // 會在 400 節點預算外偷跑大量工作（review 發現）——BFS 反正只處理
            // 前 400 節點，這裡就先掐
            let n = CFArrayGetCount(arr).min(400);
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
        let mut range = CFRange {
            location: 0,
            length: 0,
        };
        let ok = unsafe {
            AXValueGetValue(
                v.0,
                K_AX_VALUE_CFRANGE,
                &mut range as *mut CFRange as *mut c_void,
            )
        };
        (ok && range.location >= 0).then_some((range.location as usize, range.length as usize))
    }

    /// 取游標前後文。拿不到游標位置時退而取尾段（聽寫時游標通常在文末）。
    fn text_around_cursor(focused: &Owned, value: &str, deadline: std::time::Instant) -> String {
        // 巨大文件防護：AXValue 可能是整份文件（MB 級），
        // 超過上限就直接取尾段，不為它展開整串 Vec<char>
        const VALUE_BYTES_CAP: usize = 512 * 1024;
        if value.len() > VALUE_BYTES_CAP {
            let tail_start = value
                .char_indices()
                .rev()
                .nth(CTX_CURSOR_CHARS * 2 - 1)
                .map(|(i, _)| i)
                .unwrap_or(0);
            return value[tail_start..].to_string();
        }
        let chars: Vec<char> = value.chars().collect();
        if std::time::Instant::now() < deadline {
            if let Some((pos, _)) = attr_range(focused, "AXSelectedTextRange") {
                // AX 的 location 是 UTF-16 單位，這裡以字元近似（CJK 情境差距可忽略），夾住邊界
                let pos = pos.min(chars.len());
                let lo = pos.saturating_sub(CTX_CURSOR_CHARS);
                let hi = (pos + CTX_CURSOR_CHARS).min(chars.len());
                return chars[lo..hi].iter().collect();
            }
        }
        let lo = chars.len().saturating_sub(CTX_CURSOR_CHARS * 2);
        chars[lo..].iter().collect()
    }

    /// 廣度優先走訪焦點視窗的 AX 樹，收集可見文字（預算上限＋牆鐘預算）。
    /// 只收內容性角色；按鈕/連結（書籤列、工具列）噪音多，排除。
    /// 密碼欄雙重防護：role 不在允收清單天然跳過，subrole 是
    /// AXSecureTextField（未聚焦的一般 TextField 也可能如此標記）明確跳過。
    fn collect_visible_text(window: &Owned, deadline: std::time::Instant) -> String {
        let mut chunks: Vec<String> = Vec::new();
        let mut total = 0usize;
        let mut visited = 0usize;
        let mut queue: VecDeque<Owned> = VecDeque::new();
        queue.push_back(Owned(unsafe { CFRetain(window.0) }));

        while let Some(el) = queue.pop_front() {
            if visited >= CTX_MAX_NODES
                || total >= CTX_VISIBLE_CHARS
                || std::time::Instant::now() >= deadline
            {
                break;
            }
            visited += 1;
            set_paste_target_timeout(&el);
            if let Some(role) = attr_string_before(&el, "AXRole", deadline) {
                if matches!(
                    role.as_str(),
                    "AXStaticText" | "AXTextField" | "AXTextArea" | "AXHeading"
                ) {
                    let secure = secure_or_unknown(&el, deadline);
                    if secure {
                        // 安全（或無法判定）欄位：不讀文字，也不下探 children——
                        // 有些 AX provider 會把內容以 child AXStaticText 暴露
                        continue;
                    }
                    {
                        for attr in ["AXValue", "AXTitle"] {
                            if std::time::Instant::now() >= deadline {
                                break;
                            }
                            if let Some(t) = attr_string_capped(&el, attr, CTX_VISIBLE_CHARS * 4) {
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
            }
            if std::time::Instant::now() < deadline {
                for child in attr_children(&el) {
                    queue.push_back(child);
                }
            }
        }
        chunks.join(" | ")
    }

    // Work deadline 210ms；最後一個已開始的 AX call 最多 40ms，整批硬上限約 250ms。
    const CONTEXT_CAPTURE_BUDGET_MS: u64 = 210;

    /// 讀前景 app／視窗／游標周邊／可見文字。fingerprint 與內容共用同一批
    /// retained AX refs，因此 A→B→A 也不會把 B 內容包成 A；整批約 250ms 硬上限。
    pub fn capture_snapshot() -> Option<ContextSnapshot> {
        set_global_ax_timeout_once();
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_millis(CONTEXT_CAPTURE_BUDGET_MS);
        let fingerprint_deadline =
            started + std::time::Duration::from_millis(CONTEXT_CAPTURE_BUDGET_MS / 2);
        let elements = capture_context_target_elements(fingerprint_deadline)?;
        let target = paste_target_from_elements(&elements, fingerprint_deadline)?;
        capture_snapshot_from_elements(elements, target, deadline)
    }

    pub fn capture_snapshot_for_target(expected: &PasteTarget) -> Option<ContextSnapshot> {
        set_global_ax_timeout_once();
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_millis(CONTEXT_CAPTURE_BUDGET_MS);
        let fingerprint_deadline =
            started + std::time::Duration::from_millis(CONTEXT_CAPTURE_BUDGET_MS / 2);
        let elements = capture_context_target_elements(fingerprint_deadline)?;
        let observed = paste_target_from_elements(&elements, fingerprint_deadline)?;
        if &observed != expected {
            return None;
        }
        capture_snapshot_from_elements(elements, observed, deadline)
    }

    fn capture_snapshot_from_elements(
        elements: TargetElements,
        target: PasteTarget,
        deadline: std::time::Instant,
    ) -> Option<ContextSnapshot> {
        // 防禦性檢查：正常入口已在建立 AX 元素前擋下；若未來新增內部入口，
        // 也不得讀取 Claro 自身 WebView 的 title/value/children。
        if should_skip_background_context(&elements.bundle_id) {
            tracing::info!("context skipped for Claro's own WebView");
            return None;
        }
        if is_sensitive_app(&elements.app_name, &elements.bundle_id) {
            tracing::info!("context skipped for sensitive app '{}'", elements.app_name);
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        let mut window_title = String::new();
        if !elements.app_name.is_empty() {
            parts.push(format!("App: {}", elements.app_name));
        }

        if let Some(title) = attr_string_before(&elements.window, "AXTitle", deadline) {
            if !title.is_empty() {
                window_title = title.clone();
                parts.push(format!("Window: {title}"));
            }
        }

        if std::time::Instant::now() < deadline {
            // 隱私鐵律：密碼欄（role 或 subrole 是 AXSecureTextField）整塊跳過。
            // fail-closed（review 發現）：role 讀不到＝未知元素、subrole 讀取
            // 錯誤＝未知——都不准 fail-open 去讀 AXValue
            let role_known_safe = attr_string_before(&elements.focused, "AXRole", deadline)
                .is_some_and(|r| r != "AXSecureTextField");
            if role_known_safe && !secure_or_unknown(&elements.focused, deadline) {
                // 20 萬字上限：焦點若在整份大型文件，值太大直接放棄（不整串複製）
                if std::time::Instant::now() < deadline {
                    if let Some(value) = attr_string_capped(&elements.focused, "AXValue", 200_000) {
                        if !value.trim().is_empty() {
                            let around = text_around_cursor(&elements.focused, &value, deadline);
                            let around = around.trim();
                            if !around.is_empty() {
                                parts.push(format!("Editing(around cursor): {around}"));
                            }
                        }
                    }
                }
                // 上限先掐在 AX 轉換層（review 發現：選取幾十萬字會整段複製
                // 進來才截 200 字，延遲與記憶體都失控）；4k 字已遠超顯示需求
                if std::time::Instant::now() < deadline {
                    if let Some(sel) =
                        attr_string_capped(&elements.focused, "AXSelectedText", 4_000)
                    {
                        let sel = sel.trim();
                        if !sel.is_empty() {
                            parts.push(format!("Selected: {}", truncate_chars(sel, 200)));
                        }
                    }
                }
            }
        }

        if std::time::Instant::now() < deadline {
            let visible = collect_visible_text(&elements.window, deadline);
            if !visible.is_empty() {
                parts.push(format!("Visible: {visible}"));
            }
        }

        if parts.is_empty() {
            return None;
        }
        if elements.app_id != target.app_id {
            return None;
        }
        Some(ContextSnapshot {
            text: parts.join("\n"),
            app_id: elements.app_id,
            app_name: elements.app_name.clone(),
            surface: classify_surface(&elements.app_name, &elements.bundle_id, &window_title),
            target,
        })
    }

    pub fn capture() -> Option<String> {
        capture_snapshot().map(|snapshot| snapshot.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_webview_context_skip_uses_exact_bundle_match() {
        assert!(should_skip_background_context("dev.claro.desktop"));
        assert!(!should_skip_background_context("DEV.CLARO.DESKTOP"));
        assert!(!should_skip_background_context("dev.claro.desktop.helper"));
        assert!(!should_skip_background_context("com.example.claro"));
    }

    #[test]
    fn terms_prefer_dict_then_context_dedup_and_cap() {
        let dict = vec!["PyTorch".to_string(), "GPT".to_string()];
        let ctx =
            "App: Zed\nWindow: main.rs — claro\nVisible: pytorch tokenizer | whisper.cpp | GPT";
        let terms = context_terms(&dict, ctx, 15);
        assert_eq!(terms[0], "PyTorch");
        assert_eq!(terms[1], "GPT");
        assert!(terms.contains(&"Zed".to_string()));
        assert!(terms.contains(&"whisper.cpp".to_string()));
        // pytorch（小寫）與 GPT 重複 → 去重
        assert_eq!(
            terms
                .iter()
                .filter(|t| t.to_lowercase() == "pytorch")
                .count(),
            1
        );
        assert_eq!(
            terms.iter().filter(|t| t.to_lowercase() == "gpt").count(),
            1
        );
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

    #[test]
    fn cjk_terms_include_names_but_skip_common_ui_phrases() {
        let terms = context_terms(
            &[],
            "App: Mail\nVisible: 負責人：林嘉泉 | Claro 專案 | 目前 | 設定",
            15,
        );
        assert!(terms.contains(&"林嘉泉".to_string()));
        assert!(terms.iter().all(|term| term != "目前" && term != "設定"));
    }

    #[test]
    fn sensitive_apps_are_denied_before_ax_capture() {
        assert!(is_sensitive_app("1Password", "com.1password.1password"));
        assert!(is_sensitive_app("密碼", "com.apple.Passwords"));
        assert!(!is_sensitive_app("Mail", "com.apple.mail"));
    }

    #[test]
    fn surfaces_are_explainable_and_default_neutral() {
        assert_eq!(classify_surface("Mail", "com.apple.mail", "Draft"), "email");
        assert_eq!(
            classify_surface("Slack", "com.tinyspeck.slackmacgap", "Claro"),
            "message"
        );
        assert_eq!(
            classify_surface("Cursor", "com.todesktop.cursor", "main.rs"),
            "technical"
        );
        assert_eq!(
            classify_surface("Preview", "com.apple.Preview", "file.pdf"),
            "neutral"
        );
    }

    #[test]
    fn surface_guidance_never_invents_a_profile_for_unknown_apps() {
        assert!(surface_guidance("App: Mail\nWindow: Draft").starts_with("email"));
        assert!(surface_guidance("App: Preview\nWindow: file.pdf").starts_with("neutral"));
    }
}
