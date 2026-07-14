//! 文字注入：剪貼簿全格式備份 → 寫入 → 合成 Cmd+V → 還原（SPEC §9）。
//! macOS 走 NSPasteboard item/type 層級的 eager snapshot；任一 type 無法讀取就
//! fail-closed，絕不清空使用者原本的剪貼簿。還原前再用 changeCount 與私有
//! marker 確認剪貼簿仍由 Claro 持有，避免覆蓋使用者新的 Copy。
//! 還原延遲 300ms（SPEC D8：Handy 的 50ms 會 race 慢的目標 app）。
//! CJK IME 輸入源防護在 M4（SPEC D9）。

use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

pub trait TextInjector: Send + Sync {
    /// `pre_paste_commit` 會在 50ms clipboard settle 後取得真正的 Cmd+V closure。
    /// 呼叫端必須在同一個 session/state lock 內完成最後驗證並執行 closure，
    /// 讓 Esc／新 session 不可能插在「guard=true」與實際貼上之間。
    fn inject(
        &self,
        text: &str,
        pre_paste_commit: &dyn Fn(&dyn Fn() -> Result<()>) -> Result<()>,
    ) -> Result<()>;
}

pub struct MacPasteInjector;

/// macOS kVK_ANSI_V；用虛擬鍵碼而非字元，避免鍵盤配置差異
const VK_V: u32 = 9;

impl TextInjector for MacPasteInjector {
    fn inject(
        &self,
        text: &str,
        pre_paste_commit: &dyn Fn(&dyn Fn() -> Result<()>) -> Result<()>,
    ) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            objc2::rc::autoreleasepool(|_| inject_macos(text, pre_paste_commit))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (text, pre_paste_commit);
            bail!("clipboard injection is only supported on macOS")
        }
    }
}

#[cfg(target_os = "macos")]
fn inject_macos(
    text: &str,
    pre_paste_commit: &dyn Fn(&dyn Fn() -> Result<()>) -> Result<()>,
) -> Result<()> {
    use objc2_app_kit::NSPasteboard;

    // 整個 snapshot→temporary write→Cmd+V→restore 是一筆不可交錯的交易。
    // 否則 B 可能在 A 的 300ms settle 期間把 A 的暫存內容當成「原剪貼簿」，
    // 最後讓真正的使用者剪貼簿永久遺失。
    let _transaction = paste_transaction_lock()
        .lock()
        .map_err(|_| anyhow!("clipboard transaction lock poisoned"))?;

    let pasteboard = NSPasteboard::generalPasteboard();
    // snapshot 必須在 clearContents 之前完整成功；任一格式無法 eager 讀取就不注入。
    let snapshot = PasteboardSnapshot::capture(&pasteboard)?;
    let marker = make_marker();
    let injected_change_count = write_injected_text(&pasteboard, text, &marker, &snapshot)?;
    thread::sleep(Duration::from_millis(50));

    // 50ms 是給 clipboard manager/目標 app 的反應窗口；真正送 Cmd+V 前必須
    // 再驗一次 ownership。若期間內容被改寫，直接中止，不能把替代內容貼出去。
    let pre_paste_change_count = pasteboard.changeCount();
    let pre_paste_marker = current_marker(&pasteboard);
    // 貼上失敗也必須嘗試還原剪貼簿。成功時等目標 app 讀取；失敗時可立即還原。
    let paste_result = paste_if_owned(
        injected_change_count,
        pre_paste_change_count,
        &marker,
        pre_paste_marker.as_deref(),
        || pre_paste_commit(&send_cmd_v),
    )
    .context("synthesize Cmd+V");
    if paste_result.is_ok() {
        thread::sleep(Duration::from_millis(300));
    }

    let current_change_count = pasteboard.changeCount();
    let current_marker = current_marker(&pasteboard);
    let restore_result = if should_restore(
        injected_change_count,
        current_change_count,
        &marker,
        current_marker.as_deref(),
    ) {
        snapshot.restore(&pasteboard).context("restore clipboard")
    } else {
        // 使用者或 clipboard manager 已寫入新內容；不可用舊 snapshot 覆蓋。
        Ok(())
    };

    match (paste_result, restore_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(paste), Ok(())) => Err(paste),
        (Ok(()), Err(restore)) => Err(restore),
        (Err(paste), Err(restore)) => {
            Err(anyhow!("{paste}; clipboard restore also failed: {restore}"))
        }
    }
}

fn paste_transaction_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(target_os = "macos")]
const MARKER_TYPE: &str = "dev.claro.paste-owner";

/// 將 NSPasteboard 的每個 item/type 轉成自有 bytes，不保留 lazy data provider。
/// 這是 clearContents 後仍能完整重建剪貼簿的關鍵。
#[cfg(target_os = "macos")]
struct PasteboardSnapshot {
    items: Vec<Vec<(String, Vec<u8>)>>,
}

#[cfg(target_os = "macos")]
impl PasteboardSnapshot {
    fn capture(pasteboard: &objc2_app_kit::NSPasteboard) -> Result<Self> {
        let Some(items) = pasteboard.pasteboardItems() else {
            return Ok(Self { items: Vec::new() });
        };
        let mut snapshot = Vec::with_capacity(items.len());
        for (item_index, item) in items.iter().enumerate() {
            let types = item.types();
            if types.is_empty() {
                bail!("clipboard item {item_index} has no readable types");
            }
            let mut saved_item = Vec::with_capacity(types.len());
            for data_type in types.iter() {
                let type_name = data_type.to_string();
                let data = item.dataForType(&data_type).with_context(|| {
                    format!("clipboard item {item_index} type '{type_name}' is not materialized")
                })?;
                saved_item.push((type_name, data.to_vec()));
            }
            snapshot.push(saved_item);
        }
        Ok(Self { items: snapshot })
    }

    fn restore(&self, pasteboard: &objc2_app_kit::NSPasteboard) -> Result<()> {
        use objc2::runtime::ProtocolObject;
        use objc2_app_kit::{NSPasteboardItem, NSPasteboardWriting};
        use objc2_foundation::{NSArray, NSData, NSString};

        let mut objects = Vec::with_capacity(self.items.len());
        for (item_index, saved_item) in self.items.iter().enumerate() {
            let item = NSPasteboardItem::new();
            for (type_name, bytes) in saved_item {
                let data_type = NSString::from_str(type_name);
                let data = NSData::with_bytes(bytes);
                if !item.setData_forType(&data, &data_type) {
                    bail!("could not rebuild clipboard item {item_index} type '{type_name}'");
                }
            }
            let object: objc2::rc::Retained<ProtocolObject<dyn NSPasteboardWriting>> =
                ProtocolObject::from_retained(item);
            objects.push(object);
        }

        pasteboard.clearContents();
        if objects.is_empty() {
            return Ok(());
        }
        let objects = NSArray::from_retained_slice(&objects);
        if !pasteboard.writeObjects(&objects) {
            bail!("NSPasteboard.writeObjects returned false");
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn write_injected_text(
    pasteboard: &objc2_app_kit::NSPasteboard,
    text: &str,
    marker: &[u8],
    snapshot: &PasteboardSnapshot,
) -> Result<isize> {
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSData, NSString};

    let item = NSPasteboardItem::new();
    let text = NSString::from_str(text);
    // SAFETY: AppKit exports this process-lifetime NSString constant.
    let string_type = unsafe { NSPasteboardTypeString };
    if !item.setString_forType(&text, string_type) {
        bail!("could not set clipboard text");
    }
    let marker_type = NSString::from_str(MARKER_TYPE);
    let marker_data = NSData::with_bytes(marker);
    if !item.setData_forType(&marker_data, &marker_type) {
        bail!("could not set clipboard ownership marker");
    }
    let object: objc2::rc::Retained<ProtocolObject<dyn NSPasteboardWriting>> =
        ProtocolObject::from_retained(item);
    let objects = NSArray::from_retained_slice(&[object]);

    pasteboard.clearContents();
    if !pasteboard.writeObjects(&objects) {
        // clearContents 已經破壞原剪貼簿；即使注入寫入失敗也要立即回復。
        return match snapshot.restore(pasteboard) {
            Ok(()) => Err(anyhow!("NSPasteboard.writeObjects returned false")),
            Err(restore) => Err(anyhow!(
                "NSPasteboard.writeObjects returned false; clipboard restore also failed: {restore}"
            )),
        };
    }
    Ok(pasteboard.changeCount())
}

#[cfg(target_os = "macos")]
fn current_marker(pasteboard: &objc2_app_kit::NSPasteboard) -> Option<Vec<u8>> {
    use objc2_foundation::NSString;
    let marker_type = NSString::from_str(MARKER_TYPE);
    pasteboard
        .dataForType(&marker_type)
        .map(|data| data.to_vec())
}

#[cfg(target_os = "macos")]
fn make_marker() -> Vec<u8> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}-{sequence}", std::process::id()).into_bytes()
}

/// 純邏輯：只有 changeCount 未變且 marker 仍是這次注入值才能還原。
/// 抽出來單測，避免測試觸碰使用者的 general pasteboard。
fn should_restore(
    injected_change_count: isize,
    current_change_count: isize,
    expected_marker: &[u8],
    current_marker: Option<&[u8]>,
) -> bool {
    injected_change_count == current_change_count && current_marker == Some(expected_marker)
}

/// 貼上前 ownership gate。closure 只在 changeCount 與私有 marker 都仍匹配時
/// 才會執行，讓測試能證明 clipboard 被替換時完全不會送出 Cmd+V。
fn paste_if_owned(
    injected_change_count: isize,
    current_change_count: isize,
    expected_marker: &[u8],
    current_marker: Option<&[u8]>,
    paste: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if !should_restore(
        injected_change_count,
        current_change_count,
        expected_marker,
        current_marker,
    ) {
        bail!("clipboard ownership changed before paste; paste aborted");
    }
    paste()
}

fn send_cmd_v() -> Result<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).context("init enigo")?;
    run_cmd_v_sequence(|action| {
        let (key, direction) = match action {
            CmdVAction::PressMeta => (Key::Meta, Direction::Press),
            CmdVAction::ClickV => (Key::Other(VK_V), Direction::Click),
            CmdVAction::ReleaseMeta => (Key::Meta, Direction::Release),
        };
        enigo.key(key, direction).map_err(Into::into)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmdVAction {
    PressMeta,
    ClickV,
    ReleaseMeta,
}

/// 將鍵序列抽成純邏輯，讓測試能證明 Click 失敗時仍一定送出 Meta release，
/// 又不必合成任何全域鍵盤事件。
fn run_cmd_v_sequence(mut send: impl FnMut(CmdVAction) -> Result<()>) -> Result<()> {
    send(CmdVAction::PressMeta)?;
    // Click 失敗也不能用 `?` 早退，否則會把 Meta 留在邏輯按下狀態。
    let click_result = send(CmdVAction::ClickV);
    let release_result = send(CmdVAction::ReleaseMeta);
    match (click_result, release_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(click), Ok(())) => Err(click),
        (Ok(()), Err(release)) => Err(release),
        (Err(click), Err(release)) => Err(anyhow!(
            "Cmd+V click failed: {click}; Meta release also failed: {release}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        paste_if_owned, paste_transaction_lock, run_cmd_v_sequence, should_restore, CmdVAction,
    };
    use anyhow::bail;

    #[test]
    fn restore_only_when_change_count_and_marker_match() {
        let marker = b"claro-owner";
        assert!(should_restore(12, 12, marker, Some(marker)));
        assert!(!should_restore(12, 13, marker, Some(marker)));
        assert!(!should_restore(12, 12, marker, Some(b"someone-else")));
        assert!(!should_restore(12, 12, marker, None));
    }

    #[test]
    fn pre_paste_guard_never_calls_paste_after_clipboard_replacement() {
        let marker = b"claro-owner";
        let mut called = false;
        let error = paste_if_owned(12, 13, marker, Some(b"replacement"), || {
            called = true;
            Ok(())
        })
        .unwrap_err();

        assert!(!called);
        assert!(error.to_string().contains("paste aborted"));
    }

    #[test]
    fn pre_paste_guard_calls_paste_only_while_ownership_matches() {
        let marker = b"claro-owner";
        let mut called = false;
        paste_if_owned(12, 12, marker, Some(marker), || {
            called = true;
            Ok(())
        })
        .unwrap();
        assert!(called);
    }

    #[test]
    fn changed_target_never_calls_paste_even_when_clipboard_is_owned() {
        let marker = b"claro-owner";
        let mut called = false;
        let error = paste_if_owned(12, 12, marker, Some(marker), || {
            let target_allowed = false;
            if !target_allowed {
                bail!("paste target or session changed before paste; paste aborted");
            }
            called = true;
            Ok(())
        })
        .unwrap_err();

        assert!(!called);
        assert!(error.to_string().contains("target or session changed"));
    }

    #[test]
    fn clipboard_transaction_lock_rejects_interleaving() {
        let guard = paste_transaction_lock().lock().unwrap();
        assert!(paste_transaction_lock().try_lock().is_err());
        drop(guard);
        assert!(paste_transaction_lock().try_lock().is_ok());
    }

    #[test]
    fn cmd_v_releases_meta_even_when_click_fails() {
        let mut actions = Vec::new();
        let error = run_cmd_v_sequence(|action| {
            actions.push(action);
            if action == CmdVAction::ClickV {
                bail!("simulated click failure");
            }
            Ok(())
        })
        .unwrap_err();

        assert!(error.to_string().contains("simulated click failure"));
        assert_eq!(
            actions,
            vec![
                CmdVAction::PressMeta,
                CmdVAction::ClickV,
                CmdVAction::ReleaseMeta
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_snapshot_restores_all_items_and_formats() {
        use super::PasteboardSnapshot;
        use objc2::runtime::ProtocolObject;
        use objc2_app_kit::{
            NSApplication, NSPasteboard, NSPasteboardItem, NSPasteboardTypeHTML,
            NSPasteboardTypeString, NSPasteboardWriting,
        };
        use objc2_foundation::{NSArray, NSData, NSString};

        objc2::rc::autoreleasepool(|_| {
            assert!(NSApplication::load());
            let pasteboard_name = NSString::from_str(&format!(
                "dev.claro.tests.{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let pasteboard = NSPasteboard::pasteboardWithName(&pasteboard_name);
            let text_type = unsafe { NSPasteboardTypeString };
            let html_type = unsafe { NSPasteboardTypeHTML };
            let binary_type = NSString::from_str("dev.claro.test-binary");

            let first = NSPasteboardItem::new();
            assert!(first.setString_forType(&NSString::from_str("plain text"), text_type));
            assert!(first.setData_forType(&NSData::with_bytes(&[0, 1, 2, 255]), &binary_type));
            let second = NSPasteboardItem::new();
            assert!(second.setData_forType(&NSData::with_bytes(b"<b>rich</b>"), html_type));

            let first: objc2::rc::Retained<ProtocolObject<dyn NSPasteboardWriting>> =
                ProtocolObject::from_retained(first);
            let second: objc2::rc::Retained<ProtocolObject<dyn NSPasteboardWriting>> =
                ProtocolObject::from_retained(second);
            let objects = NSArray::from_retained_slice(&[first, second]);
            let initial_count = pasteboard.changeCount();
            pasteboard.clearContents();
            assert!(pasteboard.writeObjects(&objects));
            let populated_count = pasteboard.changeCount();
            assert!(populated_count > initial_count);

            let snapshot = PasteboardSnapshot::capture(&pasteboard).unwrap();
            pasteboard.clearContents();
            assert!(pasteboard.changeCount() > populated_count);
            snapshot.restore(&pasteboard).unwrap();

            let restored = pasteboard.pasteboardItems().unwrap();
            assert_eq!(restored.len(), 2);
            let mut restored = restored.iter();
            let restored_first = restored.next().unwrap();
            let restored_second = restored.next().unwrap();
            assert_eq!(
                restored_first.stringForType(text_type).unwrap().to_string(),
                "plain text"
            );
            assert_eq!(
                restored_first.dataForType(&binary_type).unwrap().to_vec(),
                vec![0, 1, 2, 255]
            );
            assert_eq!(
                restored_second.dataForType(html_type).unwrap().to_vec(),
                b"<b>rich</b>"
            );
            pasteboard.clearContents();
        });
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_pre_paste_guard_detects_replaced_named_pasteboard() {
        use super::{current_marker, write_injected_text, PasteboardSnapshot};
        use objc2_app_kit::{NSApplication, NSPasteboard};
        use objc2_foundation::NSString;

        objc2::rc::autoreleasepool(|_| {
            assert!(NSApplication::load());
            let pasteboard_name = NSString::from_str(&format!(
                "dev.claro.tests.pre-paste.{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let pasteboard = NSPasteboard::pasteboardWithName(&pasteboard_name);
            pasteboard.clearContents();
            let snapshot = PasteboardSnapshot::capture(&pasteboard).unwrap();

            let marker = b"claro-owner";
            let injected_count =
                write_injected_text(&pasteboard, "Claro text", marker, &snapshot).unwrap();
            let mut first_called = false;
            paste_if_owned(
                injected_count,
                pasteboard.changeCount(),
                marker,
                current_marker(&pasteboard).as_deref(),
                || {
                    first_called = true;
                    Ok(())
                },
            )
            .unwrap();
            assert!(first_called);

            // 模擬 clipboard manager 在 50ms 內換掉內容；測試 closure 不會送鍵。
            write_injected_text(&pasteboard, "replacement", b"other-owner", &snapshot).unwrap();
            let mut replacement_pasted = false;
            let error = paste_if_owned(
                injected_count,
                pasteboard.changeCount(),
                marker,
                current_marker(&pasteboard).as_deref(),
                || {
                    replacement_pasted = true;
                    Ok(())
                },
            )
            .unwrap_err();
            assert!(!replacement_pasted);
            assert!(error.to_string().contains("paste aborted"));
            pasteboard.clearContents();
        });
    }
}
