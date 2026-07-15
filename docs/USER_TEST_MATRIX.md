# Claro 使用者旅程與極端情境測試矩陣

日期：2026-07-15。下表的 `Automated` 與 `Native` 是**驗證方法／目標，不是已通過狀態**；實際完成狀態以 `ROADMAP.md` 進度與驗收紀錄為準。`Automated` 表示 repo 內可重跑，`Native` 表示必須在 Developer ID 簽章 `.app` 與真實 Mac/App/IME 上驗證。模擬回饋是以流程與程式碼推演，不冒充真人訪談。Accuracy Pass 已用本機 ad-hoc 簽章的 release bundle 實際檢查 Home／Onboarding／Settings／History 與 sidecar，沒有白屏、截字或阻斷 log；沒有出聲、沒有觸發熱鍵，因此不能替代真實麥克風、貼上與正式發行簽章驗收。

## 使用者角色

| 角色 | 主要期待 | 最可能的負面回饋 | Claro 對策 |
|---|---|---|---|
| 完全不懂技術者 | 安裝後照提示完成 | 「Ollama、Whisper、Qwen 是什麼？」 | Onboarding 只顯示這台 Mac 的推薦方案；外部服務收進進階 |
| 8 GB Apple Silicon | 不要卡頓或 swap，也不要為省資源犧牲太多準確率 | 「速度快，但人名與技術詞一直錯」 | 完整 large-v3 Q5；STT→LLM 分階段載入 |
| 8 GB Intel | 能用且預期清楚 | 「等很久卻不知道是否壞掉」 | compact policy、清楚原樣轉錄 fallback；Intel 真機尚待驗收 |
| 16/32 GB Pro | 速度與品質最大化 | 「硬體很多但 App 還在用 Turbo」 | 完整 large-v3；Apple 可用時優先系統整理；LLM prewarm 尚未接入，不能宣稱已極致 |
| 隱私敏感者 | 看得到資料流 | 「畫面被讀了什麼？」 | Context 本次稽核、清除、denylist、local-only、history toggle |
| Apple Intelligence 不可用 | 不裝第三方工具 | 「是不是要裝 Ollama？」 | 一鍵下載 Claro 內建整理；不需 Homebrew/Ollama |
| 慢網路／磁碟接近滿 | 可續傳、不浪費流量 | 「失敗後是不是重來？」 | Range partial、SHA-256、磁碟預檢、持久錯誤與續傳文案 |
| VoiceOver／鍵盤使用者 | 控制可讀、焦點清楚 | 「狀態只靠動畫或顏色」 | semantic headings/labels、ARIA live/meter、focus-visible、reduced-motion；原生 overlay 仍需實測 |
| 中英混講開發者 | 專案詞、姓名、email 與識別字正確 | 「句子通順，但關鍵 token 一錯就不能用」 | 完整 large-v3、AX ASCII+CJK 詞彙偏置、確定性字典、Meaning Lock；email／URL 仍列真人 corpus gate |

## 核心旅程

| # | 旅程 | Automated | Native | 通過條件 |
|---|---|---:|---:|---|
| 1 | 新安裝 8 GB，權限→mic→推薦 STT→內建整理 | UI QA + policy tests | 8 GB AS | 不出現 Ollama/CLI 要求；明確同意下載；effective CLEAN |
| 2 | 16 GB + Apple Intelligence 可用 | UI QA + policy tests | 16 GB AS/macOS 26+ | Apple 為推薦；免額外模型下載 |
| 3 | 權限拒絕後恢復 | command/UI static | clean account | 有 Accessibility/Microphone 系統設定入口，免重啟接回 hotkey |
| 4 | 第一次聽寫 | completion backend gate | TextEdit | 未成功 paste 前不能完成；成功後才寫 setup_completed |
| 5 | 明確改口 7am→3pm | guard fixture | 多 provider | 只保留 3pm；其他句子的否定/待辦不丟 |
| 6 | 跳序、完全重複、購物清單 | guard fixture | Apple/Builtin | 可重排、合併完全重複、明確列舉可用 bullets，不新增標題 |
| 7 | 跨 App Context／焦點切換 | fingerprint equality＋pending queue tests | Mail→Slack／Cursor | keyDown App id 不同時丟棄 Context；paste target 不同時不自動貼上；結果依序進 process-memory recovery queue |
| 7b | 同 App 視窗／欄位切換 | full-fingerprint Context regression＋同批 AX refs static | 同 App 兩視窗／兩欄位 | metadata hash 不同時不自動貼上；ContextSnapshot fingerprint 與 keyDown 目標一致；真實 App 的 AX metadata 可辨識率仍待矩陣 |
| 8 | 密碼管理器與 secure field | denylist tests | Passwords/1Password | 零 Context 內容；仍可純聽寫 |
| 9 | 空白、太短、STT/paste 失敗 | History UI QA | fault injection | History 顯示錯誤與恢復資訊，不再把空文字紀錄隱藏 |
| 10 | 下載中斷與磁碟不足 | downloader tests/UI QA | throttled network/full disk | request 前預檢；重試續傳；hash 不符不載入 |
| 11 | 富文字/圖片/檔案剪貼簿 | NSPasteboard tests | Office/Finder | 完整還原；使用者中途 Copy 不被覆蓋 |
| 12 | Esc 取消 | state/llama cancel＋pre-Cmd+V guard tests | Apple/Builtin/HTTP | overlay 立即取消；builtin 檢查 cancel、6 秒 generation／8 秒 total 邏輯預算；HTTP/Apple 最遲 5 秒返回；50ms clipboard settle 後仍重驗 session／focus |
| 13 | 隱藏視窗待機 | build/static | Instruments 10 min | hidden 時停止 2 秒 polling；模型卸載後 RSS <300 MB |
| 14 | 注音/拼音/日文 IME | 尚缺 | 三種 IME | 貼上成功、輸入源正確還原、無重複字 |
| 15 | 快速連續兩段／上一段晚到 | state stale-session＋session-slot＋stale-watchdog regressions | TextEdit／Slack | 舊 session 不得取走、覆蓋或貼出新 session 的 target/context/result |
| 16 | 5 分鐘 force-stop | state-machine＋stale-watchdog generation test；300 秒 wall test 尚缺 | 長錄音 | 到上限後只送一次 force-stop、只處理一次、狀態回 IDLE、overlay 不殘留 |
| 17 | History 關閉＋失焦／貼上失敗 | FIFO pending queue regressions＋History UI QA | 任意 App | 不落盤；多段結果依序保留，可在本次執行期間複製或捨棄，不被後續成功／失敗覆蓋 |
| 18 | config/history 損毀或磁碟寫滿 | parser tests；併發 locking／disk-full regression 尚缺 | fault injection | config 安全回預設；History 壞行跳過；落盤失敗不影響聽寫與救援文字 |
| 19 | 睡眠喚醒／錄音中切麥克風路由 | 尚缺 | AirPods/USB | 不 crash、不保留幽靈錄音；舊 mic test 不得替新裝置通過 gate |
| 20 | keyDown 遇到遲緩／無回應 AX App | 40ms per-call／約250ms batch deadline static；fake-delay regression 尚缺 | 可控制 AX 延遲的測試 App | keyDown 只固定 focus reference（不讀內容／逐項 metadata）；keyDown→錄音提示 p95 ≤150ms；後續 target/context 與 paste-time fingerprint 各約 ≤250ms，逾時 fail closed |
| 21 | 既有 Turbo 使用者收到精準度建議 | tier gate＋UI QA | Home→Settings | 只有推薦模型的 accuracy tier 真正更高才顯示；CTA 不下載；dismiss 可記住；Intel Turbo→Turbo Q5 不得誤稱升級 |
| 22 | 台灣姓名／產品名／email／URL | 20 案無聲 TTS regression | 經同意的真人 corpus | no-prompt Turbo 基線可重跑；`zh`／`auto` 分報告；目前 email／URL 與專有詞仍是 release gate，不以 LLM 猜字掩蓋 |
| 23 | 按下即說／放開最後一字 | audio post-roll tests | 內建麥克風、AirPods、USB | 開麥與 AX seed 並行；正常停止保留 200ms 尾音；取消與裝置斷線不等待；真人首尾漏字率仍待量測 |
| 24 | 下載完成時仍在聽寫／模型載入失敗 | atomic swap＋race regressions | 大模型切換 | 不覆寫現役引擎或 config；不吞掉 Up／Esc；明示已下載但需重按「使用」，不假裝自動切換 |
| 25 | 內建整理冷啟動超時 | budget／cancel／try-lock tests | release bundle cold/warm | 不排隊、不讓 LLM 失敗拖垮聽寫；超時安全退回原文並保留 warm model；阻塞式 hash／Metal load 仍須量 p50／p95 |

## keyDown AX 延遲：最小可驗證實作方案

1. keyDown 讓 `audio::start_capture` 與最小 `AXFocusedUIElement` seed 並行；seed 不讀文字或逐項 metadata。麥克風 stream ready 後才完成 bounded target fingerprint、讀設定與擷取 Context，避免 AX 排在開麥前切掉第一個音節。
2. target fingerprint 與 ContextSnapshot 使用固定的 window／focused element refs；App id 或完整 fingerprint 不符就丟棄 Context。PasteTarget 取不到可辨識 metadata 時 unknown→unknown 不得放行。
3. Context worker 與 paste-time fingerprint capture 各設約 250ms wall-clock 上限；單次 AX messaging timeout 40ms。逾時時不自動貼上，文字進 History 或 process-memory pending queue。
4. 已覆蓋舊 session 不得取走新 session slot、同 App 不同 fingerprint 不得使用 Context／自動貼上、stale watchdog 與 Cmd+V 前 target guard；仍需注入 500ms fake AX，驗證 bounded return 與 audio callback 的 p95。
5. Native 驗收記錄 keyDown→第一個錄音狀態事件的 p50/p95，至少測 TextEdit、Mail、Chrome、Electron App 與刻意延遲的 AX 測試 App；不以單次順暢操作代替數字。

## 黑箱競品對照語料

同一段錄音在 Email、Message、Document、Technical 四種 App 各跑 RAW/CLEAN/ORGANIZE，記錄：

- fact anchor retention
- content coverage
- correction resolution
- context hallucination
- app-format hit rate
- paste success
- cold/warm p50/p95
- peak RSS / memory pressure / energy impact

Typeless 只做人工公開行為對照，不分析 binary 或私有流量，也不把輸出用於訓練、fine-tune、prompt distillation 或自動資料抽取。

## 發行前仍需真機矩陣

- 8 GB Intel、8 GB Apple Silicon、16 GB、32 GB。
- macOS 11（目前宣稱的最低版本）、12、13、14、15、26；macOS 11 至少要完成 clean launch、權限、離線 RAW 聽寫。未通過前屬外部發行 blocker。
- TextEdit、Mail、Safari、Chrome、Slack、Teams、Discord、Cursor、VS Code、Office、Notion。
- AirPods、Bluetooth、USB 麥克風熱插拔、睡眠／喚醒。
- 快速連續兩段、5 分鐘 force-stop、錄音中切換音訊路由、config/history 損毀與磁碟寫滿 fault injection。
- 多螢幕、全螢幕、Spaces、Stage Manager。
- VoiceOver、Full Keyboard Access、Reduce Motion、Increase Contrast。
- Developer ID、notarization、Gatekeeper、0.1.0→0.1.1 updater。
