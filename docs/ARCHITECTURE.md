# Claro 架構與原理（維護者手冊）

> 目標讀者：要改 Claro 程式碼的人。讀完你應該知道每一段資料流經過哪些檔案、
> 為什麼是這個設計、以及哪些地方有前人踩過的地雷。
> 產品層規格在 `docs/SPEC.md`（決策記錄 D1–D10）；本文講「程式怎麼動」。

## 0. 一句話架構

Tauri v2 桌面 app：**Rust 後端**擁有所有重活（熱鍵、錄音、STT、潤飾、貼上），
**React 前端**只是設定與統計的皮，兩者用 Tauri command/event 溝通。
所有推理（whisper.cpp、llama.cpp）**in-process、Metal 加速、全本地**。

```
┌─ 使用者按住熱鍵 ─────────────────────────────────────────────────┐
│                                                                  │
│ hotkey.rs ──事件──▶ pipeline.rs dispatcher（單執行緒序列化）      │
│                        │                                         │
│                        ├─▶ state_machine.rs（純邏輯，決定動作）   │
│                        │                                         │
│                        ▼                                         │
│              audio.rs 錄音（cpal 原生取樣率 → rubato 重採 16k）   │
│                        │  放開熱鍵                                │
│                        ▼                                         │
│              context.rs AX 擷取螢幕上下文（±2s 超時，可關）       │
│                        ▼                                         │
│              stt/whisper.rs 轉錄（initial_prompt = 字典+上下文詞彙）│
│                        ▼                                         │
│              textproc.rs 清理 → OpenCC 繁化 → 標點正規化 → 字典    │
│                        ▼                                         │
│              polish.rs 潤飾（apple/builtin/ollama/lmstudio/custom）│
│                        ▼  （失敗一律退回原文）                     │
│              inject.rs 剪貼簿備份 → Cmd+V → 還原                  │
│                        ▼                                         │
│              history.rs 寫入 ~/.claro/history.jsonl               │
└──────────────────────────────────────────────────────────────────┘
```

## 1. 執行緒模型（最重要的一節）

**鐵律：熱鍵事件必須由單一 dispatcher 執行緒序列化後餵狀態機。**
keyDown/keyUp 若在不同執行緒亂序處理，錄音會卡死（prototype 實測踩過）。

| 執行緒 | 檔案 | 職責 |
|---|---|---|
| main | `lib.rs` | Tauri event loop；所有重初始化在 `setup()` 內（single-instance 檢查之後） |
| hotkey | `hotkey.rs` | 專屬執行緒持有 `HotkeyManager`（內含 `!Sync` 的 mpsc receiver），try_recv 輪詢事件＋控制命令 |
| dispatcher | `pipeline.rs::run_dispatcher` | 唯一消費熱鍵事件的地方；呼叫狀態機、觸發動作 |
| 錄音 level poller | `pipeline.rs::start_recording` | 30ms 間隔送音量給 overlay；兼錄音上限 watchdog |
| processing | `pipeline.rs::start_processing` | 每個 session 一條：上下文→STT→潤飾→貼上 |
| context capture | `pipeline.rs` 內 spawn | AX 呼叫放子執行緒，主流程 `recv_timeout(2s)`——上下文永遠不准卡住聽寫 |
| llm idle watcher | `llm.rs` | 每 10s 檢查內建模型閒置，5 分鐘未用即卸載 |
| stt idle watcher | `pipeline.rs::spawn_stt_idle_watcher` | 同款政策卸載 whisper（large-v3-turbo 常駐 1.6GB 是待機大頭）；只在狀態機 IDLE＋engine `try_lock` 成功才動手；回載走「錄音開始即背景預載」（1.4s 載入被說話時間蓋掉）＋`process_session` 同步補載保底；門檻可用 `CLARO_STT_IDLE_SECS` 覆寫（驗證用） |
| accessibility poller | `lib.rs::init_core` | 未授權時每 2s 輪詢，授權後自動啟用熱鍵 |

跨執行緒溝通一律 `crossbeam_channel`；共享狀態一律 `Mutex`（`Core` 結構）。

## 2. 狀態機（`state_machine.rs`）

純邏輯、無 IO、可完整單元測試。狀態：

```
IDLE ──keyDown──▶ HOLD ──放開<0.35s──▶ HANDSFREE（免持）
                   │                        │
                   └─放開≥0.35s─┐           └─keyDown─┐
                                ▼                     ▼
                            PROCESSING ◀──────────────┘
                                │ 完成（session 序號吻合才作數）
                                ▼
                              IDLE
```

- **session 序號**：每次進 PROCESSING 遞增；完成回報帶序號，過期結果直接作廢。
  這防止「上一句還在轉錄、使用者又講了一句」的結果錯位。
- **Esc 語意**：錄音中＝取消（背景仍轉錄一份進 history 可救回）；處理中＝結果只進 history 不貼上。
- Esc 攔截是**條件式**的：只在非 IDLE 時註冊全域 Esc（`hotkey.rs Ctl::ArmEsc/DisarmEsc`），
  IDLE 時完全放行，不干擾系統。
- 移植時必須逐案通過 `prototype/tests/test_state_machine.py` 的對應案例（已對齊）。

## 3. 音訊（`audio.rs`）

- cpal 用**裝置原生取樣率**擷取（不強迫硬體改率——某些裝置會拒絕），
  rubato `FftFixedIn` 重採樣到 16kHz mono f32（whisper 的輸入格式）。
- RMS 平滑：`level = level*0.6 + rms*0.4`，餵 overlay 波形。
- 防呆：錄音 <0.3s 或 RMS <0.01（靜音）直接標 error/silent，不進轉錄。
- 錄音上限 5 分鐘（`MAX_RECORDING_S`），由 level poller 兼任 watchdog 送 `ForceStop`。

## 4. STT（`stt/`）

- 抽象：`SttEngine` trait（`id/load/unload/transcribe`）＋ `SttRequest`（audio/language/initial_prompt）。
- 實作：`whisper.rs` 走 `transcribe-cpp`（whisper.cpp 家族，Metal）。
- 模型目錄：`registry.rs` 宣告式清單（id/label/url/size/recommended），
  檔案放 `~/Library/Application Support/Claro/models/`。加模型＝加一行。
- `condition_on_prev_tokens: false`：降低幻覺循環（prototype 實測）。
- **initial_prompt 是準確度關鍵**：`build_initial_prompt(terms)` 產
  「以下是繁體中文為主、夾雜英文技術術語的口語內容，可能提到：X、Y。」
  terms = 個人字典的正確詞 ＋ 螢幕上下文抽出的英文/技術詞（上限 15）。
  這讓 Whisper 在解碼階段就偏向正確拼法，而不是事後修。

## 5. 螢幕上下文（`context.rs`）

macOS Accessibility API 抓：前景 app 名稱、視窗標題、焦點元件游標前後文（±500 字）、
選取文字（200 字）、視窗可見文字 BFS（1200 字/400 節點，只收
StaticText/TextField/TextArea/Heading 內容性角色）。

**踩過的坑與防護（改這個檔前必讀）：**
- system-wide AX 元素查 `AXFocusedApplication` 會回 -25204（CannotComplete）——
  **必須走 NSWorkspace 拿前景 pid** 再 `AXUIElementCreateApplication`。
- AX 呼叫可能無限卡住：開機第一次 capture 時對 system-wide 元素設
  `AXUIElementSetMessagingTimeout(0.25s)`（全程序生效）；BFS 另有 800ms 牆鐘預算；
  pipeline 端再包一層 2 秒 `recv_timeout`。三層保險，聽寫永不被上下文卡死。
- CF 記憶體：AX 回傳都是 create rule（+1 retain），用 `Owned` RAII 包，drop 時 CFRelease。
  CFArray 的元素歸陣列持有，要留用必須自己 CFRetain。
- **隱私鐵律**：焦點元件 role/subrole 是 `AXSecureTextField` 整塊跳過；
  BFS 收文字前也檢查 subrole（未聚焦的密碼欄可能 role 是普通 TextField）。
  上下文只存在記憶體、傳給潤飾後即丟，永不落盤、永不進 history。
- 大文件防護：讀字串屬性前先 `CFStringGetLength`，超過 20 萬字放棄，不做整串複製。

## 6. 潤飾層（`polish.rs` ＋ `llm.rs` ＋ `swift/apple_intelligence.swift`）

`Polisher` enum，`from_settings()` 依 config 建構：

| provider | 路徑 | 原理 |
|---|---|---|
| `apple` | Swift bridge 靜態連結 | macOS 26 FoundationModels 系統端上模型（~3B），零下載、不占本程序記憶體 |
| `builtin` | `llm.rs`（llama-cpp-2） | in-process llama.cpp，GGUF 模型（Qwen3-4B 推薦），Metal 全層 offload |
| `ollama`/`lmstudio` | HTTP | OpenAI-compatible chat/completions（localhost:11434 / :1234） |
| `custom` | HTTP | 任意 OpenAI-compatible 端點，API key 存 macOS Keychain（`security` CLI），不進 JSON |
| `off` | — | 出廠預設。**不悄悄外連是產品鐵律** |

**合約（不可違背）**：`polish()` 失敗 → 呼叫端一律退回原文。聽寫不可因 LLM 掛掉而失敗。
共用防呆 `guard()`：空輸出、或長度 > max(3×原文, 原文+200) → 退回原文
（LLM「開始自由發揮」的訊號，實測攔下 Apple 端上模型把聽寫當指令回答的殘餘案例）。

### Apple bridge 細節（`swift/apple_intelligence.swift` + `build.rs`）

- build.rs 把 Swift 檔編成 .o → libtool 打包 → 靜態連進 Rust binary；
  SDK 沒有 FoundationModels 時自動改編 stub（同名符號，回報「系統過舊」），
  外加 `-weak_framework FoundationModels` 讓 binary 在舊 macOS 能啟動。
- **必須用 `xcrun libtool`**：PATH 上可能有 GNU libtool（conda/brew）不認 `-static`。
- **用 `DynamicGenerationSchema` 不用 `@Generable` macro**：macro plugin 只隨完整
  Xcode 發佈，CommandLineTools 環境編不過；且 guided generation（強制模型只能回
  「清理後的轉錄」欄位）是抑制 3B 模型「把聽寫內容當指令回答」的關鍵——實測
  plain prompt 會生成整段程式碼或直接回答問題。
- C ABI：`claro_apple_ai_status()`（0=可用…4=系統過舊）＋
  `claro_apple_ai_polish(instructions, prompt) -> JSON 字串`（strdup 配 `claro_apple_ai_free`）。
  Swift 端 `Task.detached + semaphore.wait(timeout: 30s)`——逾時回錯誤，
  晚到的結果被 closure 持有的 box 安全丟棄。

### 內建 llama.cpp 細節（`llm.rs`）

- `LlamaBackend` 全程序一份（OnceLock＋初始化鎖防首次並發 double-init）。
- 模型生命週期＝Handy 實證模式：**用時載入 → 閒置 watcher（10s 巡檢，5 分鐘未用卸載）
  → 換模型先 drop 舊的再載新的**（避免雙駐留峰值）。待機 RSS 因此能回落。
- 生成：GGUF 內建 chat template → tokenize → 整批 decode → greedy 逐 token
  （糾錯任務要最保守、可重現）。
- **地雷：CJK 一個字常拆成多個 token（半個 UTF-8 序列）。生成的 token 必須
  整串收完再一次 `tokens_to_str`——逐 token 轉字串會把拆開的字靜默丟掉**
  （實測「寫的」變「的」）。
- whisper.cpp 與 llama.cpp 各自 vendor 一份 ggml，同程序共存實測無符號衝突
  （兩者都是近期上游，ABI 相容；若未來升級後出現詭異 crash，先查這裡）。

### 退出路徑（`lib.rs` RunEvent::Exit）——最常炸的地方

ggml/Metal 資源**留給 atexit teardown 會 `ggml_abort`**（SIGABRT＋crash 對話框）。
所以退出時必須主動 unload whisper 與 llama。但引擎鎖可能被背景載入/轉錄/潤飾占住：
最多重試 3 秒，還拿不到就 `libc::_exit(0)` 跳過 atexit——寧可少跑 teardown
也不給使用者 crash 對話框。signal handler（ctrlc）只准走 `AppHandle::exit(0)`，
直接 `std::process::exit` 同樣會炸。

## 7. 文字後處理（`textproc.rs`）

順序固定：`clean_transcript`（CJK 間空白清理，手刻——regex crate 沒有 lookbehind）
→ `to_traditional`（OpenCC s2twp，ferrous-opencc 純 Rust）→ `normalize_cjk_punct`
（CJK 字後的半形標點→全形，但下一字是 ASCII 英數時不動，保護 "3.14"）→ `apply_dict`。
**OpenCC 必須在 LLM 之後再過一次**——LLM 輸出可能帶簡體（pipeline 裡潤飾結果
會再走一次 to_traditional＋normalize）。

## 8. 貼上（`inject.rs`）

剪貼簿備份（純文字）→ 寫入結果 → CGEvent 合成 Cmd+V → **等 300ms** → 還原剪貼簿。
300ms 是 prototype 實測值；Handy 用 50ms 會跟慢的目標 app race（貼到一半剪貼簿被還原）。
已知限制：富文本/圖片剪貼簿內容不還原（只備份純文字）。

## 9. 設定與資料（`settings.rs`、`history.rs`）

- `~/.claro/config.json`：扁平 schema（與 prototype/Swift menu bar 相容——
  **schema 同時存在 Rust 與 `prototype/mic_indicator.swift` 兩邊，改動要同步**；
  Swift 端 read-merge-write 會保留未知鍵）。檔案 0600、目錄 0700。
- 多鍵更新用 `update_config_keys`（單次讀寫）——分次寫會在連續操作時留下混搭中間態。
- API key 永不進 JSON，走 macOS Keychain（service "Claro LLM"）。
- `history.jsonl`：append-only，每筆含 raw/text/status/duration/timings。
  上下文**永不**寫進去。

## 10. 前端（`desktop/src/`）

- `App.tsx`：側欄殼＋每 2s 輪詢 `get_status`＋事件監聽（模型下載進度、麥克風電平）。
- 頁面：`Home`（統計卡＋未就緒 banner）、`History`、`Settings`（所有設定列）。
- **視窗拖曳靠 `data-tauri-drag-region` 屬性**——`-webkit-app-region` 是 Electron 的，
  在 Tauri 完全無效（踩過）。屬性只對元素本身生效，子元素（按鈕）不受影響。
- 寫設定的併發原則：合併值走 ref（不吃 stale closure）、請求走 promise 佇列串行化
  （見 `saveLlm`/`saveDict`）。
- 本機服務偵測結果（Ollama/LM Studio 模型清單）綁 provider 儲存，
  渲染時比對——晚到的舊回應不能污染已切換的 provider。

## 11. 打包與權限（macOS 特有的坑）

- **驗證一律用 `npm run tauri build` 的 bundle**。`cargo build --release` 直接編出的
  binary 是 dev context，webview 會去連 devUrl（localhost:1420）→ 白屏。
  白屏除錯：`CLARO_DEVTOOLS=1` 啟動自動開 inspector。
- `src-tauri/Info.plist` 會在打包時合併——**`NSMicrophoneUsageDescription` 必須存在**，
  否則 Finder 啟動的 app 麥克風被 TCC 靜默拒絕（連授權窗都不跳）。
  從終端機啟動會繼承終端機權限，所以「終端機測過沒事」不代表 bundle 沒事。
- 輔助使用權限跟著簽章走：**未簽章的開發版每次重建 cdhash 都變**，權限即失效。
  app 內建補救：啟動時跳系統授權提示＋首頁 banner＋每 2s 輪詢授權後自動啟用。
  正式發布（M6）簽章後此問題消失。
- 熱鍵可設定（config `hotkey`，handy-keys 字串格式），換鍵走 `Ctl::SetMain`
  熱換註冊，不用重啟。

## 12. 測試策略（全部無聲）

| 層 | 指令 | 蓋什麼 |
|---|---|---|
| Rust 單元 | `cargo test --lib` | 狀態機全案例、textproc、settings、polish 映射、context 詞彙抽取、hotkey preset 解析 |
| STT 管線 | `cargo run --example transcribe_file x.wav` | WAV 直進完整 STT（測試音用 `say -v Meijia -o` 無聲渲染） |
| Apple bridge | `cargo run --example apple_polish` | 真實 FoundationModels 端到端 |
| 內建 LLM | `cargo run --example builtin_polish [x.wav]` | whisper＋llama 同程序共存＋糾錯品質 |
| HTTP 潤飾 | `cargo run --example test_polish`＋mock server | 防呆三路徑（正常/空輸出/長度爆炸） |
| 上下文 | `cargo run --example show_context` | AX 擷取＋詞彙抽取（需輔助使用權限的終端機） |
| 前端 | happy-dom 探測 dist bundle | mock IPC 渲染設定頁，驗證各區塊存在 |
| CI | GitHub Actions（macOS） | `npm run build`＋`cargo test --lib`（stub 路徑） |

**驗證鐵律：絕不透過喇叭播音、絕不合成全域鍵盤事件**（會干擾正在用電腦的人）。

## 13. 怎麼加東西（常見維護任務）

- **加 whisper 模型**：`stt/registry.rs` 加一行 spec（id/url/bytes）。
- **加內建 LLM 模型**：`llm.rs::LLM_MODELS` 加一行；先驗證 GGUF URL 與 chat template。
- **加雲端 preset**：`Settings.tsx::CUSTOM_PRESETS` 加一行（base_url 必須 OpenAI-compatible）。
- **加潤飾 provider**：`polish.rs` enum 加變體＋`from_settings` 映射＋UI select 加 option；
  記得走 `guard()`、失敗退原文。
- **加設定鍵**：`settings.rs::default_config` 加預設＋accessor；前端 types.ts 同步；
  若 Swift menu bar 也要讀，同步 `prototype/mic_indicator.swift` 的 schema。
