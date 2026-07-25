import { useEffect, useId, useRef, useState } from "react";
import {
  codexConsentIsReady,
  codexPrimaryConsentIsValid,
  codexTestIsPending,
} from "../codexProviderState";
import type {
  CodexCliStatus,
  CodexEnableOptions,
  CodexPreferences,
  CodexTestFailureReason,
  CodexTestState,
} from "../types";

const MAX_CORRECTION_PREFERENCES = 1000;
const CORRECTION_AUTOSAVE_DELAY_MS = 700;

type CorrectionSavePhase = "saved" | "dirty" | "saving" | "error";

const TEST_FAILURE_COPY: Record<CodexTestFailureReason, string> = {
  timeout:
    "Codex 沒有在本次測試時間內完成。設定已保留；正式聽寫遇到相同情況會使用本機結果。",
  rate_limited:
    "目前無法使用 Codex 額度。請稍後再試；Claro 仍可使用本機語音辨識。",
  auth_required: "Codex 登入已失效。請重新登入後，再按「重新檢查」。",
  unavailable:
    "Codex 目前無法使用。設定已保留；Claro 仍可使用本機語音辨識。",
  consent_changed:
    "Codex 的登入、版本或受控能力已改變。這次結果未採用；請重新檢查並確認。",
  output_rejected:
    "Codex 有回應，但結果未通過內容保護，因此沒有使用這次校字結果。",
  unknown:
    "無法完成受控校字測試。設定已保留；重新檢查 Codex 後可以再試。",
};

interface StatusCopy {
  title: string;
  detail: string;
  tone: "neutral" | "success" | "warning" | "error";
}

function codexStatusCopy(status: CodexCliStatus): StatusCopy {
  const version = status.version ? ` ${status.version}` : "";
  switch (status.availability) {
    case "checking":
      return {
        title: "正在檢查 Codex…",
        detail: "這只檢查安裝與登入狀態，不會送出文字或使用 Codex 額度。",
        tone: "neutral",
      };
    case "not_installed":
      return {
        title: "沒有找到 Codex CLI",
        detail: "Claro 不會自動安裝任何工具。安裝 Codex 後可回來重新檢查。",
        tone: "warning",
      };
    case "auth_required":
      return {
        title: `Codex${version} 尚未登入`,
        detail: "請先完成 Codex CLI 登入，再回到 Claro 重新檢查。",
        tone: "warning",
      };
    case "unsupported":
      return {
        title: `Codex${version} 版本不支援`,
        detail: "這個版本缺少 Claro 所需的受控文字輸出能力。更新 Codex 後再試。",
        tone: "warning",
      };
    case "unavailable":
      return {
        title: "暫時無法確認 Codex 狀態",
        detail:
          "Claro 不會在狀態不明時送出文字。你仍可使用本機語音辨識或其他整理方式。",
        tone: "error",
      };
    case "ready":
      if (status.auth_mode === "chatgpt") {
        return {
          title: `Codex${version} 已可使用`,
          detail:
            "使用這台 Mac 現有的 ChatGPT／Codex 登入與方案額度；Claro 不需要、讀取或保存 API Key。",
          tone: "success",
        };
      }
      if (status.auth_mode === "api_key") {
        return {
          title: `Codex${version} 已可使用`,
          detail:
            "Codex CLI 目前使用可能依 API 用量計費的登入方式。Claro 不會讀取或保存金鑰；實際費用依你的 Codex 設定。",
          tone: "warning",
        };
      }
      return {
        title: `Codex${version} 已登入`,
        detail:
          "Claro 無法確認目前使用方案額度或 API 計費。啟用前請先確認你的 Codex 帳戶設定；Claro 不會讀取或保存金鑰。",
        tone: "warning",
      };
  }
}

function statusClass(tone: StatusCopy["tone"]): string {
  if (tone === "error") return "config-error";
  if (tone === "warning") return "config-warning";
  return "setup-inline-state";
}

export interface CodexProviderPanelProps {
  status: CodexCliStatus;
  preferences: CodexPreferences;
  testState: CodexTestState;
  globalContextEnabled: boolean;
  enablePending?: boolean;
  preferencesSaving?: boolean;
  correctionDraftValue?: string;
  onRefresh: () => void;
  onEnable: (options: CodexEnableOptions) => void;
  onSaveCorrectionPreferences: (value: string) => Promise<void> | void;
  onCorrectionDraftChange?: (value: string) => void;
  onShareContextTermsChange: (enabled: boolean) => void;
  onTest: () => void;
  onCancelTest: () => void;
}

export default function CodexProviderPanel({
  status,
  preferences,
  testState,
  globalContextEnabled,
  enablePending = false,
  preferencesSaving = false,
  correctionDraftValue,
  onRefresh,
  onEnable,
  onSaveCorrectionPreferences,
  onCorrectionDraftChange,
  onShareContextTermsChange,
  onTest,
  onCancelTest,
}: CodexProviderPanelProps) {
  const titleId = useId();
  const consentTitleId = useId();
  const consentHelpId = useId();
  const capabilityExampleId = useId();
  const contextConsentTitleId = useId();
  const correctionId = useId();
  const correctionHelpId = useId();
  const correctionSaveStatusId = useId();
  const correctionSaveErrorId = useId();
  const contextHelpId = useId();
  const contextToggleLabelId = useId();
  const primaryConsentHeadingRef = useRef<HTMLDivElement>(null);
  const connectedContentRef = useRef<HTMLDivElement>(null);
  const contextConsentHeadingRef = useRef<HTMLDivElement>(null);
  const contextToggleRef = useRef<HTMLInputElement>(null);
  const primaryConsentWasVisible = useRef(false);
  const restoreContextToggleFocus = useRef(false);
  const lastSavedCorrection = useRef(preferences.correction_preferences);
  const correctionDraftRef = useRef(
    correctionDraftValue ?? preferences.correction_preferences,
  );
  const correctionAutosaveTimer = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const correctionSaveRequest = useRef(0);
  const activeCorrectionSave = useRef<{
    value: string;
    promise: Promise<void>;
  } | null>(null);
  const saveCorrectionPreferencesRef = useRef(
    onSaveCorrectionPreferences,
  );
  const [acknowledged, setAcknowledged] = useState(false);
  const [consentContextTerms, setConsentContextTerms] = useState(false);
  const [confirmContextTerms, setConfirmContextTerms] = useState(false);
  const [contextTermsAcknowledged, setContextTermsAcknowledged] =
    useState(false);
  const [localCorrectionDraft, setLocalCorrectionDraft] = useState(
    preferences.correction_preferences,
  );
  const [savedCorrection, setSavedCorrection] = useState(
    preferences.correction_preferences,
  );
  const [correctionSavePhase, setCorrectionSavePhase] =
    useState<CorrectionSavePhase>("saved");
  const [correctionSaveError, setCorrectionSaveError] = useState<string | null>(
    null,
  );
  const correctionDraft = correctionDraftValue ?? localCorrectionDraft;

  useEffect(() => {
    const previousSaved = lastSavedCorrection.current;
    lastSavedCorrection.current = preferences.correction_preferences;
    setSavedCorrection(preferences.correction_preferences);
    if (correctionDraftValue === undefined) {
      setLocalCorrectionDraft((current) =>
        // probe 或其他設定更新不應吃掉使用者尚未儲存的草稿。
        current === previousSaved
          ? preferences.correction_preferences
          : current,
      );
    }
    if (
      (correctionDraftValue ?? correctionDraftRef.current).trim() ===
      preferences.correction_preferences
    ) {
      setCorrectionSavePhase("saved");
      setCorrectionSaveError(null);
    }
  }, [correctionDraftValue, preferences.correction_preferences]);

  useEffect(() => {
    correctionDraftRef.current = correctionDraft;
  }, [correctionDraft]);

  useEffect(() => {
    saveCorrectionPreferencesRef.current = onSaveCorrectionPreferences;
  }, [onSaveCorrectionPreferences]);

  useEffect(
    () => () => {
      if (correctionAutosaveTimer.current !== null) {
        clearTimeout(correctionAutosaveTimer.current);
      }
      const normalized = correctionDraftRef.current.trim();
      if (
        normalized !== lastSavedCorrection.current &&
        activeCorrectionSave.current?.value !== normalized
      ) {
        // Settings 頁會保留受控 draft；這裡仍 best-effort flush，確保
        // 其他 unmount 路徑不會把已輸入內容靜默丟掉。
        void Promise.resolve()
          .then(() => saveCorrectionPreferencesRef.current(normalized))
          .catch(() => {});
      }
    },
    [],
  );

  useEffect(() => {
    // 登入類型、CLI 版本或 consent target 改變後必須重新確認；不能沿用
    // 前一個 ChatGPT/API-key 計費情境下的勾選狀態。
    setAcknowledged(false);
    setConsentContextTerms(false);
    setContextTermsAcknowledged(false);
    setConfirmContextTerms(false);
  }, [
    status.auth_mode,
    status.version,
    globalContextEnabled,
    preferences.consent_valid,
    preferences.context_consent_valid,
  ]);

  const copy = codexStatusCopy(status);
  const statusReady = status.availability === "ready";
  const consentValid = codexPrimaryConsentIsValid(preferences);
  const codexActive =
    statusReady &&
    codexConsentIsReady(preferences, globalContextEnabled);
  const testPending = codexTestIsPending(testState);
  const normalizedDraft = correctionDraft.trim();
  const correctionChanged = normalizedDraft !== savedCorrection;

  const persistCorrectionDraft = (
    draft = correctionDraftRef.current,
  ): Promise<void> => {
    if (correctionAutosaveTimer.current !== null) {
      clearTimeout(correctionAutosaveTimer.current);
      correctionAutosaveTimer.current = null;
    }
    const normalized = draft.trim();
    if (normalized === lastSavedCorrection.current) {
      setCorrectionSavePhase("saved");
      setCorrectionSaveError(null);
      return Promise.resolve();
    }

    const active = activeCorrectionSave.current;
    if (active?.value === normalized) return active.promise;

    const request = ++correctionSaveRequest.current;
    setCorrectionSavePhase("saving");
    setCorrectionSaveError(null);
    const promise = Promise.resolve()
      .then(() => saveCorrectionPreferencesRef.current(normalized))
      .then(() => {
        if (request !== correctionSaveRequest.current) return;
        lastSavedCorrection.current = normalized;
        setSavedCorrection(normalized);
        setCorrectionSaveError(null);
        setCorrectionSavePhase(
          correctionDraftRef.current.trim() === normalized
            ? "saved"
            : "dirty",
        );
      })
      .catch((reason) => {
        if (request === correctionSaveRequest.current) {
          setCorrectionSavePhase("error");
          setCorrectionSaveError(
            reason instanceof Error && reason.message
              ? reason.message
              : "自動儲存失敗。請重新檢查 Codex 後再試一次。",
          );
        }
        throw reason;
      });

    activeCorrectionSave.current = { value: normalized, promise };
    void promise.then(
      () => {
        if (activeCorrectionSave.current?.promise === promise) {
          activeCorrectionSave.current = null;
        }
      },
      () => {
        if (activeCorrectionSave.current?.promise === promise) {
          activeCorrectionSave.current = null;
        }
      },
    );
    return promise;
  };

  const updateCorrectionDraft = (value: string) => {
    correctionDraftRef.current = value;
    if (correctionDraftValue === undefined) {
      setLocalCorrectionDraft(value);
    }
    onCorrectionDraftChange?.(value);
    setCorrectionSaveError(null);

    if (correctionAutosaveTimer.current !== null) {
      clearTimeout(correctionAutosaveTimer.current);
      correctionAutosaveTimer.current = null;
    }
    if (value.trim() === lastSavedCorrection.current) {
      setCorrectionSavePhase("saved");
      return;
    }

    setCorrectionSavePhase("dirty");
    correctionAutosaveTimer.current = setTimeout(() => {
      correctionAutosaveTimer.current = null;
      void persistCorrectionDraft(value).catch(() => {});
    }, CORRECTION_AUTOSAVE_DELAY_MS);
  };

  const correctionStatus =
    correctionSavePhase === "saving"
      ? { tone: "blue", label: "自動儲存中…" }
      : correctionSavePhase === "error"
        ? { tone: "red", label: "未儲存" }
        : correctionChanged || correctionSavePhase === "dirty"
          ? { tone: "amber", label: "即將自動儲存" }
          : { tone: "green", label: "已儲存" };

  useEffect(() => {
    const visible = statusReady && !consentValid;
    if (visible && !primaryConsentWasVisible.current) {
      primaryConsentHeadingRef.current?.focus();
    } else if (
      !visible &&
      primaryConsentWasVisible.current &&
      consentValid
    ) {
      connectedContentRef.current?.focus();
    }
    primaryConsentWasVisible.current = visible;
  }, [consentValid, statusReady]);

  useEffect(() => {
    if (confirmContextTerms) {
      contextConsentHeadingRef.current?.focus();
    } else if (restoreContextToggleFocus.current) {
      contextToggleRef.current?.focus();
      restoreContextToggleFocus.current = false;
    }
  }, [confirmContextTerms]);

  const closeContextConsent = () => {
    restoreContextToggleFocus.current = true;
    setContextTermsAcknowledged(false);
    setConfirmContextTerms(false);
  };

  const correctionPreferencesEditor = (
    <div className="row" style={{ alignItems: "flex-start" }}>
      <div className="min-w-0 flex-1">
        <label className="row-label" htmlFor={correctionId}>
          正確拼法清單（選填）
        </label>
        <div className="row-sub" id={correctionHelpId}>
          這份清單只儲存在本機，你可以在同意 Codex
          前先檢視、修改或清空。每行一個，或用逗號分隔，例如「Claude、Flutter、Whisper」。這不是模糊錯字清單：
          空白合併、字母不同或含數字的固定拼法請用個人字典明確指定。啟用
          Codex
          後，只有在本次轉錄中呈現為「左側至少四字母＋單一連字號＋右側兩字母」且內容保護可採用的項目才會送出；請勿填入密碼或金鑰。
        </div>
        <textarea
          id={correctionId}
          className="text-area no-drag mt-2 w-full"
          name="codex-correction-preferences"
          rows={4}
          maxLength={MAX_CORRECTION_PREFERENCES}
          autoComplete="off"
          spellCheck={false}
          translate="no"
          disabled={preferencesSaving}
          placeholder={"Claude\nFlutter\nWhisper"}
          value={correctionDraft}
          aria-describedby={`${correctionHelpId} ${correctionSaveStatusId}${
            correctionSaveError ? ` ${correctionSaveErrorId}` : ""
          }`}
          onChange={(event) => updateCorrectionDraft(event.target.value)}
          onBlur={() => {
            if (correctionChanged) {
              void persistCorrectionDraft().catch(() => {});
            }
          }}
        />
        <div className="codex-draft-status">
          <span>
            {correctionDraft.length}/{MAX_CORRECTION_PREFERENCES} 字
          </span>
          <span
            className={`pill ${correctionStatus.tone}`}
            id={correctionSaveStatusId}
            role="status"
            aria-live="polite"
          >
            {correctionStatus.label}
          </span>
        </div>
        {correctionSaveError && (
          <div
            className="codex-draft-error"
            id={correctionSaveErrorId}
            role="alert"
          >
            <span>{correctionSaveError} 草稿仍保留。</span>
            <button
              className="btn no-drag"
              onClick={() => {
                void persistCorrectionDraft().catch(() => {});
              }}
            >
              重試儲存
            </button>
          </div>
        )}
      </div>
      <button
        className="btn no-drag"
        disabled={
          !correctionChanged ||
          preferencesSaving ||
          correctionSavePhase === "saving"
        }
        onClick={() => {
          void persistCorrectionDraft(normalizedDraft).catch(() => {});
        }}
      >
        {correctionSavePhase === "saving" ? "儲存中…" : "立即儲存"}
      </button>
    </div>
  );

  return (
    <section
      aria-labelledby={titleId}
      aria-busy={
        status.availability === "checking" ||
        enablePending ||
        preferencesSaving ||
        correctionSavePhase === "saving" ||
        testPending
      }
    >
      <div className="row" style={{ alignItems: "flex-start" }}>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="row-label" id={titleId}>
              Codex 專業拼法
            </span>
            <span className="pill amber">實驗</span>
            <span className="pill blue">需要網路</span>
            {consentValid && (
              <span className={`pill ${codexActive ? "green" : "amber"}`}>
                {codexActive ? "使用中" : "已同意・目前未使用"}
              </span>
            )}
          </div>
          <div
            className={`${statusClass(copy.tone)} mt-2`}
            role={copy.tone === "error" ? "alert" : "status"}
            aria-live={copy.tone === "error" ? "assertive" : "polite"}
          >
            <span>
              <b>{copy.title}</b>
              <span className="block">{copy.detail}</span>
            </span>
          </div>
          <p className="codex-capability-example" id={capabilityExampleId}>
            目前只嘗試極窄的連字號修正，例如 Clau-de → Claude。Py Torch →
            PyTorch 或 Pie Torch → PyTorch 請用個人字典明確指定。
          </p>
        </div>
        <button
          className="btn no-drag"
          disabled={status.availability === "checking"}
          onClick={onRefresh}
        >
          {status.availability === "checking" ? "檢查中…" : "重新檢查"}
        </button>
      </div>

      {correctionPreferencesEditor}

      {statusReady && !consentValid && (
        <section
          className="consent-panel"
          aria-labelledby={consentTitleId}
          aria-busy={enablePending}
        >
          <div className="min-w-0 flex-1">
            <div
              className="consent-title"
              id={consentTitleId}
              ref={primaryConsentHeadingRef}
              tabIndex={-1}
            >
              允許 Claro 使用 Codex 統一專業拼法？
            </div>
            <p id={consentHelpId}>
              Claro 會先在這台 Mac 完成語音辨識，再把限定的文字資料送給
              OpenAI Codex。每次最多採用三個符合極窄結構規則的連字號修正；這能降低誤改，
              但不能證明語意一定正確，重要文字仍請檢查。
            </p>
            <ul aria-describedby={capabilityExampleId}>
              <li>
                <b>會送出：</b>
                轉錄文字，以及與本次轉錄相關且內容保護可採用的正確拼法與個人詞彙正確文字；
                三類候選詞合計最多 32 項。
              </li>
              <li>
                <b>選擇性送出：</b>
                只有移除單一尾端連字號後，字母與大小寫逐字相同、且內容保護可採用的有限畫面候選詞；需另外同意。
              </li>
              <li>
                <b>不會送出：</b>
                音訊、App 名、視窗標題、周邊完整句子、整個畫面、整個專案或 API Key。
              </li>
              <li>
                <b>安全退回：</b>
                找不到可採用候選時完全不呼叫 Codex、也不使用額度；Codex
                不可用、逾時或結果未通過內容保護時，使用本機結果。
              </li>
            </ul>
            <p className="mt-2">
              同意會綁定目前的登入類型與 Codex CLI
              版本；Claro 無法辨識帳戶身分。若你在同一登入類型下切換帳戶，停用或重新確認前，文字會送到當下登入的帳戶。
            </p>
            <label className="local-only-control mt-2">
              <input
                type="checkbox"
                name="codex-consent"
                checked={acknowledged}
                disabled={enablePending}
                aria-describedby={`${consentHelpId} ${capabilityExampleId}`}
                onChange={(event) => setAcknowledged(event.target.checked)}
              />
              <span>
                我了解上述文字資料會送到 OpenAI，並使用目前 Codex
                登入的額度或計費設定。
              </span>
            </label>
            <label className="local-only-control mt-2">
              <input
                type="checkbox"
                name="codex-share-context-terms-consent"
                checked={consentContextTerms}
                disabled={!globalContextEnabled || enablePending}
                aria-describedby={contextHelpId}
                onChange={(event) =>
                  setConsentContextTerms(event.target.checked)
                }
              />
              <span>也允許送出本機萃取的有限畫面詞彙</span>
            </label>
            <p id={contextHelpId}>
              {globalContextEnabled
                ? "只送移除單一尾端連字號後，字母與大小寫逐字相同、且內容保護可採用的候選詞。"
                : "目前「螢幕上下文」已關閉，因此只會送轉錄文字，以及與本次轉錄相關且內容保護可採用的正確拼法與個人詞彙正確文字。"}
            </p>
          </div>
          <div className="consent-actions">
            <button
              className="btn-primary no-drag"
              disabled={!acknowledged || enablePending}
              onClick={() =>
                onEnable({
                  share_context_terms:
                    globalContextEnabled && consentContextTerms,
                })
              }
            >
              {enablePending ? "正在啟用…" : "同意並使用 Codex"}
            </button>
          </div>
        </section>
      )}

      {consentValid && (
        <>
          <div
            className="codex-data-boundary"
            ref={connectedContentRef}
            role="note"
            tabIndex={-1}
          >
            <b>{codexActive ? "資料界線：" : "同意仍有效，目前未使用："}</b>
            {codexActive
              ? "音訊與辨識留在本機；Codex 只收到已同意的文字範圍，失敗時使用本機結果。"
              : "目前不是可執行的專業校字狀態，因此聽寫不會呼叫 Codex；切回專業校字後才會使用既有同意。"}
          </div>

          <div className="row">
            <div className="min-w-0 flex-1">
              <span className="row-label" id={contextToggleLabelId}>
                有限畫面詞彙
              </span>
              <div className="row-sub" id={contextHelpId}>
                {globalContextEnabled
                  ? "開啟後只送移除單一尾端連字號後，字母與大小寫逐字相同、且內容保護可採用的候選詞。"
                  : "目前「螢幕上下文」已關閉，不會送出任何畫面詞彙。"}
              </div>
            </div>
            <label className="local-only-control">
              <input
                ref={contextToggleRef}
                type="checkbox"
                name="codex-share-context-terms"
                checked={
                  globalContextEnabled && preferences.share_context_terms
                }
                disabled={!globalContextEnabled || preferencesSaving}
                aria-labelledby={contextToggleLabelId}
                aria-describedby={contextHelpId}
                onChange={(event) => {
                  if (!event.target.checked) {
                    onShareContextTermsChange(false);
                    return;
                  }
                  if (preferences.context_consent_valid) {
                    onShareContextTermsChange(true);
                    return;
                  }
                  setContextTermsAcknowledged(false);
                  setConfirmContextTerms(true);
                }}
              />
              <span>
                {globalContextEnabled && preferences.share_context_terms
                  ? "已開啟"
                  : "已關閉"}
              </span>
            </label>
          </div>

          {confirmContextTerms && (
            <section
              className="consent-panel"
              aria-labelledby={contextConsentTitleId}
              aria-busy={preferencesSaving}
            >
              <div className="min-w-0 flex-1">
                <div
                  className="consent-title"
                  id={contextConsentTitleId}
                  ref={contextConsentHeadingRef}
                  tabIndex={-1}
                >
                  允許 Codex 使用有限畫面詞彙？
                </div>
                <p>
                  Claro 只會送出目前畫面中，移除單一尾端連字號後，字母與大小寫逐字相同、且內容保護可採用的候選詞；不送周邊完整句子、音訊或整個專案。
                </p>
                <label className="local-only-control mt-2">
                  <input
                    type="checkbox"
                    name="codex-context-terms-consent"
                    checked={contextTermsAcknowledged}
                    disabled={preferencesSaving}
                    onChange={(event) =>
                      setContextTermsAcknowledged(event.target.checked)
                    }
                  />
                  <span>我了解這些詞彙會送到 OpenAI Codex。</span>
                </label>
              </div>
              <div className="consent-actions">
                <button
                  className="btn no-drag"
                  disabled={preferencesSaving}
                  onClick={closeContextConsent}
                >
                  保持關閉
                </button>
                <button
                  className="btn-primary no-drag"
                  disabled={!contextTermsAcknowledged || preferencesSaving}
                  onClick={() => {
                    restoreContextToggleFocus.current = true;
                    onShareContextTermsChange(true);
                  }}
                >
                  {preferencesSaving ? "正在儲存…" : "允許有限畫面詞彙"}
                </button>
              </div>
            </section>
          )}

          <div className="row" style={{ alignItems: "flex-start" }}>
            <div className="min-w-0 flex-1">
              <span className="row-label">測試 Codex 校字</span>
              <div className="row-sub">
                {codexActive
                  ? "送出一段 Claro 合成測試文字；不使用目前視窗內容，但會使用 Codex 額度，實際用量取決於目前的 CLI、模型與帳戶設定。"
                  : "目前未使用 Codex；切回可執行的專業校字狀態後才能測試。"}
              </div>
              {testState.phase === "success" && (
                <div className="setup-inline-state mt-2" role="status">
                  <b>受控校字測試已通過</b>
                  <span className="block">測試文字：{testState.input}</span>
                  <span className="block">校字結果：{testState.output}</span>
                </div>
              )}
              {testState.phase === "cancelled" && (
                <div className="setup-inline-state mt-2" role="status">
                  已取消測試，設定未變更。
                </div>
              )}
              {testState.phase === "failed" && (
                <div className="config-error mt-2" role="alert">
                  {TEST_FAILURE_COPY[testState.reason]}
                </div>
              )}
              {testPending && (
                <div className="setup-inline-state mt-2" role="status">
                  {testState.phase === "cancelling"
                    ? "正在取消測試…"
                    : "正在測試 Codex 校字…"}
                </div>
              )}
            </div>
            {testPending ? (
              <button
                className="btn danger-quiet no-drag"
                disabled={testState.phase === "cancelling"}
                onClick={onCancelTest}
              >
                {testState.phase === "cancelling" ? "取消中…" : "取消測試"}
              </button>
            ) : (
              <button
                className="btn no-drag"
                disabled={
                  !statusReady ||
                  !codexActive ||
                  preferencesSaving ||
                  enablePending
                }
                onClick={onTest}
              >
                測試
              </button>
            )}
          </div>
        </>
      )}
    </section>
  );
}
