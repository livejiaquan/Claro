import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "@fontsource/baloo-2/700.css";
import "./index.css";
import type { DownloadProgress, MicLevel, Status } from "./types";
import Home from "./pages/Home";
import History from "./pages/History";
import Settings from "./pages/Settings";
import { IconHistory, IconHome, IconSettings } from "./ui";

type Page = "home" | "history" | "settings";

const NAV: { id: Page; label: string; icon: () => React.ReactElement }[] = [
  { id: "home", label: "首頁", icon: IconHome },
  { id: "history", label: "歷史紀錄", icon: IconHistory },
  { id: "settings", label: "設定", icon: IconSettings },
];

export default function App() {
  const [page, setPage] = useState<Page>("home");
  const [status, setStatus] = useState<Status | null>(null);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [mic, setMic] = useState<MicLevel>({ level: 0, active: false });
  const [toast, setToast] = useState<string | null>(null);
  const micRef = useRef(false);
  micRef.current = mic.active;
  const toastTimer = useRef<number | undefined>(undefined);

  const refresh = () =>
    invoke<Status>("get_status")
      .then(setStatus)
      .catch(() => {});

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 2000);
    const un1 = listen<DownloadProgress>("model-download", (e) => {
      setProgress(e.payload.done || e.payload.error ? null : e.payload);
      if (e.payload.error) showToast(`下載失敗：${e.payload.error}`);
      if (e.payload.done) {
        showToast("模型下載完成");
        refresh();
      }
    });
    const un2 = listen<MicLevel>("mic-level", (e) => setMic(e.payload));
    return () => {
      clearInterval(t);
      un1.then((f) => f());
      un2.then((f) => f());
      if (micRef.current) invoke("mic_test_stop").catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 離開設定頁自動停止麥克風測試
  useEffect(() => {
    if (page !== "settings" && micRef.current) invoke("mic_test_stop").catch(() => {});
  }, [page]);

  const showToast = (msg: string) => {
    setToast(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 1800);
  };

  const ready = status && status.model_present && status.accessibility && status.hotkey_active;
  const needsSetup = status && !ready;

  return (
    <div className="flex h-screen">
      {/* 側欄 */}
      <aside
        className="w-[210px] shrink-0 flex flex-col px-3 pb-4 pt-11 titlebar-drag"
        style={{ borderRight: "1px solid var(--hairline)" }}
      >
        <div className="flex items-center px-2 mb-6">
          <span className="logo-wave" aria-hidden>
            <i />
            <i />
            <i />
          </span>
          <span className="wordmark">Claro</span>
        </div>

        <nav className="space-y-1">
          {NAV.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              className={`nav-item no-drag ${page === id ? "active" : ""}`}
              onClick={() => setPage(id)}
            >
              <Icon />
              {label}
              {id === "settings" && needsSetup && (
                <span className="dot ml-auto" style={{ background: "var(--amber)" }} />
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
            whisper-{status?.model_id ?? "…"}・v0.1.0
          </div>
        </div>
      </aside>

      {/* 內容 */}
      <main className="flex-1 overflow-y-auto">
        <div className="titlebar-drag h-9 sticky top-0 z-10" />
        <div className="px-9 pb-10 max-w-[880px]">
          {!status ? (
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
            <Home status={status} onCopied={() => showToast("已複製")} gotoHistory={() => setPage("history")} />
          ) : page === "history" ? (
            <History onCopied={() => showToast("已複製")} />
          ) : (
            <Settings status={status} mic={mic} progress={progress} refresh={refresh} />
          )}
        </div>
      </main>

      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}
