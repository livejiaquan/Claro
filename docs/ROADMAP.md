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

- [ ] **P0 Delivery Trust Pass（第二輪可靠性修補中，2026-08-11）**：使用者實測發現 History 已有結果但切換視窗後未貼上；第一輪已把 Context 的 session-bound AX fingerprint 與文字交付拆開，Cmd+V 跟隨當下外部前景 App，並加入 processing／terminal event 與 pending recovery banner。新版安裝後的 34 秒真實失敗證據顯示：輔助使用已授權、前景 App gate 通過，但 clipboard transaction 在 5ms 內、50ms settle 與 Cmd+V 之前失敗；unified log 同時顯示 NSPasteboard 正在 materialize RTF／UTF 文字 flavors。第二輪因此將安全界線由「任一 flavor 不可讀就整段 fail closed」縮至「同一 item 至少保留一種可還原格式」，跳過壞掉的冗餘 lazy flavor，但整個 item 不可備份時仍 fail closed。聚焦測試涵蓋部分 flavor 不可讀、整 item 不可讀、ownership 被替換、焦點切換與 Cmd release cleanup；最終仍須以新 bundle 完成跨 App 真實貼上。
- [x] Phase 0：稽核＋競品研究＋SPEC/ROADMAP/CLAUDE.md（2026-07-05）
- [ ] **P0 STT Accuracy Pass（程式修正完成、產品準確率尚待真人驗收，2026-07-15）**：已確認相容 id `large-v3-mlx` 實際解析為速度版 large-v3-turbo；CLEAN Meaning Lock 刻意不猜回 STT 錯字。現在 16GB+ Apple Silicon 推薦完整 large-v3、8–12GB 推薦 large-v3 Q5、Intel 推薦 Turbo Q5；既有 Turbo 使用者有可略過的升級入口，下載仍須明確點擊。錄音啟動與 AX seed 並行、正常停止保留 200ms 尾音；Whisper prompt 改成純詞彙；個人字典新安裝預設為空並移除完全相符的舊危險預設；模型切換改為 load＋config 成功後才 commit；history 記錄模型、language、詞彙提示與正規化前收音診斷。Qwen3-ASR 0.6B／1.7B 已接入同一 transcribe.cpp family 供離線 eval，但因 256-token／長音訊分段尚未驗收，production UI、resolve 與 command 全部 fail closed。20 案重生 TTS／no-prompt Turbo 基線為 CER 5.38%、term recall 69.44%、anchor accuracy 82.43%，`zh`／`auto` 內容完全相同；失分集中 email、URL 與專有詞，且測試環境退回 CPU，不能拿 latency 當產品數字。評測支援格式等價 reference 與 fixture hash，避免 oracle prompt／舊音檔污染分數。**剩餘 release gate**：正式 `.app` 的 STT/LLM cold/warm、完整 large-v3 實機 latency/RSS、可重跑台灣真人 corpus（CER／formatting／term accuracy／Context on-off／auto-vs-zh／p50/p95）與多使用者盲測；完成前不宣稱準確率已解決或優於 Typeless。
- [ ] **P0 Trust & Release Pass（可靠性／UX 實作完成，最終原生驗收中，2026-07-23）**：輸出行為拆為 RAW／CLEAN／ORGANIZE 並與 provider 分離；CLEAN 加入否定、數字、日期時間、條件、因果／情態與過度刪減 guard，ORGANIZE 與雲端傳送各有獨立 backend consent gate；新安裝 `local_only=true`，雲端同意綁定 endpoint origin；history 可稽核 raw/final、requested/effective mode、provider、fallback 原因與 timings；onboarding 必須通過輔助使用、麥克風測試、已驗證模型與一次真實貼上，後端才會寫入 `setup_completed`；剪貼簿改為完整 NSPasteboard item/type 備份與 ownership/changeCount 防覆蓋；STT／內建 LLM 下載固定 immutable revision＋SHA-256，含嚴格續傳、原子落盤與持久校驗 marker；Swift indicator 以 Tauri sidecar 打包，release runtime 不再依賴 repo 路徑。2026-07-23 補強下載準備階段的可取消鎖等待、跨 STT/LLM 共用下載 gate 釋放、終態事件與 backend busy flag 的先後契約；Settings／Onboarding 現在有 preparing／downloading／cancelling／cancelled／failed／complete 明確狀態、保留續傳進度、取消、重試與可存取的 status/alert。效能紀錄以熱鍵放開為起點，新增 `release_to_paste_ms`、`focus_guard_ms`、`inject_ms`，並提供只輸出數字分位數、不讀 transcript/context 的摘要指令。前端加入 skip link、`focus-visible`、表單語意與 780px 最小視窗防溢位；production CSP、最小 Tauri capability、Dependabot、依賴稽核、frontend/Rust CI 與發行版版本／hardened runtime／危險 entitlement gate 已落地。最新程式已通過 145/145 Rust 單元測試、`cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo check --all-targets --locked`、前端 lint、13 個 Vitest＋1 個 Node 測試、Vite production build與 arm64 debug `.app` bundle；本機再 ad-hoc 簽章後，主程式／sidecar、macOS 11.0 minos、Info.plist 麥克風用途說明與 bundle signature 均驗證通過。Playwright 代表狀態已實際檢查 920×640 與 780×560 的 Onboarding／Settings／History、下載中／取消／錯誤／重試、鍵盤 skip link與 timings，無水平溢位，console 0 error／0 warning。依驗證鐵律未出聲、未觸發熱鍵；本輪未啟動原生 app，因此乾淨 TCC、真實麥克風、真實模型下載與跨 App paste 仍未完成，完成前不標示 P0 通過。
- [ ] **Codex 專業校字實驗（2026-07-23 開始；MVP 實作與本機驗證完成）**：保留 CLEAN 不改字契約，另增明確 opt-in 的 CORRECT；本機 Whisper 後可選擇使用已安裝／登入的 Codex CLI，免另下載 Claro 內建 LLM。Codex 雖由本機 CLI 啟動，推理固定標示為 cloud；MVP 使用 stable `codex exec`、ephemeral、空 cwd、approval never、依能力探測動態停用全部非 removed features、stdin 與 strict structured output，不讀 credential、不自動安裝／登入／更新。主同意送轉錄，以及本機詞彙／正確拼法／另行同意的畫面候選中，與本次轉錄相關、具有唯一 source 且 correction guard 可採用的項目；三類共用 32 項上限，三類皆空時不啟動 Codex、不使用模型額度。完整 Context 不送，raw App 名／視窗標題在擷取時分離且不進雲端候選。正確拼法清單是不可覆寫固定安全契約的不可信資料，不當作真正 system prompt；含數字項目不送 Codex，改由本機個人字典處理。雲端同意以 contract-v2 綁 provider／OpenAI service／auth kind／raw 與 normalized CLI 版本／executable path＋SHA-256／完整 feature capability fingerprint／固定 runner contract，並在每次 stdin 前完整重驗；任一未登入、能力改變、限額、忙碌、逾時、取消、schema 或 guard 錯誤都退 deterministic base text。2026-07-26 adversarial review 實際證實 `A PI→API` 與 `The Rapist→TheRapist` 可繞過舊 target-only whitespace grammar，因此 production guard 已全面關閉自動空白合併；`Py Torch→PyTorch` 等已知 source→target 改由個人字典確定性處理。Codex 只留尾端兩字母單連字號的極窄實驗 heuristic（`Clau-de→Claude`），UI 與文件明示它不是語意證明；`Under-score→Underscore`、純 case、其他 separator、字母不同、同音誤認與含數字固定格式一律拒絕。深度 review 同時把自我測試取消改為 request-scoped，讓 runtime preflight 與 stdin writer 都即時停止；capability probe 使用隔離 HOME＋cwd，退出會回收全部 probe process groups；設定 mutation 拆成欄位級 command、測試結果改為 tagged union，補上完整同意 target、runner 32 項 defense-in-depth、資料界線前置、Home 完整雲端摘要、History provider、onboarding 可選入口、動態同意 focus 與焦點歸還、欄位自動儲存／錯誤重試、長錯誤換行，以及 WCAG AA 小字／pill／主要按鈕對比。最後複核再加入 mode/provider fail-closed capability matrix、切離 Codex 時 CORRECT 原子降為 CLEAN、可返回檢視與清空的同意前拼法清單、跨頁草稿、AI 潤飾區 action-local error/focus、disabled mode 語意、reduced-motion scroll，以及以主 `codex exec` stdin 是否開始寫入 payload 為準的 History 稽核，避免把 runtime/version/login preflight 誤標為已外送。兩筆收斂前模型能力 spike 的校字與 prompt-injection 結果正確，但需 4.84–5.46 秒、CLI reported tokens 13,102–16,236，已否定「一定更快／token 很少」的假設。最終整合樹已通過 192/192 Rust 單元測試、76/76 Vitest、6/6 Node tests、`cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、ESLint、TypeScript／Vite production build、UI QA fixture 建置與 `git diff --check`；本輪未呼叫真實 Codex，因此沒有使用帳戶額度。**剩餘正式推薦 gate**：重跑目前 UI 的實際 bundle screenshot／窄視窗視覺驗收（本輪瀏覽器控制不可用，只完成 source、DOM 與 QA fixture 檢查）、warm p50/p95、timeout/token 統計、多人真人技術聽寫的 term accuracy／false-correction，以及公開發行簽章；完成前維持實驗，不進新安裝推薦。
- [ ] **P0 外部發行驗收**：Developer ID 簽章＋Apple 公證＋App/DMG Gatekeeper、完整 Xcode 的 x86_64／universal CI、macOS 11 最低版本 clean launch／離線 RAW 聽寫、乾淨 Mac 的 TCC／首次聽寫 smoke，以及 updater 0.1.0→0.1.1 實測。這些仍屬 M6，未完成前不得宣稱可公開發布；本機 `--bundles app` 已成功，但無 Developer ID 的預設產物不具可散佈簽章。release workflow 現在會先檢查 tag 與三份版本一致，再要求 Developer ID hardened runtime、拒絕 debug/JIT/library-validation 例外 entitlement，並只上傳通過簽章、公證與 Gatekeeper 驗證的候選檔；仍不會自動發布。updater 尚未加入，必須先具備可信的 HTTPS endpoint、簽章公鑰與 0.1.0→0.1.1 實測條件，不能以假設定佯裝完成。
- [x] M1 —（2026-07-05）42 項 Rust 測試綠；全自動 e2e（合成熱鍵＋TTS 過內建麥克風）驗證免持路徑逐字正確；產品級 UI（側欄＋首頁統計＋歷史＋設定，Handy/Typeless 風格經使用者確認）；overlay 沿用 prototype 膠囊。PTT 按住路徑由使用者真手驗收。
- [ ] M2 — **大部分完成；Accuracy Pass 擴充中（2026-07-15）**：App icon＋品牌；模型庫原有 6 個 Whisper 變體（含 q5 量化），逐一下載/切換/刪除、引擎熱換免重啟；Qwen3-ASR 0.6B／1.7B GGUF 的 registry／family dispatch 只供離線 eval，通過長音訊與真人 corpus 前不可在 production 啟用。切換 transaction 先載入候選並寫 config，失敗回載原模型。**AI 潤飾提前上線**（M4 前移）：off/Ollama/自訂 OpenAI-compatible，API key 進鑰匙圈，保守糾錯 prompt＋防呆（mock server 三路徑驗證），設定頁一鍵測試；CJK 標點正規化；single-instance 防雙開；SIGTERM/退出的 Metal teardown 修復（零 crash report）；權限授予後熱鍵免重啟重試。**STT 閒置卸載（2026-07-07）**：與 LLM 同款 5 分鐘看門狗（`spawn_stt_idle_watcher`，只在 IDLE＋engine try_lock 才動手），錄音開始即背景回載（載入 1.4s 被說話時間蓋掉）、`process_session` 同步補載保底；實測（正式 bundle、`CLARO_STT_IDLE_SECS=20` 縮短門檻）RSS 1586MB→31MB，SIGTERM 乾淨退出零 crash，SPEC §12 待機 <300MB 達標。低記憶體路徑維持 STT/LLM 互斥且潤飾後立即卸載 LLM。**剩**：Qwen 長音訊分段／本機端到端與真人 corpus；SenseVoice/Moonshine（ONNX 引擎）。
- [ ] M3 — **核心與 P0 隱私面已前移完成（2026-07-12）**：`context.rs` AX 上下文擷取（NSWorkspace 前景 app→AXFocusedWindow/AXFocusedUIElement/游標前後文/可見文字 BFS，預算 500/1200/400 同 prototype；AXSecureTextField 永不讀；全域開關 `context_enabled`，內容永不落盤）；詞彙抽取對 Whisper 進 `initial_prompt`，完整 bounded Context 只在 ORGANIZE 進潤飾 prompt；Qwen3-ASR 現有 port 不支援 prompt，不能把 deterministic 字典替換當成同等解碼偏置。內建敏感 App denylist、本次 Context 稽核／清除、session-bound capture 與 salted App/視窗/焦點 metadata hash 已完成，可阻擋可辨識的目標切換。個人字典 UI（config `dictionary` 鍵、設定頁 CRUD、詞彙同步偏置辨識）。**教訓：system-wide AX 元素的 AXFocusedApplication 會回 -25204，必須走 NSWorkspace 拿 pid**。**剩**：使用者自訂 per-app 停用規則、真實跨 App/同 App 焦點矩陣，以及 Whisper Context on/off 的 term accuracy／誤偏置率。
- [ ] M4 — 前置研究完成（2026-07-06）：潤飾 runtime 選型定案（SPEC D3 改版），競品證據與本機 FoundationModels 實測見 `docs/research/llm-polish-runtime.md`。**Apple Intelligence provider 已上線（2026-07-06，前移）**：Swift bridge（DynamicGenerationSchema guided generation——CLT 環境編不了 @Generable macro，動態 schema 等價且實測是抑制「把聽寫當指令回答」的關鍵）＋ build.rs 編譯/弱連結（SDK 無 FoundationModels 時自動改編 stub）＋可用性偵測（未開啟/不支援/下載中）進設定 UI。潤飾 provider 擴充：Ollama/LM Studio 本機服務偵測（模型清單下拉＋重新偵測）、自訂 API 六組雲端 preset。P0 Meaning Lock 禁止 LLM 取代任何文字／英數 token，專有詞校正只走 STT Context 偏置與使用者明確建立的字典。**內嵌 llama.cpp 已上線（2026-07-06）**：`llm.rs`（llama-cpp-2/Metal、greedy、閒置 5 分鐘 watcher卸載、先 drop 再載）；模型目錄 Qwen3-4B-Instruct-2507 Q4（推薦）＋Gemma 3 4B，UI 內下載/切換/刪除；與 whisper.cpp 的兩份 ggml 同程序共存實測無衝突；**教訓：CJK 一字多 token，必須整串收完再 detokenize，逐 token 轉會丟字**。本次 Accuracy Pass 發現少量 builtin cold-start 樣本退回原文；現已加入 8 秒 total cap、生成最多 6 秒且不排隊，阻塞式 cold load 超時後保留 warm model但本段不再追加生成。**剩**：正式 `.app` cold/warm p50/p95、把高記憶體 profile 的 prewarm 策略真正接入、完整 CJK 評測集與 IME 貼上防護
- [ ] M5
- [ ] M6
