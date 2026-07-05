import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { DownloadProgress, LlmConfig, MicLevel, ModelInfo, Status } from "../types";
import { Hotkey, LevelBar, Row, Section } from "../ui";

export default function Settings({
  status,
  mic,
  progress,
  refresh,
  onToast,
}: {
  status: Status;
  mic: MicLevel;
  progress: DownloadProgress | null;
  refresh: () => void;
  onToast: (msg: string) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [llm, setLlm] = useState<LlmConfig | null>(null);
  const [keyDraft, setKeyDraft] = useState("");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);

  const loadModels = () => invoke<ModelInfo[]>("list_models").then(setModels).catch(() => {});
  const loadLlm = () => invoke<LlmConfig>("get_llm_config").then(setLlm).catch(() => {});

  useEffect(() => {
    loadModels();
    loadLlm();
  }, []);
  // 下載進度事件會改變模型清單狀態
  useEffect(() => {
    loadModels();
  }, [progress?.model_id, progress?.done, status.model_id]);

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

  const saveLlm = (next: Partial<LlmConfig>) => {
    if (!llm) return;
    const merged = { ...llm, ...next };
    setLlm(merged);
    setTestResult(null);
    invoke("set_llm_config", {
      provider: merged.provider,
      model: merged.model,
      baseUrl: merged.base_url,
    }).catch((e) => setError(String(e)));
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

  const runTest = () => {
    setTesting(true);
    setTestResult(null);
    setError(null);
    invoke<string>("test_polish")
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

  return (
    <div className="page-in">
      <h1 className="text-[24px] font-bold tracking-tight mb-6">設定</h1>

      <Section title="聽寫">
        <Row label="快捷鍵" sub="按住說話、放開出字；快點一下＝免持模式">
          <Hotkey />
        </Row>
      </Section>

      <Section title="聲音">
        <Row label="輸入裝置" sub={!status.input_device ? "跟隨系統預設" : "已固定，不隨系統切換"}>
          <select
            className="select no-drag"
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
        {models.map((m) => {
          const isDownloading = progress?.model_id === m.id && !progress.done;
          const pct =
            isDownloading && progress!.total_mb
              ? Math.min(100, (progress!.downloaded_mb / progress!.total_mb) * 100)
              : null;
          return (
            <div className="row" key={m.id} style={{ alignItems: "flex-start" }}>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="row-label">{m.label}</span>
                  {m.recommended && <span className="pill blue">推薦</span>}
                  {m.active && (
                    <span className="pill green">
                      <span className="dot" />
                      使用中
                    </span>
                  )}
                </div>
                <div className="row-sub">
                  {m.desc}・{m.size_mb >= 1024 ? `${(m.size_mb / 1024).toFixed(1)} GB` : `${m.size_mb} MB`}
                </div>
                {isDownloading && (
                  <div className="mt-2 w-[240px]">
                    <div className="progress-track">
                      <div
                        className="progress-fill"
                        style={{ width: pct !== null ? `${pct}%` : "30%" }}
                      />
                    </div>
                    <div className="text-[11px] mt-1" style={{ color: "var(--muted)" }}>
                      {progress!.downloaded_mb}/{progress!.total_mb ?? "?"} MB
                    </div>
                  </div>
                )}
              </div>
              <div className="flex items-center gap-2 pt-0.5">
                {m.downloaded && !m.active && (
                  <>
                    <button className="btn no-drag" onClick={() => call("set_model", { id: m.id })}>
                      使用
                    </button>
                    <button
                      className="btn no-drag"
                      style={{ color: "var(--red)" }}
                      onClick={() => call("delete_model", { id: m.id }, () => onToast("已刪除"))}
                    >
                      刪除
                    </button>
                  </>
                )}
                {!m.downloaded && !isDownloading && (
                  <button className="btn no-drag" onClick={() => call("download_model", { id: m.id })}>
                    下載
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </Section>

      <Section title="AI 潤飾">
        <Row
          label="潤飾引擎"
          sub="聽寫後用本地或自選的 LLM 做保守糾錯（去填充詞、修同音錯字、補標點）。文字只送到你選的端點。"
        >
          <select
            className="select no-drag"
            value={llm?.provider ?? "off"}
            onChange={(e) => saveLlm({ provider: e.target.value })}
          >
            <option value="off">關閉（純轉錄）</option>
            <option value="ollama">Ollama（本機）</option>
            <option value="custom">自訂 API（OpenAI 相容）</option>
          </select>
        </Row>
        {llm && llm.provider !== "off" && (
          <>
            <Row
              label="模型名稱"
              sub={llm.provider === "ollama" ? "例如 qwen3:4b、llama3.2:3b（需先 ollama pull）" : "例如 gpt-4o-mini、deepseek-chat"}
            >
              <input
                className="select no-drag"
                style={{ minWidth: 200 }}
                value={llm.model}
                placeholder={llm.provider === "ollama" ? "qwen3:4b" : "gpt-4o-mini"}
                onChange={(e) => saveLlm({ model: e.target.value })}
              />
            </Row>
            {llm.provider === "custom" && (
              <>
                <Row label="Base URL" sub="OpenAI 相容端點，例如 https://api.openai.com/v1">
                  <input
                    className="select no-drag"
                    style={{ minWidth: 260 }}
                    value={llm.base_url}
                    placeholder="https://api.openai.com/v1"
                    onChange={(e) => saveLlm({ base_url: e.target.value })}
                  />
                </Row>
                <Row
                  label="API Key"
                  sub={llm.has_key ? "已存入 macOS 鑰匙圈（不會寫進任何檔案）" : "存入 macOS 鑰匙圈"}
                >
                  <div className="flex items-center gap-2">
                    <input
                      className="select no-drag"
                      style={{ minWidth: 180 }}
                      type="password"
                      value={keyDraft}
                      placeholder={llm.has_key ? "••••••••" : "sk-…"}
                      onChange={(e) => setKeyDraft(e.target.value)}
                    />
                    <button className="btn no-drag" onClick={saveKey} disabled={!keyDraft}>
                      儲存
                    </button>
                  </div>
                </Row>
              </>
            )}
            <Row label="測試潤飾" sub="送一句「嗯我們用 hyTorch，那個，跑訓練」看看效果">
              <button className="btn no-drag" onClick={runTest} disabled={testing}>
                {testing ? "測試中…" : "測試"}
              </button>
            </Row>
            {testResult && (
              <div className="row" style={{ background: "var(--green-soft)" }}>
                <div className="text-[13px]" style={{ color: "var(--green)" }}>
                  ✓ {testResult}
                </div>
              </div>
            )}
          </>
        )}
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
              <button className="btn no-drag" onClick={retryHotkey}>
                重試
              </button>
            )}
            <span className={`pill ${permsOk ? "green" : "amber"}`}>
              <span className="dot" />
              {permsOk ? "已授權" : "需要授權"}
            </span>
          </div>
        </Row>
      </Section>

      <Section title="關於">
        <Row label="Claro" sub="全程本地處理；只有你自己開啟的潤飾端點會收到文字。">
          <span className="text-[12.5px]" style={{ color: "var(--faint)" }}>
            v0.1.0
          </span>
        </Row>
      </Section>

      {error && (
        <p className="text-[13px] -mt-2" style={{ color: "var(--red)" }}>
          {error}
        </p>
      )}
    </div>
  );
}
