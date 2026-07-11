import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { HistoryEntry, PolishMode } from "../types";
import { IconCopy } from "../ui";

const STATUS_PILL: Record<string, { label: string; cls: string } | undefined> = {
  pasted: { label: "已貼上", cls: "green" },
  cancelled: { label: "已取消", cls: "amber" },
  error: { label: "錯誤", cls: "red" },
  too_short: { label: "太短", cls: "amber" },
  silent: { label: "靜音", cls: "amber" },
};

const MODE_LABEL: Record<PolishMode, string> = {
  raw: "RAW 原樣轉錄",
  clean: "CLEAN 保守校訂",
  organize: "ORGANIZE 條理整理",
};

const PROVIDER_LABEL: Record<string, string> = {
  apple: "Apple Intelligence",
  builtin: "Claro 內建模型",
  ollama: "Ollama",
  lmstudio: "LM Studio",
  custom: "自訂端點",
};

const OUTCOME_LABEL: Record<string, string> = {
  raw: "原樣輸出",
  changed: "已整理",
  unchanged: "無需變更",
  fallback: "安全退回原文",
};

const FALLBACK_LABEL: Record<string, string> = {
  organize_consent_required: "尚未確認條理整理",
  provider_unavailable: "整理引擎不可用",
  local_only: "僅限本機已阻擋雲端",
  cloud_consent_required: "尚未確認雲端資料傳送",
  invalid_custom_url: "自訂端點格式不正確",
  provider_error: "整理引擎執行失敗",
  empty_output: "整理結果為空",
  length_violation: "整理結果長度異常",
  low_overlap: "內容重疊度過低",
  high_novelty: "整理結果新增過多內容",
  meaning_anchor_mismatch: "重要事實或語意錨點不一致",
};

function historyMetadata(entry: HistoryEntry) {
  const mode = entry.polish?.mode ?? entry.polish_mode ?? entry.mode;
  const provider = entry.polish?.provider ?? entry.polish_provider ?? entry.provider;
  const outcome = entry.polish?.outcome ?? entry.polish_outcome ?? entry.outcome;
  const fallbackReason = entry.polish?.fallback_reason ?? entry.fallback_reason;
  const sttMs = entry.timings?.stt_ms ?? entry.timings?.stt;
  const polishMs = entry.timings?.polish_ms ?? entry.timings?.polish;
  return { mode, provider, outcome, fallbackReason, sttMs, polishMs };
}

function formatDuration(ms: number | null | undefined) {
  if (ms === null || ms === undefined) return null;
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)} 秒` : `${Math.round(ms)} ms`;
}

export default function History({ onCopied }: { onCopied: () => void }) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());

  const loadHistory = useCallback(() => {
    setLoading(true);
    setError(null);
    invoke<HistoryEntry[]>("get_history", { n: 200 })
      .then(setEntries)
      .catch((reason) => setError(String(reason)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => loadHistory(), [loadHistory]);

  const list = [...entries].reverse().filter((entry) => entry.text || entry.raw);

  const copy = (text: string) =>
    invoke("copy_text", { text })
      .then(onCopied)
      .catch((reason) => setError(`複製失敗：${String(reason)}`));

  const toggle = (id: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div className="page-in">
      <h1 className="text-[24px] font-bold tracking-tight mb-1">歷史紀錄</h1>
      <p className="text-[13px] mb-6" style={{ color: "var(--muted)" }}>
        只存在你的機器上（~/.claro/history.jsonl）。展開紀錄可核對辨識原文、最後輸出與安全退回原因。
      </p>

      {error && (
        <div className="config-error mb-4" role="alert">
          <span>無法讀取歷史紀錄：{error}</span>
          <button className="btn no-drag" onClick={loadHistory}>重新載入</button>
        </div>
      )}

      <div className="card history-list" aria-busy={loading}>
        {loading && (
          <div className="history-state" role="status">正在載入歷史紀錄…</div>
        )}
        {!loading && !error && list.length === 0 && (
          <div className="history-state">還沒有紀錄。完成第一次聽寫後，原文與輸出會顯示在這裡。</div>
        )}
        {!loading && list.map((entry, index) => {
          const id = `${entry.ts}-${index}`;
          const open = expanded.has(id);
          const pill = STATUS_PILL[entry.status];
          const finalText = entry.text || entry.raw || "";
          const rawText = typeof entry.raw === "string" ? entry.raw : "";
          const hasSavedRaw = rawText.length > 0;
          const meta = historyMetadata(entry);
          const hasAuditMetadata = Boolean(meta.mode || meta.provider || meta.outcome || meta.fallbackReason);
          return (
            <article className={`history-entry ${open ? "expanded" : ""}`} key={id}>
              <div className="history-summary">
                <button
                  className="history-toggle no-drag"
                  aria-expanded={open}
                  aria-controls={`${id}-details`}
                  onClick={() => toggle(id)}
                >
                  <span className="history-chevron" aria-hidden="true">›</span>
                  <span className="flex-1 min-w-0 text-left">
                    <span className="hist-text">{finalText}</span>
                    <span className="history-meta-line">
                      {pill && <span className={`pill ${pill.cls}`}>{pill.label}</span>}
                      {meta.mode && <span className="pill blue">{MODE_LABEL[meta.mode]}</span>}
                      {meta.outcome === "fallback" && <span className="pill amber">安全退回</span>}
                      <span className="hist-time">
                        {new Date(entry.ts).toLocaleString("zh-TW", {
                          month: "numeric",
                          day: "numeric",
                          hour: "2-digit",
                          minute: "2-digit",
                        })}
                        ・{entry.duration_s}s
                      </span>
                    </span>
                  </span>
                </button>
                <button
                  className="copy-btn no-drag history-copy"
                  aria-label="複製最後輸出"
                  title="複製最後輸出"
                  onClick={() => copy(finalText)}
                  disabled={!finalText}
                >
                  <IconCopy />
                </button>
              </div>

              {open && (
                <div className="history-details" id={`${id}-details`}>
                  <div className="history-text-grid">
                    <section className="history-text-card" aria-label="辨識原文">
                      <div className="history-detail-heading">
                        <span>辨識原文</span>
                        <button
                          className="copy-btn no-drag"
                          aria-label="複製辨識原文"
                          title="複製辨識原文"
                          onClick={() => copy(rawText)}
                          disabled={!rawText}
                        >
                          <IconCopy />
                        </button>
                      </div>
                      <p>{hasSavedRaw ? rawText : "舊紀錄未保存辨識原文"}</p>
                    </section>
                    <section className="history-text-card final" aria-label="最後輸出">
                      <div className="history-detail-heading">
                        <span>最後輸出</span>
                        <button
                          className="copy-btn no-drag"
                          aria-label="複製最後輸出"
                          title="複製最後輸出"
                          onClick={() => copy(finalText)}
                          disabled={!finalText}
                        >
                          <IconCopy />
                        </button>
                      </div>
                      <p>{finalText}</p>
                      {hasSavedRaw && rawText === finalText && <small>本次輸出未改動文字。</small>}
                    </section>
                  </div>

                  <dl className="history-audit-grid">
                    <div>
                      <dt>整理模式</dt>
                      <dd>{meta.mode ? MODE_LABEL[meta.mode] : "舊紀錄未記錄"}</dd>
                    </div>
                    <div>
                      <dt>處理引擎</dt>
                      <dd>{meta.provider ? (PROVIDER_LABEL[meta.provider] ?? meta.provider) : meta.mode === "raw" ? "未使用" : "未記錄"}</dd>
                    </div>
                    <div>
                      <dt>結果</dt>
                      <dd>{meta.outcome ? (OUTCOME_LABEL[meta.outcome] ?? meta.outcome) : hasAuditMetadata ? "未記錄" : "舊紀錄未記錄"}</dd>
                    </div>
                    <div>
                      <dt>耗時</dt>
                      <dd>
                        STT {formatDuration(meta.sttMs) ?? "未記錄"}
                        {meta.polishMs !== null && meta.polishMs !== undefined && `・整理 ${formatDuration(meta.polishMs)}`}
                      </dd>
                    </div>
                    {meta.fallbackReason && (
                      <div className="history-fallback">
                        <dt>安全退回原因</dt>
                        <dd>{FALLBACK_LABEL[meta.fallbackReason] ?? meta.fallbackReason}</dd>
                      </div>
                    )}
                  </dl>
                </div>
              )}
            </article>
          );
        })}
      </div>
    </div>
  );
}
