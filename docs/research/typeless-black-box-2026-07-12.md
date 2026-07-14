# Typeless 2.0 與 Mac 語音輸入產品研究

日期：2026-07-12。研究只使用公開行為、官方文件與開源原始碼；不反組譯 proprietary binary、不攔截私有流量。Typeless 的可見輸出只作人工產品評測，不進訓練集、fine-tune、prompt distillation 或自動資料抽取；擴大使用前需另行確認授權與條款。

## 1. Research Question

Claro 如何在不要求 Ollama、維持本地優先與 Meaning Lock 的前提下，達到 Typeless 等產品的上下文理解、改口處理、App-aware formatting 與安裝後可用體驗？

## 2. Scope and Context

比較 Typeless 2.0、Superwhisper、Spokenly、VoiceInk 與 Handy。焦點：Context 資料、擷取時序、App/Mode、LLM 潤飾、硬體推薦、隱私與 onboarding。

## 3. Key Findings

1. Typeless 的產品基準已不是文法校正，而是 discourse-level composition：旁註、晚期改口、任意順序與名稱指涉。
2. Superwhisper、Spokenly、VoiceInk 都已有 App/Mode/context 控制；Handy 也已有 alpha post-processing。舊的「本地派沒有 Context/LLM」判斷失效。
3. Claro 的可突出組合是 Meaning Lock、可稽核 AX Context、CJK-first 與零外部 runtime；不是單靠「本地」。
4. Typeless 的具體 Context API、模型與 verifier 未公開，不能聲稱逆向證實。
5. Claro 原先的 guard 會拒絕「7am，不對，3pm」與實質去重；本輪加入具停頓邊界的明確改口 scope、連續改口、否定反例、完全重複合併與列表格式 fixture。`不對外`、`不要改成` 不會被當成改口。

## 4. Evidence Summary

- Typeless 官方明示音訊、使用中 App 與有限相關文字在雲端即時處理，並提供不同 App tone、個人化與 selected-text editing。
- Typeless 2.0 release notes 明示 side note、late correction、arbitrary order 與 grounded name fill。
- Superwhisper 官方列出 selected text、clipboard、app context、App/site rules 與 app 內 local llama.cpp LLM，也明說 Context 時序可能抓錯 App、過量 Context 可能 hallucinate。
- Spokenly Modes 可帶 clipboard、focused app、cursor text、active browser URL，並可依 App/site 自動啟用。
- VoiceInk Mode 可使用 selected text、clipboard、screen OCR，並以 App/site/voice trigger/shortcut 自動切換。
- Handy 官方目前已有 Apple Intelligence、雲端 provider 與 custom endpoint 的 alpha post-processing。

## 5. Comparative Analysis

| 能力 | 市場成熟做法 | Claro 2026-07-12 |
|---|---|---|
| 改口 | Typeless 只保留 final decision | 明確 marker 限定可刪舊 span；其餘錨點仍鎖定 |
| App 格式 | 固定 Mode、App/site trigger 或黑箱 tone | 可解釋 Email/Message/Document/Technical/Neutral；未知不猜 |
| Context | selected/clipboard/app/OCR/URL 組合 | AX App/window/cursor/selected/visible；keyDown 綁 session；salted metadata hash 阻擋可辨識的同 App 目標切換 |
| Context 信任 | 多數產品只有開關或 Mode 設定 | 本次 Context 在設定可查看、清除，不落盤 |
| 本地 LLM | Superwhisper app 內 llama.cpp；Handy Apple 路徑 | Apple Intelligence 或 Claro app 內 llama.cpp |
| 低 RAM | 競品依硬體建議不同模型 | 8–12 GB/Intel 用量化 STT，STT→LLM 分階段載入 |

## 6. Tradeoffs

- Context 越多，專有名詞命中率越高，但 hallucination 與隱私風險也越高；Claro 採 bounded AX text、敏感 App denylist、session binding 與可稽核記憶體快照。
- 嚴格 Meaning Lock 會犧牲部分自由改寫；這是 Claro 有意的產品差異。CLEAN 禁止 LLM 取代任何文字／英數 token，專有詞校正只走 STT Context 偏置與使用者字典；ORGANIZE 只放寬明確改口、完整句讀重排、完全重複與排版，不以低相似度猜等價。
- 8 GB 機器若同時留住 STT/4B LLM，延遲較低但容易 memory pressure；分階段載入降低峰值，下一次 STT 在錄音時回載。
- 自動下載能縮短步驟，但違反慢網路與明確同意邊界；Claro 改成單一推薦與「下載並使用」，不靜默下載。

## 7. Recommended Approach

1. 保留 RAW/CLEAN/ORGANIZE 三契約，CLEAN 不做 tone 改寫，ORGANIZE 才使用 surface guidance。
2. keyDown 建立 ContextSnapshot 與獨立貼上目標指紋；放開後最多等 Context 250 ms，內容目標不符就丟棄。處理中切 App、視窗或焦點時不自動貼上；History 關閉時以 process-memory pending result 供複製救回。
3. 以可解釋 App 分類先達到 email/message/document/technical formatting；個人 tone learning 延後且需 opt-in。
4. Apple Intelligence 可用時走系統端上模型；否則 app 內 Qwen 4B。Ollama/LM Studio/BYOK 只放進進階區。
5. 用公開 fixture 比較 Meaning preservation、Context hallucination、format hit rate 與 p50/p95，不做 proprietary reverse engineering。

## 8. Alternative Approaches

- **完全自由的 LLM rewrite**：輸出更像 Typeless，但事實漂移風險不符合 Claro 定位。
- **截圖＋VLM**：能理解非 AX UI，但需螢幕錄影權限、較慢且耗電；只適合 post-v0.1 opt-in 後備。
- **要求 Ollama**：開發者彈性高，但一般使用者門檻不合格；只保留既有服務連接。
- **全部打包模型**：安裝即離線，但 App 體積與更新成本過高；維持明確同意的一鍵下載。

## 9. Implementation Considerations

- Correction guard 必須保留 correction scope 之外的否定、數字、日期與條件。
- Context 不得寫入 History；稽核與貼上目標 hash 只存於本次 process memory。
- 8/16/32 GB policy 需純函數 fixture，加上實機 `system_profiler`/Instruments 驗收。
- 本機/雲端 provider 逾時要落回 deterministic base text；內建 llama 每 token 檢查 cancel。
- App-aware formatting 不得新增稱呼、主旨、標題、分類或 code fence。

## 10. Open Questions / Unknowns

- Typeless 使用 AX、OCR、clipboard 或 URL 的具體組合未知。
- Typeless 2.0 是否使用 clause timestamps、多階段 planner 或 structured provenance 未公開。
- Claro 在 8 GB Intel、8 GB Apple Silicon、16/32 GB 的真實 peak footprint、thermal 與 battery 仍需多機實測。
- 中文姓名 Context bias、注音/拼音/日文 IME 與跨 App paste guard 仍需原生實機矩陣。

## 第一方來源

- Typeless：[Data Controls](https://www.typeless.com/data-controls)、[2.0 release notes](https://www.typeless.com/help/release-notes/macos)、[Installation demos](https://www.typeless.com/help/installation-and-setup)、[Terms](https://www.typeless.com/terms)
- Superwhisper：[Context](https://superwhisper.com/docs/common-issues/context)、[Modes](https://superwhisper.com/docs/modes/modes)、[Models](https://superwhisper.com/models)、[Hallucinations](https://superwhisper.com/docs/common-issues/hallucinations)
- Spokenly：[Modes](https://spokenly.app/docs/modes)、[Local Only](https://spokenly.app/docs/local-only-mode)
- VoiceInk：[Context](https://tryvoiceink.com/docs/context-awareness)、[Mode Triggers](https://tryvoiceink.com/docs/mode-triggers)、[History](https://tryvoiceink.com/docs/transcription-history)
- Handy：[Post-Processing](https://handy.computer/docs/post-processing)、[GitHub](https://github.com/cjpais/Handy)
