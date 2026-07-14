import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { HistoryEntry, PendingResult, PolishMode } from "../types";
import { IconCopy } from "../ui";

const STATUS_PILL: Record<string, { label: string; cls: string } | undefined> = {
  pasted: { label: "已貼上", cls: "green" },
  cancelled: { label: "已取消", cls: "amber" },
  error: { label: "錯誤", cls: "red" },
  too_short: { label: "太短", cls: "amber" },
  silent: { label: "靜音", cls: "amber" },
  mic_unavailable: { label: "麥克風不可用", cls: "red" },
  stt_failed: { label: "辨識失敗", cls: "red" },
  paste_failed: { label: "貼上失敗", cls: "red" },
  focus_changed: { label: "已保留、未貼上", cls: "amber" },
};

const EMPTY_STATUS_TEXT: Record<string, string> = {
  error: "本次聽寫發生錯誤",
  too_short: "錄音太短，沒有送出辨識",
  silent: "沒有偵測到足夠的語音",
  mic_unavailable: "麥克風目前不可用",
  stt_failed: "語音模型未能完成辨識",
  paste_failed: "文字已保留，但無法貼到目標 App",
  focus_changed: "處理期間焦點已切換；文字已保留在歷史紀錄",
};

const STATUS_RECOVERY: Record<string, string> = {
  error: "回到「首次設定」重新檢查權限、麥克風與語音模型後再試一次。",
  too_short: "按住快捷鍵久一點，說完後再放開。",
  silent: "在「首次設定」測試麥克風音量，或改選另一個輸入裝置。",
  mic_unavailable: "打開 macOS 麥克風權限，或在設定中改選可用的輸入裝置。",
  stt_failed: "到「語音模型」重試；若持續失敗，可刪除後重新下載模型。",
  paste_failed: "先複製上方最後輸出，再重新檢查 Claro 的輔助使用權限。",
  focus_changed: "為避免貼到錯誤的 App，Claro 沒有自動貼上；可複製上方最後輸出。",
};

const MODE_LABEL: Record<PolishMode, string> = {
  raw: "原樣轉錄",
  clean: "保守校訂",
  organize: "條理整理",
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

export default function History({
  onCopied,
  historyEnabled,
}: {
  onCopied: () => void;
  historyEnabled: boolean;
}) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [pending, setPending] = useState<PendingResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [confirmClear, setConfirmClear] = useState(false);

  const loadHistory = useCallback(() => {
    setLoading(true);
    setError(null);
    Promise.all([
      invoke<HistoryEntry[]>("get_history", { n: 200 }),
      invoke<PendingResult | null>("get_pending_result"),
    ])
      .then(([history, memoryResult]) => {
        setEntries(history);
        setPending(memoryResult);
      })
      .catch((reason) => setError(String(reason)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => loadHistory(), [loadHistory]);

  const list = [...entries].reverse();

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

  const clearAll = () =>
    invoke("clear_history")
      .then(() => {
        setEntries([]);
        setPending(null);
        setConfirmClear(false);
      })
      .catch((reason) => setError(`清除失敗：${String(reason)}`));

  return (
    <div className="page-in">
      <div className="flex items-start justify-between gap-4 mb-6">
        <div>
          <h1 className="text-[24px] font-bold tracking-tight mb-1">歷史紀錄</h1>
          <p className="text-[13px]" style={{ color: "var(--muted)" }}>
            紀錄只保存在這台 Mac。成功與失敗都會顯示，方便恢復與檢查。
          </p>
        </div>
        {entries.length > 0 && (
          <div className="flex items-center gap-2">
            {confirmClear ? (
              <>
                <button className="btn no-drag" onClick={() => setConfirmClear(false)}>取消</button>
                <button className="btn danger-quiet no-drag" onClick={clearAll}>確認全部清除</button>
              </>
            ) : (
              <button className="btn danger-quiet no-drag" onClick={() => setConfirmClear(true)}>清除歷史</button>
            )}
          </div>
        )}
      </div>

      {error && (
        <div className="config-error mb-4" role="alert">
          <span>無法讀取歷史紀錄：{error}</span>
          <button className="btn no-drag" onClick={loadHistory}>重新載入</button>
        </div>
      )}

      {!historyEnabled && (
        <div className="config-warning mb-4" role="status">
          歷史保存已關閉；新聽寫不會寫入磁碟。未貼上的結果只會暫存在本次 Claro 執行期間。
        </div>
      )}

      {pending && (
        <section className="card p-4 mb-4" aria-label="記憶體中的未貼上結果">
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <strong className="text-[13px]">有一段文字未自動貼上</strong>
              <p className="text-[12px] mt-1" style={{ color: "var(--muted)" }}>
                {pending.reason === "focus_changed"
                  ? "焦點已改變，為避免貼錯位置而保留在記憶體。"
                  : "目標 App 未接受貼上，結果已保留在記憶體。"}
              </p>
              <p className="text-[13px] mt-3 whitespace-pre-wrap select-text">{pending.text}</p>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <button className="btn no-drag" onClick={() => copy(pending.text)}>複製</button>
              <button
                className="btn danger-quiet no-drag"
                onClick={() => invoke("clear_pending_result").then(loadHistory).catch((reason) => setError(String(reason)))}
              >
                捨棄
              </button>
            </div>
          </div>
        </section>
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
          const summaryText = finalText || EMPTY_STATUS_TEXT[entry.status] || "本次聽寫沒有產生文字";
          const rawText = typeof entry.raw === "string" ? entry.raw : "";
          const hasSavedRaw = rawText.length > 0;
          const meta = historyMetadata(entry);
          const hasAuditMetadata = Boolean(meta.mode || meta.provider || meta.outcome || meta.fallbackReason);
          const recovery = STATUS_RECOVERY[entry.status];
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
                    <span className="hist-text">{summaryText}</span>
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
                      <p>{hasSavedRaw ? rawText : finalText ? "舊紀錄未保存辨識原文" : "本次沒有可用的辨識原文"}</p>
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
                      <p>{finalText || summaryText}</p>
                      {hasSavedRaw && rawText === finalText && <small>本次輸出未改動文字。</small>}
                    </section>
                  </div>

                  <dl className="history-audit-grid">
                    <div>
                      <dt>整理模式</dt>
                      <dd>{meta.mode ? MODE_LABEL[meta.mode] : "舊紀錄未記錄"}</dd>
                    </div>
                    <div>
                      <dt>文字整理位置</dt>
                      <dd>{meta.provider ? (PROVIDER_LABEL[meta.provider] ?? meta.provider) : meta.mode === "raw" ? "未使用" : "未記錄"}</dd>
                    </div>
                    <div>
                      <dt>結果</dt>
                      <dd>{meta.outcome ? (OUTCOME_LABEL[meta.outcome] ?? meta.outcome) : hasAuditMetadata ? "未記錄" : "舊紀錄未記錄"}</dd>
                    </div>
                    <div>
                      <dt>耗時</dt>
                      <dd>
                        語音辨識 {formatDuration(meta.sttMs) ?? "未記錄"}
                        {meta.polishMs !== null && meta.polishMs !== undefined && `・整理 ${formatDuration(meta.polishMs)}`}
                      </dd>
                    </div>
                    {meta.fallbackReason && (
                      <div className="history-fallback">
                        <dt>安全退回原因</dt>
                        <dd>{FALLBACK_LABEL[meta.fallbackReason] ?? meta.fallbackReason}</dd>
                      </div>
                    )}
                    {recovery && (
                      <div className="history-fallback">
                        <dt>下一步</dt>
                        <dd>{recovery}</dd>
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
