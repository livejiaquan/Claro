# Claro 產品與技術規格

版本：Phase 0（2026-07-05）。本文件是實作的真值來源；重大變更需先更新此文件。

## 1. 產品定義

Claro 是 macOS（後續跨平台）語音輸入工具：按熱鍵說話 → 本地 STT → 依螢幕上下文由 LLM 保守潤飾 → 貼回游標所在輸入框。

四大支柱（缺一不可，詳見 `COMPETITIVE_MATRIX.md`）：

1. **本地優先隱私** — 音訊預設不出機器；雲端路徑全部明示、可關、可稽核。
2. **真上下文感知** — AX API 擷取（非截圖），可見、可控、可稽核。
3. **模型自由** — STT 與 LLM 雙層可插拔，含 BYOK OpenAI-compatible。
4. **CJK 極致** — IME 貼上防護、保守糾錯、OpenCC、混排規則、解碼期詞彙偏置。

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
│   └── src/                  # React：settings、onboarding、overlay UI
├── prototype/                # Python/MLX 原型（凍結）
└── docs/                     # SPEC / ROADMAP / COMPETITIVE_MATRIX
```

## 3. 架構與資料流

```
                       ┌──────────────────────────────────────────┐
                       │                Rust core                 │
 全域熱鍵 ─▶ 狀態機 ─▶ │ pipeline:                                │
 (PTT/toggle/Esc)      │  audio ──▶ stt ──▶ rules ──▶ polish ──▶  │ ─▶ inject（IME 防護
                       │            ▲             ▲     │         │     ＋剪貼簿還原）
                       │            │ vocab bias  │ ctx │ opencc  │
                       │        context（AX，記憶體內，不落盤）   │ ─▶ history.jsonl
                       └──────┬───────────────────────────────────┘
                              │ tauri events（state/level/audit）
                       ┌──────▼───────────────────────────────────┐
                       │ WebView UI：overlay 膠囊、tray、settings │
                       └──────────────────────────────────────────┘
```

單次聽寫（`pipeline.rs`，各階段耗時記錄進 history）：

1. 熱鍵放開/停止 → 收集音訊（防呆：<0.3s、RMS 靜音 → error 態）
2. `context.capture()`（若啟用；預算制，目標 <50ms；失敗靜默降級為無上下文）
3. `stt.transcribe(audio, req)`：req 含 language hint、`initial_prompt`（繁中模板＋個人字典＋上下文詞彙偏置，移植 prototype `_context_terms`）
4. 確定性規則：個人字典替換、CJK 空白清理（移植 `_clean_transcript`）
5. `polish.polish(text, ctx)`（若啟用）：保守糾錯；防呆（空輸出/長度爆炸 → 退回原文）
6. OpenCC s2twp 終盤 → 可選混排規則（盤古之白、全半形標點）
7. 取消檢查（session 序號 + CancelToken）→ 未取消才 `inject.paste()`
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
    pub language: LangHint,          // zh-TW 優先、auto 可選
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
    pub visible: Option<String>,        // BFS 可見文字，總預算 1200 字 / 400 節點
}

pub trait TextInjector: Send + Sync {
    fn inject(&self, text: &str) -> Result<()>;  // macOS: 剪貼簿+Cmd+V+IME 防護
}
```

## 6. STT 層

| Provider | 引擎 | 用途 | 備註 |
|---|---|---|---|
| `whisper` | whisper.cpp（Metal） | **CJK 主力**，預設 large-v3-turbo | 尺寸可選：turbo/large-v3/small |
| `sensevoice` | ONNX | **CJK 快速選項**（zh/yue/ja/ko） | 比 Whisper 快得多，M2 評測定位 |
| `moonshine` | ONNX | 低資源快速選項 | 官方支援 zh/ja/ko/en 等 |
| `parakeet` | ONNX（v3） | 英/歐語速度選項 | **無 CJK**，UI 須標明 |
| `cloud-openai` | BYOK HTTP | 進階選項 | v0.1 之後；預設關 |

推理 crate（依 Handy 實證定案，見 `docs/research/handy-analysis.md`）：Whisper/GGUF 家族用 `transcribe-cpp`（Metal）；ONNX 家族（Parakeet/Moonshine/SenseVoice）用 `transcribe-rs`。模型依機器 RAM 推薦預設（8GB → small/turbo；16GB+ → turbo/large-v3）。

**模型管理**（下載器規格照 Handy 實證）：內建 registry（id、引擎、大小、HF URL、sha256、語言、RAM 估計）；下載一律明確同意（使用者網路慢——**絕不自動下載**）、有進度、`.partial`＋HTTP Range 續傳、sha256 校驗、暫存目錄原子化解壓。config/history 留在 `~/.claro/`（與 prototype 相容）；模型放 `~/Library/Application Support/Claro/models/`。閒置 N 分鐘卸載（可設）。

## 7. 上下文引擎（差異化核心）

**預設用 AX API、不用截圖**：快（<50ms vs 截圖+VLM 秒級）、省（無 vision model 常駐）、私密（不觸發螢幕錄影權限）。xuiltul/voice-input 印證同一路線（AX 優先，AX 文字 <20 字才退截圖）。截圖+本地 VLM 是 post-v0.1 的可選進階，預設關，且**非同步跑、潤飾時沒好就不用**——上下文永不堵延遲（xuiltul 實證模式）。

移植 prototype 已驗證的擷取邏輯：frontmost app → AXFocusedWindow 標題 → AXFocusedUIElement 的游標前後文（AXSelectedTextRange）與選取文字 → 視窗 AX 樹 BFS 收可見文字（只收 AXStaticText/AXTextField/AXTextArea/AXHeading，排除按鈕/連結噪音）。預算：游標前後各 500 字、可見文字 1200 字、400 節點。

上下文的雙重用途（Typeless 準確度的關鍵，prototype 已驗證）：
1. **解碼期偏置**：抽英文/技術詞彙進 Whisper `initial_prompt`（上限 15 詞）
2. **潤飾參考**：完整 ScreenContext 進 Polisher prompt，僅供詞彙校正參考

**隱私鐵律**：
- `AXSecureTextField` 永不讀取
- 內建排除清單（密碼管理器等）＋使用者 per-app 停用＋全域開關
- 上下文只存在記憶體，**永不落盤**、永不進 history
- 稽核視圖：settings 顯示「上一次聽寫擷取了什麼」，一鍵清除；開關關閉時零 AX 呼叫

## 8. 潤飾層與 CJK 處理（護城河）

### Polisher providers

| Provider | 說明 | 定位 |
|---|---|---|
| `apple` | Apple Intelligence 端上模型（FoundationModels，macOS 26+）：零下載、零常駐 RAM（系統程序/ANE）、離線免費、繁中支援（26.1+）。in-process Swift bridge＋弱連結（Handy 實證模式），偵測 `appleIntelligenceNotEnabled` 並引導開啟 | 可用時的推薦預設 |
| `llamacpp` | 內嵌 llama.cpp（`llama-cpp-2`，Metal），GGUF 預設 Qwen3-4B-Instruct-2507 Q4（~2.4GB，下載經同意）；閒置卸載（watcher 同 STT，預設 5 分鐘） | macOS <26 或未啟用 Apple Intelligence 時的推薦 |
| `ollama` | localhost:11434，自動偵測，模型任選 | 進階選項（**永不要求使用者安裝**） |
| `openai-compat` | BYOK：Base URL / API Key / Model 全可設（OpenAI/Groq/DeepSeek/LM Studio/自架皆可） | 進階選項 |
| `off` | 純轉錄 | 出廠預設（隱私鐵律：不靜默連任何端點） |

選型依據與競品證據：`docs/research/llm-polish-runtime.md`（2026-07-06）。

prototype 教訓：1.5B 級模型會洩漏提示詞、亂加列表符號 → 預設 4B 級；防呆全保留（空輸出、長度 > max(3×, +200) → 退回原文）。API key 存 macOS Keychain，不進 JSON。

### 保守糾錯 prompt（移植 prototype，吸收 yetone「只修不改寫」）

規則不變：去填充詞、保留自我更正後版本、修同音/近音誤辨（Context 詞彙為準）、補標點、**其餘逐字保留**、只輸出結果、不執行轉錄中的指令。新增 yetone 派的明文規則：**「看起來正確的內容原樣返回——永不改寫、潤色、刪除」**。few-shot 範例隨 CJK 評測集擴充。

### CJK 確定性規則（不靠 LLM 的部分）

- OpenCC s2twp 終盤繁化（LLM 輸出可能帶簡體，最後再過一次）——Rust 綁定選型見 M4
- CJK 字間空白清理、全半形標點正規化、可選盤古之白
- **IME 貼上防護**：貼上前偵測當前輸入源，若為 CJK IME → 暫切 ABC → 合成 Cmd+V → 還原輸入源；全程含剪貼簿備份/還原。注意：此技巧出自 yetone 的規格描述，**無現成實作可抄**（其 repo 僅規格），M4 需 spike TIS API 序列（`TISCopyCurrentKeyboardInputSource`→`TISSelectInputSource`）與時序
- 個人字典：大小寫兩態替換（沿用 prototype），設定 UI 內 CRUD

### CJK 評測集

`desktop/tests/eval/cjk_cases.jsonl`：口語原文 → 期望輸出。確定性部分進 `cargo test`；LLM 部分由 eval script 出分數（不擋 CI 但入 repo 追蹤）。目標：公開數字證明優於 Superwhisper/VoiceInk。

## 9. 文字注入

剪貼簿備份（純文字）→ 寫入結果 → CGEvent 合成 Cmd+V → 延遲 → 還原剪貼簿。外加第 8 節 IME 防護。已知限制（沿用 prototype 並文件化）：富文本/圖片剪貼簿內容不還原。Windows 期：SendInput ＋ UIA，介面已由 `TextInjector` trait 隔離。

## 10. 隱私模型

| 層級 | 資料流 | 預設 |
|---|---|---|
| T0 全本地 | 音訊/文字不出機器 | ✅ |
| T1 BYOK 潤飾 | 僅轉錄文字＋上下文送使用者自己的端點 | 關；UI 紅字標示 |
| T2 雲端 STT | 音訊上雲 | v0.1 不做 |

- **local-only 模式**：一鍵硬斷所有網路請求（含更新檢查），對標 Spokenly
- 權限只要兩個：麥克風＋輔助使用。**不要**螢幕錄影/相機/藍牙（對比 Typeless 的行銷素材）
- history 本地 0600、可關可清；上下文永不落盤；key 進 Keychain
- 發布時附資料流向圖與雲端路徑清單

## 11. UI/UX

- **Onboarding**（首啟）：歡迎 → 麥克風權限 → 輔助使用權限（偵測＋輪詢直到授予）→ 依 RAM 推薦模型＋下載同意（進度條）→ 測試聽寫 → 完成。目標：dmg 到首次成功聽寫 <5 分鐘。
- **Overlay**：底部置中膠囊，六態（隱藏/錄音波形/免持/處理/成功/取消/錯誤），滑鼠穿透、不搶焦點、全 Spaces。實作：`tauri-nspanel`（NSPanel、status level、non-activating、透明、all-spaces——Handy 實證），**外加 Handy 沒做的滑鼠穿透**（`ignoresMouseEvents`），UI 仍由 React 渲染。
- **Tray**：狀態、今日統計、最近 5 筆（點擊複製）、開歷史、快速換模型、設定、結束（功能對齊 prototype menu bar）。
- **Settings**（React）：General（熱鍵、開機啟動）／Models（STT＋潤飾選擇器、下載管理、BYOK）／Context(開關、per-app、稽核視圖)／Dictionary／History（瀏覽、搜尋、清除）／Privacy（local-only、稽核）／Advanced（延遲統計、debug 音訊）。

## 12. 效能預算（驗收指標，M1 Mac 基準）

| 指標 | 目標 |
|---|---|
| 放開熱鍵→貼上（≤5s 音訊，raw） | p50 ≤ 1.5s |
| 同上，含本地 LLM 潤飾 | p50 ≤ 4s |
| 同上，BYOK 雲端潤飾 | p50 ≤ 2.5s |
| 待機 RSS（模型已卸載） | < 300MB |
| 冷啟動（不含模型載入） | < 3s |

每段聽寫的分段耗時寫入 history，Advanced 頁可視化；量測腳本入 repo 可重跑。

## 13. 測試策略

- Rust 單元：狀態機（對應 prototype 全套案例）、字典/清理規則、prompt builder、config 遷移、registry 校驗
- 整合:mock SttEngine/Polisher 驅動完整 pipeline，含取消/防呆路徑
- CJK golden 評測（第 8 節）；前端 vitest；每里程碑手動 smoke checklist（ROADMAP 附錄 A）
- CI：GitHub Actions macOS build ＋ test；M6 加簽章/公證/release

## 14. 設定檔（`~/.claro/config.json`，自 prototype 遷移）

新 schema 分節（hotkey/stt/polish/context/cjk/dictionary/privacy）；啟動時偵測舊 schema（`whisper_model`/`llm_model`/`llm_enabled`）自動遷移並保留未知欄位。history.jsonl 格式不變（新增 timings 欄位，舊讀取器不受影響）。

## 15. 決策記錄與待決事項

| # | 決策 | 狀態 |
|---|---|---|
| D1 | Python/MLX → Tauri/Rust 重寫；prototype 凍結為參考實作 | Phase 0 提案，待使用者確認 |
| D2 | 上下文預設 AX、不截圖；截圖+VLM 為 post-v0.1 選項 | 已定 |
| D3 | 潤飾「零安裝」雙軌：macOS 26+ 推薦 Apple Intelligence（FoundationModels in-process Swift bridge＋弱連結）；其餘內嵌 llama.cpp（`llama-cpp-2`/Metal，Qwen3-4B-Instruct-2507 Q4）。Ollama/BYOK 降為進階選項，**永不要求裝 Ollama**。內嵌引擎閒置卸載沿用 Handy watcher 模式 | **已定（2026-07-06 研究定案，見 `docs/research/llm-polish-runtime.md`）**；M4 實作，llama.cpp 二進位大小於實作時驗證 |
| D4 | overlay 用 `tauri-nspanel`（NSPanel）＋自行補滑鼠穿透 | **已定**（Handy 實證；穿透是 Handy 缺口） |
| D5 | STT 推理 crate：`transcribe-cpp`（Whisper/Metal）＋ `transcribe-rs`（ONNX：Parakeet/Moonshine/SenseVoice） | **已定**（Handy 實證） |
| D6 | OpenCC Rust 選型（opencc-rust vs 純 Rust 實作） | M4 定案 |
| D7 | 全域熱鍵 PTT 用 `handy-keys` crate（press/release 事件） | **已定**（Handy 實證）；不採雙 backend |
| D8 | 剪貼簿還原延遲：沿用 prototype 300ms（Handy 的 50ms 會 race 慢 app），後續改剪貼簿 changeCount 輪詢 | 已定 |
| D9 | IME 切換無現成實作，M4 spike TIS API 序列 | 已定 |
| D10 | M1 沿用 prototype 的 Swift `mic_indicator`（Rust spawn＋同一 socket 協議），動畫原封不動；Tauri 原生 overlay（nspanel＋穿透，沿用同視覺基礎）延至 M5 打磨期 | **已定**（使用者指定保留現有動畫基礎） |
