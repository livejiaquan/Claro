import type {
  CodexCliStatus,
  CodexPreferences,
  CodexStatus,
  CodexTestFailureReason,
  CodexTestState,
} from "./types";

// 時間前綴讓開發期 HMR 重新載入 module 後也不會從 1 重用仍在後端收尾的 id；
// 目前毫秒 epoch × 1000 仍低於 Number.MAX_SAFE_INTEGER。
let lastCodexTestRequestId = Date.now() * 1000;

/**
 * request id 必須跨 Settings remount 保持唯一；component-local counter 會在
 * 離開再返回設定頁時從 1 重新開始，可能撞到仍在後端收尾的舊請求。
 */
export function allocateCodexTestRequestId(): number {
  lastCodexTestRequestId += 1;
  return lastCodexTestRequestId;
}

/** 離開頁面時同時阻止 queued invoke，並 best-effort 取消已送到後端的測試。 */
export function cancelCodexTestOnDispose(
  requestId: number | null,
  cancelled: Set<number>,
  cancelBackend: (requestId: number) => Promise<unknown>,
): void {
  if (requestId === null) return;
  cancelled.add(requestId);
  void Promise.resolve()
    .then(() => cancelBackend(requestId))
    .catch(() => {
      // unmount 後沒有可顯示的 recovery UI；後端仍有自己的 timeout 上限。
    });
}

export interface CodexProviderUiState {
  status: CodexCliStatus;
  status_request_generation: number;
  test: CodexTestState;
  test_request_generation: number;
}

export type CodexProviderAction =
  | { type: "status_requested"; generation: number }
  | {
      type: "status_resolved";
      generation: number;
      status: CodexCliStatus;
    }
  | { type: "test_started"; generation: number }
  | { type: "test_cancel_requested"; generation: number }
  | { type: "test_cancel_failed"; generation: number }
  | { type: "test_cancelled"; generation: number }
  | {
      type: "test_succeeded";
      generation: number;
      input: string;
      output: string;
    }
  | {
      type: "test_failed";
      generation: number;
      reason: CodexTestFailureReason;
    }
  | { type: "test_reset" };

export const checkingCodexStatus = (): CodexCliStatus => ({
  availability: "checking",
  version: null,
  auth_mode: "unknown",
  error_code: null,
});

export const createCodexProviderUiState = (): CodexProviderUiState => ({
  status: checkingCodexStatus(),
  status_request_generation: 0,
  test: { phase: "idle" },
  test_request_generation: 0,
});

export function normalizeCodexStatus(status: CodexStatus): CodexCliStatus {
  const availability = (() => {
    switch (status.availability) {
      case "ready":
        return "ready";
      case "not_installed":
        return "not_installed";
      case "not_authenticated":
        return "auth_required";
      case "unsupported":
        return "unsupported";
      case "missing_capability":
      case "probe_failed":
        return "unavailable";
    }
  })();

  return {
    availability,
    version: status.version,
    auth_mode:
      status.auth_mode === "chat_gpt"
        ? "chatgpt"
        : status.auth_mode === "api_key"
          ? "api_key"
          : "unknown",
    error_code: status.error_code,
  };
}

export function nextCodexStatusRequestGeneration(
  state: Pick<CodexProviderUiState, "status_request_generation">,
): number {
  return state.status_request_generation + 1;
}

export function nextCodexTestRequestGeneration(
  state: Pick<CodexProviderUiState, "test_request_generation">,
): number {
  return state.test_request_generation + 1;
}

export function codexProviderReducer(
  state: CodexProviderUiState,
  action: CodexProviderAction,
): CodexProviderUiState {
  switch (action.type) {
    case "status_requested":
      if (action.generation <= state.status_request_generation) return state;
      return {
        ...state,
        status: checkingCodexStatus(),
        status_request_generation: action.generation,
      };
    case "status_resolved":
      if (action.generation !== state.status_request_generation) return state;
      return { ...state, status: action.status };
    case "test_started":
      if (action.generation <= state.test_request_generation) return state;
      return {
        ...state,
        test: { phase: "running" },
        test_request_generation: action.generation,
      };
    case "test_cancel_requested":
      if (
        action.generation !== state.test_request_generation ||
        state.test.phase !== "running"
      ) {
        return state;
      }
      return { ...state, test: { phase: "cancelling" } };
    case "test_cancel_failed":
      if (
        action.generation !== state.test_request_generation ||
        state.test.phase !== "cancelling"
      ) {
        return state;
      }
      // 傳輸層沒能送出取消命令，不代表原測試失敗。回到 running，
      // 讓原本的 request-scoped 結果仍能正常收斂並顯示。
      return { ...state, test: { phase: "running" } };
    case "test_cancelled":
      if (
        action.generation !== state.test_request_generation ||
        (state.test.phase !== "running" && state.test.phase !== "cancelling")
      ) {
        return state;
      }
      return { ...state, test: { phase: "cancelled" } };
    case "test_succeeded":
      if (
        action.generation !== state.test_request_generation ||
        state.test.phase !== "running"
      ) {
        return state;
      }
      return {
        ...state,
        test: {
          phase: "success",
          input: action.input,
          output: action.output,
        },
      };
    case "test_failed":
      if (
        action.generation !== state.test_request_generation ||
        (state.test.phase !== "running" && state.test.phase !== "cancelling")
      ) {
        return state;
      }
      return {
        ...state,
        test: { phase: "failed", reason: action.reason },
      };
    case "test_reset":
      return { ...state, test: { phase: "idle" } };
  }
}

export function codexConsentIsReady(
  preferences: CodexPreferences,
  globalContextEnabled: boolean,
): boolean {
  return (
    codexPrimaryConsentIsValid(preferences) &&
    preferences.correct_mode_active &&
    (!preferences.share_context_terms ||
      !globalContextEnabled ||
      preferences.context_consent_valid)
  );
}

/** 雲端資料範圍與 CORRECT 行為同意仍有效；和「目前是否正在使用」分開。 */
export function codexPrimaryConsentIsValid(
  preferences: CodexPreferences,
): boolean {
  return preferences.consent_valid && preferences.correct_consent_valid;
}

export function codexTestIsPending(test: CodexTestState): boolean {
  return test.phase === "running" || test.phase === "cancelling";
}

/** queued test 尚未 invoke 前也能被取消；consume 後不污染後續 generation。 */
export function consumeCancelledCodexTest(
  cancelled: Set<number>,
  generation: number,
): boolean {
  if (!cancelled.has(generation)) return false;
  cancelled.delete(generation);
  return true;
}
