import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { HistoryEntry } from "../types";
import { IconCopy } from "../ui";

const STATUS_PILL: Record<string, { label: string; cls: string } | undefined> = {
  pasted: { label: "已貼上", cls: "green" },
  cancelled: { label: "已取消", cls: "amber" },
  error: { label: "錯誤", cls: "red" },
  too_short: { label: "太短", cls: "amber" },
  silent: { label: "靜音", cls: "amber" },
};

export default function History({ onCopied }: { onCopied: () => void }) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);

  useEffect(() => {
    invoke<HistoryEntry[]>("get_history", { n: 200 }).then(setEntries).catch(() => {});
  }, []);

  const list = [...entries].reverse().filter((e) => e.text || e.raw);

  const copy = (text: string) => invoke("copy_text", { text }).then(onCopied).catch(() => {});

  return (
    <div className="page-in">
      <h1 className="text-[24px] font-bold tracking-tight mb-1">歷史紀錄</h1>
      <p className="text-[13px] mb-6" style={{ color: "var(--muted)" }}>
        只存在你的機器上（~/.claro/history.jsonl）。被取消的聽寫也救得回來。
      </p>

      <div className="card">
        {list.length === 0 && (
          <div className="row" style={{ color: "var(--muted)", fontSize: 13 }}>
            還沒有紀錄。
          </div>
        )}
        {list.map((e, i) => {
          const pill = STATUS_PILL[e.status];
          const text = e.text || e.raw;
          return (
            <div className="hist-row" key={i}>
              <div className="flex-1 min-w-0">
                <div className="hist-text">{text}</div>
                <div className="flex items-center gap-2 mt-1.5">
                  {pill && <span className={`pill ${pill.cls}`}>{pill.label}</span>}
                  <span className="hist-time">
                    {new Date(e.ts).toLocaleString("zh-TW", {
                      month: "numeric",
                      day: "numeric",
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                    ・{e.duration_s}s
                  </span>
                </div>
              </div>
              <button className="copy-btn no-drag mt-1" title="複製" onClick={() => copy(text)}>
                <IconCopy />
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}
