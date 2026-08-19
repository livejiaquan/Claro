import React, { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "@fontsource/baloo-2/700.css";
import "./index.css";
import type {
  DictationEvent,
  DownloadProgress,
  MicLevel,
  PendingResult,
  Status,
} from "./types";
import { visiblePendingResult } from "./dictationState";
import Home from "./pages/Home";
import History from "./pages/History";
import Onboarding from "./pages/Onboarding";
import Settings from "./pages/Settings";
import { IconHistory, IconHome, IconSettings, IconSetup } from "./ui";

type Page = "home" | "history" | "setup" | "settings";

const NAV: { id: Page; label: string; icon: () => React.ReactElement }[] = [
  { id: "home", label: "首頁", icon: IconHome },
  { id: "history", label: "歷史紀錄", icon: IconHistory },
  { id: "setup", label: "首次設定", icon: IconSetup },
  { id: "settings", label: "設定", icon: IconSettings },
];

function dictationFailureCopy(outcome: DictationEvent["outcome"]) {
  switch (outcome) {
    case "focus_changed":
      return {
        title: "文字已辨識，但目前沒有外部貼上位置",
        detail: "結果已保留；先按「複製結果」，再回輸入框按 Cmd+V，不用重講。",
      };
    case "paste_failed":
      return {
        title: "文字已辨識，但 Claro 無法送出貼上",
        detail: "結果已保留；按下「複製結果」即可貼上，不用重講。",
      };
    case "stt_failed":
      return {
        title: "這段聽寫沒有完成",
        detail: "語音模型未能完成辨識；這次沒有可恢復文字，請先檢查模型後再試。",
      };
    case "silent":
      return {
        title: "沒有辨識到足夠的聲音",
        detail: "請確認麥克風有收到聲音，再重新說一次。",
      };
    default:
      return {
        title: "這段聽寫沒有完成",
        detail: "Claro 遇到問題；若下方有保留文字，請直接複製，不用重講。",
      };
  }
}

export default function App() {
  const [page, setPage] = useState<Page>("home");
  const [settingsFocusTarget, setSettingsFocusTarget] = useState<
    "codex" | null
  >(null);
  // 留在 App scope，切頁不會讓尚未成功寫入的 Codex 清單草稿消失。
  const [codexCorrectionDraft, setCodexCorrectionDraft] = useState<
    string | null
  >(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [llmProgress, setLlmProgress] = useState<DownloadProgress | null>(null);
  const [dictationEvent, setDictationEvent] = useState<DictationEvent | null>(null);
  const [pendingResult, setPendingResult] = useState<PendingResult | null>(null);
  const [mic, setMic] = useState<MicLevel>({ level: 0, active: false, generation: 0, passed: false, timed_out: false });
  const micGeneration = useRef(0);
  const dictationSession = useRef(0);
  const [toast, setToast] = useState<string | null>(null);
  const micRef = useRef(false);
  const toastTimer = useRef<number | undefined>(undefined);
  const initialRouteApplied = useRef(false);

  const refresh = useCallback(() =>
    invoke<Status>("get_status")
      .then((next) => {
        setStatus(next);
        setStatusError(null);
      })
      .catch((reason) => setStatusError(String(reason))), []);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 1800);
  }, []);

  const loadPendingResult = useCallback(() => {
    invoke<PendingResult | null>("get_pending_result")
      .then(setPendingResult)
      .catch(() => setPendingResult(null));
  }, []);

  const copyPendingResult = useCallback(() => {
    if (!pendingResult) return;
    invoke("copy_text", { text: pendingResult.text })
      .then(() => showToast("已複製保留文字，這次不用重講"))
      .catch(() => showToast("複製失敗，請到歷史紀錄再試"));
  }, [pendingResult, showToast]);

  useEffect(() => {
    if (
      progress &&
      progress.activation_status !== "none" &&
      status?.model_id === progress.model_id
    ) {
      setProgress(null);
    }
  }, [progress, status?.model_id]);

  useEffect(() => {
    refresh();
    loadPendingResult();
    let timer: number | undefined;
    const syncPolling = () => {
      if (timer !== undefined) window.clearInterval(timer);
      timer = undefined;
      // 主視窗被 hide 時不再每 2 秒列舉 CoreAudio 裝置與模型狀態。
      if (document.visibilityState === "visible") {
        refresh();
        timer = window.setInterval(refresh, 2000);
      }
    };
    document.addEventListener("visibilitychange", syncPolling);
    syncPolling();
    const un1 = listen<DownloadProgress>("model-download", (e) => {
      setProgress(e.payload.done && e.payload.activation_status === "none" ? null : e.payload);
      if (e.payload.activation_status === "waiting_for_idle") {
        showToast("模型已下載；本次聽寫結束後請按「使用」切換");
      } else if (e.payload.activation_status === "retry_required") {
        showToast("模型已下載，但切換失敗；請到設定重試使用");
      } else if (e.payload.error) {
        showToast(
          e.payload.error.includes("下載已取消")
            ? "已取消下載；已完成的部分保留，下次會續傳"
            : `下載失敗：${e.payload.error}`,
        );
      } else if (e.payload.done) {
        showToast("模型下載完成");
      }
      if (e.payload.downloaded || e.payload.done) refresh();
    });
    const un2 = listen<MicLevel>("mic-level", (e) => {
      if (e.payload.generation < micGeneration.current) return;
      micGeneration.current = e.payload.generation;
      micRef.current = e.payload.active;
      setMic(e.payload);
    });
    const un3 = listen<DownloadProgress>("llm-model-download", (e) => {
      setLlmProgress(e.payload.done ? null : e.payload);
      if (e.payload.error)
        showToast(
          // 使用者主動取消不是失敗；後端用固定訊息讓前端能區分。
          e.payload.error.includes("下載已取消")
            ? "已取消下載；已完成的部分保留，下次會續傳"
            : `下載失敗：${e.payload.error}`,
        );
      if (e.payload.done) showToast("模型下載完成");
    });
    const un4 = listen<DictationEvent>("dictation-status", (e) => {
      const next = e.payload;
      if (next.session < dictationSession.current) return;
      dictationSession.current = next.session;
      setDictationEvent(next);
      if (next.phase === "finished" && next.recovery_available) {
        // backend 先把 pending result 放入 queue，再送 terminal event；不把
        // transcript 放進 event，這裡只取回已保留的本機結果。
        loadPendingResult();
      }
      refresh();
    });
    const un5 = listen<string>("navigate-page", (e) => {
      if (e.payload === "history") setPage("history");
    });
    return () => {
      if (timer !== undefined) window.clearInterval(timer);
      document.removeEventListener("visibilitychange", syncPolling);
      un1.then((f) => f());
      un2.then((f) => f());
      un3.then((f) => f());
      un4.then((f) => f());
      un5.then((f) => f());
      if (micRef.current) invoke("mic_test_stop").catch(() => {});
    };
  }, [loadPendingResult, refresh, showToast]);

  useEffect(() => {
    // History can discard or clear the backend recovery queue. Refresh when the
    // user leaves that page so a stale parent copy cannot reappear as a banner.
    if (page !== "history") loadPendingResult();
  }, [loadPendingResult, page]);

  // 第一次開啟尚未完成設定時，直接帶使用者進入引導；之後仍可自由切換頁面。
  useEffect(() => {
    if (!status || initialRouteApplied.current) return;
    initialRouteApplied.current = true;
    if (!status.setup_completed) setPage("setup");
  }, [status]);

  // 離開設定／首次設定頁自動停止麥克風測試
  useEffect(() => {
    if (page !== "settings" && page !== "setup" && micRef.current) invoke("mic_test_stop").catch(() => {});
  }, [page]);

  // 視窗拖曳：Tauri v2 內建的 data-tauri-drag-region 只認 event.target「本身」
  // 有沒有該屬性——logo、標題等子元素全不觸發，實際幾乎拖不動（使用者實測）。
  // 改成自己監聽 mousedown 用 closest() 判定：拖曳區的子元素也可拖，
  // 互動元件（按鈕/輸入框/.no-drag）排除。
  useEffect(() => {
    const onMouseDown = (e: MouseEvent) => {
      if (e.button !== 0 || e.detail > 1) return;
      const el = e.target as HTMLElement | null;
      if (!el || typeof el.closest !== "function") return;
      if (el.closest("button, a, input, select, textarea, [role='button'], .no-drag")) return;
      if (!el.closest("[data-tauri-drag-region]")) return;
      getCurrentWindow().startDragging().catch(() => {});
    };
    document.addEventListener("mousedown", onMouseDown);
    return () => document.removeEventListener("mousedown", onMouseDown);
  }, []);

  const ready = Boolean(
    status &&
    status.model_present &&
    status.accessibility &&
    status.hotkey_active &&
    status.setup_completed,
  );
  const needsSetup = Boolean(status && !ready);
  const dictationProcessing = dictationEvent?.phase === "processing";
  const dictationFailed =
    dictationEvent?.phase === "finished" &&
    dictationEvent.outcome !== "pasted" &&
    dictationEvent.outcome !== "cancelled";
  const currentPendingResult = visiblePendingResult(pendingResult, dictationEvent);
  const showRecovery = Boolean(currentPendingResult);
  const failureCopy = dictationFailureCopy(dictationEvent?.outcome ?? "error");

  return (
    <div className="app-shell flex h-screen">
      <a className="skip-link no-drag" href="#main-content">
        跳至主要內容
      </a>
      {/* 側欄 */}
      {/* data-tauri-drag-region：Tauri 的拖曳靠這個屬性（-webkit-app-region 是 Electron 的，無效） */}
      <aside
        data-tauri-drag-region
        className="app-sidebar w-[210px] shrink-0 flex flex-col px-3 pb-4 pt-11 titlebar-drag"
        style={{ borderRight: "1px solid var(--hairline)" }}
      >
        <div data-tauri-drag-region className="flex items-center px-2 mb-6">
          <span className="logo-wave" aria-hidden>
            <i />
            <i />
            <i />
          </span>
          <span className="wordmark">Claro</span>
        </div>

        <nav className="space-y-1" aria-label="主要導覽">
          {NAV.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              className={`nav-item no-drag ${page === id ? "active" : ""}`}
              onClick={() => {
                if (id === "settings") setSettingsFocusTarget(null);
                setPage(id);
              }}
              aria-current={page === id ? "page" : undefined}
            >
              <Icon />
              {label}
              {id === "setup" && needsSetup && (
                <span className="dot ml-auto" style={{ background: "var(--amber)" }} aria-label="尚未就緒" />
              )}
            </button>
          ))}
        </nav>

        <div className="mt-auto px-2 space-y-1.5">
          {status && (
            <div className="flex items-center gap-2 text-[12px]" style={{ color: "var(--muted)" }}>
              <span
                className={`dot ${status.dictation_state !== "idle" ? "pulse" : ""}`}
                style={{
                  background:
                    status.dictation_state === "recording"
                      ? "var(--red)"
                      : status.dictation_state === "processing"
                        ? "var(--accent)"
                        : ready
                          ? "var(--green)"
                          : "var(--amber)",
                }}
              />
              {status.dictation_state === "recording"
                ? "錄音中"
                : status.dictation_state === "processing"
                  ? "處理中"
                  : ready
                    ? "待命中"
                    : "尚未就緒"}
            </div>
          )}
          <div className="text-[11px]" style={{ color: "var(--faint)" }}>
            Claro v0.1.0
          </div>
        </div>
      </aside>

      {/* 內容 */}
      <main className="app-main flex-1 overflow-y-auto" id="main-content" tabIndex={-1}>
        <div data-tauri-drag-region className="titlebar-drag h-9 sticky top-0 z-10" />
        <div className="px-9 pb-10 max-w-[880px]">
          {dictationProcessing && (
            <div
              className="dictation-feedback processing"
              role="status"
              aria-live="polite"
              aria-busy="true"
            >
              <span className="dictation-feedback-icon dot pulse" aria-hidden="true" />
              <div className="dictation-feedback-copy">
                <strong>正在處理這段聽寫…</strong>
                <p>請稍候，完成後會自動貼上；需要停止時可以按 Esc。</p>
              </div>
            </div>
          )}

          {dictationFailed && (
            <div
              className={`dictation-feedback ${currentPendingResult ? "recovery" : "failure"}`}
              role="alert"
              aria-live="assertive"
            >
              <span className="dictation-feedback-icon" aria-hidden="true">
                {currentPendingResult ? "!" : "×"}
              </span>
              <div className="dictation-feedback-copy">
                <strong>{failureCopy.title}</strong>
                <p>{failureCopy.detail}</p>
                {currentPendingResult && (
                  <p className="dictation-recovery-text select-text">{currentPendingResult.text}</p>
                )}
              </div>
              <div className="dictation-feedback-actions">
                {currentPendingResult && (
                  <button className="btn-primary no-drag" onClick={copyPendingResult}>
                    複製結果
                  </button>
                )}
                <button className="btn no-drag" onClick={() => setPage("history")}>
                  查看歷史
                </button>
              </div>
            </div>
          )}

          {showRecovery && !dictationFailed && page !== "history" && (
            <div
              className="dictation-feedback recovery"
              role="status"
              aria-live="polite"
            >
              <span className="dictation-feedback-icon" aria-hidden="true">!</span>
              <div className="dictation-feedback-copy">
                <strong>有一段文字尚未自動貼上</strong>
                <p>結果已保留在 Claro；你可以直接複製，不用重講。</p>
                <p className="dictation-recovery-text select-text">{currentPendingResult?.text}</p>
              </div>
              <div className="dictation-feedback-actions">
                <button className="btn-primary no-drag" onClick={copyPendingResult}>
                  複製結果
                </button>
                <button className="btn no-drag" onClick={() => setPage("history")}>
                  查看歷史
                </button>
              </div>
            </div>
          )}

          {!status && statusError ? (
            <div className="page-in status-fatal" role="alert">
              <h1>無法讀取 Claro 狀態</h1>
              <p>{statusError}</p>
              <button className="btn-primary no-drag" onClick={refresh}>重新載入</button>
            </div>
          ) : !status ? (
            <div className="page-in space-y-4 pt-2" aria-busy>
              <div className="h-8 w-64 rounded-lg" style={{ background: "rgba(0,0,0,0.06)" }} />
              <div className="h-4 w-96 rounded-lg" style={{ background: "rgba(0,0,0,0.05)" }} />
              <div className="grid grid-cols-4 gap-3 pt-3">
                {[0, 1, 2, 3].map((i) => (
                  <div key={i} className="h-28 rounded-[14px]" style={{ background: "rgba(0,0,0,0.05)" }} />
                ))}
              </div>
            </div>
          ) : page === "home" ? (
            <Home
              status={status}
              onCopied={() => showToast("已複製")}
              gotoHistory={() => setPage("history")}
              gotoSetup={() => setPage("setup")}
              gotoSettings={() => {
                setSettingsFocusTarget(null);
                setPage("settings");
              }}
            />
          ) : page === "history" ? (
            <History
              onCopied={() => showToast("已複製")}
              historyEnabled={status.history_enabled}
            />
          ) : page === "setup" ? (
            <Onboarding
              status={status}
              mic={mic}
              progress={progress}
              llmProgress={llmProgress}
              onDownloadStart={(kind) =>
                kind === "stt" ? setProgress(null) : setLlmProgress(null)
              }
              refresh={refresh}
              onToast={showToast}
              onDone={() => {
                if (status.setup_completed) {
                  setPage("home");
                  return;
                }
                invoke("complete_setup")
                  .then(() => {
                    refresh();
                    setPage("home");
                  })
                  .catch((reason) => showToast(`無法完成設定：${String(reason)}`));
              }}
              onOpenSettings={() => {
                setSettingsFocusTarget("codex");
                setPage("settings");
              }}
            />
          ) : (
            <Settings
              status={status}
              mic={mic}
              progress={progress}
              llmProgress={llmProgress}
              onDownloadStart={(kind) =>
                kind === "stt" ? setProgress(null) : setLlmProgress(null)
              }
              refresh={refresh}
              onToast={showToast}
              onOpenSetup={() => setPage("setup")}
              focusTarget={settingsFocusTarget}
              onFocusTargetHandled={() => setSettingsFocusTarget(null)}
              codexCorrectionDraft={codexCorrectionDraft}
              onCodexCorrectionDraftChange={setCodexCorrectionDraft}
            />
          )}
        </div>
      </main>

      {toast && <div className="toast" role="status" aria-live="polite">{toast}</div>}
    </div>
  );
}
