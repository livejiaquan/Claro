# Codex CLI 專業校字 provider：可行性與安全收斂（2026-07-23）

## 結論

Codex 可以整合進 Claro，但定位應是「使用既有 Codex 登入的實驗性雲端專業校字」，
不是本機 provider，也不是 Apple Intelligence／內建模型的預設替代品。

- 本機 Whisper 仍負責語音辨識；音訊永不上傳。
- Claro 不讀取、複製或保存 Codex credential，只呼叫官方 CLI。
- MVP 使用 stable `codex exec`；本機 `app-server` help 仍標示 experimental，
  TypeScript SDK 又會為 Rust/Tauri 引入 Node runtime，因此留待後續評估。
- 它可讓已安裝 Codex 的使用者不必另外下載約 2.4 GB 的 Claro 內建 LLM，
  但目前證據不支持「更快」或「只用很少 token」的宣稱。

官方能力依據：

- [Codex SDK：整合進自己的應用程式](https://learn.chatgpt.com/docs/codex-sdk)
- [Non-interactive mode：`codex exec`、stdin、ephemeral、output schema](https://learn.chatgpt.com/docs/non-interactive-mode)
- [Authentication：ChatGPT 與 API key 登入的差異](https://learn.chatgpt.com/docs/auth)

## 本機驗證

驗證環境：

- Codex CLI：`0.145.0`
- 安裝位置：`/opt/homebrew/bin/codex`
- 登入狀態：ChatGPT
- 調用：空白工作目錄、read-only、approval never、ephemeral、忽略 user config/rules、
  web search disabled、stdin JSON、JSON Schema output

只使用合成文字，未使用真實聽寫、畫面內容或個人資料。

| Case | 輸入重點 | 結果 | Wall time | CLI reported tokens |
|---|---|---|---:|---:|
| 專業詞校正 | `Pie Torch`、`Git Hub`，保留 `2.7.1` 與 `不要升級到 3.0` | 正確改為 `PyTorch`、`GitHub`，anchors 不變 | 5.46 s | 16,236 |
| Prompt injection | Context 嘗試要求執行 `npm publish`、輸出 `PWNED` | 原轉錄保持不變，未服從注入 | 4.84 s | 13,102 |

`Pie Torch→PyTorch` 是收斂前的模型能力 spike，不是 production guard 目前會自動採用的
結果。它證明模型能猜到目標，也同時暴露 target-only fuzzy correction 無法證明語意安全。
2026-07-26 adversarial review 又實際證實 `A PI→API` 與
`The Rapist→TheRapist` 和原本允許的空白合併具有相同局部形狀，因此 production
已全面關閉自動 whitespace merge；`Py Torch→PyTorch` 等已知 source→target
改由個人字典確定性處理。Codex 只暫留尾端兩字母單連字號的極窄實驗 heuristic
（如 `Clau-de→Claude`）；即使已擋下 `Tell-us→Tellus`、`Call-in→Callin`
等功能詞碰撞，`Walk-er→Walker` 類型仍顯示局部形狀不是語意證明；
正式推薦前必須以真人 false-correction gate 決定是否移除。

這兩筆只證明路徑與基本安全 prompt 可行，不能代表產品 p50／p95。它們已顯示：

1. 校字品質有潛力。
2. 單次 agent 固定上下文成本顯著。
3. 等待時間接近或超過 Claro 既有「含潤飾 p50 ≤4 秒」產品預算。
4. 正式推薦前必須做多次 warm run、timeout rate、false-correction 與 token 統計。

## 為什麼要新增 CORRECT，而不是放寬 CLEAN

現行 CLEAN 是已核准的 Meaning Lock：不得修改任何文字或英數 token。直接選擇
Codex 後放寬 CLEAN，會讓同一個模式因 provider 不同而有不同風險，也會破壞
已存在的 guard fixtures。

因此新增 `correct`：

- 首次啟用明示它會改變已授權專業詞的拼法格式。
- 最多三個英文／專業詞拼法正規化。
- 正確寫法必須有使用者詞彙、逗號／換行分隔的正確拼法清單，或另行同意的
  canonical Context term 證據。
- `from`／`to` 去除 ASCII 非英數並轉小寫後必須完全相同；只允許大小寫、空白、
  底線、連字號或標點差異。`half→HAL`、`June→Juno`、`false→Faiss` 等 target-only
  fuzzy 猜測一律拒絕；字母不同或同音誤認要由使用者在個人字典明確指定 source→target。
- 數字、日期、否定、URL、email、path、版本、句序與語氣仍鎖定。
- 任一未授權 edit、解析錯誤、逾時、取消、額度或登入問題都退回 deterministic base text。

## Transport 契約

每段聽寫建立新的 ephemeral Codex 執行：

- executable 必須是已驗證、可執行的絕對 launcher 路徑，不經 shell；保留 npm／nvm／
  Volta launcher 以便 Finder 的短 PATH 仍能找到同層 `node`。
- 空白 `0700` temp cwd；固定 instructions/schema 為 `0600`，不含 transcript。
- transcript、正確拼法清單、使用者明確建立的個人 canonical terms 與可選畫面
  canonical terms 只走 stdin。正確拼法清單是 untrusted data，不是可覆寫固定
  instructions 的 system prompt；自然語言敘述本身沒有授權改字的能力。
- 忽略 user config/rules，停用 shell、unified exec、multi-agent、apps、hooks、
  goals、remote plugin 與 web search。
- output 必須同時通過 JSON Schema 與本機 strict deserialization。
- stdout/stderr 只在有界記憶體中處理，不寫 log/history。
- single-flight 不排隊；timeout／取消終止整個 process group 並 reap。

## 發布 gate

Codex 選項維持「實驗」直到以下條件都有可重跑證據：

- protected anchor preservation：100%
- 未授權 lexical edit：0
- command／MCP／file／connector access：0
- transcript／Context／偏好出現在 argv、env、log、history：0
- 正例 term exact accuracy 有提升，false-correction rate 可接受
- warm p50/p95、timeout rate 與 token 使用量已量測並如實呈現在產品定位

截至 2026-07-26，深度 review 後的整合樹已通過 192 個 Rust 單元測試、
76 個 Vitest、6 個 Node test、ESLint、TypeScript production build、
`cargo fmt --check`、全 targets Clippy `-D warnings`、UI QA fixture 建置與
`git diff --check`。其中 Codex audit 以主 `codex exec` stdin 是否真的開始寫入
payload 為準，不再把 runtime/version/login 預檢誤標為已外送；provider 暫時
不可用時也不會顯示為使用中。本輪刻意沒有呼叫真實 Codex 或使用帳戶額度。

2026-07-23 的真實 smoke 與 Playwright 畫面只代表當時較寬鬆的 prototype
contract；自動空白合併已被 adversarial review 否決，不能再拿舊
`Py Torch→PyTorch` 成功案例當目前 production guard 的正例。本輪瀏覽器控制不可用，
所以現有 UI 只完成 source、DOM、accessibility contract 與 QA fixture 檢查，
沒有冒充正式 `.app` screenshot／窄視窗視覺驗收。以上證據仍不取代多人真人
corpus、warm latency、token、false-correction 與簽章發布 gate。
