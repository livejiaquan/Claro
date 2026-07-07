import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { HistoryEntry, Status } from "../types";
import { Hotkey, IconChars, IconCopy, IconMic, IconSpeed, IconStack, StatCard } from "../ui";

function isToday(ts: string) {
  const d = new Date(ts);
  const now = new Date();
  return (
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate()
  );
}

export default function Home({
  status,
  onCopied,
  gotoHistory,
}: {
  status: Status;
  onCopied: () => void;
  gotoHistory: () => void;
}) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);

  useEffect(() => {
    invoke<HistoryEntry[]>("get_history", { n: 500 }).then(setEntries).catch(() => {});
    const t = setInterval(
      () => invoke<HistoryEntry[]>("get_history", { n: 500 }).then(setEntries).catch(() => {}),
      5000,
    );
    return () => clearInterval(t);
  }, []);

  const pasted = entries.filter((e) => e.status === "pasted");
  const today = pasted.filter((e) => isToday(e.ts));
  const todayChars = today.reduce((s, e) => s + (e.text?.length ?? 0), 0);
  const sttSamples = pasted.filter((e) => e.timings?.stt_ms).slice(-20);
  const avgStt = sttSamples.length
    ? sttSamples.reduce((s, e) => s + (e.timings!.stt_ms! / 1000), 0) / sttSamples.length
    : null;
  const recent = [...pasted].reverse().slice(0, 3);

  const copy = (text: string) => {
    invoke("copy_text", { text }).then(onCopied).catch(() => {});
  };

  return (
    <div className="page-in">
      <h1 className="text-[24px] font-bold tracking-tight">自然說話，直接出字。</h1>
      <p className="text-[14px] mt-2 mb-7" style={{ color: "var(--muted)" }}>
        在任何輸入框，按住 <Hotkey combo={status.hotkey} /> 說話，放開就貼上；快點一下進入免持模式，
        <kbd className="keycap" style={{ height: 22, fontSize: 11.5 }}>esc</kbd> 取消。
      </p>

      {/* 未就緒時大聲說清楚：使用者不該需要猜「為什麼沒反應」 */}
      {!status.hotkey_active && (
        <div
          className="card mb-7"
          style={{ borderLeft: "3px solid var(--amber)", background: "rgba(255,159,10,0.06)" }}
        >
          <div className="row" style={{ alignItems: "flex-start" }}>
            <div className="flex-1 min-w-0">
              <div className="row-label">快捷鍵尚未啟用——需要「輔助使用」權限</div>
              <div className="row-sub">
                macOS 要求語音輸入工具取得輔助使用權限才能監聽快捷鍵與貼上文字。
                到 系統設定 → 隱私與安全性 → 輔助使用，勾選 Claro。授權後會<b>自動啟用</b>，不用重啟。
                <br />
                <b>清單裡已勾選卻仍看到這則訊息？</b>這是 app 更新後舊授權失效（權限綁定舊版簽章）——
                按「重設授權」清掉舊條目，再到系統設定重新勾選即可。
              </div>
            </div>
            <div className="flex flex-col gap-2 shrink-0">
              <button
                className="btn no-drag"
                onClick={() => invoke("open_accessibility_settings").catch(() => {})}
              >
                打開系統設定
              </button>
              <button
                className="btn no-drag"
                onClick={() => invoke("reset_accessibility").catch(() => {})}
              >
                重設授權
              </button>
            </div>
          </div>
        </div>
      )}
      {status.hotkey_active && !status.model_present && (
        <div
          className="card mb-7"
          style={{ borderLeft: "3px solid var(--amber)", background: "rgba(255,159,10,0.06)" }}
        >
          <div className="row">
            <div className="flex-1 min-w-0">
              <div className="row-label">還沒有語音模型</div>
              <div className="row-sub">到 設定 → 語音模型 下載一個模型（推薦 Large v3 Turbo）就能開始聽寫。</div>
            </div>
          </div>
        </div>
      )}

      <div className="grid grid-cols-2 lg:grid-cols-4 gap-3 mb-7">
        <StatCard icon={<IconMic />} value={String(today.length)} unit="次" label="今日聽寫" />
        <StatCard icon={<IconChars />} value={todayChars.toLocaleString()} unit="字" label="今日字數" />
        <StatCard
          icon={<IconSpeed />}
          value={avgStt ? avgStt.toFixed(1) : "—"}
          unit={avgStt ? "秒" : undefined}
          label="平均出字速度"
        />
        <StatCard icon={<IconStack />} value={pasted.length.toLocaleString()} unit="次" label="累計聽寫" />
      </div>

      <div className="flex items-baseline justify-between mb-2 px-1">
        <div className="section-title" style={{ margin: 0 }}>
          最近聽寫
        </div>
        <button
          className="text-[12.5px] font-medium no-drag"
          style={{ color: "var(--accent)" }}
          onClick={gotoHistory}
        >
          查看全部 →
        </button>
      </div>
      <div className="card">
        {recent.length === 0 && (
          <div className="row" style={{ color: "var(--muted)", fontSize: 13 }}>
            還沒有聽寫紀錄——按住 <Hotkey combo={status.hotkey} /> 說第一句話吧。
          </div>
        )}
        {recent.map((e, i) => (
          <div className="hist-row" key={i}>
            <div className="flex-1 min-w-0">
              <div className="hist-text">{e.text}</div>
            </div>
            <span className="hist-time">
              {new Date(e.ts).toLocaleTimeString("zh-TW", { hour: "2-digit", minute: "2-digit" })}
            </span>
            <button className="copy-btn no-drag" title="複製" onClick={() => copy(e.text)}>
              <IconCopy />
            </button>
          </div>
        ))}
      </div>

      <p className="text-[12px] mt-6 flex items-center gap-1.5" style={{ color: "var(--faint)" }}>
        <svg viewBox="0 0 20 20" width="13" height="13" fill="none" stroke="currentColor" strokeWidth="1.6">
          <rect x="4.5" y="8.5" width="11" height="8" rx="2" />
          <path d="M7 8.5V6.8a3 3 0 0 1 6 0v1.7" />
        </svg>
        全程本地處理，語音與文字都不離開這台機器。
        {status.dictation_state === "recording" && (
          <span className="pill red ml-2">
            <span className="dot pulse" />
            錄音中
          </span>
        )}
      </p>
    </div>
  );
}
