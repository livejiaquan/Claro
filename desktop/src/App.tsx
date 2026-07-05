import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./index.css";

interface Status {
  model_id: string;
  model_present: boolean;
  model_approx_mb: number;
  accessibility: boolean;
  hotkey_active: boolean;
  input_device: string | null;
  default_input: string | null;
  input_devices: string[];
  dictation_state: "idle" | "recording" | "processing";
}

interface DownloadProgress {
  downloaded_mb: number;
  total_mb: number | null;
  done: boolean;
  error: string | null;
}

interface MicLevel {
  level: number;
  active: boolean;
}

function Row({ ok, label, hint }: { ok: boolean; label: string; hint?: string }) {
  return (
    <div className="flex items-start gap-3 py-2.5">
      <span className={`mt-0.5 text-lg leading-none ${ok ? "text-green-500" : "text-amber-500"}`}>
        {ok ? "✓" : "●"}
      </span>
      <div>
        <div className="font-medium">{label}</div>
        {hint && !ok && <div className="text-sm text-neutral-400 mt-0.5">{hint}</div>}
      </div>
    </div>
  );
}

const STATE_LABEL = { idle: "待命", recording: "錄音中", processing: "處理中" } as const;

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fatal, setFatal] = useState<string | null>(null);
  const [micLevel, setMicLevel] = useState<MicLevel>({ level: 0, active: false });
  const micActiveRef = useRef(false);
  micActiveRef.current = micLevel.active;

  const refresh = () =>
    invoke<Status>("get_status")
      .then((s) => {
        setStatus(s);
        setFatal(null);
      })
      .catch((e) => setFatal(String(e)));

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, 2000);
    const unlistenDl = listen<DownloadProgress>("model-download", (e) => {
      setProgress(e.payload);
      if (e.payload.error) setError(e.payload.error);
      if (e.payload.done) {
        setProgress(null);
        refresh();
      }
    });
    const unlistenMic = listen<MicLevel>("mic-level", (e) => setMicLevel(e.payload));
    return () => {
      clearInterval(timer);
      unlistenDl.then((f) => f());
      unlistenMic.then((f) => f());
      if (micActiveRef.current) invoke("mic_test_stop").catch(() => {});
    };
  }, []);

  const startDownload = () => {
    setError(null);
    invoke("download_model").catch((e) => setError(String(e)));
  };

  const toggleMicTest = () => {
    setError(null);
    if (micLevel.active) {
      invoke("mic_test_stop").catch(() => {});
    } else {
      invoke("mic_test_start").catch((e) => setError(String(e)));
    }
  };

  const changeDevice = (name: string) => {
    setError(null);
    if (micLevel.active) invoke("mic_test_stop").catch(() => {});
    invoke("set_input_device", { name })
      .then(refresh)
      .catch((e) => setError(String(e)));
  };

  if (!status) {
    return (
      <main className="min-h-screen bg-neutral-950 text-neutral-100 flex items-center justify-center p-8">
        <div className="text-center">
          <h1 className="text-2xl font-semibold mb-2">Claro</h1>
          {fatal ? (
            <p className="text-sm text-red-400 break-all max-w-md">狀態載入失敗：{fatal}</p>
          ) : (
            <p className="text-sm text-neutral-400">載入中…</p>
          )}
        </div>
      </main>
    );
  }

  const pct =
    progress && progress.total_mb
      ? Math.min(100, (progress.downloaded_mb / progress.total_mb) * 100)
      : null;
  // RMS 通常 0~0.3，放大顯示；>0.01 即超過靜音門檻
  const levelPct = Math.min(100, micLevel.level * 400);
  const ready = status.model_present && status.accessibility && status.hotkey_active;

  return (
    <main className="min-h-screen bg-neutral-950 text-neutral-100 flex items-center justify-center p-8">
      <div className="w-full max-w-md">
        <div className="flex items-baseline justify-between mb-1">
          <h1 className="text-2xl font-semibold">Claro</h1>
          <span
            className={`text-xs px-2 py-0.5 rounded-full ${
              status.dictation_state === "idle"
                ? "bg-neutral-800 text-neutral-400"
                : status.dictation_state === "recording"
                  ? "bg-red-500/20 text-red-400"
                  : "bg-blue-500/20 text-blue-400"
            }`}
          >
            {STATE_LABEL[status.dictation_state]}
          </span>
        </div>
        <p className="text-neutral-400 text-sm mb-5">
          按住 <kbd className="px-1.5 py-0.5 rounded bg-neutral-800 text-xs">⌥⇧C</kbd> 說話，
          放開即貼到游標處；快速點放進入免持模式，Esc 取消。
        </p>

        <div className="rounded-xl bg-neutral-900 border border-neutral-800 px-4 py-1 divide-y divide-neutral-800">
          <Row
            ok={status.accessibility && status.hotkey_active}
            label="輔助使用權限"
            hint="系統設定 → 隱私與安全性 → 輔助使用，勾選 Claro 後重新啟動"
          />
          <Row
            ok={status.model_present}
            label={`語音模型 whisper-${status.model_id}`}
            hint={`約 ${(status.model_approx_mb / 1024).toFixed(1)} GB，需要下載一次，之後完全離線`}
          />

          {/* 輸入裝置 */}
          <div className="py-2.5">
            <div className="flex items-center justify-between gap-3">
              <div className="font-medium shrink-0">輸入裝置</div>
              <select
                value={status.input_device ?? ""}
                onChange={(e) => changeDevice(e.target.value)}
                className="flex-1 min-w-0 rounded-lg bg-neutral-800 border border-neutral-700 px-2 py-1.5 text-sm"
              >
                <option value="">
                  系統預設{status.default_input ? `（${status.default_input}）` : ""}
                </option>
                {status.input_devices.map((d) => (
                  <option key={d} value={d}>
                    {d}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex items-center gap-3 mt-2.5">
              <button
                onClick={toggleMicTest}
                className={`shrink-0 text-sm rounded-lg px-3 py-1.5 border ${
                  micLevel.active
                    ? "bg-red-500/20 border-red-500/40 text-red-300"
                    : "bg-neutral-800 border-neutral-700 hover:bg-neutral-700"
                }`}
              >
                {micLevel.active ? "停止測試" : "測試麥克風"}
              </button>
              <div className="flex-1 h-2 rounded bg-neutral-800 overflow-hidden">
                <div
                  className={`h-full transition-[width] duration-75 ${
                    micLevel.level > 0.01 ? "bg-green-500" : "bg-neutral-600"
                  }`}
                  style={{ width: `${micLevel.active ? Math.max(levelPct, 2) : 0}%` }}
                />
              </div>
            </div>
            {micLevel.active && (
              <p className="text-xs text-neutral-500 mt-1.5">
                對著麥克風說話——綠色代表音量足夠（超過靜音門檻）。
              </p>
            )}
          </div>
        </div>

        {!status.model_present && (
          <div className="mt-4">
            {progress ? (
              <div>
                <div className="h-2 rounded bg-neutral-800 overflow-hidden">
                  <div
                    className="h-full bg-blue-500 transition-all"
                    style={{ width: pct !== null ? `${pct}%` : "30%" }}
                  />
                </div>
                <div className="text-sm text-neutral-400 mt-2">
                  {progress.downloaded_mb} MB
                  {progress.total_mb ? ` / ${progress.total_mb} MB` : ""}
                </div>
              </div>
            ) : (
              <button
                onClick={startDownload}
                className="w-full rounded-lg bg-blue-600 hover:bg-blue-500 py-2.5 font-medium"
              >
                同意並下載模型
              </button>
            )}
          </div>
        )}

        {error && <p className="mt-3 text-sm text-red-400">{error}</p>}

        {ready && (
          <p className="mt-4 text-sm text-green-400">
            一切就緒。關掉這個視窗 Claro 仍在背景待命；點 Dock 圖示可再開啟。
          </p>
        )}
      </div>
    </main>
  );
}
