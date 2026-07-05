# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Claro：macOS 全本地語音輸入工具。按住熱鍵說話，放開後文字貼到游標處。目標：本地隱私 × 上下文感知潤飾 × 模型可插拔 × CJK 極致，做到同類產品數一數二（對標 Typeless，架構對標 Handy）。

## 目前狀態（隨里程碑更新）

- **Phase 0 完成並經使用者核准（2026-07-05）**：規格見 `docs/SPEC.md`（技術真值來源，決策 D1–D10）、`docs/ROADMAP.md`（里程碑與進度）、`docs/COMPETITIVE_MATRIX.md`（定位）、`docs/research/`（競品原始碼分析）。
- **M1 程式碼完成（2026-07-05），待使用者手動驗收**（授予輔助使用權限＋實際聽寫）：Tauri v2 重寫於 `desktop/`；Python/MLX 版凍結為 `prototype/`（行為真值來源，只修不增）。37 個 Rust 單元測試全綠；TTS 音檔端到端轉錄驗證過（中英混雜正確、繁體、Metal）。
- M1 的 overlay 直接由 Rust spawn `prototype/mic_indicator`（同 socket 協議，動畫沿用——使用者指定）；Tauri 原生 overlay 延至 M5。

## Desktop（Tauri）常用指令

```bash
cd desktop
npm install                    # 首次
npm run tauri dev              # 開發模式（含前端 HMR）
npm run tauri build            # 產 Claro.app（src-tauri/target/release/bundle/macos/）
cd src-tauri
cargo test --lib               # Rust 單元測試（不需麥克風/模型）
cargo run --example download_model [id]        # 下載 whisper 模型（同 app 的 downloader）
cargo run --example transcribe_file x.wav      # WAV 走完整 STT 管線（可用 say -v Meijia 產測試音）
```

需要 `cmake`（brew）與 Rust stable。模型放 `~/Library/Application Support/Claro/models/`；config/history 沿用 `~/.claro/`（與 prototype 相容）。熱鍵/Esc 攔截需要輔助使用權限——dev 模式授權給終端機，bundle 授權給 Claro.app（從終端機啟動 .app 內的 binary 會繼承終端機的權限，最適合開發驗證）。

**已踩的坑**：`cargo build --release` 直接編出的 binary 是 dev context——webview 會去連 `devUrl`（localhost:1420），dev server 沒開就白屏。**驗證跑的一律用 `npm run tauri build` 產物**（`bundle/macos/Claro.app`）。白屏除錯：`CLARO_DEVTOOLS=1` 啟動會自動打開 inspector。

任何 session 開工前：先讀 `docs/ROADMAP.md` 的進度區與當前里程碑 DoD；架構問題查 `docs/SPEC.md`（含決策記錄 D1–D7）。完成里程碑後更新兩者。

## Python 原型（`prototype/`，已凍結，行為參考）

```bash
python3 -m venv venv && source venv/bin/activate   # venv 在 repo 根目錄
pip install -r prototype/requirements.txt
cd prototype
./build_indicator.sh          # 改過 mic_indicator.swift 後必須重編（swiftc）
python3 main.py               # 執行；--history 印最近 20 筆；--debug 音訊存 /tmp/voicerec_debug/
python -m pytest              # 全部測試；單一測試：python -m pytest tests/test_state_machine.py::test_quick_tap_enters_handsfree
```

測試不需麥克風或模型：`test_integration.py` 用 monkeypatch 塞假 `mlx_whisper`/`mlx_lm`/`sounddevice` 進 `sys.modules` 再 import `main`。

原型架構（Rust 移植時的行為基準）：

- **管線**：音訊 → `_transcribe`（mlx-whisper，上下文詞彙進 `initial_prompt`）→ `_apply_dict` → `_llm_refine`（mlx-lm 保守糾錯）→ `_to_traditional`（OpenCC s2twp 終盤）→ `_paste_text`（剪貼簿備份+Cmd+V+還原）。
- **狀態機**（`state_machine.py`）：純邏輯，IDLE→HOLD→(tap<0.35s→HANDSFREE｜放開→PROCESSING)，session 序號作廢過期結果。**移植時必須逐案通過 `tests/test_state_machine.py` 的對應案例**。
- **執行緒**：CGEventTap 回呼只入列，單一 dispatcher 序列化餵狀態機（keyDown/keyUp 亂序會卡死錄音——已踩過的坑）；重活全在背景執行緒。
- **上下文**（`_get_window_context`）：AX API 抓前景 app／視窗標題／游標前後文（500 字）／選取／可見文字 BFS（1200 字、400 節點，只收內容性角色）。只在記憶體，不落盤。
- **Swift `mic_indicator` 身兼兩職**：overlay 膠囊走 Unix socket `~/.claro/indicator.sock` 文字命令（`recording`/`level`/`processing`/`success`/`cancel`/`error`/`quit`）；menu bar 則獨立直接讀寫 `~/.claro/config.json` 與 `history.jsonl`——**config/history schema 同時存在 Python 與 Swift 兩邊，改動要同步**。
- 已驗證的細節教訓：1.5B 級 LLM 會洩漏提示詞（預設用 4B+）；潤飾防呆（空輸出/長度爆炸退回原文）；LLM 輸出可能帶簡體所以 OpenCC 放最後。

## 不可違背的產品約束

- **絕不自動下載模型**（使用者網路慢）：一律明確同意後才下載。
- **隱私預設**：音訊/上下文不出機器；上下文永不落盤；`AXSecureTextField` 永不讀；權限只要麥克風＋輔助使用。
- `~/.claro/` 下檔案 0600、目錄 0700；API key 進 Keychain 不進 JSON。
- 潤飾失敗永遠退回原文，聽寫不可因 LLM 掛掉而失敗。
- 效能驗收指標見 `docs/SPEC.md` §12（短句 raw ≤1.5s、含潤飾 ≤4s、待機 <300MB）。

## 慣例

- Commit 原子化、訊息清楚，**不加任何 AI 署名**（無 Co-Authored-By、無 Generated with）。
- `docs/superpowers/` 僅留本地，不進公開發布。
- 回覆與文件預設繁體中文；程式碼註解風格跟隨現有檔案。
- 遇到不確定的最新 API/crate 用法先查證再用，不憑記憶硬寫。
