# Claro 競品定位矩陣

日期：2026-07-05。資料來源：各產品官網/GitHub/第三方評測（連結見文末），開源專案另有原始碼分析佐證。

## 一句話定位

**Claro = 本地優先隱私 × 真上下文感知潤飾 × STT/LLM 全可插拔 × CJK 做到最好** —— 現有產品沒有任何一家四項全包。

## 主矩陣

| 產品 | 隱私 | 上下文感知 | 模型彈性 | CJK / 中英混合 | 定價 | 平台 | 開發者友善 |
|---|---|---|---|---|---|---|---|
| **Claro（目標）** | ✅ 預設全本地、可離線；雲端路徑明確標示可關；local-only 斷網模式 | ✅ AX API 抓可見文字/選取/游標前後文，餵 STT 偏置＋LLM 潤飾；可開關、可稽核 | ✅ STT（Whisper 家族/Moonshine/Parakeet）與 LLM（內嵌 llama.cpp / Ollama / BYOK OpenAI-compatible）雙層可插拔 | ✅ 主打：IME 貼上防護、保守糾錯（配森→Python）、OpenCC 繁化、中英混排規則、詞彙偏置 | 開源免費核心 | macOS 先行，架構跨平台（Tauri） | 規劃：IDE 情境優化、之後 MCP |
| **Typeless**（對標） | ❌ 行銷稱 on-device，實際音訊上 AWS 雲端處理；蒐集瀏覽 URL、視窗標題、剪貼簿；要求螢幕錄影/相機/藍牙等過廣權限；無離線 | ✅ 強（雲端做）：視窗標題、app 情境、tone 隨 app 調整 | ❌ 無 BYOK、模型不可選 | ⚠️ 多語佳但無 CJK 特化；雲端大模型潤飾中文尚可 | Free 8k 字/週；Pro $30/月（年繳 $144 = $12/月） | macOS / Windows / iOS / Android | ❌ |
| **Wispr Flow** | ❌ 雲端處理 | ⚠️ app 感知 tone | ❌ | ⚠️ 多語但無特化 | Free 2k 字/週；Pro $15/月（年繳 $12/月） | Mac/Win/iOS/Android | ❌；Electron 待機 ~800MB RAM、閒置 8% CPU（反面教材） |
| **Superwhisper** | ✅ 核心 on-device（whisper.cpp），亦有雲端模式 | ⚠️ 有 context（選取文字/剪貼簿等）餵 AI modes | ✅ 多 STT 模型；AI modes 可接本地/雲 LLM | ⚠️ Whisper 百語含中文，無 CJK 深度處理 | Free 受限；Pro $8.49–9.99/月；lifetime 已漲到 $849 | Mac / Win / iOS | ⚠️ |
| **VoiceInk**（最接近的開源對手） | ✅ 100% 本地轉錄（whisper.cpp / FluidAudio Parakeet）；增強可選雲端（僅送文字） | ✅ 有 "Context Aware"（螢幕內容感知）＋ per-app Power Mode | ⚠️ STT 本地多模型；增強靠 BYOK（Groq 等）/本地 | ❌ 無 CJK 特化、無 IME 處理 | 開源 GPL-3.0；App $39.99 買斷 | macOS（14.4+，Apple Silicon） | ⚠️ 助理模式 |
| **Voibe** | ✅ 全本地 Whisper | ❌ 無螢幕上下文 | ❌ 固定 Whisper | ⚠️ 無特化 | $7.5/月、$59/年、$149 買斷 | macOS | ✅ 開發者模式（VS Code/Cursor 檔名/指令辨識） |
| **Spokenly** | ✅ 本地模型免費＋local-only 斷網模式；雲端 BYOK | ❌ | ✅ STT 彈性佳（Whisper turbo、Parakeet v2/v3、BYOK 雲端）；LLM 潤飾弱 | ⚠️ 無特化 | 本地免費無限；雲端才收費 | Mac / iPhone / Windows | ✅ 有 MCP |
| **Handy**（開源架構標竿） | ✅ 純本地、開源 | ❌ 刻意不做（哲學：只做轉錄） | ⚠️ STT 多模型（Whisper/Parakeet/Moonshine）；**刻意無 LLM 潤飾** | ❌ | 免費開源（MIT） | Mac / Win / Linux（Tauri + Rust） | ⚠️ |
| **murmur** | ✅ 本地 STT（遠端失敗自動退本地） | ❌ | ⚠️ STT 本地+OpenAI-compatible；polish 有 Heuristic/Ollama/Cloud | ❌ 無 IME 處理 | 免費開源 | Tauri（macOS） | ❌ |
| **yetone/voice-input** | ✅ 本地/BYOK | ⚠️ 少量 | ⚠️ | ✅ **唯一明確處理 CJK IME 貼上問題**（惟公開 repo 僅規格文件，無實作源碼）；「正確就原樣返回」保守糾錯哲學 | 免費開源 | macOS | ⚠️ 作者自用工具 |
| **xuiltul/voice-input** | ✅ 本地（Ollama 生態） | ✅ AX 優先＋截圖→VLM 後備；vision 非同步不堵潤飾 | ⚠️ faster-whisper＋Ollama/OpenAI 格式 | ⚠️ 預設日文 prompt，無 IME 處理 | 免費開源 | macOS（Python，需自架 server） | ⚠️ |

## 我們要贏的欄位與打法

1. **隱私（vs Typeless/Wispr）**：Typeless 的「on-device」只指歷史存本地、音訊實際上雲（第三方逆向已證實），且權限過廣。Claro 主打「音訊預設不出機器、權限最小化、雲端路徑全部可見可關」。這是行銷上最鋒利的一刀。
2. **上下文感知（vs 所有本地派）**：Handy/Voibe/Spokenly 全都沒有螢幕上下文；VoiceInk 有但黑盒。Claro 用 AX API（比截圖快、省、私密）＋「上下文可視化/可稽核」做出差異：使用者能看到這次聽寫抓了什麼、隨時關掉、per-app 排除。
3. **模型彈性（vs Typeless/Wispr/Voibe）**：STT 與 LLM 都是 trait 層可插拔；BYOK OpenAI-compatible 讓進階使用者接任何雲端；離線使用者用內嵌 llama.cpp 或 Ollama。對齊 Spokenly 的 STT 彈性，再補上它沒有的 LLM 潤飾層。
4. **CJK / 中英混合（護城河，無人認真做）**：
   - IME 攔截 Cmd+V 問題（yetone 技巧：貼上前暫切 ABC 輸入源，貼完還原）
   - 保守糾錯 prompt（同音/近音誤辨：配森→Python、傑森→JSON；只修不改寫）
   - OpenCC s2twp 確定性繁化終盤
   - 中文標點/全半形/盤古之白規則
   - 螢幕詞彙餵 Whisper initial_prompt 做解碼期偏置（不只事後修）
   - 公開 CJK 評測集，用數字證明比 Superwhisper/VoiceInk 準
5. **定價**：核心開源免費（vs Typeless $30/月、Superwhisper lifetime $849）。

## 各競品借鏡 / 避開

| 來源 | 借鏡 | 避開 |
|---|---|---|
| Typeless | 上下文感知的產品體驗、tone per app、選字編輯 UX | 假 on-device 行銷、過廣權限、訂閱鎖定 |
| Handy | Tauri+Rust 架構、模型管理/下載、打包簽章 auto-update、跨平台佈局、輕量體感 | 「不做潤飾」的哲學留給我們切入 |
| murmur | audio/transcribe/polish/inject 四模組切法、polish 永不外拋錯誤、遠端退本地 | 功能太陽春、prompt 不夠保守 |
| yetone | CJK IME 切換技巧（規格）、「正確就原樣返回」prompt 哲學 | 無實作可抄，IME 需自行 spike |
| xuiltul | AX 優先＋截圖後備分層、vision 非同步不堵延遲、串流 partial 轉錄 | Python 自架 server 的部署複雜度 |
| VoiceInk | 買斷/開源商業模式、Power Mode per-app、個人字典 | GPL 授權傳染性、macOS-only 綁死 Swift |
| Spokenly | local-only 斷網模式、本地免費策略、MCP | 潤飾層薄弱 |
| Wispr Flow | — | Electron 待機 800MB / 8% CPU；我們用 Tauri 並設記憶體預算 |

## 資料來源

- Typeless 隱私逆向與定價：[Voibe: Typeless privacy issues](https://www.getvoibe.com/resources/typeless-privacy-issues/)、[Typeless review](https://www.getvoibe.com/resources/typeless-review/)、[Weesper: Typeless review](https://weesperneonflow.ai/en/blog/2026-06-11-typeless-review-cloud-dictation-2026/)
- VoiceInk：[GitHub](https://github.com/beingpax/VoiceInk)、[官網](https://tryvoiceink.com/)、[評測](https://www.getvoibe.com/resources/voiceink-review/)
- Spokenly：[官網](https://spokenly.app/)、[local-only mode](https://spokenly.app/docs/local-only-mode)、[pricing](https://spokenly.app/pricing)
- Voibe：[pricing](https://www.getvoibe.com/pricing/)、[Weesper 評測](https://weesperneonflow.ai/en/blog/2026-06-04-voibe-review-offline-mac-dictation-2026/)
- Wispr Flow：[pricing](https://wisprflow.ai/pricing)、[評測（RAM 數據）](https://www.getvoibe.com/resources/wispr-flow-review/)
- Superwhisper：[官網](https://superwhisper.com/)、[定價分析](https://www.speakmac.app/blog/speakmac-vs-superwhisper-comparison)
- STT 模型語言支援：[Parakeet-tdt-0.6b-v3（25 歐語、無 CJK）](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3)、[Moonshine（含 zh/ja/ko）](https://github.com/moonshine-ai/moonshine)
- 開源專案原始碼分析：`docs/research/handy-analysis.md`、`docs/research/small-competitors-analysis.md`
