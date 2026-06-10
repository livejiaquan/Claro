# Claro

macOS 語音輸入工具。按下快捷鍵說話，鬆開後文字直接出現在游標位置。全部在本地處理，資料不出機器。

## 特色

- **全本地處理** — 使用 mlx-whisper + mlx-lm，Apple Silicon 加速，不需網路
- **中英混雜辨識** — 針對台灣繁體中文 + 英文技術術語最佳化
- **個人字典** — 自訂術語對照表，不怕專有名詞聽錯
- **螢幕上下文感知** — 自動讀取前景 App 與視窗資訊，提升辨識準確度
- **Typeless 風格 UI** — 底部浮動工具列，即時狀態回饋

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

# 3. 編譯 Swift 工具列
swiftc -o mic_indicator mic_indicator.swift -framework Cocoa
```

## 使用

```bash
source venv/bin/activate
python3 main.py
```

### 快捷鍵

| 按鍵 | 動作 |
|------|------|
| `Option + Shift + C` | 開始 / 停止錄音 |

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
音訊 → Whisper (mlx-whisper) → 個人字典 → LLM 潤飾 (mlx-lm) → 輸出
        ↑
    Accessibility API 讀取前景 App 資訊
```

## License

MIT
