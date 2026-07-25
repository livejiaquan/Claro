import { invoke } from "@tauri-apps/api/core";
import {
  useEffect,
  useReducer,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import CodexProviderPanel from "../components/CodexProviderPanel";
import {
  allocateCodexTestRequestId,
  cancelCodexTestOnDispose,
  codexProviderReducer,
  consumeCancelledCodexTest,
  createCodexProviderUiState,
  normalizeCodexStatus,
} from "../codexProviderState";
import DownloadStatus from "../components/DownloadStatus";
import {
  resolveDownloadState,
  type DownloadKind,
} from "../downloadState";
import {
  displaySttModelLabel,
  isPreviewSttModel,
  resolveLlmConfig,
  type ContextAudit,
  type CodexEnableOptions,
  type CodexPreferences,
  type CodexStatus,
  type CodexTestResult,
  type DictEntry,
  type DownloadProgress,
  type LlmConfig,
  type MicLevel,
  type ModelInfo,
  type PolishMode,
  type ResolvedLlmConfig,
  type Status,
} from "../types";
import { Hotkey, LevelBar, Row, Section } from "../ui";

/** 自訂 API 的常用服務 preset：帶入 Base URL 與建議模型（都走 OpenAI 相容介面） */
const CUSTOM_PRESETS = [
  { id: "openai", label: "OpenAI", baseUrl: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  { id: "groq", label: "Groq", baseUrl: "https://api.groq.com/openai/v1", model: "llama-3.3-70b-versatile" },
  { id: "deepseek", label: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", model: "deepseek-chat" },
  { id: "gemini", label: "Google Gemini", baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai", model: "gemini-2.5-flash" },
  { id: "openrouter", label: "OpenRouter", baseUrl: "https://openrouter.ai/api/v1", model: "openai/gpt-4o-mini" },
];

const POLISH_MODES: { id: PolishMode; title: string; tag?: string; description: string }[] = [
  {
    id: "raw",
    title: "原樣轉錄",
    description: "不使用文字整理模型；只套用繁體、個人字典與基本標點。",
  },
  {
    id: "clean",
    title: "保守校訂",
    tag: "預設",
    description: "只處理有停頓邊界的語助詞、明確改口與標點；不改字、不跨句重排、不濃縮。",
  },
  {
    id: "correct",
    title: "專業校字",
    tag: "實驗・需確認",
    description: "目前只支援 Codex；只嘗試極窄的單連字號修正，仍可能誤改。",
  },
  {
    id: "organize",
    title: "條理整理",
    tag: "需確認",
    description: "鎖定姓名、數字、時間、否定與其他事實後，只允許完整句子重排、分段與明確清單格式。",
  },
];

function modeSupportedByProvider(mode: PolishMode, provider: string): boolean {
  if (mode === "raw") return true;
  if (mode === "correct") return provider === "codex";
  return provider !== "codex";
}

function preferredScrollBehavior(): ScrollBehavior {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
    ? "auto"
    : "smooth";
}

const OUTCOME_LABEL: Record<string, string> = {
  provider_missing: "尚未選擇處理引擎",
  provider_off: "尚未選擇處理引擎",
  provider_incomplete: "處理引擎設定尚未完成",
  provider_unavailable: "處理引擎目前不可用",
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
  local_only: "僅限本機已阻擋雲端引擎",
  invalid_endpoint: "自訂端點格式不正確",
  invalid_custom_url: "自訂端點格式不正確",
};

function codexMutationErrorCopy(reason: unknown): string {
  const code = String(reason);
  if (code.includes("codex_request_stale")) {
    return "設定在處理期間已改變，Claro 沒有套用舊請求。請確認目前選項後再試一次。";
  }
  if (code.includes("codex_not_installed")) return "沒有找到 Codex CLI。安裝後請重新檢查。";
  if (code.includes("codex_auth_required")) return "Codex CLI 尚未登入。登入後請重新檢查。";
  if (code.includes("codex_unsupported")) return "目前的 Codex CLI 版本不支援安全校字。更新後請重新檢查。";
  if (code.includes("codex_consent_required")) return "Codex 的登入或 CLI 版本已改變，請重新檢查並確認資料傳送。";
  if (code.includes("codex_test_duplicate")) return "這次測試已經在執行中。";
  if (code.includes("螢幕上下文已關閉")) return "螢幕上下文已關閉，因此不能分享畫面詞彙。";
  return "Codex 設定目前無法完成。請重新檢查 Codex 後再試一次。";
}

function sttModelSortRank(model: ModelInfo) {
  if (model.active) return 0;
  if (model.available && model.recommended) return 1;
  if (model.available) return 2;
  if (model.preview) return 4;
  return 3;
}

export default function Settings({
  status,
  mic,
  progress,
  llmProgress,
  onDownloadStart,
  refresh,
  onToast,
  onOpenSetup,
  focusTarget,
  onFocusTargetHandled,
  codexCorrectionDraft,
  onCodexCorrectionDraftChange,
}: {
  status: Status;
  mic: MicLevel;
  progress: DownloadProgress | null;
  llmProgress: DownloadProgress | null;
  onDownloadStart: (kind: DownloadKind) => void;
  refresh: () => void;
  onToast: (msg: string) => void;
  onOpenSetup: () => void;
  focusTarget: "codex" | null;
  onFocusTargetHandled: () => void;
  codexCorrectionDraft: string | null;
  onCodexCorrectionDraftChange: Dispatch<SetStateAction<string | null>>;
}) {
  const [error, setError] = useState<string | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [llm, setLlm] = useState<ResolvedLlmConfig | null>(null);
  const [keyDraft, setKeyDraft] = useState("");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [pendingMode, setPendingMode] = useState<PolishMode | null>(null);
  const [codexUi, dispatchCodex] = useReducer(
    codexProviderReducer,
    undefined,
    () => createCodexProviderUiState(),
  );
  const [codexEnablePending, setCodexEnablePending] = useState(false);
  const [codexPreferencesSaving, setCodexPreferencesSaving] = useState(false);
  const [codexCorrectionSaving, setCodexCorrectionSaving] = useState(false);
  const [codexError, setCodexError] = useState<string | null>(null);
  const [polishError, setPolishError] = useState<string | null>(null);
  const setCodexCorrectionDraft = onCodexCorrectionDraftChange;
  const [codexPolicySaving, setCodexPolicySaving] = useState(false);
  const codexStatusGeneration = useRef(0);
  const activeCodexTestRequestId = useRef<number | null>(null);
  const cancelledCodexTests = useRef(new Set<number>());
  const llmLoadGeneration = useRef(0);
  const llmRef = useRef<ResolvedLlmConfig | null>(null);
  const llmSaveQueue = useRef<Promise<unknown>>(Promise.resolve());
  const llmSaveSeq = useRef(0);
  const codexSaveQueue = useRef<Promise<unknown>>(Promise.resolve());
  const codexMutationSeq = useRef(0);
  const codexCorrectionSaveSeq = useRef(0);
  const codexCorrectionPending = useRef<{
    value: string;
    promise: Promise<void>;
  } | null>(null);
  const modeConsentHeadingRef = useRef<HTMLDivElement>(null);
  const modeReturnFocusRef = useRef<HTMLElement | null>(null);
  const polishProviderSelectRef = useRef<HTMLSelectElement>(null);
  const codexErrorRef = useRef<HTMLDivElement>(null);
  const polishErrorRef = useRef<HTMLDivElement>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [confirmCloud, setConfirmCloud] = useState(false);
  const [confirmDisableLocalOnly, setConfirmDisableLocalOnly] = useState(false);
  // 清單/錯誤都綁 provider：偵測回應晚到時不能污染已切換的 provider（race）
  const [localModels, setLocalModels] = useState<{ provider: string; models: string[] } | null>(null);
  const [localModelsErr, setLocalModelsErr] = useState<{ provider: string; msg: string } | null>(null);

  const [builtinLlms, setBuiltinLlms] = useState<ModelInfo[]>([]);
  const [dict, setDict] = useState<DictEntry[] | null>(null);
  const [dictDraft, setDictDraft] = useState<DictEntry>({ from: "", to: "" });
  const [contextAudit, setContextAudit] = useState<ContextAudit | null>(null);
  const [downloadIntent, setDownloadIntent] = useState<{
    key: string;
    phase: "starting" | "cancelling";
  } | null>(null);

  const loadModels = () => invoke<ModelInfo[]>("list_models").then(setModels).catch(() => {});
  const acceptLlmConfig = (config: LlmConfig) => {
    ++llmLoadGeneration.current;
    const resolved = resolveLlmConfig(config);
    llmRef.current = resolved;
    setLlm(resolved);
    setCodexCorrectionDraft((current) =>
      current !== null &&
      current.trim() === resolved.codex_correction_preferences
        ? null
        : current,
    );
  };
  const loadLlm = () => {
    const generation = ++llmLoadGeneration.current;
    return invoke<LlmConfig>("get_llm_config")
      .then((config) => {
        if (generation === llmLoadGeneration.current) acceptLlmConfig(config);
      })
      .catch((e) => {
        if (generation === llmLoadGeneration.current) setError(String(e));
      });
  };
  const loadBuiltinLlms = () =>
    invoke<ModelInfo[]>("list_builtin_llms").then(setBuiltinLlms).catch(() => {});
  const loadDict = () => invoke<DictEntry[]>("get_dictionary").then(setDict).catch(() => {});
  const loadContextAudit = () =>
    invoke<ContextAudit | null>("get_context_audit").then(setContextAudit).catch(() => {});

  // 字典整存（串行化，同 saveLlm 的理由）
  const dictSaveQueue = useRef<Promise<unknown>>(Promise.resolve());
  const saveDict = (entries: DictEntry[]) => {
    setDict(entries);
    dictSaveQueue.current = dictSaveQueue.current.then(() =>
      invoke("set_dictionary", { entries }).catch((e) => setError(String(e))),
    );
  };

  const loadLocalModels = (provider: string) => {
    setLocalModels(null);
    setLocalModelsErr(null);
    invoke<string[]>("list_provider_models", { provider })
      .then((models) => setLocalModels({ provider, models }))
      .catch((e) => setLocalModelsErr({ provider, msg: String(e) }));
  };

  const probeCodex = () => {
    setCodexError(null);
    const generation = ++codexStatusGeneration.current;
    dispatchCodex({ type: "status_requested", generation });
    invoke<CodexStatus>("get_codex_status")
      .then((result) => {
        dispatchCodex({
          type: "status_resolved",
          generation,
          status: normalizeCodexStatus(result),
        });
        // probe 可能發現登入類型已改變；重新讀 derived consent gate。
        loadLlm();
      })
      .catch(() => {
        dispatchCodex({
          type: "status_resolved",
          generation,
          status: {
            availability: "unavailable",
            version: null,
            auth_mode: "unknown",
            error_code: "probe_failed",
          },
        });
      });
  };

  useEffect(() => {
    loadModels();
    loadLlm();
    loadBuiltinLlms();
    loadDict();
    loadContextAudit();
    // 僅在 mount 載入；各 loader 的 closure 每次 render 會重建。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(
    () => () => {
      const requestId = activeCodexTestRequestId.current;
      activeCodexTestRequestId.current = null;
      cancelCodexTestOnDispose(
        requestId,
        cancelledCodexTests.current,
        (activeRequestId) =>
          invoke<boolean>("cancel_codex_test", {
            requestId: activeRequestId,
          }),
      );
    },
    [],
  );
  // 內建 LLM 下載進度改變清單狀態
  useEffect(() => {
    loadBuiltinLlms();
  }, [
    llmProgress?.model_id,
    llmProgress?.done,
    llmProgress?.downloaded,
    llmProgress?.error,
    llm?.model,
    llm?.provider,
  ]);

  // 選到本機服務時偵測其模型清單
  useEffect(() => {
    if (llm && (llm.provider === "ollama" || llm.provider === "lmstudio")) {
      loadLocalModels(llm.provider);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [llm?.provider]);
  useEffect(() => {
    if (llm?.provider === "codex") probeCodex();
    else {
      dispatchCodex({ type: "test_reset" });
      setCodexError(null);
    }
    // provider 切換後才探測；Codex 不在首次設定的必要路徑，也不背景消耗額度。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [llm?.provider]);
  useEffect(() => {
    if (!codexError) return;
    const frame = requestAnimationFrame(() => {
      codexErrorRef.current?.scrollIntoView({
        block: "nearest",
        behavior: preferredScrollBehavior(),
      });
      codexErrorRef.current?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [codexError]);
  useEffect(() => {
    if (!polishError) return;
    const frame = requestAnimationFrame(() => {
      polishErrorRef.current?.scrollIntoView({
        block: "nearest",
        behavior: preferredScrollBehavior(),
      });
      polishErrorRef.current?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [polishError]);
  useEffect(() => {
    if (focusTarget !== "codex" || !llm) return;
    const frame = requestAnimationFrame(() => {
      polishProviderSelectRef.current?.scrollIntoView({
        block: "center",
        behavior: preferredScrollBehavior(),
      });
      polishProviderSelectRef.current?.focus();
      onFocusTargetHandled();
    });
    return () => cancelAnimationFrame(frame);
  }, [focusTarget, llm, onFocusTargetHandled]);
  // 下載進度事件會改變模型清單狀態
  useEffect(() => {
    loadModels();
  }, [
    progress?.model_id,
    progress?.done,
    progress?.downloaded,
    progress?.activation_status,
    progress?.error,
    status.model_id,
  ]);

  const call = (cmd: string, args?: Record<string, unknown>, then?: () => void) => {
    setError(null);
    invoke(cmd, args)
      .then(() => {
        refresh();
        loadModels();
        then?.();
      })
      .catch((e) => setError(String(e)));
  };

  const requestDownload = async (
    kind: DownloadKind,
    modelId: string,
    command: string,
    args: Record<string, unknown>,
  ) => {
    const key = `${kind}:${modelId}`;
    setError(null);
    setDownloadIntent({ key, phase: "starting" });
    onDownloadStart(kind);
    try {
      await invoke(command, args);
      refresh();
      if (kind === "stt") loadModels();
      else loadBuiltinLlms();
    } catch (reason) {
      setDownloadIntent((current) => (current?.key === key ? null : current));
      setError(String(reason));
    }
  };

  const cancelDownload = (kind: DownloadKind, modelId: string) => {
    const key = `${kind}:${modelId}`;
    setError(null);
    setDownloadIntent({ key, phase: "cancelling" });
    invoke("cancel_download").catch((reason) => {
      setDownloadIntent((current) => (current?.key === key ? null : current));
      setError(`無法取消下載：${String(reason)}`);
    });
  };

  const intentMatches = (
    kind: DownloadKind,
    modelId: string,
    phase: "starting" | "cancelling",
  ) => downloadIntent?.key === `${kind}:${modelId}` && downloadIntent.phase === phase;

  const sttDownloadState = (model: ModelInfo) =>
    resolveDownloadState({
      modelId: model.id,
      downloaded: model.downloaded,
      backendDownloading: model.downloading,
      progress,
      startRequested: intentMatches("stt", model.id, "starting"),
      cancelRequested: intentMatches("stt", model.id, "cancelling"),
    });

  const llmDownloadState = (model: ModelInfo) =>
    resolveDownloadState({
      modelId: model.id,
      downloaded: model.downloaded,
      backendDownloading: model.downloading,
      progress: llmProgress,
      startRequested: intentMatches("llm", model.id, "starting"),
      cancelRequested: intentMatches("llm", model.id, "cancelling"),
    });

  // 連續操作（選 preset 後馬上改欄位）的兩道防線：
  // 1) llmRef 即時反映最新值——閉包裡的舊 llm 不會把欄位倒退
  // 2) 寫入請求串行化——舊請求不會晚到蓋掉新請求
  // 刻意在 render 同步：事件可能在 effect 前連續觸發，非同步同步會重現欄位倒退 race。
  // eslint-disable-next-line react-hooks/refs
  llmRef.current = llm;
  // 第三道防線（review 發現）：佇列中「較舊請求」的後端回應不能落地——
  // 快速連續輸入時，第一個字元的回應會把畫面與 llmRef 倒退成舊前綴，
  // 後續字元再從舊值 merge，最終覆蓋掉完整的新值
  const saveLlm = (next: Partial<ResolvedLlmConfig>, confirmed = false) => {
    const cur = llmRef.current;
    if (!cur) return;
    setPolishError(null);
    const switchingAwayFromCodex =
      next.provider !== undefined &&
      cur.provider === "codex" &&
      next.provider !== "codex";
    const downgradeCodexCorrection =
      switchingAwayFromCodex && cur.polish_mode === "correct";
    const hasDirtyCodexDraft =
      next.provider !== undefined &&
      next.provider !== "codex" &&
      codexCorrectionDraft !== null &&
      codexCorrectionDraft.trim() !== cur.codex_correction_preferences;
    // select change 前通常先觸發 textarea blur；這裡再補一道 Settings-scope
    // flush。保留會 reject 的 promise，儲存失敗時就不切走 Codex，避免草稿
    // 在 provider panel unmount 後靜默遺失。
    const blockingDraftSave = hasDirtyCodexDraft
      ? saveCodexCorrectionPreferences(codexCorrectionDraft)
      : Promise.resolve();
    const pendingCodexSaves = switchingAwayFromCodex || hasDirtyCodexDraft
      ? Promise.all([codexSaveQueue.current, blockingDraftSave]).then(
          () => undefined,
        )
      : Promise.resolve();
    const previousLlmSaves = llmSaveQueue.current;
    const merged = {
      ...cur,
      ...next,
      ...(downgradeCodexCorrection
        ? { polish_mode: "clean" as const, effective_mode: "clean" as const }
        : {}),
    };
    llmRef.current = merged;
    ++llmLoadGeneration.current;
    setLlm(merged);
    if (next.provider !== undefined) {
      setPendingMode(null);
      setConfirmCloud(false);
      setConfirmDisableLocalOnly(false);
    }
    setTestResult(null);
    const seq = ++llmSaveSeq.current;
    llmSaveQueue.current = previousLlmSaves
      .then(() => pendingCodexSaves)
      .then(() =>
        invoke<LlmConfig | null>("set_llm_config", {
          provider: merged.provider,
          model: merged.model,
          baseUrl: merged.base_url,
          confirmed,
        }),
      )
      .then((updated) => {
        if (seq === llmSaveSeq.current) {
          if (updated) acceptLlmConfig(updated);
          else loadLlm();
          if (confirmed) {
            setConfirmCloud(false);
            onToast("已確認此雲端端點");
          } else if (downgradeCodexCorrection) {
            onToast("已離開 Codex；文字整理改為保守校訂");
          }
        }
      })
      .catch((e) => {
        // 吞在 queue 尾端，讓一次失敗不會永久阻塞後續設定；同時重讀後端
        // authoritative 狀態，撤回尚未成功的 optimistic provider 切換。
        if (seq === llmSaveSeq.current) {
          setPolishError(`無法更新文字整理位置：${String(e)}`);
          loadLlm();
        }
      });
  };

  const applyMode = (mode: PolishMode, confirmed: boolean) => {
    setPolishError(null);
    const invokeMode = () =>
      invoke<LlmConfig | null>("set_polish_mode", { mode, confirmed });
    const applyUpdated = (updated: LlmConfig | null) => {
        if (updated) acceptLlmConfig(updated);
        else loadLlm();
        setPendingMode(null);
        if (confirmed) {
          requestAnimationFrame(() => modeReturnFocusRef.current?.focus());
        }
        onToast(
          mode === "raw"
            ? "已切換為原樣轉錄"
            : mode === "clean"
              ? "已切換為保守校訂"
              : mode === "correct"
                ? "已啟用專業校字"
                : "已啟用條理整理",
        );
    };
    if (llmRef.current?.provider === "codex") {
      const seq = ++codexMutationSeq.current;
      setCodexPolicySaving(true);
      codexSaveQueue.current = codexSaveQueue.current
        .then(() => llmSaveQueue.current)
        .then(invokeMode)
        .then((updated) => {
          if (seq === codexMutationSeq.current) applyUpdated(updated);
        })
        .catch((e) => {
          if (seq === codexMutationSeq.current) {
            setPolishError(
              `無法切換文字整理模式：${codexMutationErrorCopy(e)}`,
            );
          }
        })
        .finally(() => {
          if (seq === codexMutationSeq.current) setCodexPolicySaving(false);
        });
      return;
    }
    invokeMode()
      .then(applyUpdated)
      .catch((e) =>
        setPolishError(`無法切換文字整理模式：${String(e)}`),
      );
  };

  const requestMode = (mode: PolishMode) => {
    if (!llm || mode === llm.polish_mode) return;
    if (mode === "correct" && !llm.correct_consent_valid) {
      modeReturnFocusRef.current =
        document.activeElement instanceof HTMLElement ? document.activeElement : null;
      setPendingMode("correct");
      return;
    }
    if (mode === "organize" && !llm.organize_consent_valid) {
      modeReturnFocusRef.current =
        document.activeElement instanceof HTMLElement ? document.activeElement : null;
      setPendingMode("organize");
      return;
    }
    applyMode(mode, false);
  };

  useEffect(() => {
    if (!pendingMode) return;
    requestAnimationFrame(() => modeConsentHeadingRef.current?.focus());
  }, [pendingMode]);

  const cancelPendingMode = () => {
    setPendingMode(null);
    requestAnimationFrame(() => modeReturnFocusRef.current?.focus());
  };

  const applyLocalOnly = (enabled: boolean, confirmed: boolean) => {
    setPolishError(null);
    const invokeLocalOnly = () =>
      invoke<LlmConfig | null>("set_local_only", { enabled, confirmed });
    const applyUpdated = (updated: LlmConfig | null) => {
        if (updated) acceptLlmConfig(updated);
        else loadLlm();
        setConfirmDisableLocalOnly(false);
        setConfirmCloud(false);
        onToast(enabled ? "已限制為僅限本機" : "已允許使用雲端端點");
    };
    if (llmRef.current?.provider === "codex") {
      const seq = ++codexMutationSeq.current;
      setCodexPolicySaving(true);
      codexSaveQueue.current = codexSaveQueue.current
        .then(() => llmSaveQueue.current)
        .then(invokeLocalOnly)
        .then((updated) => {
          if (seq === codexMutationSeq.current) applyUpdated(updated);
        })
        .catch((e) => {
          if (seq === codexMutationSeq.current) {
            setPolishError(
              `無法更新雲端資料限制：${codexMutationErrorCopy(e)}`,
            );
          }
        })
        .finally(() => {
          if (seq === codexMutationSeq.current) setCodexPolicySaving(false);
        });
      return;
    }
    invokeLocalOnly()
      .then(applyUpdated)
      .catch((e) =>
        setPolishError(`無法更新雲端資料限制：${String(e)}`),
      );
  };

  const applyCodexConfig = (updated: LlmConfig | null) => {
    setCodexError(null);
    if (updated) acceptLlmConfig(updated);
    else loadLlm();
    refresh();
  };

  const enableCodex = (options: CodexEnableOptions) => {
    const seq = ++codexMutationSeq.current;
    setCodexEnablePending(true);
    setCodexError(null);
    codexSaveQueue.current = codexSaveQueue.current
      .then(() => llmSaveQueue.current)
      .then(() => {
        if (llmRef.current?.provider !== "codex") return null;
        return invoke<LlmConfig | null>("enable_codex_provider", {
          shareContextTerms: options.share_context_terms,
          confirmed: true,
        });
      })
      .then((updated) => {
        if (seq === codexMutationSeq.current && llmRef.current?.provider === "codex") {
          applyCodexConfig(updated);
          onToast("已啟用 Codex 專業校字");
        }
      })
      .catch((e) => {
        if (String(e).includes("codex_request_stale")) {
          loadLlm();
          return;
        }
        if (seq === codexMutationSeq.current) {
          setCodexError(codexMutationErrorCopy(e));
        }
      })
      .finally(() => {
        if (seq === codexMutationSeq.current) setCodexEnablePending(false);
      });
  };

  function saveCodexCorrectionPreferences(
    correctionPreferences: string,
  ): Promise<void> {
    const normalized = correctionPreferences.trim();
    const pending = codexCorrectionPending.current;
    if (pending?.value === normalized) return pending.promise;

    const seq = ++codexCorrectionSaveSeq.current;
    const previousLlmSaves = llmSaveQueue.current;
    setCodexCorrectionSaving(true);

    const operation = codexSaveQueue.current
      .then(() => previousLlmSaves)
      .then(() =>
        invoke<LlmConfig | null>("set_codex_correction_preferences", {
          correctionPreferences: normalized,
        }),
      );
    // 內部 queue 必須吞掉失敗，否則一次離線會讓後續重試永久卡在 rejected promise。
    codexSaveQueue.current = operation.catch(() => undefined);

    const promise = operation
      .then((updated) => {
        if (seq !== codexCorrectionSaveSeq.current) return;
        setCodexCorrectionDraft((current) =>
          current !== null && current.trim() === normalized ? null : current,
        );
        if (llmRef.current?.provider === "codex") {
          applyCodexConfig(updated);
        }
      })
      .catch((reason) => {
        throw new Error(codexMutationErrorCopy(reason));
      })
      .finally(() => {
        if (seq === codexCorrectionSaveSeq.current) {
          setCodexCorrectionSaving(false);
        }
      });

    codexCorrectionPending.current = { value: normalized, promise };
    void promise.then(
      () => {
        if (codexCorrectionPending.current?.promise === promise) {
          codexCorrectionPending.current = null;
        }
      },
      () => {
        if (codexCorrectionPending.current?.promise === promise) {
          codexCorrectionPending.current = null;
        }
      },
    );
    return promise;
  }

  const setCodexContextTerms = (enabled: boolean, confirmed: boolean) => {
    const seq = ++codexMutationSeq.current;
    setCodexPreferencesSaving(true);
    setCodexError(null);
    codexSaveQueue.current = codexSaveQueue.current
      .then(() => llmSaveQueue.current)
      .then(() => {
        if (llmRef.current?.provider !== "codex") return null;
        return invoke<LlmConfig | null>("set_codex_context_terms_enabled", {
          enabled,
          confirmed,
        });
      })
      .then((updated) => {
        if (seq === codexMutationSeq.current && llmRef.current?.provider === "codex") {
          applyCodexConfig(updated);
          onToast(enabled ? "已允許有限畫面詞彙" : "已關閉有限畫面詞彙");
        }
      })
      .catch((e) => {
        if (seq === codexMutationSeq.current) {
          setCodexError(codexMutationErrorCopy(e));
        }
      })
      .finally(() => {
        if (seq === codexMutationSeq.current) setCodexPreferencesSaving(false);
      });
  };

  const runCodexTest = () => {
    const generation = allocateCodexTestRequestId();
    activeCodexTestRequestId.current = generation;
    cancelledCodexTests.current.delete(generation);
    dispatchCodex({ type: "test_started", generation });
    codexSaveQueue.current
      .then(() => llmSaveQueue.current)
      .then(() => {
        if (
          consumeCancelledCodexTest(
            cancelledCodexTests.current,
            generation,
          )
        ) {
          if (activeCodexTestRequestId.current === generation) {
            activeCodexTestRequestId.current = null;
          }
          dispatchCodex({ type: "test_cancelled", generation });
          return null;
        }
        return invoke<CodexTestResult>("test_codex_polish", {
          requestId: generation,
        });
      })
      .then((result) => {
        if (!result) return;
        if (
          consumeCancelledCodexTest(
            cancelledCodexTests.current,
            generation,
          )
        ) {
          if (activeCodexTestRequestId.current === generation) {
            activeCodexTestRequestId.current = null;
          }
          dispatchCodex({ type: "test_cancelled", generation });
          return;
        }
        cancelledCodexTests.current.delete(generation);
        if (activeCodexTestRequestId.current === generation) {
          activeCodexTestRequestId.current = null;
        }
        if (result.phase === "success") {
          dispatchCodex({
            type: "test_succeeded",
            generation,
            input: result.input,
            output: result.output,
          });
        } else if (result.phase === "cancelled") {
          dispatchCodex({ type: "test_cancelled", generation });
        } else {
          dispatchCodex({
            type: "test_failed",
            generation,
            reason: result.reason,
          });
        }
      })
      .catch(() => {
        if (
          consumeCancelledCodexTest(
            cancelledCodexTests.current,
            generation,
          )
        ) {
          if (activeCodexTestRequestId.current === generation) {
            activeCodexTestRequestId.current = null;
          }
          dispatchCodex({ type: "test_cancelled", generation });
          return;
        }
        cancelledCodexTests.current.delete(generation);
        if (activeCodexTestRequestId.current === generation) {
          activeCodexTestRequestId.current = null;
        }
        dispatchCodex({
          type: "test_failed",
          generation,
          reason: "unknown",
        });
      });
  };

  const cancelCodexTest = () => {
    const generation = activeCodexTestRequestId.current;
    if (generation === null) return;
    cancelledCodexTests.current.add(generation);
    dispatchCodex({ type: "test_cancel_requested", generation });
    invoke<boolean>("cancel_codex_test", { requestId: generation })
      .catch(() => {
        cancelledCodexTests.current.delete(generation);
        // 取消命令未送達時，原測試仍可能正常完成；不要把 active id 清掉，
        // 也不要丟棄稍後回來的真實結果。
        dispatchCodex({ type: "test_cancel_failed", generation });
        onToast("取消未送達；Codex 測試仍在進行");
      });
  };

  const saveKey = () => {
    invoke("set_llm_key", { key: keyDraft })
      .then(() => {
        setKeyDraft("");
        loadLlm();
        onToast("API Key 已存入鑰匙圈");
      })
      .catch((e) => setError(String(e)));
  };

  const deleteKey = () => {
    invoke("set_llm_key", { key: "" })
      .then(() => {
        setKeyDraft("");
        loadLlm();
        onToast("API Key 已從鑰匙圈刪除");
      })
      .catch((e) => setError(String(e)));
  };

  const runTest = () => {
    setTesting(true);
    setTestResult(null);
    setError(null);
    // 排在存檔佇列後面：剛改完 provider 立刻按測試，要測到「新」設定
    llmSaveQueue.current
      .then(() => invoke<string>("test_polish"))
      .then((r) => setTestResult(r))
      .catch((e) => setError(String(e)))
      .finally(() => setTesting(false));
  };

  const retryHotkey = () => {
    invoke<boolean>("retry_hotkey")
      .then((ok) => {
        refresh();
        onToast(ok ? "快捷鍵已啟用" : "還沒偵測到授權——到系統設定勾選 Claro 後再試");
      })
      .catch((e) => setError(String(e)));
  };

  const permsOk = status.accessibility && status.hotkey_active;
  const setupReady = permsOk && status.model_present && status.setup_completed;
  const orderedModels = models
    .map((model, index) => ({ model, index }))
    .sort((a, b) => sttModelSortRank(a.model) - sttModelSortRank(b.model) || a.index - b.index)
    .map(({ model }) => model);
  const anyDownloadActive =
    orderedModels.some((model) => sttDownloadState(model).active) ||
    builtinLlms.some((model) => llmDownloadState(model).active);
  const codexPreferences: CodexPreferences = {
    correction_preferences: llm?.codex_correction_preferences ?? "",
    share_context_terms: llm?.codex_share_context_terms ?? false,
    consent_valid: llm?.codex_consent_valid ?? false,
    context_consent_valid: llm?.codex_context_consent_valid ?? false,
    correct_consent_valid: llm?.correct_consent_valid ?? false,
    correct_mode_active: llm?.polish_mode === "correct",
  };
  const codexMutationPending =
    codexEnablePending ||
    codexPreferencesSaving ||
    codexCorrectionSaving ||
    codexPolicySaving ||
    codexUi.test.phase === "running" ||
    codexUi.test.phase === "cancelling";
  const modeControlsDisabled =
    !llm || (llm.provider === "codex" && codexMutationPending);

  return (
    <div className="page-in">
      <h1 className="text-[24px] font-bold tracking-tight mb-6">設定</h1>

      <div className={`setup-compact ${setupReady ? "ready" : "pending"}`}>
        <div>
          <b>{setupReady ? "聽寫必要條件已就緒" : "首次設定尚未完成"}</b>
          <span>{setupReady ? "可隨時重新測試麥克風、權限與語音模型。" : "用引導頁逐項完成權限、麥克風與語音模型。"}</span>
        </div>
        <button className="btn no-drag" onClick={onOpenSetup}>{setupReady ? "重新檢查" : "繼續設定"}</button>
      </div>

      {error && <div className="config-error mb-5" role="alert">{error}</div>}

      <Section title="聽寫">
        <Row label="快捷鍵" sub="按住說話、放開出字；快點一下＝免持模式。改完立即生效。">
          <div className="flex items-center gap-3">
            <Hotkey combo={status.hotkey} />
            <select
              className="select no-drag"
              name="hotkey"
              value={status.hotkey}
              onChange={(e) =>
                invoke("set_hotkey", { combo: e.target.value })
                  .then(() => {
                    refresh();
                    onToast("快捷鍵已更新");
                  })
                  .catch((err) => setError(String(err)))
              }
            >
              <option value="Opt+Shift+C">⌥⇧C（預設）</option>
              <option value="CmdRight">右 ⌘（單鍵按住）</option>
              <option value="OptRight">右 ⌥（單鍵按住）</option>
              <option value="Fn">fn（單鍵按住）</option>
              <option value="F5">F5</option>
            </select>
          </div>
        </Row>
      </Section>

      <Section title="聲音">
        <Row label="輸入裝置" sub={!status.input_device ? "跟隨系統預設" : "已固定，不隨系統切換"}>
          <select
            className="select no-drag"
            name="input-device"
            value={status.input_device ?? ""}
            onChange={(e) => {
              if (mic.active) invoke("mic_test_stop").catch(() => {});
              call("set_input_device", { name: e.target.value });
            }}
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
        </Row>
        <Row
          label="麥克風測試"
          alignTop
          sub={
            mic.active
              ? mic.level > 0.01
                ? "收音正常——這就是聽寫時的音量。"
                : "太安靜——對著麥克風說幾句話，或換一個輸入裝置。"
              : mic.timed_out
                ? "30 秒內沒有偵測到聲音；請檢查權限或改用其他輸入裝置。"
                : "確認 Claro 聽得到你。"
          }
        >
          <div className="flex items-center gap-3 w-[260px] pt-1">
            <button
              className={`btn no-drag ${mic.active ? "recording" : ""}`}
              onClick={() => call(mic.active ? "mic_test_stop" : "mic_test_start")}
            >
              {mic.active ? "停止" : "測試"}
            </button>
            <LevelBar level={mic.level} active={mic.active} />
          </div>
        </Row>
      </Section>

      <Section title="語音模型">
        {orderedModels.map((m) => {
          const available = m.available === true;
          const downloadState = sttDownloadState(m);
          const activationStatus = progress?.model_id === m.id ? progress.activation_status : "none";
          const activationPending = activationStatus !== "none";
          return (
            <div
              className="row"
              key={m.id}
              style={{ alignItems: "flex-start" }}
              aria-busy={downloadState.active}
            >
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="row-label">{displaySttModelLabel(m)}</span>
                  {isPreviewSttModel(m) && <span className="pill amber">驗證中</span>}
                  {m.recommended && <span className="pill blue">推薦</span>}
                  {available && m.active && m.downloaded && (
                    <span className="pill green">
                      <span className="dot" />
                      使用中
                    </span>
                  )}
                  {available && !m.active && m.downloaded && (
                    <span className="pill green">已下載</span>
                  )}
                  {available && m.active && !m.downloaded && <span className="pill amber">已選擇・待下載</span>}
                </div>
                <div className="row-sub">
                  {m.desc}・{m.size_mb >= 1024 ? `${(m.size_mb / 1024).toFixed(1)} GB` : `${m.size_mb} MB`}
                </div>
                {!available && (
                  <div className="model-validation-note">
                    完成長音訊分段與台灣真人語料驗收前，不提供下載或啟用。
                  </div>
                )}
                <DownloadStatus
                  state={downloadState}
                  label={displaySttModelLabel(m)}
                  className="w-[240px]"
                  onCancel={() => cancelDownload("stt", m.id)}
                />
                {activationPending && (
                  <div className="model-validation-note" role="status">
                    {activationStatus === "waiting_for_idle"
                      ? "已下載完成；本次聽寫結束後，請按「使用」切換。"
                      : `已下載完成，但切換失敗：${progress?.error ?? "未知錯誤"}。請按「使用」重試。`}
                  </div>
                )}
              </div>
              <div className="flex items-center gap-2 pt-0.5">
                {!available && (
                  <>
                    <button className="btn no-drag" disabled>尚未開放</button>
                    {m.downloaded &&
                      (pendingDelete === `stt:${m.id}` ? (
                        <>
                          <button className="btn no-drag" onClick={() => setPendingDelete(null)}>取消</button>
                          <button
                            className="btn danger-quiet no-drag"
                            aria-label={`確認刪除 ${displaySttModelLabel(m)}`}
                            onClick={() =>
                              call("delete_model", { id: m.id }, () => {
                                setPendingDelete(null);
                                onToast("已刪除");
                              })
                            }
                          >
                            確認刪除
                          </button>
                        </>
                      ) : (
                        <button
                          className="btn danger-quiet no-drag"
                          onClick={() => setPendingDelete(`stt:${m.id}`)}
                        >
                          刪除
                        </button>
                      ))}
                  </>
                )}
                {available && m.downloaded && !m.active && (
                  <>
                    <button className="btn no-drag" onClick={() => call("set_model", { id: m.id })}>
                      使用
                    </button>
                    {pendingDelete === `stt:${m.id}` ? (
                      <>
                        <button className="btn no-drag" onClick={() => setPendingDelete(null)}>取消</button>
                        <button
                          className="btn danger-quiet no-drag"
                          aria-label={`確認刪除 ${m.label}`}
                          onClick={() =>
                            call("delete_model", { id: m.id }, () => {
                              setPendingDelete(null);
                              onToast("已刪除");
                            })
                          }
                        >
                          確認刪除
                        </button>
                      </>
                    ) : (
                      <button
                        className="btn danger-quiet no-drag"
                        onClick={() => setPendingDelete(`stt:${m.id}`)}
                      >
                        刪除
                      </button>
                    )}
                  </>
                )}
                {available && !m.downloaded && downloadState.canStart && (
                  <button
                    className="btn no-drag"
                    disabled={anyDownloadActive}
                    title={anyDownloadActive ? "請先完成目前的模型下載" : undefined}
                    onClick={() =>
                      void requestDownload(
                        "stt",
                        m.id,
                        "download_model",
                        { id: m.id },
                      )
                    }
                  >
                    {downloadState.phase === "failed" || downloadState.phase === "cancelled"
                      ? "重試並續傳"
                      : "下載"}
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </Section>

      <Section title="AI 潤飾">
        {polishError && (
          <div
            className="config-error"
            role="alert"
            ref={polishErrorRef}
            tabIndex={-1}
          >
            <span>{polishError}</span>
            <button
              className="btn no-drag"
              onClick={() => {
                setPolishError(null);
                loadLlm();
              }}
            >
              重新載入目前設定
            </button>
          </div>
        )}
        <fieldset
          className="mode-picker"
          disabled={modeControlsDisabled}
        >
          <legend>文字整理模式</legend>
          <p className="mode-picker-help">模式決定 Claro 可以怎麼改文字；處理引擎與資料位置在下方另外選擇。</p>
          <div className="mode-grid">
            {POLISH_MODES.map((mode) => {
              const unavailable =
                modeControlsDisabled ||
                !modeSupportedByProvider(mode.id, llm?.provider ?? "off");
              const descriptionId = `polish-mode-${mode.id}-description`;
              return (
                <label
                  className={`mode-card ${llm?.polish_mode === mode.id ? "selected" : ""} ${unavailable ? "disabled" : ""}`}
                  aria-disabled={unavailable}
                  key={mode.id}
                >
                  <input
                    type="radio"
                    name="polish-mode"
                    value={mode.id}
                    checked={llm?.polish_mode === mode.id}
                    disabled={unavailable}
                    aria-describedby={`${descriptionId} polish-mode-provider-help`}
                    onChange={() => requestMode(mode.id)}
                  />
                  <span className="mode-card-title">
                    {mode.title}
                    {mode.tag && (
                      <span
                        className={`pill ${
                          mode.id === "organize" || mode.id === "correct" ? "amber" : "blue"
                        }`}
                      >
                        {mode.tag}
                      </span>
                    )}
                  </span>
                  <span
                    className="mode-card-description"
                    id={descriptionId}
                  >
                    {mode.description}
                  </span>
                </label>
              );
            })}
          </div>
          {llm?.provider === "codex" ? (
            <p className="mode-picker-help" id="polish-mode-provider-help">
              Codex 目前只支援「專業校字」；要使用保守校訂或條理整理，請先在下方選擇其他引擎。
            </p>
          ) : (
            <p className="mode-picker-help" id="polish-mode-provider-help">
              「專業校字」目前只支援 Codex；選擇 Codex
              並完成雲端同意後會自動切換。
            </p>
          )}
        </fieldset>

        {pendingMode === "correct" && (
          <section className="consent-panel" aria-labelledby="correct-confirm-title">
            <div>
              <div
                ref={modeConsentHeadingRef}
                id="correct-confirm-title"
                className="consent-title"
                tabIndex={-1}
              >
                啟用「專業校字」？
              </div>
              <p>這個實驗模式會嘗試統一已授權專業詞的拼法格式，與不改字的「保守校訂」不同；結構 guard 只能降低風險，不能證明語意一定正確。</p>
              <ul>
                <li><b>可能改變：</b>只移除符合極窄規則的單一尾端連字號；仍可能誤判，重要文字請檢查。</li>
                <li><b>不會自動處理：</b>空白合併、字母不同、同音誤認或其他分隔；已知 source→target 請用個人字典。</li>
                <li><b>必須保留：</b>數字、日期、否定、條件、URL、版本、句序與語氣。</li>
                <li><b>安全退回：</b>沒有足夠證據、處理逾時或內容保護未通過時，使用本機結果。</li>
              </ul>
            </div>
            <div className="consent-actions">
              <button className="btn no-drag" onClick={cancelPendingMode}>取消</button>
              <button className="btn-primary no-drag" onClick={() => applyMode("correct", true)}>
                我了解，啟用專業校字
              </button>
            </div>
          </section>
        )}

        {pendingMode === "organize" && (
          <section className="consent-panel" aria-labelledby="organize-confirm-title">
            <div>
              <div
                ref={modeConsentHeadingRef}
                id="organize-confirm-title"
                className="consent-title"
                tabIndex={-1}
              >
                啟用「條理整理」？
              </div>
              <p>這個模式不只校訂，會改變句子順序與段落結構。重要內容仍應由你檢查。</p>
              <ul>
                <li><b>可能改變：</b>完整句子的順序、段落、完全重複片段與明確清單格式。</li>
                <li><b>必須保留：</b>姓名、術語、數字、時間、否定、條件、因果與待辦。</li>
                <li><b>絕不允許：</b>新增原文沒有的資訊、猜測或替你回答。</li>
              </ul>
            </div>
            <div className="consent-actions">
              <button className="btn no-drag" onClick={cancelPendingMode}>取消</button>
              <button className="btn-primary no-drag" onClick={() => applyMode("organize", true)}>
                我了解，啟用條理整理
              </button>
            </div>
          </section>
        )}

        <Row
          label="文字整理位置"
          sub={
            llm?.polish_mode === "raw"
              ? "原樣轉錄不會執行文字整理；這個選擇會保留給其他模式使用。"
              : "引擎不會放寬模式的行為界線；若所選引擎不支援目前模式，Claro 會改用相容模式或安全退回原樣轉錄。"
          }
        >
          <select
            ref={polishProviderSelectRef}
            className="select no-drag"
            aria-label="文字整理位置"
            name="polish-provider"
            value={llm?.provider ?? "off"}
            disabled={!llm || codexMutationPending}
            onChange={(e) => saveLlm({ provider: e.target.value })}
          >
            <option value="off" disabled>尚未選擇整理方式</option>
            <optgroup label="Claro 本機方案">
              <option value="apple" disabled={!!llm && [1, 4].includes(llm.apple_status)}>
                Apple Intelligence（免安裝{llm?.apple_status === 0 ? "・推薦" : ""}）
              </option>
              <option value="builtin">Claro 內建模型（不需其他 App）</option>
            </optgroup>
            <optgroup label="進階：只在你已經使用時選擇">
              <option value="codex">Codex 專業拼法（使用既有登入・雲端）</option>
              <option value="ollama">連接既有 Ollama</option>
              <option value="lmstudio">連接既有 LM Studio</option>
              <option value="custom">自訂 API（OpenAI 相容）</option>
            </optgroup>
          </select>
        </Row>

        {llm?.provider === "codex" && (
          <>
            {codexError && (
              <div
                className="config-error"
                role="alert"
                ref={codexErrorRef}
                tabIndex={-1}
              >
                <span>{codexError}</span>
                <button className="btn no-drag" onClick={probeCodex}>
                  重新檢查 Codex
                </button>
              </div>
            )}
            <CodexProviderPanel
              status={codexUi.status}
              preferences={codexPreferences}
              testState={codexUi.test}
              globalContextEnabled={status.context_enabled}
              enablePending={codexEnablePending}
              preferencesSaving={codexPreferencesSaving || codexPolicySaving}
              correctionDraftValue={
                codexCorrectionDraft ??
                codexPreferences.correction_preferences
              }
              onRefresh={probeCodex}
              onEnable={enableCodex}
              onSaveCorrectionPreferences={saveCodexCorrectionPreferences}
              onCorrectionDraftChange={setCodexCorrectionDraft}
              onShareContextTermsChange={(enabled) =>
                setCodexContextTerms(enabled, enabled)
              }
              onTest={runCodexTest}
              onCancelTest={cancelCodexTest}
            />
          </>
        )}

        <Row
          label="僅限本機"
          sub={
            llm?.provider === "codex" && llm.local_only
              ? "已硬性阻擋 Codex。要重新啟用，請先在上方檢查登入並確認資料傳送。"
              : "開啟後硬性阻擋所有雲端整理；模式與雲端設定會保留，但不會送出資料。"
          }
        >
          <label className="local-only-control">
            <input
              type="checkbox"
              name="local-only"
              aria-label="僅限本機"
              checked={llm?.local_only ?? false}
              disabled={
                !llm ||
                (llm.provider === "codex" &&
                  (llm.local_only || codexMutationPending))
              }
              onChange={(e) => {
                if (!e.target.checked) setConfirmDisableLocalOnly(true);
                else applyLocalOnly(true, true);
              }}
            />
            <span>{llm?.local_only ? "已開啟" : "已關閉"}</span>
          </label>
        </Row>

        {confirmDisableLocalOnly && llm?.provider === "custom" && (
          <section className="consent-panel" aria-labelledby="network-confirm-title">
            <div>
              <div id="network-confirm-title" className="consent-title">允許使用雲端端點？</div>
              <p>
                關閉「僅限本機」後，使用外部 API 時，轉錄文字
                {llm?.polish_mode === "organize" && status.context_enabled ? "與目前視窗上下文" : ""}可能送到
                {llm?.execution_location === "cloud" && (llm.endpoint_origin || llm.base_url) ? (
                  <b> {llm.endpoint_origin || llm.base_url}</b>
                ) : (
                  "你之後明確設定的雲端端點"
                )}
                ；音訊不會傳送。雲端同意只對當前端點有效，更換端點後會再次詢問。
              </p>
            </div>
            <div className="consent-actions">
              <button className="btn no-drag" onClick={() => setConfirmDisableLocalOnly(false)}>保持僅限本機</button>
              <button className="btn-primary no-drag" onClick={() => applyLocalOnly(false, true)}>我了解，允許雲端端點</button>
            </div>
          </section>
        )}

        {llm && llm.polish_mode !== "raw" && llm.effective_mode === "raw" && llm.blocked_reason && (
          <div className="config-warning" role="status">
            <span><b>目前安全退回原樣轉錄：</b>{OUTCOME_LABEL[llm.blocked_reason] ?? "整理設定尚未就緒"}，不會送出雲端資料。</span>
            {llm.provider === "custom" && llm.blocked_reason === "cloud_consent_required" && (
              <button className="btn no-drag" onClick={() => setConfirmCloud(true)}>檢視並允許</button>
            )}
          </div>
        )}

        {confirmCloud && llm?.provider === "custom" && llm.execution_location === "cloud" && (
          <section className="consent-panel" aria-labelledby="cloud-endpoint-confirm-title">
            <div>
              <div id="cloud-endpoint-confirm-title" className="consent-title">允許雲端整理？</div>
              <p>
                目前的整理模式會把轉錄文字
                {llm.polish_mode === "organize" && status.context_enabled ? "與目前視窗上下文" : ""}送到
                <b> {llm.endpoint_origin ?? llm.base_url}</b>；音訊不會傳送。允許後仍可隨時重新開啟「僅限本機」。
              </p>
            </div>
            <div className="consent-actions">
              <button className="btn no-drag" onClick={() => setConfirmCloud(false)}>取消</button>
              <button className="btn-primary no-drag" onClick={() => applyLocalOnly(false, true)}>我了解，允許此端點</button>
            </div>
          </section>
        )}

        {llm?.provider === "builtin" && (
          <>
            {builtinLlms.map((m) => {
              const downloadState = llmDownloadState(m);
              return (
                <div
                  className="row"
                  key={m.id}
                  style={{ alignItems: "flex-start" }}
                  aria-busy={downloadState.active}
                >
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="row-label">{m.label}</span>
                      {m.recommended && <span className="pill blue">推薦</span>}
                      {m.active && m.downloaded && (
                        <span className="pill green">
                          <span className="dot" />
                          使用中
                        </span>
                      )}
                      {!m.active && m.downloaded && <span className="pill green">已下載</span>}
                    </div>
                    <div className="row-sub">
                      {m.desc}・{m.size_mb >= 1024 ? `${(m.size_mb / 1024).toFixed(1)} GB` : `${m.size_mb} MB`}
                      ・閒置 5 分鐘自動釋放記憶體
                    </div>
                    <DownloadStatus
                      state={downloadState}
                      label={m.label}
                      className="w-[240px]"
                      onCancel={() => cancelDownload("llm", m.id)}
                    />
                  </div>
                  <div className="flex items-center gap-2 pt-0.5">
                    {m.downloaded && !m.active && (
                      <>
                        <button className="btn no-drag" onClick={() => saveLlm({ model: m.id })}>
                          使用
                        </button>
                        {pendingDelete === `llm:${m.id}` ? (
                          <>
                            <button className="btn no-drag" onClick={() => setPendingDelete(null)}>取消</button>
                            <button
                              className="btn danger-quiet no-drag"
                              aria-label={`確認刪除 ${m.label}`}
                              onClick={() =>
                                invoke("delete_builtin_llm", { id: m.id })
                                  .then(() => {
                                    setPendingDelete(null);
                                    loadBuiltinLlms();
                                    onToast("已刪除");
                                  })
                                  .catch((e) => setError(String(e)))
                              }
                            >
                              確認刪除
                            </button>
                          </>
                        ) : (
                          <button
                            className="btn danger-quiet no-drag"
                            onClick={() => setPendingDelete(`llm:${m.id}`)}
                          >
                            刪除
                          </button>
                        )}
                      </>
                    )}
                    {!m.downloaded && downloadState.canStart && (
                      <button
                        className="btn no-drag"
                        disabled={anyDownloadActive}
                        title={anyDownloadActive ? "請先完成目前的模型下載" : undefined}
                        onClick={() =>
                          void requestDownload(
                            "llm",
                            m.id,
                            "download_builtin_llm",
                            { id: m.id },
                          )
                        }
                      >
                        {downloadState.phase === "failed" ||
                        downloadState.phase === "cancelled"
                          ? "重試並續傳"
                          : "下載"}
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </>
        )}

        {llm?.provider === "apple" && (
          <Row
            label="系統模型狀態"
            sub={
              llm.apple_status === 0
                ? "使用 macOS 內建的端上模型：不用下載、不占 Claro 記憶體、文字不出機器。"
                : llm.apple_status === 2
                  ? "到 系統設定 → Apple Intelligence 與 Siri 開啟後即可使用。"
                  : llm.apple_status === 3
                    ? "系統正在下載 Apple Intelligence 模型，稍後就緒。"
                    : llm.apple_status === 1
                      ? "這台 Mac 不支援 Apple Intelligence（需要 Apple Silicon）。"
                      : "需要 macOS 26 以上。"
            }
          >
            <span className={`pill ${llm.apple_status === 0 ? "green" : "amber"}`}>
              <span className="dot" />
              {llm.apple_status === 0
                ? "可用"
                : llm.apple_status === 2
                  ? "未開啟"
                  : llm.apple_status === 3
                    ? "準備中"
                    : "不可用"}
            </span>
          </Row>
        )}

        {llm && (llm.provider === "ollama" || llm.provider === "lmstudio") && (() => {
          // 只採用「屬於目前 provider」的偵測結果——晚到的舊回應直接無視
          const detected = localModels?.provider === llm.provider ? localModels.models : null;
          const detectErr = localModelsErr?.provider === llm.provider ? localModelsErr.msg : null;
          return (
            <Row
              label="模型"
              sub={
                detectErr
                  ? llm.provider === "ollama"
                    ? "未偵測到既有 Ollama 服務。Claro 不需要 Ollama；若你本來就在使用，啟動後再重新偵測。"
                    : "未偵測到既有 LM Studio 服務。Claro 不需要 LM Studio；若你本來就在使用，啟動本機伺服器後再重新偵測。"
                  : detected && detected.length === 0
                    ? llm.provider === "ollama"
                      ? "服務已連線但沒有可用模型；請先在你既有的 Ollama 環境準備模型。"
                      : "服務在跑但沒有載入模型——在 LM Studio 載入一個模型。"
                    : "建議 4B 級以上的指令模型，例如 qwen3:4b。"
              }
            >
              <div className="flex items-center gap-2">
                {detected && detected.length > 0 ? (
                  <select
                    className="select no-drag"
                    style={{ minWidth: 200 }}
                    name="local-model"
                    value={llm.model}
                    onChange={(e) => saveLlm({ model: e.target.value })}
                  >
                    {!detected.includes(llm.model) && (
                      <option value={llm.model}>{llm.model || "— 選擇模型 —"}</option>
                    )}
                    {detected.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                ) : (
                  <input
                    className="select no-drag"
                    style={{ minWidth: 200 }}
                    name="local-model"
                    autoComplete="off"
                    spellCheck={false}
                    translate="no"
                    value={llm.model}
                    placeholder="例如 qwen3:4b…"
                    onChange={(e) => saveLlm({ model: e.target.value })}
                  />
                )}
                <button className="btn no-drag" onClick={() => loadLocalModels(llm.provider)}>
                  重新偵測
                </button>
              </div>
            </Row>
          );
        })()}

        {llm?.provider === "custom" && (
          <>
            <Row label="常用服務" sub="選一個自動帶入端點與建議模型，或維持自訂。">
              <select
                className="select no-drag"
                aria-label="常用雲端服務"
                name="cloud-preset"
                value={CUSTOM_PRESETS.find((p) => p.baseUrl === llm.base_url)?.id ?? ""}
                onChange={(e) => {
                  const p = CUSTOM_PRESETS.find((x) => x.id === e.target.value);
                  if (p) saveLlm({ base_url: p.baseUrl, model: p.model });
                }}
              >
                <option value="">自訂端點…</option>
                {CUSTOM_PRESETS.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </select>
            </Row>
            <Row label="Base URL" sub="OpenAI 相容端點">
              <input
                className="select no-drag"
                aria-label="自訂 API Base URL"
                style={{ minWidth: 260 }}
                type="url"
                inputMode="url"
                name="custom-api-base-url"
                autoComplete="off"
                spellCheck={false}
                translate="no"
                value={llm.base_url}
                placeholder="例如 https://api.openai.com/v1…"
                onChange={(e) => saveLlm({ base_url: e.target.value })}
              />
            </Row>
            <Row label="模型名稱" sub="該服務的模型 ID">
              <input
                className="select no-drag"
                aria-label="自訂 API 模型名稱"
                style={{ minWidth: 200 }}
                name="custom-api-model"
                autoComplete="off"
                spellCheck={false}
                translate="no"
                value={llm.model}
                placeholder="例如 gpt-4o-mini…"
                onChange={(e) => saveLlm({ model: e.target.value })}
              />
            </Row>
            <Row
              label="API Key"
              sub={llm.has_key ? "已存入 macOS 鑰匙圈（不會寫進任何檔案）" : "存入 macOS 鑰匙圈"}
            >
              <div className="flex items-center gap-2">
                <input
                  className="select no-drag"
                  aria-label="API Key"
                  style={{ minWidth: 180 }}
                  type="password"
                  name="custom-api-key"
                  autoComplete="off"
                  spellCheck={false}
                  translate="no"
                  value={keyDraft}
                  placeholder={llm.has_key ? "••••••••" : "sk-…"}
                  onChange={(e) => setKeyDraft(e.target.value)}
                />
                <button className="btn no-drag" onClick={saveKey} disabled={!keyDraft}>
                  儲存
                </button>
                {llm.has_key && (
                  <button className="btn danger-quiet no-drag" onClick={deleteKey}>
                    刪除 Key
                  </button>
                )}
              </div>
            </Row>
          </>
        )}

        {llm && llm.provider !== "off" && llm.provider !== "codex" && (
          <>
            <Row label="測試潤飾" sub="送一句「嗯我們用 hyTorch，那個，跑訓練」看看效果">
              <button className="btn no-drag" onClick={runTest} disabled={testing}>
                {testing ? "測試中…" : "測試"}
              </button>
            </Row>
            {testResult && (
              <div className="row polish-test-result">
                <div className="polish-test-success">
                  ✓ {testResult}
                </div>
              </div>
            )}
          </>
        )}
      </Section>

      <Section title="螢幕上下文">
        <Row
          label="讀取畫面詞彙"
          sub="按下快捷鍵時讀取當下 App、標題與游標周邊文字，綁定到該次聽寫。內容只在記憶體；密碼欄與常見密碼管理器永不讀取。"
        >
          <select
            className="select no-drag"
            name="context-enabled"
            value={status.context_enabled ? "on" : "off"}
            onChange={(e) => {
              const enabled = e.target.value === "on";
              invoke("set_context_enabled", { enabled })
                .then(() => {
                  if (!enabled) setContextAudit(null);
                  refresh();
                  loadLlm();
                })
                .catch((err) => setError(String(err)));
            }}
          >
            <option value="on">開啟（建議）</option>
            <option value="off">關閉</option>
          </select>
        </Row>
        <Row
          label="上一次使用的上下文"
          sub="只保留在本次 Claro 執行期間，方便你確認系統看見了什麼；不會寫入設定或歷史紀錄。"
        >
          <div className="flex items-center gap-2">
            <button className="btn no-drag" onClick={loadContextAudit}>重新讀取</button>
            {contextAudit && (
              <button
                className="btn danger-quiet no-drag"
                onClick={() =>
                  invoke("clear_context_audit")
                    .then(() => setContextAudit(null))
                    .catch((reason) => setError(String(reason)))
                }
              >
                從記憶體清除
              </button>
            )}
          </div>
        </Row>
        {contextAudit ? (
          <div className="row" style={{ alignItems: "flex-start" }}>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 mb-2">
                <span className="pill blue">{contextAudit.app_name}</span>
                <span className="pill">{contextAudit.surface}</span>
              </div>
              <pre className="context-audit-text">{contextAudit.text}</pre>
            </div>
          </div>
        ) : (
          <div className="setup-inline-state">本次執行期間還沒有使用畫面上下文。</div>
        )}
      </Section>

      <Section title="隱私與歷史">
        <Row
          label="保存本機歷史"
          sub="關閉後，新聽寫不會寫入 history.jsonl；既有紀錄會保留，直到你在歷史頁清除。"
        >
          <label className="local-only-control">
            <input
              type="checkbox"
              name="history-enabled"
              checked={status.history_enabled}
              onChange={(event) =>
                invoke("set_history_enabled", { enabled: event.target.checked })
                  .then(refresh)
                  .catch((reason) => setError(String(reason)))
              }
            />
            <span>{status.history_enabled ? "已開啟" : "已關閉"}</span>
          </label>
        </Row>
      </Section>

      <Section title="個人字典">
        <Row
          label="常用詞修正"
          sub="左邊填常被認錯的寫法、右邊填正確寫法。正確詞彙也會直接提示語音模型，讓它下次少認錯。"
        >
          <span />
        </Row>
        {(dict ?? []).map((e, i) => (
          <div className="row" key={i}>
            <div className="flex items-center gap-2 flex-1">
              <input
                className="select no-drag"
                aria-label={`誤認詞 ${i + 1}`}
                style={{ minWidth: 140 }}
                name={`dictionary-from-${i + 1}`}
                autoComplete="off"
                spellCheck={false}
                value={e.from}
                onChange={(ev) => {
                  const next = [...(dict ?? [])];
                  next[i] = { ...next[i], from: ev.target.value };
                  saveDict(next); // 用本地 next 存，避免 stale state；佇列已串行化
                }}
              />
              <span style={{ color: "var(--faint)" }}>→</span>
              <input
                className="select no-drag"
                aria-label={`正確詞 ${i + 1}`}
                style={{ minWidth: 140 }}
                name={`dictionary-to-${i + 1}`}
                autoComplete="off"
                spellCheck={false}
                value={e.to}
                onChange={(ev) => {
                  const next = [...(dict ?? [])];
                  next[i] = { ...next[i], to: ev.target.value };
                  saveDict(next);
                }}
              />
            </div>
            <button
              className="btn danger-quiet no-drag"
              onClick={() => saveDict((dict ?? []).filter((_, j) => j !== i))}
            >
              刪除
            </button>
          </div>
        ))}
        <div className="row">
          <div className="flex items-center gap-2 flex-1">
            <input
              className="select no-drag"
              aria-label="新增誤認詞"
              style={{ minWidth: 140 }}
              name="dictionary-new-from"
              autoComplete="off"
              spellCheck={false}
              placeholder="常被聽錯的詞（如 克拉洛）…"
              value={dictDraft.from}
              onChange={(e) => setDictDraft({ ...dictDraft, from: e.target.value })}
            />
            <span style={{ color: "var(--faint)" }}>→</span>
            <input
              className="select no-drag"
              aria-label="新增正確詞"
              style={{ minWidth: 140 }}
              name="dictionary-new-to"
              autoComplete="off"
              spellCheck={false}
              placeholder="你的正確寫法（如 Claro）…"
              value={dictDraft.to}
              onChange={(e) => setDictDraft({ ...dictDraft, to: e.target.value })}
            />
          </div>
          <button
            className="btn no-drag"
            disabled={!dictDraft.from.trim() || !dictDraft.to.trim()}
            onClick={() => {
              saveDict([...(dict ?? []), dictDraft]);
              setDictDraft({ from: "", to: "" });
              onToast("已加入字典");
            }}
          >
            加入
          </button>
        </div>
      </Section>

      <Section title="權限">
        <Row
          label="輔助使用"
          sub={
            permsOk
              ? "已授權——全域快捷鍵與貼上功能正常。"
              : "系統設定 → 隱私與安全性 → 輔助使用，勾選 Claro 後按「重試」。"
          }
        >
          <div className="flex items-center gap-2">
            {!permsOk && (
              <>
                <button
                  className="btn no-drag"
                  onClick={() => invoke("open_accessibility_settings").catch(() => {})}
                >
                  打開系統設定
                </button>
                <button className="btn no-drag" onClick={retryHotkey}>
                  重試
                </button>
              </>
            )}
            <span className={`pill ${permsOk ? "green" : "amber"}`}>
              <span className="dot" />
              {permsOk ? "已授權" : "需要授權"}
            </span>
          </div>
        </Row>
      </Section>

      <Section title="關於">
        <Row
          label="Claro"
          sub="語音辨識與音訊固定在本機。自訂端點只收到你同意的文字範圍；Codex 主同意包含轉錄與個人正確詞彙，畫面詞彙另行同意，完整畫面內容永不送給 Codex。"
        >
          <span className="text-[12.5px]" style={{ color: "var(--faint)" }}>
            v0.1.0
          </span>
        </Row>
      </Section>

    </div>
  );
}
