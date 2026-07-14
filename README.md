# Claro

**macOS 全本地語音輸入。按住快捷鍵說話，放開，文字直接出現在游標位置。**

語音、螢幕內容、輸出的文字——全部在你的機器上處理。沒有帳號、沒有訂閱、
預設不發出任何網路請求。針對「繁體中文為主、夾雜英文技術術語」的說話方式最佳化。

> 開發進行中（pre-release）。可從原始碼建置使用；簽章安裝檔在 roadmap 上（M6）。

## 為什麼是 Claro

| | Claro | 一般雲端聽寫 |
|---|---|---|
| 語音資料 | 不離開機器 | 上傳雲端 |
| 螢幕上下文 | 本地讀取、用完即丟、永不儲存 | 多半上傳 |
| 辨識模型 | 自選（whisper 家族，可換大小） | 固定 |
| 潤飾模型 | 自選（Apple 端上模型／內建模型／自帶 API） | 固定 |
| 費用 | 免費開源 | 訂閱 |

### 四個支柱

1. **本地優先隱私**——STT 與潤飾都在本機跑（Metal 加速）。潤飾預設關閉；
   就算開，預設選項（Apple Intelligence／內建模型）也完全離線。
   密碼欄永不讀取；上下文永不落盤；設定檔 0600；API key 進 Keychain。
2. **真上下文感知**——聽寫時讀取目前視窗的內容（app、標題、游標周邊文字），
   把畫面上出現過的術語在**辨識階段**就餵給模型。你在看 PyTorch 文件時說
   「派托奇」，出來的就是 PyTorch。
3. **模型可插拔**——辨識模型 6 種 whisper 變體 UI 內下載切換；潤飾引擎五選一：
   Apple Intelligence（macOS 26+，免安裝）、內建模型（llama.cpp，免安裝）、
   Ollama、LM Studio、自訂 OpenAI-compatible API（OpenAI/Anthropic/Groq/
   DeepSeek/Gemini/OpenRouter preset）。
4. **CJK 極致**——繁體中文（台灣用語）終盤 OpenCC 正規化、全形標點修正、
   中英混排處理、個人字典（常錯的詞教一次就好）。

## 安裝（從原始碼）

需求：macOS 14+、Apple Silicon、Rust stable、Node 18+、`brew install cmake`。

```bash
git clone https://github.com/livejiaquan/Claro && cd Claro/desktop
npm install
npm run tauri build
# 產物：src-tauri/target/release/bundle/macos/Claro.app（拖進 Applications）
# 或 dmg：src-tauri/target/release/bundle/dmg/
```

首次啟動：

1. 授予**輔助使用**權限（系統會提示；授權後 app 自動啟用，不用重啟）。
2. 到 設定 → 語音模型 下載一個模型（推薦 Large v3 Turbo，1.5GB）。
3. 第一次聽寫時允許**麥克風**。

就這兩個權限，不多要。

## 使用

- **按住 ⌥⇧C 說話，放開出字**（快捷鍵可在設定改成右 ⌘／右 ⌥／fn 單鍵按住）。
- **快點一下**＝免持模式，再按一下結束；免持單次上限 5 分鐘。
- **Esc**＝取消（內容仍存在歷史，可救回）。
- 設定 → AI 潤飾，三檔輸出模式：**RAW** 完全逐字；**CLEAN**（建議）去填充詞、
  保留明確改口的最終版、補標點——**絕不改動任何字詞**，專有名詞的修正交給
  螢幕詞彙偏置與個人字典；**ORGANIZE**（需明確開啟）可重排分段與格式化列舉，
  但姓名、數字、日期、否定、條件一律原樣保留。
  任何模式下潤飾失敗永遠退回原始轉錄——聽寫不會因為 LLM 掛掉而失敗；
  歷史紀錄可查看每句的 raw/final 與實際生效的模式。
- 設定 → 個人字典：左邊填常被認錯的寫法、右邊填正確寫法，
  同時會提示辨識模型少認錯。

## 隱私模型

- **T0（預設）**：語音、文字、螢幕上下文全部不出機器。潤飾用 Apple 端上模型
  或內建模型時也是 T0。
- **T1（自選）**：你自己設定雲端潤飾端點時，「轉錄文字＋螢幕上下文」會送到
  **你指定的**端點。UI 會明確標示。
- 音訊永不落盤；上下文只在記憶體（設定頁可查看上次擷取了什麼、一鍵清除）；
  歷史紀錄存本地 `~/.claro/history.jsonl`（0600，可清）。
- 剪貼簿以完整 NSPasteboard 項目／格式備份還原（圖片、富文本都保留），
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
