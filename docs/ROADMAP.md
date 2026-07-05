# Claro Roadmap

原則：每個里程碑結束時 **能 build、能實際跑、有測試、有可展示的成果**。完成後更新本文件進度與 CLAUDE.md。

現況基線：Python/MLX 原型（`prototype/`，原 repo 根目錄）已能端到端聽寫，是行為參考實作與 CJK 處理的真值來源；Tauri 版以功能對齊它為第一目標，再超越。

## M1 — 端到端最小骨架（Tauri）

**目標**：全新 Tauri app 在 macOS 上完成「按住熱鍵 → 錄音 → 本地 Whisper 轉錄 → 貼到游標處」的完整迴圈，離線可用。

範圍：
- Tauri v2 scaffold（Rust workspace + React/TS/Tailwind 前端）＋ CI（GitHub Actions macOS build + test）
- `hotkey`：全域熱鍵用 `handy-keys`（press/release 事件，SPEC D7），push-to-talk 長按＋快點進 toggle 免持；移植 Python 版狀態機（含 0.35s tap 閾值、session 序號、Esc 取消語意）為純 Rust 模組
- `audio`：cpal 以裝置原生取樣率擷取 → rubato 重採樣 16kHz mono f32、RMS 電平、5 分鐘上限、太短/靜音防呆
- `stt`：`SttEngine` trait ＋ `transcribe-cpp`（whisper.cpp 家族，Metal，SPEC D5）首個 provider，預設 large-v3-turbo；模型首次使用經同意才下載
- `inject`：剪貼簿備份 → 寫入 → 合成 Cmd+V → 還原
- overlay：**沿用 prototype 的 `mic_indicator`**（Rust spawn＋同一 socket 協議，動畫不動，SPEC D10）；Tauri 原生 overlay 延至 M5
- `settings`/`history`：讀寫 `~/.claro/config.json` 與 `history.jsonl`（與原型格式相容，0600）

**DoD**：關網路（模型已下載）能在任意 app 聽寫中文並貼上；`cargo test` 全綠（狀態機、音訊收集、config/history 相容性）；冷啟動（不含模型載入）< 3s；手動 smoke checklist（附錄 A）通過。

## M2 — 模型管理與設定 UI

**目標**：使用者能在設定裡自由換 STT 模型，體驗接近 Handy 的模型管理。

範圍：
- 模型註冊表＋下載管理器（進度、續傳、校驗、明確同意、磁碟用量顯示）
- STT providers 擴充：Whisper 各尺寸；Moonshine（zh 支援）；Parakeet v3（英/歐語快速選項）
- 設定視窗：General / Models / Dictionary / History 分頁；熱鍵自訂
- 個人字典搬進設定 UI＋持久化（原型是硬編碼＋一次性 CLI 參數）
- 記憶體策略：閒置 N 分鐘卸載模型、依機器 RAM 推薦預設模型

**DoD**：全程 UI 完成換模型→下載→聽寫，不碰終端機；字典 CRUD 生效並持久化；閒置卸載可觀測（Activity Monitor 驗證）。

## M3 — 上下文引擎（差異化核心之一）

**目標**：把原型的 AX 上下文擷取移植到 Rust 並做成「可見、可控、可稽核」。

範圍：
- `context`：`ContextProvider` trait；macOS AX 實作（objc2）：前景 app、視窗標題、焦點元件游標前後文、選取文字、可見文字 BFS（預算制，移植原型的 500/1200/400 預算）
- 隱私防護：`AXSecureTextField` 永不擷取；預設排除清單（密碼管理器等）；per-app 停用；全域開關
- 上下文稽核視圖：顯示「上一次聽寫抓到了什麼」，一鍵清除
- 螢幕詞彙 → Whisper `initial_prompt` 偏置（移植 `_context_terms` 邏輯）

**DoD**：在 IDE 講出畫面上出現的技術名詞，辨識正確率可展示地優於關閉上下文時；稽核視圖能如實呈現擷取內容；關閉開關後零 AX 呼叫（log 驗證）。

## M4 — LLM 潤飾層＋CJK 極致化（差異化核心之二）

**目標**：潤飾品質對齊原型、CJK 處理超越所有競品。

範圍：
- `polish`：`Polisher` trait；providers：內嵌 llama.cpp（預設，Qwen 系列 GGUF）、Ollama（HTTP）、OpenAI-compatible BYOK（Base URL / Key / Model 可設）、off
- 保守糾錯 prompt：移植原型 prompt＋吸收 yetone「只修不改寫」設計；防呆（長度爆炸/空輸出退回原文）
- OpenCC s2twp 終盤繁化（Rust 綁定或 vendored）
- CJK 貼上防護：偵測當前輸入法，CJK IME 時暫切 ABC → 貼上 → 還原（yetone 技巧）
- 中英混排規則（可選開關：盤古之白、全半形標點正規化）
- **CJK 評測集**：golden jsonl（口語原文 → 期望輸出），`cargo test` 跑 deterministic 部分，eval script 跑 LLM 部分並出分數

**DoD**：評測集分數有 baseline 並寫進 repo；BYOK 接通任一 OpenAI-compatible 端點；IME 防護在注音/拼音輸入法下實測貼上成功；潤飾延遲落在預算內（見 M5 目標）。

## M5 — 體驗打磨與效能驗收

**目標**：達到「原生體感」與明確的延遲/記憶體預算。

範圍：
- 首次啟動引導：麥克風→輔助使用權限逐步帶領（含偵測與重試）、模型下載同意、測試聽寫
- overlay 打磨：動畫、免持提示、錯誤訊息人話化
- 延遲儀表：每段聽寫記錄各階段耗時進 history；`--debug` 面板
- 效能驗收（M1 Mac 基準）：短句（≤5s 音訊）放開熱鍵→出字，raw ≤ 1.5s、含本地 LLM 潤飾 ≤ 4s（p50）；待機（模型卸載後）RSS < 300MB
- 邊角：多顯示器 overlay 位置、輸入裝置熱插拔、麥克風被占用

**DoD**：效能數字達標並記錄在 repo（量測腳本可重跑）；新機器從 dmg 到第一次成功聽寫 < 5 分鐘（實測計時）。

## M6 — 公開發布 v0.1

**目標**：可下載、可信任、可自動更新的成品。

範圍：
- codesign + notarize；tauri-plugin-updater 自動更新；GitHub Releases CI 一鍵出版
- 使用者 README 重寫（賣點、隱私立場、安裝、FAQ）＋一頁式 landing（賣點、隱私、下載、平台）
- 隱私文件：資料流向圖、雲端路徑清單（預設全關）
- 版本策略與 changelog

**DoD**：非開發者在乾淨 Mac 下載 dmg 完成安裝與聽寫；自動更新從 v0.1.0→v0.1.1 實測成功；landing 上線。

## 之後（不承諾順序）

- Windows 移植(架構已預留:trait 層+Tauri 跨平台;上下文改 UIA、注入改 SendInput)
- 截圖＋本地 VLM 進階上下文（xuiltul 思路，預設關）
- tone per app（Typeless 的強項）、語音指令編輯選取文字
- MCP server / IDE 深度整合
- 串流轉錄降低長句延遲

## 附錄 A — Smoke checklist（每里程碑跑）

1. 按住熱鍵說「測試一二三」放開 → 文字貼到 TextEdit
2. 快點熱鍵 → 免持模式 → 再按停止 → 貼上
3. 錄音中按 Esc → 取消、內容進 history
4. 處理中按 Esc → 不貼上、內容進 history（status=cancelled）
5. 靜音錄 2 秒 → error 態、不貼上
6. 中英混講「用 PyTorch 跑 training」→ 術語正確
7. 剪貼簿原內容在貼上後還原

## 進度

- [x] Phase 0：稽核＋競品研究＋SPEC/ROADMAP/CLAUDE.md（2026-07-05）
- [x] M1 —（2026-07-05）42 項 Rust 測試綠；全自動 e2e（合成熱鍵＋TTS 過內建麥克風）驗證免持路徑逐字正確；產品級 UI（側欄＋首頁統計＋歷史＋設定，Handy/Typeless 風格經使用者確認）；overlay 沿用 prototype 膠囊。PTT 按住路徑由使用者真手驗收。
- [ ] M2 — **大部分完成（2026-07-05）**：App icon＋品牌；模型庫 6 個 whisper 變體（含 q5 量化），逐一下載/切換/刪除、引擎熱換免重啟；**AI 潤飾提前上線**（M4 前移）：off/Ollama/自訂 OpenAI-compatible，API key 進鑰匙圈，保守糾錯 prompt＋防呆（mock server 三路徑驗證），設定頁一鍵測試；CJK 標點正規化；single-instance 防雙開；SIGTERM/退出的 Metal teardown 修復（零 crash report）；權限授予後熱鍵免重啟重試。**剩**：SenseVoice/Moonshine（ONNX 引擎）、依 RAM 推薦預設、閒置卸載、字典 UI。
- [ ] M3
- [ ] M4
- [ ] M5
- [ ] M6
