import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import type { DictEntry, DownloadProgress, LlmConfig, MicLevel, ModelInfo, Status } from "../types";
import { Hotkey, LevelBar, Row, Section } from "../ui";

/** 自訂 API 的常用服務 preset：帶入 Base URL 與建議模型（都走 OpenAI 相容介面） */
const CUSTOM_PRESETS = [
  { id: "openai", label: "OpenAI", baseUrl: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  { id: "anthropic", label: "Anthropic", baseUrl: "https://api.anthropic.com/v1", model: "claude-haiku-4-5" },
  { id: "groq", label: "Groq", baseUrl: "https://api.groq.com/openai/v1", model: "llama-3.3-70b-versatile" },
  { id: "deepseek", label: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", model: "deepseek-chat" },
  { id: "gemini", label: "Google Gemini", baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai", model: "gemini-2.5-flash" },
  { id: "openrouter", label: "OpenRouter", baseUrl: "https://openrouter.ai/api/v1", model: "openai/gpt-4o-mini" },
];

export default function Settings({
  status,
  mic,
  progress,
  llmProgress,
  refresh,
  onToast,
}: {
  status: Status;
  mic: MicLevel;
  progress: DownloadProgress | null;
  llmProgress: DownloadProgress | null;
  refresh: () => void;
  onToast: (msg: string) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [llm, setLlm] = useState<LlmConfig | null>(null);
  const [keyDraft, setKeyDraft] = useState("");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);
  // 清單/錯誤都綁 provider：偵測回應晚到時不能污染已切換的 provider（race）
  const [localModels, setLocalModels] = useState<{ provider: string; models: string[] } | null>(null);
  const [localModelsErr, setLocalModelsErr] = useState<{ provider: string; msg: string } | null>(null);

  const [builtinLlms, setBuiltinLlms] = useState<ModelInfo[]>([]);
  const [dict, setDict] = useState<DictEntry[] | null>(null);
  const [dictDraft, setDictDraft] = useState<DictEntry>({ from: "", to: "" });

  const loadModels = () => invoke<ModelInfo[]>("list_models").then(setModels).catch(() => {});
  const loadLlm = () => invoke<LlmConfig>("get_llm_config").then(setLlm).catch(() => {});
  const loadBuiltinLlms = () =>
    invoke<ModelInfo[]>("list_builtin_llms").then(setBuiltinLlms).catch(() => {});
  const loadDict = () => invoke<DictEntry[]>("get_dictionary").then(setDict).catch(() => {});

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

  useEffect(() => {
    loadModels();
    loadLlm();
    loadBuiltinLlms();
    loadDict();
  }, []);
  // 內建 LLM 下載進度改變清單狀態
  useEffect(() => {
    loadBuiltinLlms();
  }, [llmProgress?.model_id, llmProgress?.done, llm?.model, llm?.provider]);

  // 選到本機服務時偵測其模型清單
  useEffect(() => {
    if (llm && (llm.provider === "ollama" || llm.provider === "lmstudio")) {
      loadLocalModels(llm.provider);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [llm?.provider]);
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

  // 連續操作（選 preset 後馬上改欄位）的兩道防線：
  // 1) llmRef 即時反映最新值——閉包裡的舊 llm 不會把欄位倒退
  // 2) 寫入請求串行化——舊請求不會晚到蓋掉新請求
  const llmRef = useRef<LlmConfig | null>(null);
  llmRef.current = llm;
  const llmSaveQueue = useRef<Promise<unknown>>(Promise.resolve());
  const saveLlm = (next: Partial<LlmConfig>) => {
    const cur = llmRef.current;
    if (!cur) return;
    const merged = { ...cur, ...next };
    llmRef.current = merged;
    setLlm(merged);
    setTestResult(null);
    llmSaveQueue.current = llmSaveQueue.current.then(() =>
      invoke("set_llm_config", {
        provider: merged.provider,
        model: merged.model,
        baseUrl: merged.base_url,
      }).catch((e) => setError(String(e))),
    );
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
        <Row label="快捷鍵" sub="按住說話、放開出字；快點一下＝免持模式。改完立即生效。">
          <div className="flex items-center gap-3">
            <Hotkey combo={status.hotkey} />
            <select
              className="select no-drag"
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
          sub="聽寫後做保守糾錯（去填充詞、修同音錯字、補標點）。文字只送到你選的引擎，預設關閉。"
        >
          <select
            className="select no-drag"
            value={llm?.provider ?? "off"}
            onChange={(e) => saveLlm({ provider: e.target.value })}
          >
            <option value="off">關閉（純轉錄）</option>
            <option value="apple" disabled={!!llm && [1, 4].includes(llm.apple_status)}>
              Apple Intelligence（免安裝{llm?.apple_status === 0 ? "・推薦" : ""}）
            </option>
            <option value="builtin">內建模型（免安裝・高精度）</option>
            <option value="ollama">Ollama（本機服務）</option>
            <option value="lmstudio">LM Studio（本機服務）</option>
            <option value="custom">自訂 API（OpenAI 相容）</option>
          </select>
        </Row>

        {llm?.provider === "builtin" && (
          <>
            {builtinLlms.map((m) => {
              const isDownloading = llmProgress?.model_id === m.id && !llmProgress.done;
              const pct =
                isDownloading && llmProgress!.total_mb
                  ? Math.min(100, (llmProgress!.downloaded_mb / llmProgress!.total_mb) * 100)
                  : null;
              return (
                <div className="row" key={m.id} style={{ alignItems: "flex-start" }}>
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
                    </div>
                    <div className="row-sub">
                      {m.desc}・{m.size_mb >= 1024 ? `${(m.size_mb / 1024).toFixed(1)} GB` : `${m.size_mb} MB`}
                      ・閒置 5 分鐘自動釋放記憶體
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
                          {llmProgress!.downloaded_mb}/{llmProgress!.total_mb ?? "?"} MB
                        </div>
                      </div>
                    )}
                  </div>
                  <div className="flex items-center gap-2 pt-0.5">
                    {m.downloaded && !m.active && (
                      <>
                        <button className="btn no-drag" onClick={() => saveLlm({ model: m.id })}>
                          使用
                        </button>
                        <button
                          className="btn no-drag"
                          style={{ color: "var(--red)" }}
                          onClick={() =>
                            invoke("delete_builtin_llm", { id: m.id })
                              .then(() => {
                                loadBuiltinLlms();
                                onToast("已刪除");
                              })
                              .catch((e) => setError(String(e)))
                          }
                        >
                          刪除
                        </button>
                      </>
                    )}
                    {!m.downloaded && !isDownloading && (
                      <button
                        className="btn no-drag"
                        onClick={() =>
                          invoke("download_builtin_llm", { id: m.id }).catch((e) =>
                            setError(String(e)),
                          )
                        }
                      >
                        下載
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
                    ? "未偵測到 Ollama——先安裝並啟動（brew install ollama），再按「重新偵測」。"
                    : "未偵測到 LM Studio——先啟動它的本機伺服器（Developer → Start Server），再按「重新偵測」。"
                  : detected && detected.length === 0
                    ? llm.provider === "ollama"
                      ? "服務在跑但沒有模型——先 ollama pull qwen3:4b。"
                      : "服務在跑但沒有載入模型——在 LM Studio 載入一個模型。"
                    : "建議 4B 級以上的指令模型，例如 qwen3:4b。"
              }
            >
              <div className="flex items-center gap-2">
                {detected && detected.length > 0 ? (
                  <select
                    className="select no-drag"
                    style={{ minWidth: 200 }}
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
                    value={llm.model}
                    placeholder="qwen3:4b"
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
                style={{ minWidth: 260 }}
                value={llm.base_url}
                placeholder="https://api.openai.com/v1"
                onChange={(e) => saveLlm({ base_url: e.target.value })}
              />
            </Row>
            <Row label="模型名稱" sub="該服務的模型 ID">
              <input
                className="select no-drag"
                style={{ minWidth: 200 }}
                value={llm.model}
                placeholder="gpt-4o-mini"
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

        {llm && llm.provider !== "off" && (
          <>
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

      <Section title="螢幕上下文">
        <Row
          label="讀取畫面詞彙"
          sub="聽寫時讀取目前視窗的內容（app 名稱、標題、游標周邊文字），把出現過的術語餵給辨識與潤飾——中英混雜的正確率關鍵。內容只在記憶體、永不儲存；密碼欄永不讀取。"
        >
          <select
            className="select no-drag"
            value={status.context_enabled ? "on" : "off"}
            onChange={(e) =>
              invoke("set_context_enabled", { enabled: e.target.value === "on" })
                .then(refresh)
                .catch((err) => setError(String(err)))
            }
          >
            <option value="on">開啟（建議）</option>
            <option value="off">關閉</option>
          </select>
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
                style={{ minWidth: 140 }}
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
                style={{ minWidth: 140 }}
                value={e.to}
                onChange={(ev) => {
                  const next = [...(dict ?? [])];
                  next[i] = { ...next[i], to: ev.target.value };
                  saveDict(next);
                }}
              />
            </div>
            <button
              className="btn no-drag"
              style={{ color: "var(--red)" }}
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
              style={{ minWidth: 140 }}
              placeholder="誤認詞（如 GBT）"
              value={dictDraft.from}
              onChange={(e) => setDictDraft({ ...dictDraft, from: e.target.value })}
            />
            <span style={{ color: "var(--faint)" }}>→</span>
            <input
              className="select no-drag"
              style={{ minWidth: 140 }}
              placeholder="正確詞（如 GPT）"
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
