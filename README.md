# Claro

macOS 語音輸入工具。按住快捷鍵說話，放開後文字直接出現在游標位置。全部在本地處理，資料不出機器。

## 特色

- **全本地處理** — 使用 mlx-whisper + mlx-lm，Apple Silicon 加速，不需網路
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
pip install -r requirements.txt

# 3. 編譯 Swift 膠囊 overlay
swiftc -o mic_indicator mic_indicator.swift -framework Cocoa -framework AVFoundation
```

## 使用

```bash
source venv/bin/activate
python3 main.py
```

### 快捷鍵

| 操作 | 動作 |
|------|------|
| 按住 `Option + Shift + C` | 說話，放開即貼到游標處 |
| 快速點放 `Option + Shift + C` | 進入免持模式（連續聆聽），再按一次停止並貼上 |
| `Esc` | 取消本次聽寫（內容仍會存入歷史紀錄） |

### 歷史紀錄

```bash
python3 main.py --history   # 印出最近 20 筆聽寫（含被取消的）
```

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
3. **螢幕錄製權限**（選擇性，用於上下文感知）— 系統設定 → 隱私與安全性 → 螢幕錄製

## 架構

```
音訊 → Whisper (mlx-whisper, large-v3-turbo) → 個人字典 → LLM 潤飾 (mlx-lm) → 貼上游標處
        ↑
    Accessibility API 讀取前景 App 資訊
```

## License

MIT
