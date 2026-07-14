import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import {
  resolveLlmConfig,
  type HistoryEntry,
  type LlmConfig,
  type ResolvedLlmConfig,
  type Status,
} from "../types";
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

const MODE_LABEL = {
  raw: "原樣轉錄",
  clean: "保守校訂",
  organize: "條理整理",
} as const;

const PROVIDER_LABEL: Record<string, string> = {
  apple: "Apple Intelligence",
  builtin: "Claro 內建模型",
  ollama: "Ollama",
  lmstudio: "LM Studio",
  custom: "自訂端點",
};

const BLOCKED_COPY: Record<string, string> = {
  provider_missing: "尚未選擇整理引擎",
  provider_off: "尚未選擇整理引擎",
  provider_incomplete: "整理引擎設定尚未完成",
  provider_unavailable: "整理引擎目前不可用",
  model_missing: "整理模型尚未下載",
  organize_consent_required: "尚未確認條理整理的行為差異",
  cloud_consent_required: "尚未確認雲端資料傳送",
  local_only: "「僅限本機」正在阻擋雲端引擎",
  invalid_endpoint: "自訂端點格式不正確",
  invalid_custom_url: "自訂端點格式不正確",
};

function privacyMessage(llm: ResolvedLlmConfig | null, failed: boolean, contextEnabled: boolean) {
  if (failed) {
    return {
      tone: "warning",
      title: "無法確認資料處理位置",
      detail: "請到設定重新載入；Claro 不會在這裡假設目前是全本地模式。",
    };
  }
  if (!llm) {
    return { tone: "neutral", title: "正在確認資料處理位置…", detail: "" };
  }

  if (llm.effective_mode === "raw") {
    if (llm.polish_mode !== "raw" && llm.blocked_reason) {
      return {
        tone: "warning",
        title: `目前退回 ${MODE_LABEL.raw}`,
        detail: `${BLOCKED_COPY[llm.blocked_reason] ?? "整理設定尚未就緒"}；目前沒有資料送往雲端。`,
      };
    }
    return {
      tone: "local",
      title: `全本地・${MODE_LABEL.raw}`,
      detail: "不使用 AI 整理；音訊與文字不離開這台 Mac。",
    };
  }

  const mode = MODE_LABEL[llm.effective_mode];
  const contextIsSent = llm.effective_mode === "organize" && contextEnabled;
  if (llm.execution_location === "on_device") {
    return {
      tone: "local",
      title: `全本地・${mode}`,
      detail: "辨識與整理都在這台 Mac 完成。",
    };
  }
  if (llm.execution_location === "local_service") {
    const data = contextIsSent ? "轉錄文字與畫面上下文" : "轉錄文字";
    return {
      tone: "local",
      title: `本機服務・${mode}`,
      detail: `${data}只送到這台 Mac 上的 ${PROVIDER_LABEL[llm.provider] ?? llm.provider}；音訊不傳送。`,
    };
  }
  if (llm.execution_location === "cloud") {
    const data = contextIsSent ? "轉錄文字與畫面上下文" : "轉錄文字";
    return {
      tone: "cloud",
      title: `雲端整理・${mode}`,
      detail: `${data}會送到 ${llm.endpoint_origin ?? "你設定的端點"}；音訊不傳送。`,
    };
  }

  return {
    tone: "warning",
    title: `目前退回 ${MODE_LABEL.raw}`,
    detail: "整理引擎尚未就緒；目前沒有資料送往雲端。",
  };
}

export default function Home({
  status,
  onCopied,
  gotoHistory,
  gotoSetup,
}: {
  status: Status;
  onCopied: () => void;
  gotoHistory: () => void;
  gotoSetup: () => void;
}) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [llm, setLlm] = useState<ResolvedLlmConfig | null>(null);
  const [llmFailed, setLlmFailed] = useState(false);

  useEffect(() => {
    const refreshHistory = () => invoke<HistoryEntry[]>("get_history", { n: 500 }).then(setEntries).catch(() => {});
    const refreshLlm = () =>
      invoke<LlmConfig>("get_llm_config")
        .then((config) => {
          setLlm(resolveLlmConfig(config));
          setLlmFailed(false);
        })
        .catch(() => setLlmFailed(true));

    refreshHistory();
    refreshLlm();
    const t = setInterval(
      () => {
        refreshHistory();
        refreshLlm();
      },
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
  const privacy = privacyMessage(llm, llmFailed, status.context_enabled);
  const permissionsReady = status.accessibility && status.hotkey_active;
  const needsSetup = !permissionsReady || !status.model_present || !status.setup_completed;

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

      {/* 即時狀態 CTA，不保存假的 onboarding 完成旗標。 */}
      {needsSetup && (
        <div className="setup-banner mb-7" role="status">
          <div className="setup-banner-copy">
            <span className="setup-banner-kicker">開始前還差一點</span>
            <div className="row-label">完成首次設定，確認 Claro 聽得到也貼得出去。</div>
            <div className="setup-banner-checks" aria-label="首次設定進度">
              <span className={permissionsReady ? "ready" : "pending"}>
                {permissionsReady ? "✓" : "○"} 輔助使用
              </span>
              <span className={status.model_present ? "ready" : "pending"}>
                {status.model_present ? "✓" : "○"} 語音模型
              </span>
              <span className={status.setup_completed ? "ready" : "pending"}>
                {status.setup_completed ? "✓" : "○"} 麥克風與首次檢查
              </span>
            </div>
          </div>
          <button className="btn-primary no-drag" onClick={gotoSetup}>開始首次設定</button>
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
            <button className="copy-btn no-drag" aria-label="複製這筆聽寫" title="複製" onClick={() => copy(e.text)}>
              <IconCopy />
            </button>
          </div>
        ))}
      </div>

      <div className={`privacy-notice ${privacy.tone}`} role="status" aria-live="polite">
        <svg aria-hidden="true" viewBox="0 0 20 20" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="1.6">
          <rect x="4.5" y="8.5" width="11" height="8" rx="2" />
          <path d="M7 8.5V6.8a3 3 0 0 1 6 0v1.7" />
        </svg>
        <span>
          <b>{privacy.title}</b>
          {privacy.detail && <span className="privacy-detail">{privacy.detail}</span>}
        </span>
        {status.dictation_state === "recording" && (
          <span className="pill red ml-2">
            <span className="dot pulse" />
            錄音中
          </span>
        )}
      </div>
    </div>
  );
}
