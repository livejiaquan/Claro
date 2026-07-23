import { useEffect, useId, useState } from "react";
import { codexConsentIsReady, codexTestIsPending } from "../codexProviderState";
import type {
  CodexCliStatus,
  CodexEnableOptions,
  CodexPreferences,
  CodexTestFailureReason,
  CodexTestState,
} from "../types";

const MAX_CORRECTION_PREFERENCES = 1000;

const TEST_FAILURE_COPY: Record<CodexTestFailureReason, string> = {
  timeout:
    "Codex 沒有在本次測試時間內完成。設定已保留；正式聽寫遇到相同情況會使用本機結果。",
  rate_limited:
    "目前無法使用 Codex 額度。請稍後再試；Claro 仍可使用本機語音辨識。",
  auth_required: "Codex 登入已失效。請重新登入後，再按「重新檢查」。",
  unavailable:
    "Codex 目前無法使用。設定已保留；Claro 仍可使用本機語音辨識。",
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
  onRefresh: () => void;
  onEnable: (options: CodexEnableOptions) => void;
  onSaveCorrectionPreferences: (value: string) => void;
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
  onRefresh,
  onEnable,
  onSaveCorrectionPreferences,
  onShareContextTermsChange,
  onTest,
  onCancelTest,
}: CodexProviderPanelProps) {
  const titleId = useId();
  const consentTitleId = useId();
  const consentHelpId = useId();
  const contextConsentTitleId = useId();
  const correctionId = useId();
  const correctionHelpId = useId();
  const contextHelpId = useId();
  const contextToggleLabelId = useId();
  const [acknowledged, setAcknowledged] = useState(false);
  const [consentContextTerms, setConsentContextTerms] = useState(false);
  const [confirmContextTerms, setConfirmContextTerms] = useState(false);
  const [contextTermsAcknowledged, setContextTermsAcknowledged] =
    useState(false);
  const [correctionDraft, setCorrectionDraft] = useState(
    preferences.correction_preferences,
  );

  useEffect(() => {
    setCorrectionDraft(preferences.correction_preferences);
  }, [preferences.correction_preferences]);

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
  const consentReady = codexConsentIsReady(
    preferences,
    globalContextEnabled,
  );
  const testPending = codexTestIsPending(testState);
  const normalizedDraft = correctionDraft.trim();
  const correctionChanged =
    normalizedDraft !== preferences.correction_preferences;

  return (
    <section
      aria-labelledby={titleId}
      aria-busy={
        status.availability === "checking" ||
        enablePending ||
        preferencesSaving ||
        testPending
      }
    >
      <div className="row" style={{ alignItems: "flex-start" }}>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="row-label" id={titleId}>
              Codex CLI 校字
            </span>
            <span className="pill blue">需要網路</span>
            {statusReady && consentReady && (
              <span className="pill green">已連接</span>
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
        </div>
        <button
          className="btn no-drag"
          disabled={status.availability === "checking"}
          onClick={onRefresh}
        >
          {status.availability === "checking" ? "檢查中…" : "重新檢查"}
        </button>
      </div>

      {statusReady && !consentReady && (
        <section
          className="consent-panel"
          aria-labelledby={consentTitleId}
          aria-busy={enablePending}
        >
          <div className="min-w-0 flex-1">
            <div className="consent-title" id={consentTitleId}>
              允許 Claro 使用 Codex 校字？
            </div>
            <p id={consentHelpId}>
              Claro 會先在本機完成語音辨識，再把轉錄文字、你儲存的正確拼法清單與個人詞彙清單送給
              OpenAI Codex。Codex 最多提出三個只涉及大小寫、空白、底線、連字號或標點的英文拼法正規化；字母不同或同音誤認不會自動替換，請改用個人字典。數字、否定、版本、URL、句序與語氣由本機內容保護鎖定。音訊不會送出。Codex
              不可用、逾時或結果未通過內容保護時，Claro 會使用本機結果。
            </p>
            <label className="local-only-control mt-2">
              <input
                type="checkbox"
                name="codex-consent"
                checked={acknowledged}
                disabled={enablePending}
                aria-describedby={consentHelpId}
                onChange={(event) => setAcknowledged(event.target.checked)}
              />
              <span>
                我了解上述文字資料會送到 OpenAI、只會自動套用受控拼法正規化，並使用我的 Codex 帳戶額度或計費設定。
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
                ? "只送詞彙清單來判斷專業名稱，不送周邊完整句子、音訊或整個專案。"
                : "目前「螢幕上下文」已關閉，因此只會送轉錄文字與校字偏好。"}
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

      {consentReady && (
        <>
          <div className="row" style={{ alignItems: "flex-start" }}>
            <div className="min-w-0 flex-1">
              <label className="row-label" htmlFor={correctionId}>
                正確拼法清單（選填）
              </label>
              <div className="row-sub" id={correctionHelpId}>
                每行一個，或用逗號分隔，例如「MLX、PyTorch、Tauri」。這不是 system prompt；
                只有完整列出的英文拼法可作為正規化目標，不會授權把相近但字母不同的單字猜成它。其他敘述不會放寬內容保護。真正的固定錯字請用個人字典。清單會隨每次 Codex 校字送出，請勿填入密碼或金鑰。
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
                placeholder={"MLX\nPyTorch\nTauri"}
                value={correctionDraft}
                aria-describedby={correctionHelpId}
                onChange={(event) => setCorrectionDraft(event.target.value)}
              />
              <div
                className="text-[11px] mt-1"
                style={{ color: "var(--faint)" }}
              >
                {correctionDraft.length}/{MAX_CORRECTION_PREFERENCES} 字
              </div>
            </div>
            <button
              className="btn no-drag"
              disabled={!correctionChanged || preferencesSaving}
              onClick={() =>
                onSaveCorrectionPreferences(normalizedDraft)
              }
            >
              {preferencesSaving ? "儲存中…" : "儲存正確拼法"}
            </button>
          </div>

          <div className="row">
            <div className="min-w-0 flex-1">
              <span className="row-label" id={contextToggleLabelId}>
                有限畫面詞彙
              </span>
              <div className="row-sub" id={contextHelpId}>
                {globalContextEnabled
                  ? "開啟後會送出本機萃取的候選詞彙，不送周邊完整句子、音訊或整個專案。"
                  : "目前「螢幕上下文」已關閉，不會送出任何畫面詞彙。"}
              </div>
            </div>
            <label className="local-only-control">
              <input
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
            >
              <div className="min-w-0 flex-1">
                <div className="consent-title" id={contextConsentTitleId}>
                  允許 Codex 使用有限畫面詞彙？
                </div>
                <p>
                  Claro
                  只會送出本機從目前畫面萃取的候選詞彙，用來判斷專業名稱；不送周邊完整句子、音訊或整個專案。
                </p>
                <label className="local-only-control mt-2">
                  <input
                    type="checkbox"
                    name="codex-context-terms-consent"
                    checked={contextTermsAcknowledged}
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
                  onClick={() => setConfirmContextTerms(false)}
                >
                  保持關閉
                </button>
                <button
                  className="btn-primary no-drag"
                  disabled={!contextTermsAcknowledged}
                  onClick={() => {
                    setConfirmContextTerms(false);
                    onShareContextTermsChange(true);
                  }}
                >
                  允許有限畫面詞彙
                </button>
              </div>
            </section>
          )}

          <div className="row" style={{ alignItems: "flex-start" }}>
            <div className="min-w-0 flex-1">
              <span className="row-label">測試 Codex 校字</span>
              <div className="row-sub">
                送出一段 Claro
                合成測試文字；不使用目前視窗內容，可能計入少量 Codex 使用量。
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
                disabled={!statusReady || preferencesSaving || enablePending}
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
