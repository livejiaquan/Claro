import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "@fontsource/fraunces/500-italic.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
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

const BARS = 44;
const LEDS = 22;

const STATE_LABEL = {
  idle: "standby",
  recording: "on air",
  processing: "working",
} as const;

/** 頂部示波器：待命時緩慢呼吸，麥克風測試/錄音時吃真實或加強波動 */
function Scope({ level, active, rec }: { level: number; active: boolean; rec: boolean }) {
  const bars = useRef<(HTMLElement | null)[]>([]);
  const buf = useRef<number[]>(new Array(BARS).fill(0.05));
  const levelRef = useRef(0);
  const activeRef = useRef(false);
  const recRef = useRef(false);
  levelRef.current = level;
  activeRef.current = active;
  recRef.current = rec;

  useEffect(() => {
    let raf = 0;
    let t = 0;
    const step = () => {
      t += 1;
      const idle =
        0.12 +
        0.07 * Math.sin(t / 34) * Math.sin(t / 13) +
        0.035 * Math.sin(t / 7 + 1.3);
      let v: number;
      if (activeRef.current) {
        v = Math.min(1, levelRef.current * 6 + idle * 0.4);
      } else if (recRef.current) {
        v = Math.min(1, idle * 2.2 + Math.random() * 0.12);
      } else {
        v = idle;
      }
      const b = buf.current;
      b.push(v);
      b.shift();
      for (let i = 0; i < BARS; i++) {
        const el = bars.current[i];
        if (el) el.style.transform = `scaleY(${Math.max(0.04, b[i])})`;
      }
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <div className={`scope ${rec ? "rec" : ""}`}>
      <div className="wavebars">
        {Array.from({ length: BARS }, (_, i) => (
          <i
            key={i}
            ref={(el) => {
              bars.current[i] = el;
            }}
          />
        ))}
      </div>
      <div className="scope-ticks">
        {Array.from({ length: 41 }, (_, i) => (
          <span key={i} />
        ))}
      </div>
    </div>
  );
}

function LedMeter({ level, active }: { level: number; active: boolean }) {
  // RMS 0~0.25 映射滿表；>0.01 即超過靜音門檻（約第 2 格）
  const lit = active ? Math.round(Math.min(1, level * 4) * LEDS) : 0;
  return (
    <div className="led-meter">
      {Array.from({ length: LEDS }, (_, i) => {
        const zone = i < LEDS * 0.62 ? "g" : i < LEDS * 0.85 ? "a" : "r";
        return <i key={i} className={i < lit ? `on ${zone}` : ""} />;
      })}
    </div>
  );
}

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fatal, setFatal] = useState<string | null>(null);
  const [mic, setMic] = useState<MicLevel>({ level: 0, active: false });
  const micRef = useRef(false);
  micRef.current = mic.active;

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
    const un1 = listen<DownloadProgress>("model-download", (e) => {
      setProgress(e.payload);
      if (e.payload.error) setError(e.payload.error);
      if (e.payload.done) {
        setProgress(null);
        refresh();
      }
    });
    const un2 = listen<MicLevel>("mic-level", (e) => setMic(e.payload));
    return () => {
      clearInterval(timer);
      un1.then((f) => f());
      un2.then((f) => f());
      if (micRef.current) invoke("mic_test_stop").catch(() => {});
    };
  }, []);

  const toggleMic = () => {
    setError(null);
    invoke(mic.active ? "mic_test_stop" : "mic_test_start").catch((e) => setError(String(e)));
  };

  const changeDevice = (name: string) => {
    setError(null);
    if (mic.active) invoke("mic_test_stop").catch(() => {});
    invoke("set_input_device", { name })
      .then(refresh)
      .catch((e) => setError(String(e)));
  };

  const startDownload = () => {
    setError(null);
    invoke("download_model").catch((e) => setError(String(e)));
  };

  if (!status) {
    return (
      <main className="chassis flex items-center justify-center">
        <div className="text-center titlebar-drag fixed inset-0" />
        <div className="relative text-center">
          <div className="wordmark text-3xl">claro</div>
          <p className="readout text-[10px] mt-3" style={{ color: "var(--muted)" }}>
            {fatal ? `error — ${fatal}` : "initializing"}
          </p>
        </div>
      </main>
    );
  }

  const st = status.dictation_state;
  const ready = status.model_present && status.accessibility && status.hotkey_active;
  const pct =
    progress && progress.total_mb
      ? Math.min(100, (progress.downloaded_mb / progress.total_mb) * 100)
      : null;

  return (
    <main className="chassis flex flex-col">
      {/* 標題列拖曳區 + 品牌 */}
      <header className="titlebar-drag pt-9 pb-5 px-6 flex items-end justify-between rise">
        <div className="flex items-baseline gap-2.5">
          <span className="wordmark text-[30px] leading-none">claro</span>
          <span className="readout text-[9px]" style={{ color: "var(--faint)" }}>
            local dictation
          </span>
        </div>
        <div className="flex items-center gap-2 pb-1">
          <span
            className={`lamp ${st === "recording" ? "rec" : st === "processing" ? "busy" : ready ? "ok" : "warn"}`}
          />
          <span className="readout text-[10px]" style={{ color: "var(--muted)" }}>
            {STATE_LABEL[st]}
          </span>
        </div>
      </header>

      {/* 示波器 */}
      <div className="rise" style={{ animationDelay: "60ms" }}>
        <Scope level={mic.level} active={mic.active} rec={st === "recording"} />
      </div>

      {/* 熱鍵儀式 */}
      <div className="px-6 pt-4 pb-1 rise" style={{ animationDelay: "120ms" }}>
        <div
          className="flex items-center justify-center gap-2 text-[13px] whitespace-nowrap"
          style={{ color: "var(--paper)" }}
        >
          按住
          <span className="flex gap-1">
            <kbd className="keycap">⌥</kbd>
            <kbd className="keycap">⇧</kbd>
            <kbd className="keycap">C</kbd>
          </span>
          說話，放開即出字
        </div>
        <div
          className="text-center text-[11px] mt-2"
          style={{ color: "var(--muted)" }}
        >
          快點一下進入免持模式・<kbd className="keycap" style={{ fontSize: "10px", padding: "3px 5px 4px" }}>esc</kbd> 取消
        </div>
      </div>

      <div className="px-6 pb-6 space-y-4">
        {/* 檢查面板 */}
        <section className="panel rise" style={{ animationDelay: "180ms" }}>
          <div className="panel-row">
            <span className={`lamp ${status.accessibility && status.hotkey_active ? "ok" : "warn"}`} />
            <div>
              <div className="row-label">輔助使用權限</div>
              {!(status.accessibility && status.hotkey_active) && (
                <div className="row-sub">系統設定 → 隱私與安全性 → 輔助使用，勾選後重啟 Claro</div>
              )}
            </div>
            <span className={`row-value ${status.accessibility && status.hotkey_active ? "ok" : "warn"}`}>
              {status.accessibility && status.hotkey_active ? "granted" : "required"}
            </span>
          </div>

          <div className="panel-row">
            <span className={`lamp ${status.model_present ? "ok" : "warn"}`} />
            <div>
              <div className="row-label">語音模型</div>
              <div className="row-sub">whisper-{status.model_id}・完全離線</div>
            </div>
            <span className={`row-value ${status.model_present ? "ok" : "warn"}`}>
              {status.model_present ? "ready" : `${(status.model_approx_mb / 1024).toFixed(1)} GB`}
            </span>
          </div>

          {/* 輸入通道 */}
          <div className="panel-row" style={{ alignItems: "flex-start" }}>
            <span className={`lamp ${mic.active ? (mic.level > 0.01 ? "ok" : "warn") : "ok"}`} />
            <div className="flex-1 min-w-0">
              <div className="flex items-center justify-between gap-3">
                <div className="row-label shrink-0">輸入裝置</div>
                <select
                  className="channel-select"
                  value={status.input_device ?? ""}
                  onChange={(e) => changeDevice(e.target.value)}
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
              <div className="flex items-center gap-3 mt-3">
                <button className={`btn ${mic.active ? "active" : ""}`} onClick={toggleMic}>
                  {mic.active ? "stop" : "test"}
                </button>
                <LedMeter level={mic.level} active={mic.active} />
              </div>
              {mic.active && (
                <div className="row-sub mt-2">
                  {mic.level > 0.01 ? "收音正常——這就是聽寫用的音量" : "太安靜——對著麥克風說幾句話"}
                </div>
              )}
            </div>
          </div>
        </section>

        {/* 模型下載 */}
        {!status.model_present && (
          <section className="rise" style={{ animationDelay: "240ms" }}>
            {progress ? (
              <div className="panel px-4 py-4">
                <div className="progress-track">
                  <div className="progress-fill" style={{ width: pct !== null ? `${pct}%` : "30%" }} />
                </div>
                <div className="readout text-[10px] mt-2.5" style={{ color: "var(--muted)" }}>
                  {progress.downloaded_mb} / {progress.total_mb ?? "?"} mb
                </div>
              </div>
            ) : (
              <button className="btn-primary" onClick={startDownload}>
                同意並下載語音模型（{(status.model_approx_mb / 1024).toFixed(1)} GB，只需一次）
              </button>
            )}
          </section>
        )}

        {error && (
          <p className="text-xs rise" style={{ color: "var(--rec)" }}>
            {error}
          </p>
        )}
      </div>

      {/* 底部銘牌 */}
      <footer
        className="mt-auto px-6 pb-5 pt-1 flex items-center justify-between rise"
        style={{ animationDelay: "300ms" }}
      >
        <span className="readout text-[9px]" style={{ color: "var(--faint)" }}>
          {ready ? "音訊不出機器・關窗仍在背景待命" : "初次設定"}
        </span>
        <span className="readout text-[9px]" style={{ color: "var(--faint)" }}>
          v0.1.0
        </span>
      </footer>
    </main>
  );
}
