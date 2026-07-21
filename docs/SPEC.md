# Claro 產品與技術規格

版本：Phase 0（2026-07-05），P0 Trust & Release Pass 更新（2026-07-12），STT Accuracy Pass 更新（2026-07-15）。本文件是實作的真值來源；重大變更需先更新此文件。

## 1. 產品定義

Claro 是 macOS（後續跨平台）語音輸入工具：按熱鍵說話 → 本地 STT → 依使用者選擇的 RAW／CLEAN／ORGANIZE 契約處理文字 → 貼回游標所在輸入框。處理模式與執行 provider 是兩條獨立設定軸。

四大支柱（缺一不可，詳見 `COMPETITIVE_MATRIX.md`）：

1. **本地優先隱私** — 音訊預設不出機器；雲端路徑全部明示、可關、可稽核。
2. **真上下文感知** — AX API 擷取（非截圖），可見、可控、可稽核。
3. **模型自由** — STT 與 LLM 雙層可插拔，含 BYOK OpenAI-compatible。
4. **CJK 極致** — 保守糾錯、OpenCC、混排規則與解碼期詞彙偏置已上線；IME 貼上防護是 M4 待辦。

非目標(v0.x):行動平台、雲端帳號體系、團隊功能、通用 AI 助理對話。

## 2. 技術棧與 repo 佈局

- Tauri v2：Rust backend ＋ React/TypeScript/Tailwind 前端。不用 Electron。
- 現有 Python/MLX 原型移至 `prototype/`，作為行為參考實作（CJK 處理、prompt、狀態機語意的真值來源），不再新增功能。

```
├── desktop/                  # Tauri app
│   ├── src-tauri/src/
│   │   ├── pipeline.rs       # 單次聽寫 session 編排
│   │   ├── state_machine.rs  # 純邏輯狀態機（自 prototype 移植）
│   │   ├── hotkey/           # 全域熱鍵、PTT/toggle
│   │   ├── audio/            # capture / level / guards
│   │   ├── stt/              # SttEngine trait + providers
│   │   ├── context/          # ContextProvider trait + macOS AX
│   │   ├── polish/           # Polisher trait + providers + CJK rules
│   │   ├── inject/           # paste + ime_guard + clipboard
│   │   ├── models/           # registry + downloader
│   │   ├── settings.rs       # config 讀寫/遷移
│   │   └── history.rs        # history.jsonl（相容 prototype）
│   └── src/                  # React：主視窗、settings、onboarding、history
├── prototype/                # Python/MLX 原型（凍結）
└── docs/                     # SPEC / ROADMAP / COMPETITIVE_MATRIX
```

## 3. 架構與資料流

```
                       ┌──────────────────────────────────────────┐
                       │                Rust core                 │
 全域熱鍵 ─▶ 狀態機 ─▶ │ pipeline:                                │
 (PTT/toggle/Esc)      │  audio ──▶ stt ──▶ rules ──▶ polish ──▶  │ ─▶ inject（防誤貼
                       │            ▲             ▲     │         │     ＋剪貼簿還原）
                       │            │ vocab bias  │ ctx │ opencc  │
                       │        context（AX，記憶體內，不落盤）   │ ─▶ history.jsonl
                       └──────┬───────────────────────────────────┘
                              │ tauri events（state/level/audit）
                       ┌──────▼───────────────────────────────────┐
                       │ WebView：主視窗/settings；Swift sidecar：膠囊/menu bar │
                       └──────────────────────────────────────────┘
```

單次聽寫（`pipeline.rs`，各階段耗時記錄進 history）：

1. 熱鍵按下即建立 session；麥克風 stream 初始化與最長 50ms 的 AX focused-element seed 並行，避免 AX 排在開麥前切掉首音節；stream ready 後完成有界 target hash 與背景 Context 擷取。正常放開保留 200ms post-roll，取消／退出立即停止；收集音訊後以正規化前 RMS／clipping ratio 留下診斷（防呆：<0.3s、RMS 靜音 → error 態）
2. processing 最多等待該 session 的 context 250ms；Context 與 fingerprint 使用同一批 retained AX refs，失敗、逾時或目標不一致時靜默降級為無上下文。貼上前與 Cmd+V 前一刻都重驗 App／視窗／焦點 fingerprint；失焦時只保留結果、不貼到錯誤位置
3. `stt.transcribe(audio, req)`：production 暫以 `zh` hint 為預設；`auto`／`zh` 必須在同一真人 corpus 成對比較後才可改預設。只有支援 decode prompt 的 Whisper family 才附 `initial_prompt`，內容只放個人字典＋上下文 canonical terms，不把它當 LLM 指令句。現行 Qwen3-ASR backend 接受 language hint，但不接受 Whisper run extension
4. 確定性規則：個人字典替換、CJK 空白清理（移植 `_clean_transcript`）
5. 依 `polish_mode` 處理：RAW 不呼叫 LLM；CLEAN 做保守校訂；ORGANIZE 在使用者明確同意後才允許跨句重排。provider 不可用、隱私 gate 阻擋或防呆不通過時，一律退回 deterministic base text
6. OpenCC s2twp 終盤 → 可選混排規則（盤古之白、全半形標點）
7. 取消檢查（session 序號 + CancelToken）→ 寫入暫存剪貼簿後、真正送 Cmd+V 前再次執行 session／focus guard；未取消且目標仍相同才 `inject.paste()`
8. history 落盤（raw、text、status、timings）；overlay 顯示結果態

執行緒模型：熱鍵事件由單一 dispatcher 序列化餵狀態機（沿用 prototype 的教訓：keyDown/keyUp 亂序會卡死；Handy 的 `transcription_coordinator` 同構，印證此設計）；audio、pipeline 各自 spawn；UI 溝通全走 tauri events。

音訊擷取（採 Handy 實證作法）：cpal 以裝置**原生取樣率**擷取、多聲道 downmix mono，再用 `rubato` 重採樣到 16kHz——不強迫硬體改率。Silero VAD（`vad-rs`）＋ smoothing（prefill/onset/hangover）於 M2 引入，用於免持模式與靜音防呆強化。

## 4. 狀態機（自 prototype 移植，語意不變）

IDLE → 按下 → HOLD → 放開：<0.35s 進 HANDSFREE（免持），否則 PROCESSING。
HANDSFREE → 再按 → PROCESSING。Esc：錄音中取消；處理中取消（結果只進 history）。
session 序號讓過期處理結果自動作廢。錄音上限 5 分鐘 → force_stop。

Rust 移植必須通過 prototype 的整套測試場景（`tests/test_state_machine.py` 逐案對應）。

## 5. 模組介面（trait 層）

新增一個 provider = 實作 trait ＋ 在 registry 註冊，不改 pipeline。

```rust
pub struct SttRequest<'a> {
    pub audio: &'a [f32],            // 16kHz mono
    pub language: Option<&'a str>,   // None = auto；production 目前傳 Some("zh")
    pub initial_prompt: Option<String>,
    pub vocab_bias: Vec<String>,     // 個人字典 + 上下文詞彙
}
pub trait SttEngine: Send + Sync {
    fn id(&self) -> EngineId;
    fn load(&mut self) -> Result<()>;
    fn unload(&mut self);
    fn transcribe(&self, req: &SttRequest, cancel: &CancelToken) -> Result<Transcript>;
}

pub trait Polisher: Send + Sync {
    fn id(&self) -> ProviderId;
    fn polish(&self, text: &str, ctx: &ScreenContext, cancel: &CancelToken) -> Result<String>;
    // Err 或防呆不過 → pipeline 一律退回未潤飾文字，聽寫永不因潤飾失敗而失敗
}

pub trait ContextProvider: Send + Sync {
    fn capture(&self, budget: &ContextBudget) -> Result<ScreenContext>;
}
pub struct ScreenContext {
    pub app_name: String,
    pub window_title: Option<String>,
    pub around_cursor: Option<String>,  // 游標前後各 500 字
    pub selected: Option<String>,       // 上限 200 字
    pub visible: Option<String>,        // BFS 可見文字，總預算 3000 字 / 800 節點
}

pub trait TextInjector: Send + Sync {
    fn inject(
        &self,
        text: &str,
        pre_paste_commit: &dyn Fn(&dyn Fn() -> Result<()>) -> Result<()>,
    ) -> Result<()>;
    // macOS: 剪貼簿備份 → settle → 在 session lock 內 guard+Cmd+V → safe restore
}
```

## 6. STT 層

| Provider | 引擎 | 用途 | 備註 |
|---|---|---|---|
| `qwen3-asr` | transcribe.cpp（GGUF／Metal） | **離線評測候選** | 0.6B／1.7B Q8_0；可接 language hint、無 `initial_prompt`；256-token 單次輸出與長音訊分段未驗收，production UI 不可下載／啟用 |
| `whisper` | transcribe.cpp（whisper.cpp／Metal） | **目前正式主力** | 完整 large-v3 為精準度優先；large-v3-turbo 為速度優先；支援 `initial_prompt` |
| `sensevoice` | ONNX | **CJK 快速選項**（zh/yue/ja/ko） | 比 Whisper 快得多，M2 評測定位 |
| `moonshine` | ONNX | 低資源快速選項 | 官方支援 zh/ja/ko/en 等 |
| `parakeet` | ONNX（v3） | 英/歐語速度選項 | **無 CJK**，UI 須標明 |
| `cloud-openai` | BYOK HTTP | 進階選項 | v0.1 之後；預設關 |

推理 crate（依 Handy 實證定案，見 `docs/research/handy-analysis.md`）：Whisper／Qwen3-ASR GGUF 家族用同一個 in-process `transcribe-cpp`（Metal），不需要 Python、Ollama 或本機服務；ONNX 家族（Parakeet/Moonshine/SenseVoice）用 `transcribe-rs`。目前硬體推薦是 Intel 使用 Turbo Q5、8–12GB Apple Silicon 使用 large-v3 Q5、16GB+ Apple Silicon 使用完整 large-v3；Turbo 只定位為速度優先。既有 Turbo 使用者由首頁可略過的升級入口前往設定，仍須自行確認約 3GB 下載，不可自動下載。Qwen 只有 publisher／port 候選證據，尚無 Claro 長音訊與真人台灣語料結果，因此不進硬體推薦。低記憶體路徑在 STT 完成後先卸載語音模型，再載入內建 LLM，潤飾後立即卸載 LLM，避免雙模型同時駐留與長時間佔用約 4B 模型記憶體。

Qwen3-ASR Q8_0 離線評測 artifacts 固定如下；來源、256-token／長音訊限制與 benchmark 邊界見 `docs/research/stt-accuracy-2026-07-15.md`：

| 模型 | Immutable revision | 檔案大小 | SHA-256 |
|---|---|---:|---|
| Qwen3-ASR 0.6B Q8_0 | `e4e16599b900eb0cb36e524514756bb92eb092b7` | 850,423,456 bytes | `f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8` |
| Qwen3-ASR 1.7B Q8_0 | `92282af1610a2db19d66f2bef1e260f5deca782d` | 2,185,030,624 bytes | `9a0d81792dfea2d5f278b8a63deb3ea6e02139ce42c2301f32ea19c4f77526b7` |

**模型管理**（下載器規格照 Handy 實證）：內建 registry（id、引擎、大小、HF URL、sha256、語言、RAM 估計）；下載一律明確同意（使用者網路慢——**絕不自動下載**）、有進度、`.partial`＋HTTP Range 續傳、sha256 校驗、暫存目錄原子化解壓。候選模型可供離線 CLI eval，但 production resolve／下載／啟用均 fail closed。切換採原子 transaction：候選模型載入與 config 寫入都成功後才更新 active；失敗回載舊引擎。若「下載並使用」完成時仍在聽寫，必須明示已下載但待切換，不可回 silent success。config/history 留在 `~/.claro/`（與 prototype 相容）；模型放 `~/Library/Application Support/Claro/models/`。閒置 N 分鐘卸載（可設）。

## 7. 上下文引擎（差異化核心）

**預設用 AX API、不用截圖**：不觸發螢幕錄影權限，也不需 vision model 常駐。keyDown 只用一次有界 AX 呼叫固定 focused element reference；不讀文字或逐項 metadata。麥克風開始後才完成 target hash，Context 另以同一批 retained AX refs 在背景擷取；worker 約 420ms 後硬停止（與錄音並行，非關鍵路徑），processing 在放開後最多等待 250ms。逾時、失敗或目標不一致就不用。`keyDown→錄音提示 p95 ≤150ms` 仍需真機矩陣量測，不是目前已證實的數字。截圖+本地 VLM 是 post-v0.1 的可選進階，預設關，且同樣不得超過當次 deadline。

移植 prototype 已驗證的擷取邏輯：frontmost app → AXFocusedWindow 標題 → AXFocusedUIElement 的游標前後文（AXSelectedTextRange）與選取文字 → 視窗 AX 樹 BFS 收可見文字（只收 AXStaticText/AXTextField/AXTextArea/AXHeading，排除按鈕/連結噪音）。預算：游標前後各 500 字、可見文字 3000 字、800 節點。可見文字額度大於 prototype 的 1200/400，因為它是「自動詞源」——螢幕上出現過的專有名詞可直接進 STT 偏置，使用者不必手動維護字典。供給放大必須搭配 `ascii_term_priority` 相關性排序，否則雜訊詞會先佔滿有限的 prompt 格子。

Claro 已實作並以 fixture 驗證的上下文雙重用途：
1. **解碼期偏置（provider-aware）**：抽英文／技術詞彙與 CJK 詞彙；Whisper 將上限 15 個 canonical terms 放進 `initial_prompt`，Qwen3-ASR 現有 transcribe.cpp port 不接受此 extension，不能宣稱已套用同等偏置
2. **處理參考**：個人字典仍可在 STT 後做確定性替換；CLEAN 不把畫面文字交給 LLM，也禁止 LLM 修改任何文字／英數 token。完整 bounded ScreenContext 只在 ORGANIZE 進 Polisher prompt；可解釋的 surface 分類（email／message／document／technical／neutral）只能決定段落或明確清單格式，不允許改動事實或語氣內容

**隱私鐵律**：
- `AXSecureTextField` 永不讀取
- 內建排除清單（密碼管理器等）＋全域開關；使用者自訂 per-app 停用仍是 M3 待辦
- 上下文只存在記憶體，**永不落盤**、永不進 history
- 稽核視圖：settings 顯示「上一次聽寫擷取了什麼」，一鍵清除；開關關閉時零 AX **文字**擷取。防誤貼仍會讀取 App／視窗／焦點 metadata 並立即 hash，只存在本次 session，不讀欄位值、不顯示、不落盤

## 8. 潤飾層與 CJK 處理（護城河）

### Polisher providers

| Provider | 說明 | 定位 |
|---|---|---|
| `apple` | Apple Intelligence 端上模型（FoundationModels，macOS 26+）：免額外模型下載，執行與硬體調度由 macOS 管理；Claro 透過 in-process Swift bridge＋弱連結呼叫，偵測 `appleIntelligenceNotEnabled` 並引導開啟 | 可用時的推薦預設 |
| `llamacpp` | 內嵌 llama.cpp（`llama-cpp-2`，Metal），GGUF 預設 Qwen3-4B-Instruct-2507 Q4（~2.4GB，下載經同意）；閒置卸載（watcher 同 STT，預設 5 分鐘） | macOS <26 或未啟用 Apple Intelligence 時的推薦 |
| `ollama` | localhost:11434，模型清單自動偵測（/api/tags） | 進階選項（**永不要求使用者安裝**） |
| `lmstudio` | localhost:1234，模型清單自動偵測（/v1/models） | 進階選項 |
| `openai-compat` | BYOK：Base URL / API Key / Model 全可設；UI 內建 OpenAI/Groq/DeepSeek/Gemini/OpenRouter preset | 進階選項 |
| `off` | 純轉錄 | 出廠預設（隱私鐵律：不靜默連任何端點） |

選型依據與競品證據：`docs/research/llm-polish-runtime.md`（2026-07-06）。

prototype 教訓：1.5B 級模型會洩漏提示詞、亂加列表符號 → 預設 4B 級；防呆全保留（空輸出、長度 > max(3×, +200) → 退回原文）。內建模型 generation 最多 6 秒，整次整理另有 8 秒 total cap；STATE 忙碌時不排隊。準備階段保留取消／總額度檢查；阻塞式 Metal cold load 本身不可中斷，若回來時已超時則保留 warm model 供下一段、本段立即退回 deterministic text，不得再追加完整 generation。正式 bundle 仍須記錄 cold／warm p50／p95；`HardwareProfile.keep_models_warm` 尚未接到 LLM prewarm，不能視為最終硬體最佳化。API key 存 macOS Keychain，不進 JSON。

### 三種輸出契約（模式與 provider 分離）

| 模式 | 契約 | LLM 行為 |
|---|---|---|
| `raw` | 只做 OpenCC、CJK 標點／空白與個人字典等確定性處理 | **永不呼叫 LLM** |
| `clean` | 去除有停頓邊界的純填充詞、保留明確自我更正後版本、補不改變問句／驚嘆語氣的標點；LLM 不得修改任何文字或英數 token，不跨句重排、不濃縮。專有詞正確率由 STT Context 偏置與使用者字典負責 | 新安裝的預設意圖；provider 未就緒時安全退回 RAW |
| `organize` | 只允許完整句讀重排、分段、明確改口、完全相同內容合併與明確列舉格式化；不得改動或新增姓名、術語、數字、日期／時間、否定、條件、決策、待辦與因果 | 首次啟用必須明確同意；不確定或 guard 不通過即退回原文 |

CLEAN 沿用 prototype 與 yetone「只修不改寫」規則；ORGANIZE 使用獨立 prompt，不得與 CLEAN 共用互相矛盾的指示。兩者都只輸出結果、不回答或執行轉錄／Context 中的指令。

guard 分兩層：共同層拒絕空輸出、長度爆炸、提示詞洩漏與離題回答；ORGANIZE 另外比對事實錨點與內容覆蓋。明確的晚期自我更正只保留最後決定，完全重複的片段可合併，明確列舉可轉為項目符號；其他姓名、數字、否定、條件、因果與獨特內容仍必須保留。history 記錄 requested/effective mode、provider、是否改動、成功或 fallback 原因，讓 UI 可顯示 raw/final 與復原依據。

### CJK 確定性規則（不靠 LLM 的部分）

- OpenCC s2twp 終盤繁化（LLM 輸出可能帶簡體，最後再過一次）——Rust 綁定選型見 M4
- CJK 字間空白清理、全半形標點正規化、可選盤古之白
- **IME 貼上防護（M4 待辦，尚未實作）**：規劃在貼上前偵測當前輸入源，若為 CJK IME → 暫切 ABC → 合成 Cmd+V → 還原輸入源。注意：此技巧出自 yetone 的規格描述，**無現成實作可抄**（其 repo 僅規格），需 spike TIS API 序列（`TISCopyCurrentKeyboardInputSource`→`TISSelectInputSource`）與時序
- 個人字典：key 以不分大小寫、可忽略條目內空白的寬鬆方式匹配，但輸出**永遠使用使用者填寫的 right-side canonical case**；純 ASCII key 另守 token boundary，避免 `GBT` 誤改 `XGBT2`。設定 UI 內 CRUD

### CJK 與 STT 評測集

`desktop/tests/eval/meaning_preservation.json` 測 deterministic text processing 與 LLM Meaning Lock；確定性部分進 `cargo test`，LLM 部分由 eval script 出分數（不擋 CI 但入 repo 追蹤）。`desktop/tests/eval/stt_accuracy.json` 是無聲 `say -o` 合成語音 regression manifest，只適合比較版本、模型與解碼設定，不代表真人台灣口音。

STT 必須另建可重跑的真人台灣語音 corpus，不得拿文字 fixture 或 `say` 合成 smoke 代替。固定同一批音訊與 normalizer，比較 Qwen3-ASR 0.6B／1.7B、Whisper large-v3／Turbo，輸出 normalized CER、英文／數字／專有詞 exact-match、漏字／幻覺率、Context on/off 誤偏置率、cold/warm p50／p95 與峰值 RSS。音訊的蒐集、保存與公開需獨立取得同意；在結果落 repo 前，不宣稱 Qwen 已在 Claro 上勝過 Whisper 或 Claro 已優於 Typeless／Superwhisper／VoiceInk。

## 9. 文字注入

macOS 使用 `NSPasteboard` 在清空前逐 item、逐 type 完整 materialize 備份（文字、RTF、HTML、圖片、file URL 與自訂格式）→ 寫入結果與 Claro owner marker → CGEvent 合成 Cmd+V → 延遲 → 僅在 pasteboard `changeCount` 與 marker 仍屬本次注入時還原。任一格式無法完整備份就 fail closed；若使用者期間另行 Copy，絕不以舊內容覆蓋。Cmd modifier 的 release 必須走 cleanup path。IME 防護仍是第 8 節所列 M4 待辦。Windows 期：SendInput ＋ UIA，介面已由 `TextInjector` trait 隔離。

## 10. 隱私模型

| 層級 | 資料流 | 預設 |
|---|---|---|
| T0 全本地 | 音訊/文字不出機器 | ✅ |
| T1 BYOK 潤飾 | 僅轉錄文字＋上下文送使用者自己的端點 | 關；UI 紅字標示 |
| T2 雲端 STT | 音訊上雲 | v0.1 不做 |

- **local-only 模式**：新安裝預設開啟；阻擋聽寫音訊、Context 與潤飾文字送往外部 endpoint。localhost／127.0.0.1／`::1` 本機服務仍可使用；自訂外部 endpoint 必須 HTTPS。使用者明確按下的 STT／LLM 模型下載屬例外，UI 必須先顯示大小並取得同意；目前尚無全程序 network interceptor，不能宣稱連更新檢查都硬斷
- ORGANIZE 行為同意與雲端資料傳送同意是兩個獨立、具版本的 backend gate，不能只靠前端文案。所選模式或資料流所需的 gate 不成立時，該路徑不得外連並安全退回原樣轉錄
- 權限只要兩個：麥克風＋輔助使用。**不要**螢幕錄影/相機/藍牙（對比 Typeless 的行銷素材）
- history 本地 0600、可關可清；上下文永不落盤；key 進 Keychain
- 發布時附資料流向圖與雲端路徑清單

## 11. UI/UX

- **Onboarding**（首啟）：輔助使用權限 → 麥克風 → 只顯示這台 Mac 的推薦語音方案並取得下載同意 → 選擇系統端上整理／Claro 內建整理／原樣轉錄 → 在其他 App 完成一次真實貼上 → 完成。Ollama、Homebrew、終端機、模型真名與 provider 術語不出現在必要流程；`setup_completed` 不能只靠按鈕或假測試通過。目標：dmg 到首次成功聽寫 <5 分鐘。
- **Overlay（目前）**：底部置中 Swift `mic_indicator` sidecar，沿用 prototype 的 Unix socket 協議與動畫；六態（隱藏/錄音波形/免持/處理/成功/取消/錯誤），滑鼠穿透、不搶焦點、全 Spaces。Rust 維持長連線，sidecar 以 60Hz 動畫更新。M5 才評估改成 Tauri `tauri-nspanel`／React，不能把候選架構當成現況。
- **Tray（目前）**：狀態、今日統計、最近 5 筆（點擊複製）、開歷史、設定、結束；快速換模型仍在規劃，模型管理目前位於主視窗。
- **Settings（目前）**（React）：熱鍵與輸入裝置、語音模型、文字整理 provider、輸出模式、Context 全域開關與稽核、Dictionary、History、local-only 與雲端目的地揭露。使用者自訂 per-app 規則、Advanced 延遲視覺化與 debug 音訊仍是後續里程碑。首頁隱私文案必須依實際 mode/provider/context 動態顯示，不得無條件宣稱全本地。

## 12. 效能預算（驗收指標，M1 Mac 基準）

| 指標 | 目標 |
|---|---|
| 放開熱鍵→貼上（≤5s 音訊，raw） | p50 ≤ 1.5s |
| 同上，含本地 LLM 潤飾 | p50 ≤ 4s |
| 同上，BYOK 雲端潤飾 | p50 ≤ 2.5s |
| 待機 RSS（模型已卸載） | < 300MB |
| 冷啟動（不含模型載入） | < 3s |

每段聽寫的分段耗時已寫入 history；Advanced 頁視覺化與可重跑量測腳本仍是 M5 驗收項目。

## 13. 測試策略

- Rust 單元：狀態機（對應 prototype 全套案例）、字典/清理規則、mode-specific prompt/guard、consent/local-only gate、config 遷移、registry 校驗、clipboard restore decision
- 整合:mock SttEngine/Polisher 驅動完整 pipeline，含取消/防呆路徑
- CJK golden 評測（第 8 節）；可重跑的真人台灣 STT corpus 與逐模型 CER／term accuracy 報告；前端 vitest；每里程碑手動 smoke checklist（ROADMAP 附錄 A）
- CI：GitHub Actions macOS build ＋ test；M6 加簽章/公證/release

## 14. 設定檔（`~/.claro/config.json`，自 prototype 遷移）

新 schema 分節（hotkey/stt/polish/context/cjk/dictionary/privacy）；目前相容扁平鍵新增 `polish_mode`（`raw|clean|organize`）、`local_only`、`history_enabled`、`organize_consent_version`、`cloud_consent_version`、`cloud_consent_origin`、`setup_completed`。遠端雲端同意綁定到確認時的 scheme + authority，更換 host、port 或 scheme 後自動失效。新安裝的 `setup_completed` 只能在後端同時確認權限、麥克風測試、已驗證模型與本次啟動至少一次成功貼上後寫入；既有已完成設定不被回溯阻擋。個人字典新安裝預設為空，只有使用者明確建立的替換才可改字；舊版若 dictionary **完全等於**未經同意內建的 `GBT→GPT`／`My Torch→PyTorch` 才移除，任何自訂增刪都原樣保留。啟動時偵測舊 schema（`whisper_model`/`llm_model`/`llm_enabled`）自動遷移並保留未知欄位。history.jsonl 維持向後相容，新增可選 `timings` 與 `polish` metadata；音訊診斷使用正規化前的 `audio_input_rms`／`audio_clipped_ratio`，不得把舊 `audio_rms` 當成實際麥克風音量。

## 15. 決策記錄與待決事項

| # | 決策 | 狀態 |
|---|---|---|
| D1 | Python/MLX → Tauri/Rust 重寫；prototype 凍結為參考實作 | **已定（Phase 0，2026-07-05）** |
| D2 | 上下文預設 AX、不截圖；截圖+VLM 為 post-v0.1 選項 | 已定 |
| D3 | 潤飾「零安裝」雙軌：macOS 26+ 推薦 Apple Intelligence（FoundationModels in-process Swift bridge＋弱連結）；其餘內嵌 llama.cpp（`llama-cpp-2`/Metal，Qwen3-4B-Instruct-2507 Q4）。Ollama/BYOK 降為進階選項，**永不要求裝 Ollama**。內嵌引擎閒置卸載沿用 Handy watcher 模式 | **已定（2026-07-06 研究定案，見 `docs/research/llm-polish-runtime.md`）**；M4 實作，llama.cpp 二進位大小於實作時驗證 |
| D4 | M5 原生 overlay 候選為 `tauri-nspanel`（NSPanel）＋滑鼠穿透；P0/M1 不是此實作 | **由 D10 收斂** |
| D5 | STT 推理 crate：`transcribe-cpp`（Whisper＋Qwen3-ASR GGUF／Metal）＋ `transcribe-rs`（ONNX：Parakeet/Moonshine/SenseVoice）；Qwen 直接 in-process，不引入 Python／Ollama | **已定（2026-07-15 擴充）**（Handy／transcribe.cpp 實證） |
| D6 | OpenCC Rust 選型為 `ferrous-opencc`，終盤使用 s2twp 規則 | **已定並實作** |
| D7 | 全域熱鍵 PTT 用 `handy-keys` crate（press/release 事件） | **已定**（Handy 實證）；不採雙 backend |
| D8 | 剪貼簿還原延遲：沿用 prototype 300ms（Handy 的 50ms 會 race 慢 app），後續改剪貼簿 changeCount 輪詢 | 已定 |
| D9 | IME 切換無現成實作，M4 spike TIS API 序列 | 已定 |
| D10 | M1 沿用 prototype 的 Swift `mic_indicator`（Rust spawn＋同一 socket 協議），動畫原封不動；Tauri 原生 overlay（nspanel＋穿透，沿用同視覺基礎）延至 M5 打磨期 | **已定**（使用者指定保留現有動畫基礎） |
| D11 | 文字行為拆為 RAW／CLEAN／ORGANIZE，與 Apple／Builtin／Local service／Cloud provider 分離；CLEAN 是預設意圖，ORGANIZE 明確 opt-in，所有不確定情況回退 deterministic RAW | **已定（2026-07-12，P0 Trust & Release Pass）** |
| D12 | macOS bundle identifier 在公開前凍結為 `dev.claro.desktop`；不以 `.app` 結尾，避免與 App bundle 副檔名混淆。未來如需變更必須附 TCC 遷移說明 | **已定（2026-07-12，P0 Trust & Release Pass）** |
| D13 | 首次設定與模型生命週期由唯讀 `HardwareProfile` 決策：Intel 推薦 Turbo Q5、8–12GB Apple Silicon 推薦 large-v3 Q5、16GB+ 推薦完整 large-v3；8–12GB 路徑不讓 STT 與 4B LLM 同時常駐。Qwen3-ASR 在長音訊分段與真人台灣 corpus 通過前只供離線 eval，production UI fail closed。只改推薦與資源策略，任何正式模型下載仍須使用者明確按下 | **已定；2026-07-15 Accuracy Pass 收斂** |
| D14 | Typeless 等閉源產品只做官方資料與人工可觀察輸出的產品評測；不反組譯、不解密私有流量、不把其輸出用於訓練、fine-tune、prompt distillation 或自動資料抽取，也不宣稱知道其模型、prompt 或內部管線。Claro 以 session-bound AX context、可稽核 surface 分類與 Meaning Lock 建立可獨立驗證的對標行為與評測目標 | **已定（2026-07-12，見 `docs/research/typeless-black-box-2026-07-12.md`）** |
| D15 | STT production 暫留 `zh` hint，`auto`／`zh` 必須以同一真人 corpus 成對驗證後才可改預設；CLEAN Meaning Lock 不承擔猜回錯字；準確度發布門檻不得以 publisher benchmark、合成 TTS 或 LLM 文句通順度代替 | **已定（2026-07-15，見 `docs/research/stt-accuracy-2026-07-15.md`）** |
