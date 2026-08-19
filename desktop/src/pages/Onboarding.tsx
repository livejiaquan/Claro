import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import DownloadStatus from "../components/DownloadStatus";
import {
  resolveDownloadState,
  type DownloadKind,
} from "../downloadState";
import {
  resolveLlmConfig,
  type DownloadProgress,
  type HardwareProfile,
  type LlmConfig,
  type MicLevel,
  type ModelInfo,
  type ResolvedLlmConfig,
  type Status,
} from "../types";
import { polishReadinessLabel } from "../privacyMessage";
import { Hotkey, LevelBar } from "../ui";

function StepStatus({ ready, label }: { ready: boolean; label: string }) {
  return (
    <span className={`setup-step-status ${ready ? "ready" : "pending"}`}>
      <span aria-hidden="true">{ready ? "✓" : "○"}</span>
      {label}
    </span>
  );
}

export default function Onboarding({
  status,
  mic,
  progress,
  llmProgress,
  onDownloadStart,
  refresh,
  onToast,
  onDone,
  onOpenSettings,
}: {
  status: Status;
  mic: MicLevel;
  progress: DownloadProgress | null;
  llmProgress: DownloadProgress | null;
  onDownloadStart: (kind: DownloadKind) => void;
  refresh: () => void;
  onToast: (message: string) => void;
  onDone: () => void;
  onOpenSettings: () => void;
}) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelsLoading, setModelsLoading] = useState(true);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [micChecked, setMicChecked] = useState(false);
  const [llm, setLlm] = useState<ResolvedLlmConfig | null>(null);
  const [builtinLlms, setBuiltinLlms] = useState<ModelInfo[]>([]);
  const [hardware, setHardware] = useState<HardwareProfile | null>(null);
  const [polishLoading, setPolishLoading] = useState(true);
  const [polishError, setPolishError] = useState<string | null>(null);
  const [polishWarning, setPolishWarning] = useState<string | null>(null);
  const [downloadIntent, setDownloadIntent] = useState<{
    key: string;
    phase: "starting" | "cancelling";
  } | null>(null);
  const [polishSelectionPending, setPolishSelectionPending] = useState(false);
  const modelsSyncStarted = useRef(false);
  const polishSyncStarted = useRef(false);

  const loadModels = useCallback((showLoading = false) => {
    if (showLoading) setModelsLoading(true);
    setModelsError(null);
    invoke<ModelInfo[]>("list_models")
      .then(setModels)
      .catch((reason) => setModelsError(String(reason)))
      .finally(() => setModelsLoading(false));
  }, []);

  useEffect(
    () => {
      const showLoading = !modelsSyncStarted.current;
      modelsSyncStarted.current = true;
      loadModels(showLoading);
    },
    [
      loadModels,
      progress?.model_id,
      progress?.done,
      progress?.downloaded,
      progress?.activation_status,
      progress?.error,
      status.model_id,
    ],
  );

  const loadPolish = useCallback((showLoading = false) => {
    if (showLoading) setPolishLoading(true);
    setPolishError(null);
    setPolishWarning(null);
    return Promise.allSettled([
      invoke<LlmConfig>("get_llm_config"),
      invoke<ModelInfo[]>("list_builtin_llms"),
      invoke<HardwareProfile>("get_hardware_profile"),
    ])
      .then(([config, builtin, profile]) => {
        if (config.status === "fulfilled") {
          setLlm(resolveLlmConfig(config.value));
        } else {
          setLlm(null);
          setPolishError("無法檢查目前的文字整理設定，請重新檢查。");
        }

        const optionalFailures: string[] = [];
        if (builtin.status === "fulfilled") {
          setBuiltinLlms(builtin.value);
        } else {
          setBuiltinLlms([]);
          optionalFailures.push("Claro 內建整理選項");
        }
        if (profile.status === "fulfilled") {
          setHardware(profile.value);
        } else {
          setHardware(null);
          optionalFailures.push("這台 Mac 的推薦方案");
        }
        if (optionalFailures.length) {
          setPolishWarning(`暫時無法取得${optionalFailures.join("與")}；你仍可先使用可見的選項。`);
        }
      })
      .finally(() => setPolishLoading(false));
  }, []);

  useEffect(
    () => {
      const showLoading = !polishSyncStarted.current;
      polishSyncStarted.current = true;
      void loadPolish(showLoading);
    },
    [
      loadPolish,
      llmProgress?.model_id,
      llmProgress?.done,
      llmProgress?.downloaded,
      llmProgress?.error,
    ],
  );

  useEffect(() => {
    if (mic.passed || (mic.active && mic.level > 0.01)) setMicChecked(true);
  }, [mic.active, mic.level, mic.passed]);

  const permissionsReady = status.accessibility && status.hotkey_active;
  const microphoneReady = micChecked || status.mic_test_passed_this_launch;
  const polishReady = Boolean(
    llm && (llm.polish_mode === "raw" || (llm.effective_mode !== "raw" && !llm.blocked_reason)),
  );
  const requiredReady = permissionsReady && microphoneReady && status.model_present;
  const firstDictationDone = status.successful_pastes_this_launch > 0;
  const completionReady = status.setup_completed || (requiredReady && firstDictationDone);
  const checkedCount =
    Number(permissionsReady) + Number(microphoneReady) + Number(status.model_present);
  const recommendedBuiltin =
    builtinLlms.find((model) => model.id === hardware?.recommended_llm_model) ?? builtinLlms[0];
  const onboardingModel =
    models.find((model) => model.recommended && model.available) ??
    models.find((model) => model.active && model.available) ??
    models.find((model) => model.available);
  const intentMatches = (
    kind: DownloadKind,
    modelId: string,
    phase: "starting" | "cancelling",
  ) => downloadIntent?.key === `${kind}:${modelId}` && downloadIntent.phase === phase;
  const onboardingSttDownload = onboardingModel
    ? resolveDownloadState({
        modelId: onboardingModel.id,
        downloaded: onboardingModel.downloaded,
        backendDownloading: onboardingModel.downloading,
        progress,
        startRequested: intentMatches("stt", onboardingModel.id, "starting"),
        cancelRequested: intentMatches("stt", onboardingModel.id, "cancelling"),
      })
    : null;
  const onboardingLlmDownload = recommendedBuiltin
    ? resolveDownloadState({
        modelId: recommendedBuiltin.id,
        downloaded: recommendedBuiltin.downloaded,
        backendDownloading: recommendedBuiltin.downloading,
        progress: llmProgress,
        startRequested: intentMatches("llm", recommendedBuiltin.id, "starting"),
        cancelRequested: intentMatches("llm", recommendedBuiltin.id, "cancelling"),
      })
    : null;
  const anyDownloadActive =
    Boolean(onboardingSttDownload?.active) || Boolean(onboardingLlmDownload?.active);

  const run = (command: string, args?: Record<string, unknown>, success?: string) => {
    setError(null);
    invoke(command, args)
      .then(() => {
        refresh();
        loadModels();
        if (success) onToast(success);
      })
      .catch((reason) => setError(String(reason)));
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
      if (kind === "stt") {
        refresh();
        loadModels();
      } else {
        await loadPolish();
      }
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

  const chooseLocalPolish = async (provider: "apple" | "builtin", model = "") => {
    setError(null);
    try {
      await invoke("set_llm_config", { provider, model, baseUrl: "", confirmed: true });
      await invoke("set_polish_mode", { mode: "clean", confirmed: true });
      await loadPolish();
      refresh();
      onToast(provider === "apple" ? "已使用 Apple 端上文字整理" : "已選擇 Claro 內建文字整理");
      return true;
    } catch (reason) {
      setError(String(reason));
      return false;
    }
  };

  const downloadAndUseBuiltin = async (model: ModelInfo) => {
    if (polishSelectionPending) return;
    const key = `llm:${model.id}`;
    setPolishSelectionPending(true);
    if (!model.downloaded) {
      setDownloadIntent({ key, phase: "starting" });
      onDownloadStart("llm");
    }
    try {
      const configured = await chooseLocalPolish("builtin", model.id);
      if (!configured) {
        setDownloadIntent((current) => (current?.key === key ? null : current));
        return;
      }
      if (!model.downloaded) {
        await invoke("download_builtin_llm", { id: model.id });
        await loadPolish();
      }
    } catch (reason) {
      setDownloadIntent((current) => (current?.key === key ? null : current));
      setError(String(reason));
    } finally {
      setPolishSelectionPending(false);
    }
  };

  const chooseRaw = async () => {
    setError(null);
    try {
      await invoke("set_polish_mode", { mode: "raw", confirmed: true });
      await loadPolish();
      refresh();
      onToast("已選擇原樣轉錄；之後可在設定開啟文字整理");
    } catch (reason) {
      setError(String(reason));
    }
  };

  const retryPermission = () => {
    setError(null);
    invoke<boolean>("retry_hotkey")
      .then((active) => {
        refresh();
        onToast(active ? "輔助使用權限已就緒" : "仍未偵測到權限，請在系統設定勾選 Claro");
      })
      .catch((reason) => setError(String(reason)));
  };

  return (
    <div className="page-in setup-page">
      <div className="setup-hero">
        <div>
          <span className="setup-eyebrow">首次設定・隨時可重新檢查</span>
          <h1>讓 Claro 聽得到，也貼得出去。</h1>
          <p>先完成 3 項必要檢查；接著做一次真正的聽寫，確認文字能貼到游標處。</p>
        </div>
        <div
          className="setup-score"
          aria-label={`已通過 ${checkedCount} 個必要檢查，共 3 個；${requiredReady ? "下一步是做第一次聽寫" : "請先完成必要檢查"}`}
        >
          <strong>{checkedCount}/3</strong>
          <span>必要檢查</span>
          {requiredReady && <span className="setup-score-next">下一步：第一次聽寫</span>}
        </div>
      </div>

      {error && <div className="config-error mb-4" role="alert">{error}</div>}

      <section className={`setup-step ${permissionsReady ? "complete" : ""}`} aria-labelledby="setup-permission-title">
        <div className="setup-step-number" aria-hidden="true">1</div>
        <div className="setup-step-body">
          <div className="setup-step-heading">
            <div>
              <h2 id="setup-permission-title">允許快捷鍵與貼上</h2>
              <p>macOS 的「輔助使用」權限讓 Claro 能在其他 App 監聽快捷鍵並貼上文字。</p>
            </div>
            <StepStatus ready={permissionsReady} label={permissionsReady ? "已就緒" : "需要授權"} />
          </div>
          {!permissionsReady && (
            <div className="setup-actions">
              <button className="btn-primary no-drag" onClick={() => invoke("open_accessibility_settings").catch((reason) => setError(String(reason)))}>
                打開系統設定
              </button>
              <button className="btn no-drag" onClick={retryPermission}>重新檢查</button>
              <button className="btn danger-quiet no-drag" onClick={() => run("reset_accessibility")}>
                清除舊授權
              </button>
            </div>
          )}
        </div>
      </section>

      <section className={`setup-step ${microphoneReady ? "complete" : ""}`} aria-labelledby="setup-mic-title">
        <div className="setup-step-number" aria-hidden="true">2</div>
        <div className="setup-step-body">
          <div className="setup-step-heading">
            <div>
              <h2 id="setup-mic-title">測試麥克風</h2>
              <p>{mic.active ? (mic.level > 0.01 ? "收音正常，測試即將完成。" : "正在聆聽；請用平常音量說幾句話。") : microphoneReady ? "收音正常，麥克風已自動釋放，可以開始第一次聽寫。" : mic.timed_out ? "30 秒內沒有偵測到聲音。請確認麥克風權限或改用其他輸入裝置，再重新測試。" : "這只檢查音量，不會辨識、保存或傳送內容。"}</p>
            </div>
            <StepStatus ready={microphoneReady} label={microphoneReady ? "本次已通過" : "尚未測試"} />
          </div>
          <div className="setup-mic-controls">
            <select
              className="select no-drag"
              aria-label="麥克風輸入裝置"
              name="onboarding-input-device"
              value={status.input_device ?? ""}
              onChange={(event) => {
                if (mic.active) invoke("mic_test_stop").catch(() => {});
                run("set_input_device", { name: event.target.value });
                setMicChecked(false);
              }}
            >
              <option value="">系統預設{status.default_input ? `（${status.default_input}）` : ""}</option>
              {status.input_devices.map((device) => <option key={device} value={device}>{device}</option>)}
            </select>
            <button
              className={`btn no-drag ${mic.active ? "recording" : ""}`}
              onClick={() => run(mic.active ? "mic_test_stop" : "mic_test_start")}
            >
              {mic.active ? "停止測試" : "開始測試"}
            </button>
            {!microphoneReady && (
              <button
                className="btn no-drag"
                onClick={() => invoke("open_microphone_settings").catch((reason) => setError(String(reason)))}
              >
                麥克風權限
              </button>
            )}
            <div className="setup-level" aria-label={mic.active ? `目前麥克風音量 ${Math.round(mic.level * 100)}%` : "麥克風測試未啟動"}>
              <LevelBar level={mic.level} active={mic.active} />
            </div>
          </div>
        </div>
      </section>

      <section
        className={`setup-step ${status.model_present ? "complete" : ""}`}
        aria-labelledby="setup-model-title"
        aria-busy={onboardingSttDownload?.active ?? false}
      >
        <div className="setup-step-number" aria-hidden="true">3</div>
        <div className="setup-step-body">
          <div className="setup-step-heading">
            <div>
              <h2 id="setup-model-title">準備語音模型</h2>
              <p>模型下載只會在你按下「下載」後開始；已下載的模型可離線辨識。</p>
            </div>
            <StepStatus ready={status.model_present} label={status.model_present ? "已準備完成" : "尚未下載"} />
          </div>

          {modelsLoading && <div className="setup-inline-state" role="status">正在檢查模型…</div>}
          {modelsError && (
            <div className="config-error" role="alert">
              <span>無法取得模型清單：{modelsError}</span>
              <button className="btn no-drag" onClick={() => loadModels(true)}>重試</button>
            </div>
          )}
          {!modelsLoading && !modelsError && (
            <div className="setup-model-list">
              {hardware && <div className="setup-inline-state">已依這台 Mac 的 {hardware.memory_gb} GB 記憶體選好推薦方案。</div>}
              {onboardingModel && !onboardingModel.downloaded && (
                <div className="setup-download-expectation" role="note">
                  <strong>下載前先知道</strong>
                  <span>
                    需要網路，約 {onboardingModel.size_mb >= 1024
                      ? `${(onboardingModel.size_mb / 1024).toFixed(1)} GB`
                      : `${onboardingModel.size_mb} MB`}；時間取決於網路速度。下載中可取消，已完成的部分會保留，下次可續傳。
                  </span>
                </div>
              )}
              {(onboardingModel ? [onboardingModel] : []).map((model) => {
                const activationStatus = progress?.model_id === model.id ? progress.activation_status : "none";
                const activationPending = activationStatus !== "none";
                const downloadState =
                  onboardingSttDownload ??
                  resolveDownloadState({
                    modelId: model.id,
                    downloaded: model.downloaded,
                    backendDownloading: model.downloading,
                    progress,
                  });
                return (
                  <div
                    className="setup-model-row"
                    key={model.id}
                    aria-busy={downloadState.active}
                  >
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <strong>Claro 語音辨識</strong>
                        <span className="pill blue">這台 Mac 推薦</span>
                        {model.active && model.downloaded && <span className="pill green">使用中</span>}
                        {!model.active && model.downloaded && <span className="pill green">已下載</span>}
                        {model.active && !model.downloaded && <span className="pill amber">已選擇・待下載</span>}
                      </div>
                      <span>{model.desc}・{model.size_mb >= 1024 ? `${(model.size_mb / 1024).toFixed(1)} GB` : `${model.size_mb} MB`}</span>
                      <DownloadStatus
                        state={downloadState}
                        label="Claro 語音辨識模型"
                        onCancel={() => cancelDownload("stt", model.id)}
                      />
                      {activationPending && (
                        <div className="setup-inline-state" role="status">
                          {activationStatus === "waiting_for_idle"
                            ? "已下載完成；本次聽寫結束後，請按「使用」切換。"
                            : `已下載完成，但切換失敗：${progress?.error ?? "未知錯誤"}。請按「使用」重試。`}
                        </div>
                      )}
                    </div>
                    {!model.downloaded && downloadState.canStart && (
                      <button
                        className="btn no-drag"
                        disabled={anyDownloadActive}
                        title={anyDownloadActive ? "請先完成目前的模型下載" : undefined}
                        onClick={() =>
                          void requestDownload(
                            "stt",
                            model.id,
                            "download_model",
                            { id: model.id, activate: true },
                          )
                        }
                      >
                        {downloadState.phase === "failed" || downloadState.phase === "cancelled"
                          ? "重試並續傳"
                          : "下載並使用"}
                      </button>
                    )}
                    {model.downloaded && !model.active && (
                      <button className="btn no-drag" onClick={() => run("set_model", { id: model.id }, "語音辨識已準備完成") }>
                        使用
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </section>

      <section
        className={`setup-step ${polishReady ? "complete" : ""}`}
        aria-labelledby="setup-polish-title"
        aria-busy={Boolean(onboardingLlmDownload?.active || polishSelectionPending)}
      >
        <div className="setup-step-number" aria-hidden="true">＋</div>
        <div className="setup-step-body">
          <div className="setup-step-heading">
            <div>
              <h2 id="setup-polish-title">文字整理（選用）</h2>
              <p>這不會修正語音辨識錯字，也不是完成設定的必要步驟；建議先用原樣轉錄，之後再決定是否整理語助詞與段落。</p>
            </div>
            <StepStatus
              ready={polishReady}
              label={polishReadinessLabel(llm, polishReady)}
            />
          </div>

          {polishLoading && <div className="setup-inline-state" role="status">正在檢查這台 Mac 的整理能力…</div>}
          {!polishLoading && polishError && (
            <div className="config-error" role="alert">
              <span>{polishError}</span>
              <button className="btn no-drag" onClick={() => void loadPolish(true)}>重新檢查</button>
            </div>
          )}
          {!polishLoading && polishWarning && (
            <div className="setup-inline-state" role="status">
              <span>{polishWarning}</span>
              <button className="btn no-drag" onClick={() => void loadPolish(true)}>重新檢查</button>
            </div>
          )}
          {!polishLoading && (
            <div className="setup-model-list">
              {llm?.apple_status === 0 && (
                <div className="setup-model-row">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <strong>Apple Intelligence</strong>
                      {hardware?.recommended_llm_provider === "apple" && <span className="pill blue">這台 Mac 推薦</span>}
                      {llm.provider === "apple" && polishReady && <span className="pill green">使用中</span>}
                    </div>
                    <span>macOS 管理的端上模型・免額外下載・內容不離開這台 Mac</span>
                  </div>
                  {!(llm.provider === "apple" && polishReady) && (
                    <button className="btn no-drag" onClick={() => chooseLocalPolish("apple")}>使用</button>
                  )}
                </div>
              )}

              {recommendedBuiltin && (
                <div
                  className="setup-model-row"
                  aria-busy={Boolean(onboardingLlmDownload?.active || polishSelectionPending)}
                >
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <strong>Claro 內建整理</strong>
                      {hardware?.recommended_llm_provider === "builtin" && <span className="pill blue">這台 Mac 推薦</span>}
                      {llm?.provider === "builtin" && polishReady && <span className="pill green">使用中</span>}
                      {recommendedBuiltin.downloaded &&
                        !(llm?.provider === "builtin" && polishReady) && (
                          <span className="pill green">已下載</span>
                        )}
                    </div>
                    <span>
                      下載約 {recommendedBuiltin.size_mb >= 1024
                        ? `${(recommendedBuiltin.size_mb / 1024).toFixed(1)} GB`
                        : `${recommendedBuiltin.size_mb} MB`}
                      ・需要網路；下載中可取消，已完成的部分會保留，下次可續傳。自動調整記憶體用量，閒置後會釋放資源
                    </span>
                    {onboardingLlmDownload && (
                      <DownloadStatus
                        state={onboardingLlmDownload}
                        label="Claro 內建整理模型"
                        onCancel={() => cancelDownload("llm", recommendedBuiltin.id)}
                      />
                    )}
                  </div>
                  {!onboardingLlmDownload?.active &&
                    !(llm?.provider === "builtin" && polishReady) && (
                    <button
                      className="btn no-drag"
                      disabled={polishSelectionPending || anyDownloadActive}
                      title={anyDownloadActive ? "請先完成目前的模型下載" : undefined}
                      onClick={() => void downloadAndUseBuiltin(recommendedBuiltin)}
                    >
                      {polishSelectionPending
                        ? "正在套用…"
                        : recommendedBuiltin.downloaded
                          ? "使用"
                          : onboardingLlmDownload?.phase === "failed" ||
                              onboardingLlmDownload?.phase === "cancelled"
                            ? "重試並續傳"
                            : "下載並使用"}
                    </button>
                  )}
                </div>
              )}

              <div className="setup-model-row">
                <div className="flex-1 min-w-0">
                  <strong>原樣轉錄（建議先用）</strong>
                  <span>只做語音轉文字、個人字典與基本標點；少一個等待與失敗環節。</span>
                </div>
                {llm?.polish_mode !== "raw" && (
                  <button className="btn no-drag" onClick={chooseRaw}>使用原樣轉錄</button>
                )}
              </div>
            </div>
          )}
          {!polishLoading && (
            <div className="setup-note-row">
              <p className="setup-note">
                已經在這台 Mac 使用 Codex？你可以在設定中選用實驗校字；它會把轉錄文字送到
                OpenAI，並使用既有登入的方案額度或計費設定。這不是完成 Claro
                的必要步驟，也不會自動安裝或登入任何工具。
              </p>
              <button className="btn no-drag" type="button" onClick={onOpenSettings}>
                查看 Codex 選項
              </button>
            </div>
          )}
        </div>
      </section>

      <section className={`setup-step setup-first-dictation ${completionReady ? "ready" : "blocked"}`} aria-labelledby="setup-first-title">
        <div className="setup-step-number setup-step-next" aria-hidden="true">→</div>
        <div className="setup-step-body">
          <div className="setup-step-heading">
            <div>
              <h2 id="setup-first-title">下一步：做第一次聽寫</h2>
              {status.setup_completed ? (
                requiredReady ? (
                  <p>先前已完成首次設定；目前權限、模型與文字整理也已就緒。</p>
                ) : (
                  <p>先前已完成首次設定；目前有項目需要重新檢查，你仍可先回首頁。</p>
                )
              ) : requiredReady ? (
                firstDictationDone ? (
                  <p>已偵測到本次啟動真正完成的聽寫與貼上，可以完成設定。</p>
                ) : (
                  <>
                    <p>這是前三項檢查後的最後確認，不計入上方檢查分數；做完後回到 Claro 完成設定。</p>
                    <ol className="setup-first-dictation-list">
                      <li>切到 TextEdit 或任意可輸入文字的 App。</li>
                      <li>按住 <Hotkey combo={status.hotkey} />，說「Claro 測試一二三」，放開。</li>
                      <li>看到文字貼上後回到 Claro；沒貼上就再試一次，或到歷史紀錄複製結果。</li>
                    </ol>
                  </>
                )
              ) : (
                <p>先完成上方 3 項必要檢查；完成後回到這裡做一次貼上測試。</p>
              )}
            </div>
            <StepStatus
              ready={completionReady}
              label={
                status.setup_completed
                  ? requiredReady ? "設定已完成" : "需重新檢查"
                  : completionReady ? "已完成真實聽寫" : requiredReady ? "等待第一次貼上" : "先完成 3 項檢查"
              }
            />
          </div>
          <div className="setup-actions">
            <button className="btn-primary no-drag" disabled={!completionReady} onClick={onDone}>
              {status.setup_completed ? "回首頁" : "完成首次設定"}
            </button>
            <span className="setup-note">這份檢查不會隱藏，之後可隨時回來重新測試。</span>
          </div>
        </div>
      </section>
    </div>
  );
}
