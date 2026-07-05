import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./index.css";

interface Status {
  model_id: string;
  model_present: boolean;
  model_approx_mb: number;
  accessibility: boolean;
  hotkey_active: boolean;
}

interface DownloadProgress {
  downloaded_mb: number;
  total_mb: number | null;
  done: boolean;
  error: string | null;
}

function Row({ ok, label, hint }: { ok: boolean; label: string; hint?: string }) {
  return (
    <div className="flex items-start gap-3 py-2">
      <span className={`mt-0.5 text-lg leading-none ${ok ? "text-green-500" : "text-amber-500"}`}>
        {ok ? "✓" : "●"}
      </span>
      <div>
        <div className="font-medium">{label}</div>
        {hint && !ok && <div className="text-sm text-neutral-400">{hint}</div>}
      </div>
    </div>
  );
}

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => invoke<Status>("get_status").then(setStatus).catch(console.error);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, 3000);
    const unlisten = listen<DownloadProgress>("model-download", (e) => {
      setProgress(e.payload);
      if (e.payload.error) setError(e.payload.error);
      if (e.payload.done) {
        setProgress(null);
        refresh();
      }
    });
    return () => {
      clearInterval(timer);
      unlisten.then((f) => f());
    };
  }, []);

  const startDownload = () => {
    setError(null);
    invoke("download_model").catch((e) => setError(String(e)));
  };

  if (!status) return null;

  const pct =
    progress && progress.total_mb
      ? Math.min(100, (progress.downloaded_mb / progress.total_mb) * 100)
      : null;

  return (
    <main className="min-h-screen bg-neutral-950 text-neutral-100 flex items-center justify-center p-8">
      <div className="w-full max-w-md">
        <h1 className="text-2xl font-semibold mb-1">Claro</h1>
        <p className="text-neutral-400 text-sm mb-6">
          按住 <kbd className="px-1.5 py-0.5 rounded bg-neutral-800 text-xs">⌥⇧C</kbd> 說話，
          放開即貼到游標處；快速點放進入免持模式，Esc 取消。
        </p>

        <div className="rounded-xl bg-neutral-900 border border-neutral-800 px-4 py-2 divide-y divide-neutral-800">
          <Row
            ok={status.accessibility && status.hotkey_active}
            label="輔助使用權限"
            hint="系統設定 → 隱私與安全性 → 輔助使用，勾選 Claro 後重新啟動"
          />
          <Row
            ok={status.model_present}
            label={`語音模型 whisper-${status.model_id}`}
            hint={`約 ${Math.round(status.model_approx_mb / 100) / 10} GB，需要下載一次，之後完全離線`}
          />
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

        {status.model_present && status.accessibility && status.hotkey_active && (
          <p className="mt-4 text-sm text-green-400">一切就緒，關掉這個視窗直接開始聽寫。</p>
        )}
      </div>
    </main>
  );
}
