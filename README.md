# Claro

**macOS 全本地語音輸入。按住快捷鍵說話，放開，文字直接出現在游標位置。**

預設情況下，語音、螢幕內容與輸出文字都在你的機器上處理；Claro 本身不要求帳號或訂閱。
只有你主動設定並允許自訂雲端端點或既有 Codex 登入時，轉錄文字才可能離開機器；音訊不會上傳。
針對「繁體中文為主、夾雜英文技術術語」的說話方式最佳化。

> 開發進行中（pre-release）。可從原始碼建置使用；簽章安裝檔在 roadmap 上（M6）。

## 為什麼是 Claro

| | Claro | 一般雲端聽寫 |
|---|---|---|
| 語音資料 | 不離開機器 | 上傳雲端 |
| 螢幕上下文 | 本地讀取、用完即丟、永不儲存 | 多半上傳 |
| 辨識模型 | 自選（whisper 家族，可換大小） | 固定 |
| 潤飾模型 | 自選（Apple 端上模型／內建模型／自帶 API／既有 Codex 登入） | 固定 |
| 費用 | 免費開源 | 訂閱 |

### 四個支柱

1. **本地優先隱私**——STT 固定在本機跑（Metal 加速）；首次設定只推薦
   Apple Intelligence／Claro 內建模型／原樣轉錄，本機限制預設開啟。
   密碼欄永不讀取；上下文永不落盤；設定檔 0600；API key 進 Keychain。
2. **真上下文感知**——聽寫時讀取目前視窗的內容（app、標題、游標周邊文字），
   把畫面上出現過的術語在**辨識階段**就餵給模型。你在看 PyTorch 文件時說
   「派托奇」，出來的就是 PyTorch。
3. **模型可插拔**——辨識模型 6 種 whisper 變體 UI 內下載切換；潤飾引擎五選一：
   Apple Intelligence（macOS 26+，免安裝）、內建模型（llama.cpp，免安裝）、
   Ollama、LM Studio、自訂 OpenAI-compatible API（OpenAI/Groq/DeepSeek/
   Gemini/OpenRouter preset）。
4. **CJK 極致**——繁體中文（台灣用語）終盤 OpenCC 正規化、全形標點修正、
   中英混排處理、個人字典（常錯的詞教一次就好）。

## 安裝（從原始碼）

需求：macOS 11+、Intel 或 Apple Silicon、Rust stable、Node 22+、`brew install cmake`。

```bash
git clone https://github.com/livejiaquan/Claro && cd Claro/desktop
npm install
npm run tauri build
# 產物：src-tauri/target/release/bundle/macos/Claro.app（拖進 Applications）
# 或 dmg：src-tauri/target/release/bundle/dmg/
```

首次啟動：

1. 在「首次設定」授予**輔助使用**權限（授權後會自動重新檢查，不用重啟）。
2. 執行一次不辨識內容的**麥克風音量測試**。
3. 明確按下下載這台 Mac 的推薦語音模型；Claro 不會自動下載。
4. 選擇 Apple 端上整理、Claro 內建整理，或原樣轉錄。
5. 到任一文字輸入框完成一次真正的聽寫與貼上。

就這兩個權限，不多要。

## 使用

- **按住 ⌥⇧C 說話，放開出字**（快捷鍵可在設定改成右 ⌘／右 ⌥／fn 單鍵按住）。
- **快點一下**＝免持模式，再按一下結束；免持單次上限 5 分鐘。
- **Esc**＝取消（內容仍存在歷史，可救回）。
- 設定 → AI 潤飾，四檔輸出模式：**RAW** 完全逐字；**CLEAN**（建議）去填充詞、
  保留明確改口的最終版、補標點——**絕不改動任何字詞**，專有名詞的修正交給
  螢幕詞彙偏置與個人字典；**CORRECT**（需明確開啟）可依你提供的正確詞彙，
  受控統一英文專業詞的大小寫、空白與連字號；字母不同或同音誤認仍交給個人字典；
  **ORGANIZE**（需明確開啟）可重排分段與格式化列舉，
  但姓名、數字、日期、否定、條件一律原樣保留。
  任何模式下潤飾失敗永遠退回原始轉錄——聽寫不會因為 LLM 掛掉而失敗；
  歷史紀錄可查看每句的 raw/final 與實際生效的模式。
- 設定 → 個人字典：左邊填常被認錯的寫法、右邊填正確寫法，
  同時會提示辨識模型少認錯。

## 隱私模型

- **T0（預設）**：語音、文字、螢幕上下文全部不出機器。潤飾用 Apple 端上模型
  或內建模型時也是 T0。
- **T1（自選）**：你自己設定並允許雲端潤飾時，UI 會明確標示實際目的地與
  送出的資料。自訂 API 送到你指定的端點；Codex CLI 選項會使用這台 Mac
  既有的 Codex 登入，把轉錄、正確拼法清單與你明確建立的個人詞彙清單送到 OpenAI。
  音訊不會送出；畫面詞彙
  只有在你另行同意後，才會以少量、已清洗的 canonical terms 送出。
- 音訊永不落盤；上下文只在記憶體（設定頁可查看上次擷取了什麼、一鍵清除）；
  歷史紀錄存本地 `~/.claro/history.jsonl`（0600，可清）。
- 剪貼簿以 NSPasteboard 項目／可讀格式備份還原（圖片、富文本都盡量保留）；
  單一冗餘格式暫時不可讀不再阻斷整段聽寫，整個項目不可備份才安全停止，
  並以 ownership/changeCount 防止覆蓋你貼上期間複製的新內容。

## 開發

```bash
cd desktop
npm run tauri dev          # 開發模式（前端 HMR）
cd src-tauri
cargo test --lib           # 單元測試（不需麥克風/模型）
```

- 架構與原理：**[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)**（維護者必讀：
  執行緒模型、每個子系統的設計原因、已知地雷、怎麼加模型/provider）
- 產品規格與決策記錄：[docs/SPEC.md](docs/SPEC.md)
- 里程碑：[docs/ROADMAP.md](docs/ROADMAP.md)
- 競品研究：[docs/research/](docs/research/)

`prototype/` 是已凍結的 Python/MLX 原型（行為參考實作），日常開發都在 `desktop/`。

## License

MIT
