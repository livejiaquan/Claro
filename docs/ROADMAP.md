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
- STT providers 擴充：Whisper 各尺寸（完整 large-v3 為目前精準度主力，Turbo 為速度優先）；Qwen3-ASR 0.6B／1.7B GGUF 先作離線候選，長音訊分段與真人台灣 corpus 通過前不進 production；Moonshine／SenseVoice 與 Parakeet v3 後續評測
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
- 螢幕詞彙 → provider-aware 解碼偏置：Whisper `initial_prompt`（移植 `_context_terms` 邏輯）；Qwen3-ASR 現有 transcribe.cpp port 不接受 prompt，明確降級而非假裝已套用

**DoD**：在 IDE 講出畫面上出現的技術名詞，辨識正確率可展示地優於關閉上下文時；稽核視圖能如實呈現擷取內容；關閉開關後零 AX 呼叫（log 驗證）。

## M4 — LLM 潤飾層＋CJK 極致化（差異化核心之二）

**目標**：潤飾品質對齊原型、CJK 處理超越所有競品。

範圍：
- `polish`：`Polisher` trait；providers（SPEC D3，2026-07-06 定案）：Apple Intelligence（FoundationModels，macOS 26+，Swift bridge＋弱連結）、內嵌 llama.cpp（`llama-cpp-2`/Metal，Qwen3-4B-Instruct-2507 Q4，閒置卸載）、Ollama（HTTP，進階）、OpenAI-compatible BYOK（進階）、off；Ollama/自訂已於 M2 提前上線
- 保守糾錯 prompt：移植原型 prompt＋吸收 yetone「只修不改寫」設計；防呆（長度爆炸/空輸出退回原文）
- OpenCC s2twp 終盤繁化（Rust 綁定或 vendored）
- CJK 貼上防護：偵測當前輸入法，CJK IME 時暫切 ABC → 貼上 → 還原（yetone 技巧）
- 中英混排規則（可選開關：盤古之白、全半形標點正規化）
- **CJK／STT 評測**：`meaning_preservation.json` 測 deterministic／LLM Meaning Lock；`stt_accuracy.json` 只作合成 TTS regression；另建經同意、可重跑的台灣真人語音 corpus，比較 Qwen3-ASR／完整 Whisper／Turbo 的 CER、term accuracy、誤偏置率與 p50／p95

**DoD**：文字與真人語音評測都有可重跑 baseline 並寫進 repo；BYOK 接通任一 OpenAI-compatible 端點；IME 防護在注音/拼音輸入法下實測貼上成功；潤飾延遲落在預算內（見 M5 目標）。

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
- [ ] **P0 STT Accuracy Pass（程式修正完成、產品準確率尚待真人驗收，2026-07-15）**：已確認相容 id `large-v3-mlx` 實際解析為速度版 large-v3-turbo；CLEAN Meaning Lock 刻意不猜回 STT 錯字。現在 16GB+ Apple Silicon 推薦完整 large-v3、8–12GB 推薦 large-v3 Q5、Intel 推薦 Turbo Q5；既有 Turbo 使用者有可略過的升級入口，下載仍須明確點擊。錄音啟動與 AX seed 並行、正常停止保留 200ms 尾音；Whisper prompt 改成純詞彙；個人字典新安裝預設為空並移除完全相符的舊危險預設；模型切換改為 load＋config 成功後才 commit；history 記錄模型、language、詞彙提示與正規化前收音診斷。Qwen3-ASR 0.6B／1.7B 已接入同一 transcribe.cpp family 供離線 eval，但因 256-token／長音訊分段尚未驗收，production UI、resolve 與 command 全部 fail closed。20 案重生 TTS／no-prompt Turbo 基線為 CER 5.38%、term recall 69.44%、anchor accuracy 82.43%，`zh`／`auto` 內容完全相同；失分集中 email、URL 與專有詞，且測試環境退回 CPU，不能拿 latency 當產品數字。評測支援格式等價 reference 與 fixture hash，避免 oracle prompt／舊音檔污染分數。**剩餘 release gate**：正式 `.app` 的 STT/LLM cold/warm、完整 large-v3 實機 latency/RSS、可重跑台灣真人 corpus（CER／formatting／term accuracy／Context on-off／auto-vs-zh／p50/p95）與多使用者盲測；完成前不宣稱準確率已解決或優於 Typeless。
- [ ] **P0 Trust & Release Pass（實作完成，最終原生驗收中，2026-07-15）**：輸出行為拆為 RAW／CLEAN／ORGANIZE 並與 provider 分離；CLEAN 加入否定、數字、日期時間、條件、因果／情態與過度刪減 guard，ORGANIZE 與雲端傳送各有獨立 backend consent gate；新安裝 `local_only=true`，雲端同意綁定 endpoint origin；history 可稽核 raw/final、requested/effective mode、provider、fallback 原因與 timings；onboarding 必須通過輔助使用、麥克風測試、已驗證模型與一次真實貼上，後端才會寫入 `setup_completed`；剪貼簿改為完整 NSPasteboard item/type 備份與 ownership/changeCount 防覆蓋；STT／內建 LLM 下載固定 immutable revision＋SHA-256，含嚴格續傳、原子落盤與持久校驗 marker；Swift indicator 以 Tauri sidecar 打包，release runtime 不再依賴 repo 路徑。最新程式已在 host 通過 135/135 Rust 測試（含 4 項 loopback mock 與 2 項原生 NSPasteboard）、`cargo check --all-targets`、TypeScript、Vite 與 arm64 `.app` bundle；本機 ad-hoc 簽章 bundle 已實際檢查 Home／Onboarding／Settings／History、sidecar 與 SIGTERM，無白屏、截字、阻斷 log 或 crash report。依驗證鐵律未出聲、未觸發熱鍵；乾淨 TCC、真實麥克風與跨 App paste 仍未完成，完成前不標示 P0 通過。
- [ ] **P0 外部發行驗收**：Developer ID 簽章＋Apple 公證＋App/DMG Gatekeeper、完整 Xcode 的 x86_64／universal CI、macOS 11 最低版本 clean launch／離線 RAW 聽寫、乾淨 Mac 的 TCC／首次聽寫 smoke，以及 updater 0.1.0→0.1.1 實測。這些仍屬 M6，未完成前不得宣稱可公開發布；本機 `--bundles app` 已成功，但無 Developer ID 的預設產物不具可散佈簽章，本輪 DMG Finder/AppleScript 步驟也未完成。release workflow 仍只產生經簽章、公證後供人工審核的候選檔，不會自動發布或啟用 updater。
- [x] M1 —（2026-07-05）42 項 Rust 測試綠；全自動 e2e（合成熱鍵＋TTS 過內建麥克風）驗證免持路徑逐字正確；產品級 UI（側欄＋首頁統計＋歷史＋設定，Handy/Typeless 風格經使用者確認）；overlay 沿用 prototype 膠囊。PTT 按住路徑由使用者真手驗收。
- [ ] M2 — **大部分完成；Accuracy Pass 擴充中（2026-07-15）**：App icon＋品牌；模型庫原有 6 個 Whisper 變體（含 q5 量化），逐一下載/切換/刪除、引擎熱換免重啟；Qwen3-ASR 0.6B／1.7B GGUF 的 registry／family dispatch 只供離線 eval，通過長音訊與真人 corpus 前不可在 production 啟用。切換 transaction 先載入候選並寫 config，失敗回載原模型。**AI 潤飾提前上線**（M4 前移）：off/Ollama/自訂 OpenAI-compatible，API key 進鑰匙圈，保守糾錯 prompt＋防呆（mock server 三路徑驗證），設定頁一鍵測試；CJK 標點正規化；single-instance 防雙開；SIGTERM/退出的 Metal teardown 修復（零 crash report）；權限授予後熱鍵免重啟重試。**STT 閒置卸載（2026-07-07）**：與 LLM 同款 5 分鐘看門狗（`spawn_stt_idle_watcher`，只在 IDLE＋engine try_lock 才動手），錄音開始即背景回載（載入 1.4s 被說話時間蓋掉）、`process_session` 同步補載保底；實測（正式 bundle、`CLARO_STT_IDLE_SECS=20` 縮短門檻）RSS 1586MB→31MB，SIGTERM 乾淨退出零 crash，SPEC §12 待機 <300MB 達標。低記憶體路徑維持 STT/LLM 互斥且潤飾後立即卸載 LLM。**剩**：Qwen 長音訊分段／本機端到端與真人 corpus；SenseVoice/Moonshine（ONNX 引擎）。
- [ ] M3 — **核心與 P0 隱私面已前移完成（2026-07-12）**：`context.rs` AX 上下文擷取（NSWorkspace 前景 app→AXFocusedWindow/AXFocusedUIElement/游標前後文/可見文字 BFS，預算 500/1200/400 同 prototype；AXSecureTextField 永不讀；全域開關 `context_enabled`，內容永不落盤）；詞彙抽取對 Whisper 進 `initial_prompt`，完整 bounded Context 只在 ORGANIZE 進潤飾 prompt；Qwen3-ASR 現有 port 不支援 prompt，不能把 deterministic 字典替換當成同等解碼偏置。內建敏感 App denylist、本次 Context 稽核／清除、session-bound capture 與 salted App/視窗/焦點 metadata hash 已完成，可阻擋可辨識的目標切換。個人字典 UI（config `dictionary` 鍵、設定頁 CRUD、詞彙同步偏置辨識）。**教訓：system-wide AX 元素的 AXFocusedApplication 會回 -25204，必須走 NSWorkspace 拿 pid**。**剩**：使用者自訂 per-app 停用規則、真實跨 App/同 App 焦點矩陣，以及 Whisper Context on/off 的 term accuracy／誤偏置率。
- [ ] M4 — 前置研究完成（2026-07-06）：潤飾 runtime 選型定案（SPEC D3 改版），競品證據與本機 FoundationModels 實測見 `docs/research/llm-polish-runtime.md`。**Apple Intelligence provider 已上線（2026-07-06，前移）**：Swift bridge（DynamicGenerationSchema guided generation——CLT 環境編不了 @Generable macro，動態 schema 等價且實測是抑制「把聽寫當指令回答」的關鍵）＋ build.rs 編譯/弱連結（SDK 無 FoundationModels 時自動改編 stub）＋可用性偵測（未開啟/不支援/下載中）進設定 UI。潤飾 provider 擴充：Ollama/LM Studio 本機服務偵測（模型清單下拉＋重新偵測）、自訂 API 六組雲端 preset。P0 Meaning Lock 禁止 LLM 取代任何文字／英數 token，專有詞校正只走 STT Context 偏置與使用者明確建立的字典。**內嵌 llama.cpp 已上線（2026-07-06）**：`llm.rs`（llama-cpp-2/Metal、greedy、閒置 5 分鐘 watcher卸載、先 drop 再載）；模型目錄 Qwen3-4B-Instruct-2507 Q4（推薦）＋Gemma 3 4B，UI 內下載/切換/刪除；與 whisper.cpp 的兩份 ggml 同程序共存實測無衝突；**教訓：CJK 一字多 token，必須整串收完再 detokenize，逐 token 轉會丟字**。本次 Accuracy Pass 發現少量 builtin cold-start 樣本退回原文；現已加入 8 秒 total cap、生成最多 6 秒且不排隊，阻塞式 cold load 超時後保留 warm model但本段不再追加生成。**剩**：正式 `.app` cold/warm p50/p95、把高記憶體 profile 的 prewarm 策略真正接入、完整 CJK 評測集與 IME 貼上防護
- [ ] M5
- [ ] M6
