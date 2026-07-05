# 本地 LLM 潤飾 runtime 選型研究

日期：2026-07-06。方法：Handy／VoiceInk shallow clone 後純本地閱讀（Codex 執行，引用均查證過檔案路徑）＋本機 FoundationModels 實測（Swift 探測程式）＋網路查證。回答的問題：**潤飾層要求使用者另裝 Ollama 合理嗎？Mac 上最省資源的加速方式是什麼？**

## 結論（已寫入 SPEC D3）

1. **不要求使用者裝 Ollama**——沒有任何競品這樣做。Ollama 降級為進階選項。
2. 「零安裝」潤飾走雙軌：
   - **macOS 26+**：Apple Intelligence 端上模型（FoundationModels API）——零下載、零常駐 RAM（模型在系統程序、系統排程 ANE/GPU）、離線、免費、繁中支援（26.1 起）。
   - **macOS < 26 或未啟用 Apple Intelligence**：內嵌 llama.cpp（`llama-cpp-2`，Metal），下載 Qwen3-4B-Instruct-2507 Q4 GGUF（~2.4GB，經同意），閒置卸載。
3. 常駐資源機制沿用 Handy 實證模式：用時載入＋閒置 watcher 卸載（預設 5 分鐘）＋先 drop 舊引擎再載新。

## 競品證據（2026-07-06 HEAD）

### Handy

- **有** LLM 後處理：providers 含 OpenAI/Z.AI/OpenRouter/Anthropic/Groq/Cerebras/**Apple Intelligence（macOS ARM64）**/Bedrock/Custom（`src-tauri/src/settings.rs:558-644`）。Custom 預設 `http://localhost:11434/v1`——Ollama 只是「自訂端點的預設值」，不是獨立 provider，更不是必裝品。
- **Apple Intelligence 是 in-process Swift bridge**（`src-tauri/swift/apple_intelligence.swift`）：`SystemLanguageModel.default` 檢查 availability → `LanguageModelSession(model:instructions:)` → 先試 structured output（`CleanedTranscript`）失敗退 plain `respond(to:)`。不可用時回 None → 退回原轉錄（永不擋聽寫）。
- **弱連結技巧**（`build.rs:365-376,465-469`）：SDK 沒有 FoundationModels 時編 stub；有時 weak-link——app 在舊 macOS 照常啟動。移植時照抄此模式。
- **沒有內嵌 llama.cpp/MLX**。零安裝本地潤飾的答案就是 FoundationModels。
- STT 記憶體機制：錄音開始才載模型（`actions.rs:436-453`）；idle watcher 每 10s 檢查（`managers/transcription.rs:274-335`），逾時選項 Never/立即/2/5/10/15分/1小時，**預設 5 分鐘**（`settings.rs:133-145`）；卸載＝置 None 靠 drop；載新前先 drop 舊。LLM 側無常駐（HTTP per-request；Apple session per-request）。

### VoiceInk

- providers：多家雲端＋Ollama＋Local CLI＋Custom（`Services/AIEnhancement/AIService.swift:4-19`）。**未用 FoundationModels**（全 repo 零命中）。**也沒有內嵌 LLM**。
- 定位：onboarding 預設潤飾 provider 是 **Groq（雲端）**，Ollama 不在 onboarding 清單，被放在設定的「Local & CLI Providers」當可選項（`Views/Onboarding/OnboardingCoordinator.swift:70-83,220-246`）。
- 記憶體策略與 Handy 相反：預設 **prewarm**（啟動 3s 後與喚醒後跑 sample 轉錄暖機，`Services/ModelPrewarmService.swift`）——用常駐 RAM 換延遲。Claro 不採：待機 RSS < 300MB 是我們的驗收指標，卸載後下次錄音開始並行預載即可（Metal mmap 載入 1-2s，與 STT 並行不增加體感延遲）。
- 有趣備案：Local CLI provider（把潤飾丟給任意 shell 命令）。點子留檔，v0.1 不做。

## FoundationModels 本機實測

探測程式（Swift 6.4，`swiftc -parse-as-library` 直編可跑，程式碼將隨 M4 實作進 repo）：

- 本機 M4 / macOS 26.6：框架正常、availability 回報 `unavailable(appleIntelligenceNotEnabled)`——**使用者未在系統設定開啟 Apple Intelligence 是真實會發生的狀態**，UI 必須偵測並引導（深連結系統設定），不可靜默失敗。
- 語言：Apple Intelligence 於 macOS 26.1 起支援繁體中文（共 16 語）。
- 端上模型 ~3B 級、上下文約 4K token（26.4 起有查詢 API）；對「保守糾錯」任務足夠，品質待啟用後用 CJK 評測集實測。
- macOS 27 起有系統內建 `fm` CLI 與 Python SDK，佐證 CLI 程序可呼叫；我們用 in-process bridge（Handy 模式），不 spawn helper。

## Rust 推理引擎選型（內嵌 fallback）

| 方案 | 評估 |
|---|---|
| **`llama-cpp-2`（選定）** | llama.cpp 官方推薦的 Rust 綁定，活躍維護（2026-06 仍更新）；Metal feature；與 STT 的 whisper.cpp 同 ggml 技術棧，建置鏈（cmake+Metal）已在專案裡踩熟；Ollama 底層同引擎，品質等價 |
| MLX（mlx-rs） | Apple Silicon 上小模型推理快 ~10-20%，但 Rust 綁定不成熟、生態小；prototype 的 MLX 是 Python 專屬。不值得 FFI 風險 |
| candle / mistral.rs | 純 Rust 可取，但 GGUF 覆蓋與 Metal 成熟度不如 llama.cpp；社群模型全是 GGUF 優先 |

「Mac 最好的加速」的正解：**系統自己的端上模型（ANE，零成本給我們用）> llama.cpp Metal ≈ MLX（差距一到兩成）**。前者當預設、llama.cpp 當 fallback，MLX 不需要。

## 模型選型（內嵌 fallback 用）

- **預設 Qwen3-4B-Instruct-2507 Q4_K_M（~2.4GB 檔案、推理 RSS ~3GB）**：多語（含繁中）指令跟隨強、非 thinking 版（無 `<think>` 洩漏問題）、GGUF 生態完整。
- prototype 實測教訓維持有效：1.5B 級會洩漏提示詞、亂加列表——不做 4B 以下預設。8GB 機器的推薦路徑是 Apple Intelligence（零 RAM）而非更小模型。
- 延遲估算（M 系 Metal、4B Q4）：prefill ~500 tok 提示詞極快，生成 40-60 tok/s、輸出 ~100 tok ≈ 2-3s——落在 SPEC §12「含潤飾 ≤4s p50」預算內；Apple 路徑更快。
- 峰值 RAM：whisper large-v3-turbo（~1.6GB）＋ Qwen3-4B（~3GB）同時駐留 ≈ 4.6GB，16GB 機器安全；閒置卸載讓待機回落 < 300MB。

## 常駐資源機制（設計定案）

1. **潤飾引擎（內嵌路徑）與 STT 共用同一套生命週期**：錄音開始時並行預載（與 STT 載入疊加，不增加體感延遲）→ 閒置 watcher（10s 巡檢，逾時卸載，預設 5 分鐘、可設）→ 換模型先 drop 再載。
2. **Apple 路徑零管理**：session per-request，模型駐留是系統的事，app RSS 不動。
3. 潤飾失敗/不可用永遠退回原文（既有合約不變）。
