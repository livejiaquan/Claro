export interface Status {
  model_id: string;
  model_label: string;
  model_present: boolean;
  model_approx_mb: number;
  accessibility: boolean;
  hotkey_active: boolean;
  input_device: string | null;
  default_input: string | null;
  input_devices: string[];
  dictation_state: "idle" | "recording" | "processing";
  context_enabled: boolean;
  hotkey: string;
  setup_completed: boolean;
  polish_mode?: PolishMode;
  effective_mode?: PolishMode;
  llm_provider?: string;
  local_only?: boolean;
  execution_location?: ExecutionLocation;
  endpoint_origin?: string | null;
  blocked_reason?: PolishBlockedReason;
}

export interface ModelInfo {
  id: string;
  label: string;
  desc: string;
  size_mb: number;
  recommended: boolean;
  downloaded: boolean;
  active: boolean;
  downloading: boolean;
}

export type PolishMode = "raw" | "clean" | "organize";

export type ExecutionLocation = "none" | "on_device" | "local_service" | "cloud";

export type PolishBlockedReason =
  | "provider_missing"
  | "provider_off"
  | "provider_incomplete"
  | "provider_unavailable"
  | "model_missing"
  | "organize_consent_required"
  | "cloud_consent_required"
  | "local_only"
  | "invalid_endpoint"
  | "invalid_custom_url"
  | null;

export interface LlmConfig {
  provider: string;
  model: string;
  base_url: string;
  has_key: boolean;
  /** Apple Intelligence 可用性：0=可用 1=裝置不支援 2=未開啟 3=模型下載中 4=系統過舊 5=其他 */
  apple_status: number;
  /** 新後端使用 polish_mode；mode 暫留為整合期間的相容欄位。 */
  polish_mode?: PolishMode;
  mode?: PolishMode;
  effective_mode?: PolishMode;
  local_only?: boolean;
  organize_consent_valid?: boolean;
  cloud_consent_valid?: boolean;
  execution_location?: ExecutionLocation;
  endpoint_origin?: string | null;
  blocked_reason?: PolishBlockedReason;
}

export interface ResolvedLlmConfig extends LlmConfig {
  polish_mode: PolishMode;
  effective_mode: PolishMode;
  local_only: boolean;
  organize_consent_valid: boolean;
  cloud_consent_valid: boolean;
  execution_location: ExecutionLocation;
  endpoint_origin: string | null;
  blocked_reason: PolishBlockedReason;
}

function endpointInfo(baseUrl: string): { location: ExecutionLocation; origin: string | null } {
  if (!baseUrl.trim()) return { location: "cloud", origin: null };
  try {
    const url = new URL(baseUrl);
    const host = url.hostname.toLowerCase();
    const loopback = host === "localhost" || host === "127.0.0.1" || host === "::1";
    return { location: loopback ? "local_service" : "cloud", origin: url.origin };
  } catch {
    return { location: "cloud", origin: null };
  }
}

/**
 * 後端 P0 欄位整合期間的單一相容層。等 Rust 全面回傳 derived fields 後，
 * 這裡仍保留 fail-closed fallback，避免未知 provider 被誤標成全本地。
 */
export function resolveLlmConfig(config: LlmConfig): ResolvedLlmConfig {
  const polishMode = config.polish_mode ?? config.mode ?? (config.provider === "off" ? "raw" : "clean");
  // 舊後端沒有回傳此欄位時也採隱私預設，避免相容層意外放寬資料流向。
  const localOnly = config.local_only ?? true;

  let inferredLocation: ExecutionLocation;
  let inferredOrigin: string | null = null;
  if (config.provider === "off" || !config.provider) {
    inferredLocation = "none";
  } else if (config.provider === "apple" || config.provider === "builtin") {
    inferredLocation = "on_device";
  } else if (config.provider === "ollama" || config.provider === "lmstudio") {
    inferredLocation = "local_service";
  } else if (config.provider === "custom") {
    const info = endpointInfo(config.base_url);
    inferredLocation = info.location;
    inferredOrigin = info.origin;
  } else {
    // 未知 provider 絕不推定為本地。
    inferredLocation = "cloud";
  }

  const executionLocation = config.execution_location ?? inferredLocation;
  const endpointOrigin = config.endpoint_origin ?? inferredOrigin;
  const organizeConsentValid = config.organize_consent_valid ?? false;
  const cloudConsentValid = config.cloud_consent_valid ?? executionLocation !== "cloud";

  let blockedReason = config.blocked_reason ?? null;
  if (!blockedReason && polishMode === "organize" && !organizeConsentValid) {
    blockedReason = "organize_consent_required";
  }
  if (!blockedReason && polishMode !== "raw" && executionLocation === "cloud" && localOnly) {
    blockedReason = "local_only";
  }
  if (!blockedReason && polishMode !== "raw" && executionLocation === "cloud" && !cloudConsentValid) {
    blockedReason = "cloud_consent_required";
  }
  if (!blockedReason && polishMode !== "raw" && (config.provider === "off" || !config.provider)) {
    blockedReason = "provider_missing";
  }

  return {
    ...config,
    polish_mode: polishMode,
    effective_mode: config.effective_mode ?? (blockedReason ? "raw" : polishMode),
    local_only: localOnly,
    organize_consent_valid: organizeConsentValid,
    cloud_consent_valid: cloudConsentValid,
    execution_location: executionLocation,
    endpoint_origin: endpointOrigin,
    blocked_reason: blockedReason,
  };
}

export interface DictEntry {
  from: string;
  to: string;
}

export interface DownloadProgress {
  model_id: string;
  downloaded_mb: number;
  total_mb: number | null;
  done: boolean;
  error: string | null;
}

export interface MicLevel {
  level: number;
  active: boolean;
}

export interface HistoryEntry {
  ts: string;
  duration_s: number;
  /** 新版紀錄一定會保存；舊版 history.jsonl 可能沒有這個欄位。 */
  raw?: string;
  text: string;
  status: string;
  timings?: {
    stt_ms?: number;
    polish_ms?: number | null;
    /** 舊版/原型相容欄位。 */
    stt?: number;
    polish?: number | null;
  };
  polish?: HistoryPolishMetadata;
  /** 舊版扁平欄位只供既有 history.jsonl 相容。 */
  polish_mode?: PolishMode;
  mode?: PolishMode;
  polish_provider?: string;
  provider?: string;
  polish_outcome?: string;
  outcome?: string;
  fallback_reason?: string | null;
}

export interface HistoryPolishMetadata {
  mode: PolishMode;
  provider?: string;
  changed: boolean;
  outcome: "raw" | "changed" | "unchanged" | "fallback";
  fallback_reason?: string | null;
}
