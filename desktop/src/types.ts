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
  successful_pastes_this_launch: number;
  history_enabled: boolean;
  mic_test_passed_this_launch: boolean;
  polish_mode?: PolishMode;
  effective_mode?: PolishMode;
  llm_provider?: string;
  local_only?: boolean;
  execution_location?: ExecutionLocation;
  endpoint_origin?: string | null;
  blocked_reason?: PolishBlockedReason;
}

export interface HardwareProfile {
  architecture: string;
  memory_gb: number;
  tier: "compact" | "balanced" | "performance";
  low_memory_mode: boolean;
  keep_models_warm: boolean;
  recommended_stt: string;
  recommended_llm_provider: "apple" | "builtin";
  recommended_llm_model: string;
  reason: string;
}

export interface ContextAudit {
  text: string;
  app_id: string;
  app_name: string;
  surface: "email" | "message" | "technical" | "document" | "neutral";
}

export interface PendingResult {
  raw: string;
  text: string;
  reason: "focus_changed" | "paste_failed";
}

export type DictationOutcome =
  | "pasted"
  | "paste_failed"
  | "focus_changed"
  | "stt_failed"
  | "silent"
  | "cancelled"
  | "error";

export interface DictationEvent {
  session: number;
  phase: "processing" | "finished";
  outcome: DictationOutcome | null;
  recovery_available: boolean;
}

export interface ModelInfo {
  id: string;
  label: string;
  desc: string;
  size_mb: number;
  recommended: boolean;
  /** false 表示只供透明展示，下載與啟用都必須 fail closed。 */
  available: boolean;
  preview: boolean;
  downloaded: boolean;
  active: boolean;
  downloading: boolean;
}

export function isPreviewSttModel(model: Pick<ModelInfo, "preview">): boolean {
  return model.preview;
}

export function displaySttModelLabel(model: Pick<ModelInfo, "label" | "preview">): string {
  return isPreviewSttModel(model) ? model.label.replace(/\s*[（(]預覽[）)]\s*$/, "") : model.label;
}

export type PolishMode = "raw" | "clean" | "correct" | "organize";

export type ExecutionLocation = "none" | "on_device" | "local_service" | "cloud";

export type CodexAvailability =
  | "checking"
  | "ready"
  | "not_installed"
  | "auth_required"
  | "unsupported"
  | "unavailable";

export type CodexAuthMode = "chatgpt" | "api_key" | "unknown";

/** Rust runner 的原始 probe 契約；UI 必須先正規化，不能直接顯示 error_code。 */
export type CodexRunnerAvailability =
  | "ready"
  | "not_installed"
  | "unsupported"
  | "missing_capability"
  | "not_authenticated"
  | "probe_failed";

export type CodexRunnerAuthMode = "chat_gpt" | "api_key";

export interface CodexStatus {
  availability: CodexRunnerAvailability;
  version: string | null;
  auth_mode: CodexRunnerAuthMode | null;
  executable_path: string | null;
  error_code: string | null;
}

/** 不含執行檔路徑與原始診斷內容、可直接交給 UI 的狀態。 */
export interface CodexCliStatus {
  availability: CodexAvailability;
  version: string | null;
  auth_mode: CodexAuthMode;
  /** 後端清理過、可安全映射成 UI 文案的代碼；不可放 CLI 原始 stderr。 */
  error_code: string | null;
}

export interface CodexPreferences {
  correction_preferences: string;
  share_context_terms: boolean;
  consent_valid: boolean;
  context_consent_valid: boolean;
  correct_consent_valid: boolean;
  correct_mode_active: boolean;
}

export interface CodexEnableOptions {
  share_context_terms: boolean;
}

export type CodexTestFailureReason =
  | "timeout"
  | "rate_limited"
  | "auth_required"
  | "unavailable"
  | "consent_changed"
  | "output_rejected"
  | "unknown";

/** Rust 測試 command 的 tagged-union 回傳值；不要再從錯誤字串猜狀態。 */
export type CodexTestResult =
  | { phase: "success"; input: string; output: string }
  | { phase: "failed"; reason: Exclude<CodexTestFailureReason, "unknown"> }
  | { phase: "cancelled" };

export type CodexTestState =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "cancelling" }
  | { phase: "cancelled" }
  | { phase: "success"; input: string; output: string }
  | { phase: "failed"; reason: CodexTestFailureReason };

export type PolishBlockedReason =
  | "provider_missing"
  | "provider_off"
  | "provider_incomplete"
  | "provider_unavailable"
  | "model_missing"
  | "organize_consent_required"
  | "correct_consent_required"
  | "cloud_consent_required"
  | "local_only"
  | "invalid_endpoint"
  | "invalid_custom_url"
  | "codex_not_installed"
  | "codex_auth_required"
  | "codex_unsupported"
  | "codex_consent_required"
  | "codex_context_consent_required"
  | "codex_unavailable"
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
  correct_consent_valid?: boolean;
  cloud_consent_valid?: boolean;
  execution_location?: ExecutionLocation;
  endpoint_origin?: string | null;
  destination_label?: string | null;
  codex_consent_valid?: boolean;
  codex_context_consent_valid?: boolean;
  codex_share_context_terms?: boolean;
  codex_correction_preferences?: string;
  blocked_reason?: PolishBlockedReason;
}

export interface ResolvedLlmConfig extends LlmConfig {
  polish_mode: PolishMode;
  effective_mode: PolishMode;
  local_only: boolean;
  organize_consent_valid: boolean;
  correct_consent_valid: boolean;
  cloud_consent_valid: boolean;
  execution_location: ExecutionLocation;
  endpoint_origin: string | null;
  destination_label: string | null;
  codex_consent_valid: boolean;
  codex_context_consent_valid: boolean;
  codex_share_context_terms: boolean;
  codex_correction_preferences: string;
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
  let inferredDestinationLabel: string | null = null;
  if (config.provider === "off" || !config.provider) {
    inferredLocation = "none";
  } else if (config.provider === "apple" || config.provider === "builtin") {
    inferredLocation = "on_device";
  } else if (config.provider === "ollama" || config.provider === "lmstudio") {
    inferredLocation = "local_service";
  } else if (config.provider === "codex") {
    inferredLocation = "cloud";
    inferredDestinationLabel = "OpenAI Codex";
  } else if (config.provider === "custom") {
    const info = endpointInfo(config.base_url);
    inferredLocation = info.location;
    inferredOrigin = info.origin;
  } else {
    // 未知 provider 絕不推定為本地。
    inferredLocation = "cloud";
  }

  // Codex 永遠是 OpenAI 雲端服務；即使舊後端誤標也不可在 UI 宣稱為本機。
  const executionLocation =
    config.provider === "codex" ? "cloud" : (config.execution_location ?? inferredLocation);
  const endpointOrigin =
    config.provider === "codex" ? null : (config.endpoint_origin ?? inferredOrigin);
  const destinationLabel =
    config.provider === "codex"
      ? "OpenAI Codex"
      : (config.destination_label ?? inferredDestinationLabel);
  const organizeConsentValid = config.organize_consent_valid ?? false;
  const correctConsentValid = config.correct_consent_valid ?? false;
  const codexConsentValid = config.codex_consent_valid ?? false;
  const codexContextConsentValid = config.codex_context_consent_valid ?? false;
  const codexShareContextTerms = config.codex_share_context_terms ?? false;
  const codexCorrectionPreferences = config.codex_correction_preferences ?? "";
  const cloudConsentValid =
    config.cloud_consent_valid ??
    (config.provider === "codex" ? codexConsentValid : executionLocation !== "cloud");

  let blockedReason = config.blocked_reason ?? null;
  if (!blockedReason && polishMode === "organize" && !organizeConsentValid) {
    blockedReason = "organize_consent_required";
  }
  if (!blockedReason && polishMode === "correct" && !correctConsentValid) {
    blockedReason = "correct_consent_required";
  }
  if (
    !blockedReason &&
    polishMode === "correct" &&
    config.provider !== "codex"
  ) {
    // 舊後端若仍把一般 provider 的 CLEAN 路徑回報成 CORRECT，前端也要
    // fail closed，不能向使用者宣稱已做專業校字。
    blockedReason = "provider_unavailable";
  }
  if (
    !blockedReason &&
    polishMode !== "raw" &&
    polishMode !== "correct" &&
    config.provider === "codex"
  ) {
    blockedReason = "provider_unavailable";
  }
  if (
    !blockedReason &&
    polishMode !== "raw" &&
    config.provider === "codex" &&
    !codexConsentValid
  ) {
    blockedReason = "codex_consent_required";
  }
  if (
    !blockedReason &&
    polishMode !== "raw" &&
    config.provider === "codex" &&
    codexShareContextTerms &&
    !codexContextConsentValid
  ) {
    blockedReason = "codex_context_consent_required";
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
    // 前端新認得的隱私 gate 必須能覆蓋舊後端傳回的過時 effective_mode。
    effective_mode: blockedReason ? "raw" : (config.effective_mode ?? polishMode),
    local_only: localOnly,
    organize_consent_valid: organizeConsentValid,
    correct_consent_valid: correctConsentValid,
    cloud_consent_valid: cloudConsentValid,
    execution_location: executionLocation,
    endpoint_origin: endpointOrigin,
    destination_label: destinationLabel,
    codex_consent_valid: codexConsentValid,
    codex_context_consent_valid: codexContextConsentValid,
    codex_share_context_terms: codexShareContextTerms,
    codex_correction_preferences: codexCorrectionPreferences,
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
  downloaded: boolean;
  activation_status: "none" | "waiting_for_idle" | "retry_required";
  error: string | null;
}

export interface MicLevel {
  level: number;
  active: boolean;
  generation: number;
  passed: boolean;
  timed_out: boolean;
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
    /** 使用者放開熱鍵到文字完成貼上的端到端延遲。 */
    release_to_paste_ms?: number;
    /** 貼上前第一次 bounded AX 目標驗證。 */
    focus_guard_ms?: number;
    /** 剪貼簿交易、最後焦點驗證、Cmd+V 與還原等待。 */
    inject_ms?: number;
    stt_model?: string;
    stt_family?: "Whisper" | "Qwen3Asr" | string;
    stt_language?: string;
    prompt_term_count?: number;
    context_term_count?: number;
    audio_input_rms?: number;
    audio_clipped_ratio?: number;
    /** 2026-07-15 前曾記錄 peak-normalized RMS，不能當作實際輸入音量。 */
    audio_rms?: number;
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
  /** false=沒有任何轉錄 payload byte 寫入主 Codex request；true=傳送已開始。 */
  codex_payload_started?: boolean;
}
