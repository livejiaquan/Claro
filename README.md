# Claro

macOS 語音輸入工具。按住快捷鍵說話，放開後文字直接出現在游標位置。全部在本地處理，資料不出機器。

> **改版進行中**：Claro 正以 Tauri（Rust）重寫為可下載安裝的桌面 app（規劃見 `docs/ROADMAP.md`）。現行可用版本是 `prototype/` 下的 Python 實作，以下說明皆以它為準（指令請在 `prototype/` 目錄執行）。

## 特色

- **全本地處理** — 使用 mlx-whisper + mlx-lm，Apple Silicon 加速；首次執行需網路下載模型，之後完全離線
- **中英混雜辨識** — 針對台灣繁體中文 + 英文技術術語最佳化
- **個人字典** — 自訂術語對照表，不怕專有名詞聽錯
- **螢幕上下文感知** — 自動讀取前景 App 與視窗資訊，提升辨識準確度
- **Typeless 風格 UI** — 底部極簡膠囊，即時波形回饋，滑鼠穿透不搶焦點
- **歷史紀錄** — 每次聽寫存本地 `~/.claro/history.jsonl`，取消或失敗都救得回來

## 系統需求

- macOS 14+ (Sonoma)
- Apple Silicon Mac (M1 以上)
- Python 3.10+

## 安裝

```bash
# 1. 建立虛擬環境
python3 -m venv venv
source venv/bin/activate

# 2. 安裝依賴
pip install -r prototype/requirements.txt

# 3. 編譯 Swift 膠囊 overlay
cd prototype
swiftc -o mic_indicator mic_indicator.swift -framework Cocoa -framework AVFoundation
```

## 使用

```bash
source venv/bin/activate
cd prototype
python3 main.py
```

### 快捷鍵

| 操作 | 動作 |
|------|------|
| 按住 `Option + Shift + C` | 說話，放開即貼到游標處 |
| 快速點放 `Option + Shift + C` | 進入免持模式（連續聆聽），再按一次停止並貼上 |
| `Esc` | 取消本次聽寫（內容仍會存入歷史紀錄） |

### Menu Bar

啟動後 menu bar 會出現麥克風圖示：即時狀態、今日使用統計、最近 5 筆聽寫（點擊複製）、開啟歷史紀錄、模型切換、結束 Claro。

### 歷史紀錄

```bash
python3 main.py --history   # 印出最近 20 筆聽寫（含被取消的）
```

### 模型設定 `~/.claro/config.json`

```json
{
  "whisper_model": "large-v3-mlx",
  "llm_model": "mlx-community/Qwen2.5-7B-Instruct-4bit",
  "llm_enabled": true
}
```

可從 menu bar 切換，或直接編輯（重啟生效）。電腦較舊可改用 `large-v3-turbo`（更快、首次需下載 1.6GB）搭配 `Qwen2.5-1.5B-Instruct-4bit`，或把 `llm_enabled` 設 `false` 關閉潤飾。不會自動下載任何模型——換新模型後第一次聽寫才會下載。

### 自訂字典

```bash
python3 main.py --add-term="GBT:GPT" --add-term="My Torch:PyTorch"
```

### 除錯模式

```bash
python3 main.py --debug  # 音訊會儲存到 /tmp/voicerec_debug/
```

## 權限

初次使用需要授予：

1. **麥克風權限** — 系統設定 → 隱私與安全性 → 麥克風
2. **輔助使用權限** — 系統設定 → 隱私與安全性 → 輔助使用

## 隱私與限制

- 全程本地處理，資料不出機器
- 聽寫歷史僅存 `~/.claro/history.jsonl`（權限 `0600`）
- 螢幕上下文只在記憶體中提供給本地 LLM，不落盤
- 剪貼簿還原僅保留純文字；貼上前複製的圖片或富文本會遺失
- 免持模式單次錄音上限 5 分鐘

## 架構

```
音訊 → Whisper (mlx-whisper) → 個人字典 → LLM 潤飾 (mlx-lm) → 貼上游標處
        ↑
    Accessibility API 讀取前景 App 資訊
```

## License

MIT
