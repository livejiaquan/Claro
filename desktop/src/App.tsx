import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "@fontsource/baloo-2/700.css";
import "./index.css";
import type { DownloadProgress, MicLevel, Status } from "./types";
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

export default function App() {
  const [page, setPage] = useState<Page>("home");
  const [status, setStatus] = useState<Status | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [llmProgress, setLlmProgress] = useState<DownloadProgress | null>(null);
  const [mic, setMic] = useState<MicLevel>({ level: 0, active: false, generation: 0, passed: false, timed_out: false });
  const micGeneration = useRef(0);
  const [toast, setToast] = useState<string | null>(null);
  const micRef = useRef(false);
  micRef.current = mic.active;
  const toastTimer = useRef<number | undefined>(undefined);
  const initialRouteApplied = useRef(false);

  const refresh = () =>
    invoke<Status>("get_status")
      .then((next) => {
        setStatus(next);
        setStatusError(null);
      })
      .catch((reason) => setStatusError(String(reason)));

  useEffect(() => {
    refresh();
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
      setProgress(e.payload.done ? null : e.payload);
      if (e.payload.error) showToast(`下載失敗：${e.payload.error}`);
      if (e.payload.done) {
        showToast("模型下載完成");
        refresh();
      }
    });
    const un2 = listen<MicLevel>("mic-level", (e) => {
      if (e.payload.generation < micGeneration.current) return;
      micGeneration.current = e.payload.generation;
      setMic(e.payload);
    });
    const un3 = listen<DownloadProgress>("llm-model-download", (e) => {
      setLlmProgress(e.payload.done ? null : e.payload);
      if (e.payload.error) showToast(`下載失敗：${e.payload.error}`);
      if (e.payload.done) showToast("模型下載完成");
    });
    return () => {
      if (timer !== undefined) window.clearInterval(timer);
      document.removeEventListener("visibilitychange", syncPolling);
      un1.then((f) => f());
      un2.then((f) => f());
      un3.then((f) => f());
      if (micRef.current) invoke("mic_test_stop").catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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

  const showToast = (msg: string) => {
    setToast(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 1800);
  };

  const ready = Boolean(
    status &&
    status.model_present &&
    status.accessibility &&
    status.hotkey_active &&
    status.setup_completed,
  );
  const needsSetup = Boolean(status && !ready);

  return (
    <div className="app-shell flex h-screen">
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
              onClick={() => setPage(id)}
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
      <main className="app-main flex-1 overflow-y-auto" id="main-content">
        <div data-tauri-drag-region className="titlebar-drag h-9 sticky top-0 z-10" />
        <div className="px-9 pb-10 max-w-[880px]">
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
            />
          ) : (
            <Settings
              status={status}
              mic={mic}
              progress={progress}
              llmProgress={llmProgress}
              refresh={refresh}
              onToast={showToast}
              onOpenSetup={() => setPage("setup")}
            />
          )}
        </div>
      </main>

      {toast && <div className="toast" role="status" aria-live="polite">{toast}</div>}
    </div>
  );
}
