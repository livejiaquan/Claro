# murmur / yetone / xuiltul 原始碼分析

日期：2026-07-05。方法：shallow clone 後純本地閱讀（Codex 執行）。

## murmur（kurenn/murmur）

- 模組切分乾淨：`audio.rs`（cpal 擷取＋電平事件）／`transcribe.rs`（whisper-rs 本地＋OpenAI-compatible 遠端 STT）／`polish.rs`／`inject.rs`（arboard+enigo），`lib.rs` 當 orchestrator，各模組只露少量進入點。
- Polish providers：`Heuristic`／`Ollama`（預設 llama3.2:3b）／`Cloud`（OpenAI-compatible）／`Auto`／`Off`；**polish 永不外拋錯誤**，失敗退 heuristic、空輸出退原文——與我們的 Polisher 合約一致。
- 遠端 STT 失敗自動退本地。溫度 0.2。
- 限制：無 CJK IME 處理；prompt 允許改文法/大小寫，不是「只修 ASR 錯誤」的保守派。

## yetone/voice-input-src

- **重要發現：這個 repo 只有一份作為 Claude prompt 的 README 規格，沒有實作原始碼**（`dist` 是指向另一 repo 的 submodule）。
- IME 技巧以「需求」形式描述：貼上前暫切 ASCII 輸入源（ABC/US），貼完還原；剪貼簿備份/還原亦然。**具體 TIS API 呼叫序列與時序需要我們自己實作驗證**（候選：`TISCopyCurrentKeyboardInputSource` → `TISSelectInputSource(ABC)` → 貼上 → 還原）。
- 潤飾哲學（README 原文精神）：「只修明顯的語音辨識錯誤……看起來正確的內容原樣返回，永不改寫、潤色或刪除」——比我們 prototype 的 prompt 更保守，值得吸收。
- STT 偏好 Apple Speech Recognition 串流；LLM 走 OpenAI-compatible 設定。

## xuiltul/voice-input

- **上下文策略與我們同路線且互相印證：AX 優先、截圖只當後備**。`MIN_AX_TEXT_LEN=20`：AX 抽到 ≥20 字就用文字上下文，否則才 `screencapture` 截前景視窗餵 vision model。
- AX 走訪參數：MAX_DEPTH=15、MAX_ELEMENTS=500、TIMEOUT_MS=200、**跳過 AXSecureTextField**、行去重。（我們 prototype：400 節點/1200 字預算，同量級。）
- Vision：Ollama `qwen3-vl:8b-instruct`，temperature 0.1，prompt 要求辨識作用中分頁/窗格、逐字轉錄主體文字、忽略非作用 UI、註記游標位置。**vision 非同步跑，最終潤飾時沒好就直接不用**——不讓上下文堵延遲，值得抄。
- 潤飾：語言別 JSON prompt（去填充詞、修明顯辨識錯、保留全部內容、不用 Markdown、不回答內容），few-shot 訊息，`context_prefix` 注入上下文；LLM 預設 glm-flash（Ollama/OpenAI 雙格式）。
- STT：faster-whisper large-v3-turbo；串流每 2s 送累積 WAV 出部分轉錄（partial）；靜音 dB 門檻跳過轉錄；串流 chunk 關 VAD、最終 pass 開 VAD，VAD 吃光但有 partial 時退回無 VAD 重轉。
- 另有 hotword cache（專案術語 24h TTL）但主路徑未接上——想法可參考（個人字典的自動化來源）。

## 對 Claro 設計的影響

1. Polisher「永不失敗」合約獲 murmur 印證；保守糾錯 prompt 向 yetone 靠攏（「正確就原樣返回」明寫進規則）。
2. AX 優先＋截圖後備的分層獲 xuiltul 印證；後備觸發條件（AX 文字太少）與「vision 不阻塞潤飾」直接採用。
3. IME 切換沒有現成實作可抄，M4 需自行 spike TIS API 序列與時序。
4. 串流 partial 轉錄（2s 累積）是 post-v0.1 降延遲的可行路徑。
