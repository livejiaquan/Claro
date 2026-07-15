import { readFile, writeFile } from "node:fs/promises";

const indexPath = new URL("../dist/index.html", import.meta.url);
const qaPath = new URL("../dist/qa.html", import.meta.url);
// Relative asset paths work both under `vite preview` (/qa.html) and Vite dev
// (/dist/qa.html), so the durable QA harness is not coupled to one server mode.
const index = (await readFile(indexPath, "utf8")).replace(
  /(src|href)="\/assets\//g,
  '$1="./assets/',
);

const bridge = String.raw`
<script>
(() => {
  const scenario = new URLSearchParams(location.search).get("scenario") || "new-8gb";
  const failDownloads = scenario === "download-error";
  const micTimesOut = scenario === "mic-timeout";
  let nextCallback = 1;
  let nextEvent = 1;
  let micGeneration = 0;
  let activeDownload = null;
  const callbacks = new Map();
  const listeners = new Map();

  const baseStatus = {
    model_id: "large-v3-turbo",
    model_label: "Large v3 Turbo",
    model_present: false,
    model_approx_mb: 1549,
    accessibility: false,
    hotkey_active: false,
    input_device: null,
    default_input: "MacBook Pro Microphone",
    input_devices: ["MacBook Pro Microphone", "Studio Display Microphone"],
    dictation_state: "idle",
    context_enabled: true,
    hotkey: "Opt+Shift+C",
    setup_completed: false,
    successful_pastes_this_launch: 0,
    history_enabled: true,
    mic_test_passed_this_launch: false,
    polish_mode: "clean",
    effective_mode: "raw",
    llm_provider: "off",
    local_only: true,
    execution_location: "none",
    endpoint_origin: null,
    blocked_reason: "provider_missing",
  };

  const state = {
    status: { ...baseStatus },
    appleStatus: 2,
    llm: {
      provider: "off", model: "", base_url: "", has_key: false, apple_status: 2,
      polish_mode: "clean", effective_mode: "raw", local_only: true,
      organize_consent_valid: false, cloud_consent_valid: true,
      execution_location: "none", endpoint_origin: null, blocked_reason: "provider_missing",
    },
    hardware: {
      architecture: "aarch64", memory_gb: 8, tier: "compact", low_memory_mode: true,
      keep_models_warm: false, recommended_stt: "large-v3-q5_0",
      recommended_llm_provider: "builtin", recommended_llm_model: "qwen3-4b-instruct-2507",
      reason: "8 GB Apple Silicon：使用量化語音模型，轉錄後先釋放 STT 再載入 Claro 內建整理模型",
    },
    models: [
      { id: "qwen3-asr-1.7b-q8_0", label: "Qwen3-ASR 1.7B（預覽）", desc: "新一代短篇多語辨識；台灣真人語料仍在驗證", size_mb: 2084, recommended: false, available: false, preview: true, downloaded: false, active: false, downloading: false },
      { id: "qwen3-asr-0.6b-q8_0", label: "Qwen3-ASR 0.6B（預覽）", desc: "輕量短篇多語辨識；台灣真人語料仍在驗證", size_mb: 811, recommended: false, available: false, preview: true, downloaded: false, active: false, downloading: false },
      { id: "large-v3-turbo", label: "Large v3 Turbo", desc: "Whisper 速度與品質的平衡選擇，支援詞彙提示", size_mb: 1549, recommended: false, available: true, preview: false, downloaded: false, active: true, downloading: false },
      { id: "large-v3-turbo-q5_0", label: "Large v3 Turbo（量化）", desc: "體積只有 1/3、更省記憶體，品質略降", size_mb: 547, recommended: false, available: true, preview: false, downloaded: false, active: false, downloading: false },
      { id: "large-v3", label: "Large v3", desc: "Whisper 最高辨識品質，支援專有名詞提示", size_mb: 2951, recommended: false, available: true, preview: false, downloaded: false, active: false, downloading: false },
      { id: "large-v3-q5_0", label: "Large v3（量化）", desc: "接近最高品質的省空間選擇", size_mb: 1031, recommended: true, available: true, preview: false, downloaded: false, active: false, downloading: false },
      { id: "small", label: "Small", desc: "輕量、舊機友善", size_mb: 465, recommended: false, available: true, preview: false, downloaded: false, active: false, downloading: false },
    ],
    builtin: [
      { id: "qwen3-4b-instruct-2507", label: "Qwen3 4B Instruct", desc: "中英混排最穩，繁中品質佳", size_mb: 2382, recommended: true, available: true, preview: false, downloaded: false, active: false, downloading: false },
    ],
    contextAudit: null,
    pendingResult: null,
    history: [],
    dictionary: [{ from: "克拉洛", to: "Claro" }],
  };

  const modelById = (id) => state.models.find((model) => model.id === id);
  const recommendModel = (id) => {
    state.models.forEach((model) => { model.recommended = model.id === id; });
    state.hardware.recommended_stt = id;
  };

  if (scenario === "new-16gb-apple") {
    Object.assign(state.status, {
      model_id: "large-v3-turbo", model_label: "Large v3 Turbo", model_present: true,
      model_approx_mb: 1549, accessibility: true, hotkey_active: true,
      mic_test_passed_this_launch: true,
    });
    modelById("large-v3-turbo").downloaded = true;
    recommendModel("large-v3");
    state.appleStatus = 0;
    state.llm.apple_status = 0;
    Object.assign(state.hardware, {
      memory_gb: 16, tier: "balanced", low_memory_mode: false, keep_models_warm: true,
      recommended_stt: "large-v3", recommended_llm_provider: "apple",
      reason: "16 GB Apple Silicon：語音模型採 balanced 配置，文字整理交由 macOS 端上模型",
    });
  }

  if (scenario === "accuracy-upgrade" || scenario === "accuracy-no-downgrade" || scenario === "ready-organize" || scenario === "history-errors" || scenario === "context-audit" || scenario === "history-off-pending") {
    Object.assign(state.status, {
      model_present: true, accessibility: true, hotkey_active: true, setup_completed: true,
      successful_pastes_this_launch: 3, mic_test_passed_this_launch: true,
      polish_mode: "organize", effective_mode: "organize", llm_provider: "builtin",
      execution_location: "on_device", blocked_reason: null,
    });
    Object.assign(state.llm, {
      provider: "builtin", model: "qwen3-4b-instruct-2507", polish_mode: "organize",
      effective_mode: "organize", execution_location: "on_device", blocked_reason: null,
      organize_consent_valid: true,
    });
    modelById("large-v3-turbo").downloaded = true;
    recommendModel("large-v3");
    Object.assign(state.hardware, {
      memory_gb: 16, tier: "balanced", low_memory_mode: false, keep_models_warm: true,
      recommended_stt: "large-v3",
      reason: "16 GB Apple Silicon：完整語音模型以辨識精準度為優先",
    });
    state.builtin[0].downloaded = true;
    state.builtin[0].active = true;
    state.contextAudit = {
      app_id: "com.tinyspeck.slackmacgap", app_name: "Slack", surface: "message",
      text: "App: Slack\nWindow: Claro product\nEditing(around cursor): 明天不上線，原因是測試尚未完成\nVisible: PyTorch | release candidate",
    };
    state.history = [
      { ts: "2026-07-12T09:42:00+08:00", duration_s: 8.4, raw: "先說結論明天不上線原因是測試尚未完成", text: "原因是測試尚未完成。先說結論，明天不上線。", status: "pasted", timings: { stt_ms: 680, polish_ms: 2200 }, polish: { mode: "organize", provider: "builtin", changed: true, outcome: "changed" } },
      { ts: "2026-07-12T09:35:00+08:00", duration_s: 3.1, raw: "用 PyTorch 跑 training", text: "用 PyTorch 跑 training。", status: "pasted", timings: { stt_ms: 710, polish_ms: 900 }, polish: { mode: "clean", provider: "builtin", changed: true, outcome: "changed" } },
    ];
  }

  // 模擬先前版本已下載、後來因驗收不足而暫停開放的預覽模型。
  if (scenario === "accuracy-upgrade") {
    modelById("qwen3-asr-0.6b-q8_0").downloaded = true;
  }

  if (scenario === "accuracy-no-downgrade") {
    recommendModel("large-v3-turbo-q5_0");
    Object.assign(state.hardware, {
      architecture: "x86_64", memory_gb: 16, tier: "compact", low_memory_mode: true,
      keep_models_warm: false, recommended_stt: "large-v3-turbo-q5_0",
      reason: "16 GB Intel：使用省記憶體語音模型",
    });
  }

  if (scenario === "history-errors") {
    state.history.push(
      { ts: "2026-07-12T09:30:00+08:00", duration_s: 0.2, raw: "", text: "", status: "too_short" },
      { ts: "2026-07-12T09:28:00+08:00", duration_s: 2.0, raw: "", text: "", status: "silent" },
      { ts: "2026-07-12T09:25:00+08:00", duration_s: 3.4, raw: "", text: "", status: "stt_failed" },
      { ts: "2026-07-12T09:20:00+08:00", duration_s: 4.0, raw: "切換後的結果", text: "切換後的結果", status: "focus_changed", polish: { mode: "clean", provider: "builtin", changed: false, outcome: "unchanged" } },
    );
  }

  if (scenario === "history-off-pending") {
    state.status.history_enabled = false;
    state.pendingResult = {
      raw: "切換視窗後的結果",
      text: "切換視窗後的結果。",
      reason: "focus_changed",
    };
  }

  function emit(event, payload) {
    for (const id of listeners.get(event) || []) {
      callbacks.get(id)?.({ event, id: 0, payload });
    }
  }

  function syncLlm() {
    Object.assign(state.status, {
      polish_mode: state.llm.polish_mode,
      effective_mode: state.llm.effective_mode,
      llm_provider: state.llm.provider,
      local_only: state.llm.local_only,
      execution_location: state.llm.execution_location,
      endpoint_origin: state.llm.endpoint_origin,
      blocked_reason: state.llm.blocked_reason,
    });
  }

  async function invoke(command, args = {}) {
    if (command === "plugin:event|listen") {
      const id = args.handler;
      if (!listeners.has(args.event)) listeners.set(args.event, new Set());
      listeners.get(args.event).add(id);
      return nextEvent++;
    }
    if (command === "plugin:event|unlisten") return null;
    if (command.startsWith("plugin:window|") || command.startsWith("plugin:opener|")) return null;
    switch (command) {
      case "get_status": return structuredClone(state.status);
      case "get_hardware_profile": return structuredClone(state.hardware);
      case "list_models": return structuredClone(state.models);
      case "list_builtin_llms": return structuredClone(state.builtin);
      case "get_llm_config": return structuredClone(state.llm);
      case "get_dictionary": return structuredClone(state.dictionary);
      case "get_context_audit": return structuredClone(state.contextAudit);
      case "get_pending_result": return structuredClone(state.pendingResult);
      case "get_history": return structuredClone(state.history);
      case "list_provider_models": return [];
      case "retry_hotkey": state.status.accessibility = true; state.status.hotkey_active = true; return true;
      case "mic_test_start":
        micGeneration += 1;
        {
        const generation = micGeneration;
        state.status.mic_test_passed_this_launch = !micTimesOut;
        setTimeout(
          () => emit("mic-level", {
            active: false,
            level: 0,
            generation,
            passed: !micTimesOut,
            timed_out: micTimesOut,
          }),
          30,
        );
        return null;
        }
      case "mic_test_stop":
        micGeneration += 1;
        emit("mic-level", { active: false, level: 0, generation: micGeneration, passed: state.status.mic_test_passed_this_launch, timed_out: false });
        return null;
      case "set_input_device":
        micGeneration += 1;
        state.status.input_device = args.name || null;
        state.status.mic_test_passed_this_launch = false;
        emit("mic-level", { active: false, level: 0, generation: micGeneration, passed: false, timed_out: false });
        return null;
      case "set_model":
        {
        const selected = state.models.find((model) => model.id === args.id);
        if (!selected?.available) throw new Error("此模型仍在驗證中，尚未開放使用");
        state.models.forEach((model) => { model.active = model.id === args.id; });
        state.status.model_id = args.id;
        state.status.model_label = selected?.label || args.id;
        state.status.model_approx_mb = selected?.size_mb || 0;
        state.status.model_present = selected?.downloaded || false;
        }
        return null;
      case "download_model": {
        if (activeDownload) throw new Error("已有 " + activeDownload + " 模型正在下載");
        const model = state.models.find((item) => item.id === args.id);
        if (!model?.available) throw new Error("此模型仍在驗證中，尚未開放下載");
        activeDownload = "STT";
        setTimeout(() => emit("model-download", { model_id: args.id, downloaded_mb: 128, total_mb: model.size_mb, done: false, downloaded: false, activation_status: "none", error: null }), 20);
        setTimeout(() => {
          if (failDownloads) {
            activeDownload = null;
            emit("model-download", { model_id: args.id, downloaded_mb: 128, total_mb: model.size_mb, done: false, downloaded: false, activation_status: "none", error: "網路連線中斷" });
            return;
          }
          model.downloaded = true;
          if (args.activate) {
            state.models.forEach((item) => { item.active = item.id === model.id; });
            state.status.model_id = model.id;
            state.status.model_label = model.label;
            state.status.model_approx_mb = model.size_mb;
          }
          activeDownload = null;
          state.status.model_present = model.active;
          emit("model-download", { model_id: args.id, downloaded_mb: model.size_mb, total_mb: model.size_mb, done: true, downloaded: true, activation_status: "none", error: null });
        }, 120);
        return null;
      }
      case "delete_model": {
        const model = state.models.find((item) => item.id === args.id);
        if (model?.active) throw new Error("使用中的模型不能刪除，先切換到其他模型");
        if (model) model.downloaded = false;
        return null;
      }
      case "set_llm_config":
        Object.assign(state.llm, { provider: args.provider, model: args.model, base_url: args.baseUrl });
        if (["apple", "builtin"].includes(args.provider)) state.llm.execution_location = "on_device";
        else if (args.provider === "off") state.llm.execution_location = "none";
        else if (args.provider === "custom") {
          const url = new URL(args.baseUrl || "https://invalid.example");
          const loopback = ["localhost", "127.0.0.1", "::1"].includes(url.hostname);
          state.llm.execution_location = loopback ? "local_service" : "cloud";
          state.llm.endpoint_origin = loopback ? null : url.origin;
        } else state.llm.execution_location = "local_service";
        state.llm.blocked_reason = args.provider === "builtin" && !state.builtin[0].downloaded
          ? "model_missing"
          : args.provider === "off"
            ? "provider_missing"
            : state.llm.execution_location === "cloud" && state.llm.local_only
              ? "local_only"
              : null;
        state.llm.effective_mode = state.llm.blocked_reason ? "raw" : state.llm.polish_mode;
        syncLlm();
        return structuredClone(state.llm);
      case "set_polish_mode":
        if (args.mode === "organize" && !state.llm.organize_consent_valid && !args.confirmed) {
          throw new Error("ORGANIZE 需要明確同意");
        }
        state.llm.polish_mode = args.mode;
        state.llm.effective_mode = args.mode === "raw" || state.llm.blocked_reason ? "raw" : args.mode;
        state.llm.organize_consent_valid ||= args.mode === "organize" && args.confirmed;
        syncLlm();
        return structuredClone(state.llm);
      case "download_builtin_llm":
        if (activeDownload) throw new Error("已有 " + activeDownload + " 模型正在下載");
        activeDownload = "LLM";
        setTimeout(() => emit("llm-model-download", { model_id: args.id, downloaded_mb: 256, total_mb: 2382, done: false, downloaded: false, activation_status: "none", error: null }), 20);
        setTimeout(() => {
          if (failDownloads) {
            activeDownload = null;
            emit("llm-model-download", { model_id: args.id, downloaded_mb: 256, total_mb: 2382, done: false, downloaded: false, activation_status: "none", error: "網路連線中斷" });
            return;
          }
          state.builtin[0].downloaded = true;
          activeDownload = null;
          state.builtin[0].active = true;
          state.llm.blocked_reason = null;
          state.llm.effective_mode = state.llm.polish_mode;
          syncLlm();
          emit("llm-model-download", { model_id: args.id, downloaded_mb: 2382, total_mb: 2382, done: true, downloaded: true, activation_status: "none", error: null });
        }, 120);
        return null;
      case "set_local_only":
        if (!args.enabled && !args.confirmed) throw new Error("關閉僅限本機需要明確同意");
        state.llm.local_only = args.enabled;
        state.llm.blocked_reason = state.llm.execution_location === "cloud" && args.enabled ? "local_only" : null;
        state.llm.effective_mode = state.llm.blocked_reason ? "raw" : state.llm.polish_mode;
        syncLlm();
        return structuredClone(state.llm);
      case "set_context_enabled": state.status.context_enabled = args.enabled; if (!args.enabled) state.contextAudit = null; return null;
      case "clear_context_audit": state.contextAudit = null; return null;
      case "clear_pending_result": state.pendingResult = null; return null;
      case "set_history_enabled": state.status.history_enabled = args.enabled; return null;
      case "clear_history": state.history = []; state.pendingResult = null; return null;
      case "set_dictionary": state.dictionary = args.entries; return null;
      case "complete_setup":
        if (!state.status.accessibility || !state.status.hotkey_active) throw new Error("輔助使用權限或快捷鍵尚未就緒");
        if (!state.status.model_present) throw new Error("語音模型尚未下載或完整性驗證失敗");
        if (!state.status.setup_completed && !state.status.mic_test_passed_this_launch) throw new Error("本次尚未完成麥克風收音測試");
        if (state.llm.polish_mode !== "raw" && (state.llm.effective_mode === "raw" || state.llm.blocked_reason)) throw new Error("文字整理尚未就緒");
        if (!state.status.setup_completed && !state.status.successful_pastes_this_launch) throw new Error("請先完成一次真正的聽寫與貼上");
        state.status.setup_completed = true;
        return null;
      case "test_polish": return "我們用 PyTorch 跑訓練。";
      default: return null;
    }
  }

  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
    invoke,
    transformCallback(callback, once = false) {
      const id = nextCallback++;
      callbacks.set(id, (...args) => {
        callback?.(...args);
        if (once) callbacks.delete(id);
      });
      return id;
    },
    unregisterCallback(id) { callbacks.delete(id); },
    convertFileSrc(path) { return path; },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__CLARO_QA__ = { scenario, state, emit };
})();
</script>`;

const qa = index
  .replace("<head>", `<head>${bridge}`)
  .replace("<title>Claro</title>", "<title>Claro User Journey QA</title>");

await writeFile(qaPath, qa);
console.log(`Prepared ${qaPath.pathname}`);
