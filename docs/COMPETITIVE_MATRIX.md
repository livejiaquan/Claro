# Claro 競品定位矩陣

日期：2026-07-12。第一方產品文件優先；無公開證據的內部實作一律標為未知。開源專案另以 `docs/research/` 的原始碼分析交叉核對。

## 一句話定位

**Claro 要贏的不是「也能語音轉文字」，而是 Meaning Lock × 可稽核上下文 × CJK-first × Mac 原生零外部 runtime（不需安裝 Ollama）。**

市場基準已經提高：Superwhisper、Spokenly、VoiceInk 都有 context/mode；Handy 也已有 alpha post-processing。不能再以「其他本地產品沒有上下文或 LLM」作為定位前提。

## 目前能力矩陣

| 產品 | 隱私與執行位置 | 上下文 | App／格式策略 | LLM 潤飾 | 首次使用 |
|---|---|---|---|---|---|
| **Claro（目前）** | STT 預設本地；Apple Intelligence 或 app 內 llama.cpp；雲端需 endpoint-bound consent；local-only 預設開 | AX：App、視窗、游標周邊、選取、可見文字；keyDown 綁 session；密碼欄與常見密碼管理器跳過；本次內容可在設定稽核；另以 salted metadata hash 阻擋可辨識的同 App 目標切換 | 可解釋的 Email／Message／Document／Technical／Neutral surface guidance；未知 App 不猜；尚無個人 tone learning 或使用者自訂 per-app 規則 | RAW／CLEAN／ORGANIZE；事實 guard、明確改口、完全重複合併、raw/final/fallback 稽核 | 依 RAM／架構推薦；Apple 可用就免下載，否則明確同意下載 app 內模型；不要求 Ollama／Homebrew |
| **Typeless 2.0** | 官方文件宣稱音訊與有限 context 在雲端即時處理、零保留，會使用第三方 LLM | 官方只明示使用中的 App 與其中相關文字；具體 OS API、長度與時序未公開 | 不同 App 自動 tone；個人風格學習；email/list 自動格式化 | 旁註只作 context、任意順序整理、描述名稱後 grounded fill、晚點改口只留最終決定 | 官方 UI／文件未提供模型選擇；權限、mic/hotkey 與三個示範流程 |
| **Superwhisper** | 本地／雲端 STT；本地 llama.cpp／雲端 LLM | selected text、clipboard、app context；History 可看送入 AI 的 prompt | 內建 Message／Email／Note／Meeting／Super；可依 App／網站自動切 Mode | 本地與雲端 LLM 成熟；官方也警告過多 context 會 hallucinate | 依硬體推薦模型、guided first dictation；零技術門檻的主要參考 |
| **Spokenly** | 本地模型；Local Only Mode 阻斷非 localhost 網路 | Mode 可使用 clipboard、focused app、cursor text、active browser URL | App／網站自動啟用；各 Mode 有獨立 prompt、模型、provider 與 output action | Built-in、BYOK、OpenAI-compatible | 基本流程是按住、說話、放開；進階能力藏在 Modes |
| **VoiceInk** | 本地 STT 預設；雲端轉錄／增強 opt-in | Mode 可選 selected text、clipboard、screen OCR；送 provider 的是文字 | App、網站、口語 trigger、shortcut 自動切 Mode | Custom prompt；本機增強以 Ollama／Local CLI 為進階路徑 | 有 onboarding 與推薦模型；History 可看 original/enhanced、prompt、timings、硬體 |
| **Handy** | 開源、全本地 STT | 官方 post-processing 文件未見 App/window context | 無 app-aware tone | **已有 alpha post-processing**：Apple Intelligence、雲端 provider、custom endpoint；一般本機 LLM 可接既有服務 | 首啟只授權、選模型、下載、開始使用，簡單直接 |

## Typeless：可證實與不可證實

官方文件明示或宣稱：

- 音訊、使用中的 App 與 App 內相關文字會送到雲端即時處理。
- 不同 App 使用不同 tone，並可選擇個人化風格。
- 選取文字後可用語音改寫、格式調整、摘要或翻譯。
- Typeless 2.0 支援旁註、晚期改口、任意順序與名稱指涉。

官方未公開：

- 是否使用 AX、截圖、OCR、clipboard、URL 或視窗標題。
- Context 擷取時機、字數預算、排序、模型、prompt、schema 與 verifier。
- per-app tone 是 bundle mapping、分類器或個人化模型。

Typeless 條款禁止反組譯、反編譯與 reverse engineering，並限制使用其 AI Output 開發或改進 AI/ML model。Claro 的邊界是**人工產品行為評測**：以相同測試意圖觀察 Mail、Slack、文件、IDE、表單的可見輸出；不分析 proprietary binary、不攔截私有流量，也不把 Typeless 輸出放進訓練集、fine-tune、prompt distillation 或自動資料抽取。若未來要擴大測試，需先確認授權與條款。

## 對標行為評測

固定語料至少包含：

1. 數字、日期、否定、條件與待辦。
2. 明確改口、跳序、完全重複、旁註。
3. 畫面專有名詞、錯誤 Context、Context prompt injection。
4. Email、Chat、Document、IDE 四種 surface。
5. 中英混講、繁體標點、emoji、注音／拼音／日文 IME。
6. 切 App、失焦、無網路、模型卸載、LLM timeout。

核心指標：事實錨點保存率、內容覆蓋率、context hallucination rate、app-format 命中率、貼上成功率、cold/warm p50/p95、峰值 RAM。

## Claro 的突出方向

1. **Meaning Lock**：CLEAN 的 LLM 不得修改任何文字／英數 token，只能移除有界填充詞、處理明確改口與補不改變語氣的標點；姓名、術語、數字、時間、否定、條件與因果不可漂移，失敗回原文。專有詞校正由 STT Context 偏置與使用者字典負責。
2. **Inspectable Context**：使用者看得到本次 App 類型與讀到的內容，能關閉、清除；上下文不落盤。
3. **CJK-first**：Context 詞彙進 STT 解碼；完整 Context 只在 ORGANIZE 提供版面判斷，CLEAN 不讀 Context 改字；另有 OpenCC、繁中標點、字典與 IME 評測。
4. **Mac-native zero-tech local polish**：Apple 系統端上模型或 Claro app 內模型；Ollama／LM Studio／BYOK 只在進階區。
5. **開源與可驗證**：公開行為 fixture、效能數字與 fallback 原因，不靠「AI 很聰明」的黑箱宣稱。

## 第一方來源

Typeless：

- [Data Controls](https://www.typeless.com/data-controls)
- [Privacy Policy](https://www.typeless.com/privacy)
- [Typeless 2.0 macOS release notes](https://www.typeless.com/help/release-notes/macos)
- [Installation & Setup demos](https://www.typeless.com/help/installation-and-setup)
- [Terms](https://www.typeless.com/terms)

Superwhisper：

- [Context Awareness](https://superwhisper.com/docs/common-issues/context)
- [Modes](https://superwhisper.com/docs/modes/modes)
- [Local and cloud models](https://superwhisper.com/models)
- [Context hallucinations](https://superwhisper.com/docs/common-issues/hallucinations)

Spokenly：

- [Modes and context](https://spokenly.app/docs/modes)
- [Local Only Mode](https://spokenly.app/docs/local-only-mode)

VoiceInk：

- [Context Awareness](https://tryvoiceink.com/docs/context-awareness)
- [Mode Triggers](https://tryvoiceink.com/docs/mode-triggers)
- [Recommended Models](https://tryvoiceink.com/docs/recommended-models)
- [Transcription History](https://tryvoiceink.com/docs/transcription-history)

Handy：

- [Getting Started](https://handy.computer/docs)
- [Post-Processing](https://handy.computer/docs/post-processing)
- [Official GitHub](https://github.com/cjpais/Handy)
