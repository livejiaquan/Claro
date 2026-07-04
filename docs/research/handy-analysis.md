# Handy（cjpais/Handy）原始碼分析

日期：2026-07-05。方法：shallow clone 後純本地閱讀（Codex 執行，引用均查證過檔案路徑）。Handy 是我們 Tauri 架構的主要參考實作。

## 專案切分

- Rust backend（`src-tauri/src/lib.rs`）：`actions`、`audio_toolkit`、`clipboard`、`commands`、`managers`、`overlay`、`settings`、`shortcut`、`transcription_coordinator`、`tray`。backend 擁有錄音、VAD、模型管理、STT、貼上注入、tray、overlay、updater。
- React 前端：onboarding／settings／模型選擇 UI，事件驅動顯示狀態；overlay 有獨立 React entry（`src/overlay/main.tsx`）。

## 全域熱鍵與 PTT

- 非 Linux 預設 `handy-keys` crate（輪詢 `HotkeyState`，發 press/release）；Linux 用 `tauri-plugin-global-shortcut`（`ShortcutState::Pressed`）。
- PTT 在 `transcription_coordinator.rs`：press 從 Idle 啟動、release 只停對應的 binding；toggle 模式只吃 press。**單一執行緒 coordinator 序列化 press/release/debounce**——與我們 prototype 的 dispatcher 結論相同。
- macOS 權限：前端 onboarding 用 `tauri-plugin-macos-permissions-api`，權限過了才初始化 Enigo/shortcut。

## 音訊管線

- `cpal` 擷取，多聲道 downmix mono f32；**用裝置原生取樣率再以 `rubato::FftFixedIn` 重採樣到 16kHz/30ms frame**（不強迫硬體改率）。
- VAD：`vad-rs`（Silero v4 onnx），`SmoothedVad` 包 prefill/onset/hangover，每 session reset。
- 停止時 drain 到 EndOfStream、flush resampler、回傳 `Vec<f32>`。

## STT 引擎

- **不用 whisper-rs**：Whisper/GGUF 家族用 `transcribe-cpp`；ONNX 家族用 `transcribe-rs`（**Parakeet／Moonshine／SenseVoice／GigaAM／Canary**）。
- 加速：macOS Metal（whisper）；ONNX 經 `transcribe_rs::accel::set_ort_accelerator`。
- 抽象是 `EngineType` enum + `LoadedEngine` holder，非 trait object（我們仍走 trait，較利於 BYOK/cloud provider）。

## 模型管理

- `ModelInfo` + `ModelSource`（Url/HuggingFace/Local）+ 內建 `catalog.json`。
- 下載：`.partial` 暫存、HTTP Range 續傳、進度事件、SHA-256 校驗、`.extracting` 目錄暫存原子化；HF 走 `hf-hub` 共享快取、可取消。
- 模型存 app data `models/`。

## Overlay

- macOS 用 `tauri-nspanel`：NSPanel、`PanelLevel::Status`、non-activating、透明、無陰影、all-spaces/fullscreen auxiliary。
- **沒有做滑鼠穿透（click-through）**——overlay 仍可被滑鼠打中（我們要補上 `ignoresMouseEvents`，這是體驗差異點）。

## 文字注入

- 剪貼簿備份 → 寫入 → `enigo` 送平台貼上鍵 → 等待 → 還原。**還原只等固定 50ms，慢的目標 app 會 race**（我們 prototype 用 300ms；保留較長延遲並文件化）。
- CJK 處理只有 OpenCC 簡繁輸出轉換，**無 IME 輸入源切換**。

## 打包與更新

- `tauri.conf.json`：updater artifacts、hardened runtime、entitlements、macOS 簽章；updater endpoint 指 GitHub Releases `latest.json`。
- `tauri_plugin_updater` + 前端 `check()`/`downloadAndInstall()`。
- Release workflow：手動開 draft release → matrix build（macOS Intel/ARM、Linux、Windows）→ `tauri-apps/tauri-action` 帶 Apple notarization 與 updater 簽章金鑰。

## 記憶體策略

- 錄音開始即背景載模型＋並行預載 VAD；換模型先 drop 舊引擎再載新（避免雙駐留）；idle watcher 每 10s 檢查、逾時卸載；麥克風 stream 可 always-on 或 on-demand（30s 後懶關閉）。

## 借鏡 / 避開

**借**：coordinator 單執行緒化；原生率擷取＋重採樣；SmoothedVad；下載器（續傳/校驗/暫存）；先 drop 再載；idle 卸載 watcher。
**避**：雙熱鍵 backend 的複雜度；50ms 剪貼簿還原 race；overlay 無穿透；新舊模型註冊表並存的 UI 過濾規則。
