import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { DownloadProgress, MicLevel, ModelInfo, Status } from "../types";
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
  refresh,
  onToast,
  onDone,
}: {
  status: Status;
  mic: MicLevel;
  progress: DownloadProgress | null;
  refresh: () => void;
  onToast: (message: string) => void;
  onDone: () => void;
}) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelsLoading, setModelsLoading] = useState(true);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [micChecked, setMicChecked] = useState(false);

  const loadModels = useCallback(() => {
    setModelsLoading(true);
    setModelsError(null);
    invoke<ModelInfo[]>("list_models")
      .then(setModels)
      .catch((reason) => setModelsError(String(reason)))
      .finally(() => setModelsLoading(false));
  }, []);

  useEffect(() => loadModels(), [loadModels, progress?.model_id, progress?.done, status.model_id]);

  useEffect(() => {
    if (mic.active && mic.level > 0.01) setMicChecked(true);
  }, [mic.active, mic.level]);

  const permissionsReady = status.accessibility && status.hotkey_active;
  const requiredReady = permissionsReady && micChecked && status.model_present;
  const checkedCount = Number(permissionsReady) + Number(micChecked) + Number(status.model_present);

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
          <p>完成權限、麥克風與語音模型三項檢查，再照最後一步做第一次聽寫。</p>
        </div>
        <div className="setup-score" aria-label={`已通過 ${checkedCount} 個檢查，共 3 個`}>
          <strong>{checkedCount}/3</strong>
          <span>本次檢查</span>
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

      <section className={`setup-step ${micChecked ? "complete" : ""}`} aria-labelledby="setup-mic-title">
        <div className="setup-step-number" aria-hidden="true">2</div>
        <div className="setup-step-body">
          <div className="setup-step-heading">
            <div>
              <h2 id="setup-mic-title">測試麥克風</h2>
              <p>{mic.active ? (mic.level > 0.01 ? "收音正常，可以繼續說幾句確認。" : "正在聆聽；請用平常音量說幾句話。") : "這只檢查音量，不會辨識、保存或傳送內容。"}</p>
            </div>
            <StepStatus ready={micChecked} label={micChecked ? "本次已通過" : "尚未測試"} />
          </div>
          <div className="setup-mic-controls">
            <select
              className="select no-drag"
              aria-label="麥克風輸入裝置"
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
            <div className="setup-level" aria-label={mic.active ? `目前麥克風音量 ${Math.round(mic.level * 100)}%` : "麥克風測試未啟動"}>
              <LevelBar level={mic.level} active={mic.active} />
            </div>
          </div>
        </div>
      </section>

      <section className={`setup-step ${status.model_present ? "complete" : ""}`} aria-labelledby="setup-model-title">
        <div className="setup-step-number" aria-hidden="true">3</div>
        <div className="setup-step-body">
          <div className="setup-step-heading">
            <div>
              <h2 id="setup-model-title">準備語音模型</h2>
              <p>模型下載只會在你按下「下載」後開始；已下載的模型可離線辨識。</p>
            </div>
            <StepStatus ready={status.model_present} label={status.model_present ? status.model_label : "尚未下載"} />
          </div>

          {modelsLoading && <div className="setup-inline-state" role="status">正在檢查模型…</div>}
          {modelsError && (
            <div className="config-error" role="alert">
              <span>無法取得模型清單：{modelsError}</span>
              <button className="btn no-drag" onClick={loadModels}>重試</button>
            </div>
          )}
          {!modelsLoading && !modelsError && (
            <div className="setup-model-list">
              {models.map((model) => {
                const downloading = progress?.model_id === model.id && !progress.done;
                const percent = downloading && progress.total_mb
                  ? Math.min(100, (progress.downloaded_mb / progress.total_mb) * 100)
                  : null;
                return (
                  <div className="setup-model-row" key={model.id}>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <strong>{model.label}</strong>
                        {model.recommended && <span className="pill blue">推薦</span>}
                        {model.active && <span className="pill green">使用中</span>}
                      </div>
                      <span>{model.desc}・{model.size_mb >= 1024 ? `${(model.size_mb / 1024).toFixed(1)} GB` : `${model.size_mb} MB`}</span>
                      {downloading && (
                        <div className="mt-2">
                          <div className="progress-track" role="progressbar" aria-label={`下載 ${model.label}`} aria-valuenow={percent ?? undefined}>
                            <div className="progress-fill" style={{ width: percent === null ? "30%" : `${percent}%` }} />
                          </div>
                          <small>{progress.downloaded_mb}/{progress.total_mb ?? "?"} MB</small>
                        </div>
                      )}
                    </div>
                    {!model.downloaded && !downloading && (
                      <button className="btn no-drag" onClick={() => run("download_model", { id: model.id })}>
                        下載
                      </button>
                    )}
                    {model.downloaded && !model.active && (
                      <button className="btn no-drag" onClick={() => run("set_model", { id: model.id }, `已切換為 ${model.label}`)}>
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

      <section className={`setup-step setup-first-dictation ${requiredReady ? "ready" : "blocked"}`} aria-labelledby="setup-first-title">
        <div className="setup-step-number" aria-hidden="true">4</div>
        <div className="setup-step-body">
          <div className="setup-step-heading">
            <div>
              <h2 id="setup-first-title">做第一次聽寫</h2>
              {requiredReady ? (
                <p>回到任意文字輸入框，按住 <Hotkey combo={status.hotkey} /> 說「Claro 測試一二三」，放開後確認文字貼上。</p>
              ) : (
                <p>完成前三項檢查後，這裡會顯示第一次聽寫操作。</p>
              )}
            </div>
            <StepStatus ready={requiredReady} label={requiredReady ? "可以開始" : "等待前面步驟"} />
          </div>
          <div className="setup-actions">
            <button className="btn-primary no-drag" disabled={!requiredReady} onClick={onDone}>
              回首頁，開始聽寫
            </button>
            <span className="setup-note">這份檢查不會隱藏，之後可隨時回來重新測試。</span>
          </div>
        </div>
      </section>
    </div>
  );
}
