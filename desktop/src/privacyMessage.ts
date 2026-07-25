import type { ResolvedLlmConfig } from "./types";

const MODE_LABEL = {
  raw: "原樣轉錄",
  clean: "保守校訂",
  correct: "專業校字",
  organize: "條理整理",
} as const;

const PROVIDER_LABEL: Record<string, string> = {
  apple: "Apple Intelligence",
  builtin: "Claro 內建模型",
  ollama: "Ollama",
  lmstudio: "LM Studio",
  custom: "自訂端點",
  codex: "OpenAI Codex",
};

const BLOCKED_COPY: Record<string, string> = {
  provider_missing: "尚未選擇整理引擎",
  provider_off: "尚未選擇整理引擎",
  provider_incomplete: "整理引擎設定尚未完成",
  provider_unavailable: "整理引擎目前不可用",
  model_missing: "整理模型尚未下載",
  correct_consent_required: "尚未確認專業校字會調整授權詞彙的拼法格式",
  organize_consent_required: "尚未確認條理整理的行為差異",
  cloud_consent_required: "尚未確認雲端資料傳送",
  codex_not_installed: "尚未找到 Codex CLI",
  codex_auth_required: "Codex CLI 尚未登入",
  codex_unsupported: "Codex CLI 版本不支援安全校字",
  codex_consent_required: "尚未確認 Codex 雲端校字",
  codex_context_consent_required: "尚未確認有限畫面詞彙傳送",
  codex_unavailable: "Codex CLI 目前不可用",
  local_only: "「僅限本機」正在阻擋雲端引擎",
  invalid_endpoint: "自訂端點格式不正確",
  invalid_custom_url: "自訂端點格式不正確",
};

export interface PrivacyMessage {
  tone: "neutral" | "local" | "cloud" | "warning";
  title: string;
  detail: string;
}

export function polishReadinessLabel(
  llm: ResolvedLlmConfig | null,
  ready: boolean,
): string {
  if (!llm || !ready) return "尚未選擇";
  if (llm.polish_mode === "raw") return "原樣轉錄已就緒";
  if (llm.execution_location === "on_device") return "全本機整理已就緒";
  if (llm.execution_location === "local_service") return "本機服務整理已就緒";
  if (llm.execution_location === "cloud") {
    const destination =
      llm.destination_label ?? llm.endpoint_origin ?? "雲端端點";
    return llm.provider === "codex"
      ? `${destination} 雲端校字已就緒`
      : `${destination} 雲端整理已就緒`;
  }
  return "尚未選擇";
}

export function privacyMessage(
  llm: ResolvedLlmConfig | null,
  failed: boolean,
  contextEnabled: boolean,
): PrivacyMessage {
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
  const boundedContextIsSent =
    llm.effective_mode === "organize" && contextEnabled;
  const codexTermsAreSent =
    llm.provider === "codex" &&
    llm.effective_mode === "correct" &&
    contextEnabled &&
    llm.codex_share_context_terms &&
    llm.codex_context_consent_valid;
  if (llm.execution_location === "on_device") {
    return {
      tone: "local",
      title: `全本地・${mode}`,
      detail: "辨識與整理都在這台 Mac 完成。",
    };
  }
  if (llm.execution_location === "local_service") {
    const data = boundedContextIsSent ? "轉錄文字與畫面上下文" : "轉錄文字";
    return {
      tone: "local",
      title: `本機服務・${mode}`,
      detail: `${data}只送到這台 Mac 上的 ${PROVIDER_LABEL[llm.provider] ?? llm.provider}；音訊不傳送。`,
    };
  }
  if (llm.execution_location === "cloud") {
    const data = boundedContextIsSent
      ? "轉錄文字與畫面上下文"
      : codexTermsAreSent
        ? "轉錄文字、正確拼法清單與個人詞彙中與本次轉錄相關且內容保護可採用的正確文字，以及有限畫面候選詞（三類候選合計最多 32 項，不含 App 名或視窗標題）"
        : llm.provider === "codex"
          ? "轉錄文字、正確拼法清單與個人詞彙中與本次轉錄相關且內容保護可採用的正確文字（三類候選合計最多 32 項）"
          : "轉錄文字";
    const destination =
      llm.destination_label ?? llm.endpoint_origin ?? "你設定的端點";
    return {
      tone: "cloud",
      title: `雲端整理・${mode}`,
      detail: `${data}會送到 ${destination}；音訊不傳送。`,
    };
  }

  return {
    tone: "warning",
    title: `目前退回 ${MODE_LABEL.raw}`,
    detail: "整理引擎尚未就緒；目前沒有資料送往雲端。",
  };
}
