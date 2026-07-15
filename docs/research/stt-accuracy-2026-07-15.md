# STT 精準度診斷與模型策略

日期：2026-07-15。範圍：Claro macOS 本地聽寫的辨識精準度、Context 詞彙偏置與潤飾層實際作用。本文把「本機已驗證」、「來源宣稱」與「尚未驗證」分開，避免把公開 benchmark 直接當成 Claro 的產品成績。

## 結論

使用者覺得目前精準度不足是合理的；本次診斷顯示問題不只是 LLM 品質，而是幾個效果疊加：

1. 本次實機設定中的相容 id `large-v3-mlx`，在 Tauri 版實際解析為 **Whisper large-v3-turbo**，不是完整 large-v3。OpenAI 的 [Turbo model card](https://huggingface.co/openai/whisper-large-v3-turbo) 說明它把 large-v3 的 decoder 從 32 層裁到 4 層，以速度換取小幅品質退化。
2. Pipeline 固定傳入 `zh`。本次以 Turbo 跑中文、技術中英混講與英文共四個 TTS smoke，`auto`／`zh` 的內容結果沒有可觀察差異；這既不能證明固定 `zh` 是問題，也不能外推真人短詞或 code-switch。現行 `transcribe-cpp 0.1.1` 的 Qwen3-ASR 實作可接受 caller language hint，因此 production 先維持 `zh`，把 `auto`／`zh` 留在真人 corpus 做成對 release gate。
3. **CLEAN 的 Meaning Lock 刻意禁止 LLM 改任何文字／英數 token**。因此 LLM 能刪除有明確停頓邊界的填充詞、整理標點，但不能把 STT 已聽錯的 `PyTorch`、姓名或數字猜回來。這是避免「看起來通順但意思被改掉」的信任契約，不是可以放寬的錯誤。
4. 本機 history metadata 的少量樣本中，內建模型的 CLEAN 路徑曾在約 6.9 秒回 `provider_error` 並退回原文；修正前的程式從模型 hash／冷載入前就開始計算生成 deadline。現在每次內建整理有 8 秒總額度、生成最多 6 秒且不得超過總額度，並以 `try_lock` 避免兩個要求排隊；阻塞式 Metal cold load 若已超時，模型保留供下一次 warm call，本段安全退回原文。cold load 本身仍不可中斷，需由正式 bundle 量測最壞值。
5. Context 詞彙偏置有效，但它只解決「已知詞彙」；不能取代更好的基礎 ASR。Qwen3-ASR 的 GGUF port 目前又沒有 Whisper `initial_prompt` 能力，所以需要保留完整 Whisper large-v3 作為技術詞／專有詞模式。

因此採取的策略不是讓 LLM 自由猜字，而是先提高 STT 底座：**16GB 以上 Apple Silicon 推薦完整 Whisper large-v3；8–12GB Apple Silicon 推薦 large-v3 Q5；Intel 推薦 Turbo Q5。** Turbo 降為速度優先選項，既有 Turbo 使用者會在首頁看到可略過的精準度升級入口；仍須自行確認下載大小並點擊，不會自動下載。Qwen3-ASR 0.6B／1.7B 只保留為離線評測候選：現行 port 每次最多生成 256 tokens、Claro 尚未完成長音訊分段，因此正式 UI 不允許下載或啟用。

## 證據分層

| 層級 | 目前能下的結論 | 不能下的結論 |
|---|---|---|
| 本機已驗證 | 實機使用的是 Turbo；CLEAN 不改詞；少量 builtin cold-start 樣本退回原文；合成 TTS smoke 中完整 large-v3 與 Context 詞彙偏置修正了 Turbo 未辨識好的個案；四案 `auto`／`zh` smoke 未見內容差異 | 不能從少量 TTS 個案推算真人 CER，也不能宣稱哪種 language profile 對真人普遍更準，或已全面解決台灣口音、環境噪音與遠講問題 |
| 模型作者／publisher 公開資料 | Qwen 官方表格中，CV-zh-tw 的 Qwen3-ASR 1.7B 為 3.77、0.6B 為 5.59、Whisper large-v3 為 7.84（表格欄名 WER，越低越好） | 這是 Qwen publisher 自己的評測，不是獨立測試，也不是 Claro GGUF port 在使用者 Mac 上的結果 |
| transcribe.cpp port 資料 | Qwen3-ASR 已可直接走 Claro 現有的 transcribe.cpp／Metal 技術棧，不需要 Python、Ollama 或另外啟動本機服務；port 有可重跑的數值驗證 | 0.6B 的中文 port 數字不能外推到 1.7B 或台灣真人聽寫；英文 LibriSpeech WER 也不能代表繁中品質 |
| 尚未驗證 | — | Claro 本機 Qwen 0.6B／1.7B 的端到端結果；真人台灣華語、台語口音、中英 code-switch、專有詞、數字日期與噪音 corpus 的 CER／term accuracy／p50／p95 |

## 本機 smoke：它證明了什麼

依專案驗證鐵律，本次只用 `say -o` 產生測試音訊，再由 `transcribe_file` 走 STT；沒有播放音訊，也沒有合成全域鍵盤事件。

- 技術句與數字句：目前 Turbo 對 `PyTorch` 與數字段落有錯；同一音訊改用 prototype 已快取的 MLX 完整 large-v3 後，該 code-switch／數字個案正確。
- 產品名句：Turbo 對 Claro／Typeless 等名稱有錯；加入 Claro、Typeless、Apple Intelligence 等 Context terms 後，完整 large-v3 在該合成個案辨識正確。

這只能支持兩個工程判斷：Turbo 不應再被當作「精準度優先」預設；Whisper 的 lexical bias 對畫面上已知專有詞有價值。`say` 聲音乾淨、發音穩定，與真人台灣使用者、內建麥克風距離及背景噪音不同，不能把這次 smoke 寫成產品準確率成績。

`desktop/tests/eval/stt_accuracy.json` 保存的是這類無聲合成 regression 的 reference、可接受格式等價句、terms、anchors、voice 與 rate，讓後續比較可重跑；它不包含真人錄音，也不能取代下面的 release corpus。產生器會把 reference／voice／rate 的 hash 與 WAV hash 寫入輸出目錄，內容變更時自動重生，避免拿舊音檔對新答案。跨 family 的基線預設 `no-prompt`；Context bias 必須用另一份明確標成 `prompt` 的報告，不能讓 Whisper 得到 oracle terms 後與不支援 prompt 的模型混比。

```bash
node desktop/scripts/generate-stt-fixtures.mjs --out /tmp/claro-stt-eval --force
cd desktop/src-tauri
cargo run --example eval_stt -- /tmp/claro-stt-eval large-v3-turbo zh no-prompt
cargo run --example eval_stt -- /tmp/claro-stt-eval large-v3-turbo auto no-prompt
```

本次重生全部 20 個 WAV 後，Turbo 的公平基線（不帶 oracle prompt）如下；`zh` 與 `auto` 的 20 案轉錄內容及三項 aggregate 完全相同：

| 語言 profile | micro CER | term recall | anchor accuracy |
|---|---:|---:|---:|
| `zh` | 5.38%（30 / 558） | 69.44%（25 / 36） | 82.43%（61 / 74） |
| `auto` | 5.38%（30 / 558） | 69.44%（25 / 36） | 82.43%（61 / 74） |

主要失分集中在 email、URL 與台灣／產品專有詞，符合「一般句子看似不差，但關鍵 token 一錯就很刺眼」的使用體感。這一輪執行環境無法建立 Metal command queue，實際退回 CPU；因此只採內容結果，**不把該次 latency 當成 macOS 產品效能證據**。合成 corpus 仍只是 regression smoke，production 是否保留 `zh` 必須由同一批真人語音成對決定。

## 公開模型資料與採用理由

### Qwen3-ASR

[Qwen3-ASR 官方 repo](https://github.com/QwenLM/Qwen3-ASR)與[技術報告](https://arxiv.org/abs/2601.21337)公布 0.6B／1.7B 兩個模型，支援 30 種語言與 22 種中文方言。作者的公開資料表中：

| Public dataset | Whisper large-v3 | Qwen3-ASR 0.6B | Qwen3-ASR 1.7B |
|---|---:|---:|---:|
| CV-zh-tw | 7.84 | 5.59 | **3.77** |
| Fleurs-zh | 4.09 | 2.88 | **2.41** |
| WenetSpeech net / meeting | 9.86 / 19.11 | 5.97 / 6.88 | **4.97 / 5.88** |

這些數字是**模型 publisher 的同表比較**，可作為導入候選的證據，不能當作 Claro 已達到的結果。

Claro 不採用 Qwen 官方 Python／vLLM 部署路徑，而使用 `Cargo.lock` 鎖定的 crates.io `transcribe-cpp`／`transcribe-cpp-sys 0.1.1`（兩個 package checksum 分別為 `b32749…874d52`、`28e526…578d0`）。該版隨 crate tarball 內嵌 [Qwen3-ASR source](https://github.com/handy-computer/transcribe.cpp/blob/d89ecb75062e8457681c563994675dc60e31db80/src/arch/qwen3_asr/model.cpp)，直接 in-process 載入 GGUF 並走 Metal，使用者不需要安裝 Python、Ollama、Homebrew 或管理服務程序。產品行為以 Cargo.lock＋crate tarball＋本機測試為準，不能拿持續變動的 GitHub `main` 代替 executable source。

### 固定 artifacts

| Claro 方案 | 固定 revision／檔案 | 大小 | SHA-256 | 定位 |
|---|---|---:|---|---|
| Qwen3-ASR 0.6B Q8_0 | [`e4e1659…/Qwen3-ASR-0.6B-Q8_0.gguf`](https://huggingface.co/handy-computer/Qwen3-ASR-0.6B-gguf/resolve/e4e16599b900eb0cb36e524514756bb92eb092b7/Qwen3-ASR-0.6B-Q8_0.gguf) | 850,423,456 bytes（約 811 MiB） | `f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8` | 離線評測候選；正式 UI 鎖定 |
| Qwen3-ASR 1.7B Q8_0 | [`92282af…/Qwen3-ASR-1.7B-Q8_0.gguf`](https://huggingface.co/handy-computer/Qwen3-ASR-1.7B-gguf/resolve/92282af1610a2db19d66f2bef1e260f5deca782d/Qwen3-ASR-1.7B-Q8_0.gguf) | 2,185,030,624 bytes（約 2,084 MiB） | `9a0d81792dfea2d5f278b8a63deb3ea6e02139ce42c2301f32ea19c4f77526b7` | 離線評測候選；正式 UI 鎖定 |

正式可用模型的下載器仍須遵守 Claro 的既有合約：固定 immutable revision、下載後完整 SHA-256 校驗、原子落盤；**沒有使用者明確點擊同意就不下載**。Qwen 候選在通過長音訊與真人語料 gate 前，即使知道 registry id 也會被 production command 拒絕。

### Port benchmark 的限制

[transcribe.cpp 0.6B 文件（v0.1.1 commit）](https://github.com/handy-computer/transcribe.cpp/blob/d89ecb75062e8457681c563994675dc60e31db80/docs/models/qwen3-asr-0.6b.md)公布 Q8_0 在 FLEURS-zh 的 port CER 為 7.64%，同一流程執行 author reference 為 7.60%，在其 bootstrap noise 內；這支持「port 與 author runtime 接近」，但它和 Qwen 官方表的 Fleurs-zh 0.6B 2.88／1.7B 2.41 明顯不同，可能來自 normalization、資料處理或評測方法差異，**不能混成同一組數字**。[1.7B port 文件（同 commit）](https://github.com/handy-computer/transcribe.cpp/blob/d89ecb75062e8457681c563994675dc60e31db80/docs/models/qwen3-asr-1.7b.md)只公布 LibriSpeech test-clean Q8_0 WER 1.61，沒有中文 port CER。

這裡有兩個不能混在一起的限制：目前 upstream [`main` family 文件](https://github.com/handy-computer/transcribe.cpp/blob/main/docs/models/qwen3-asr.md)寫的是約 87 分鐘的**音訊輸入 context 上限**，且公開 capability 只列 auto-detect；Claro 鎖定的 0.1.1 bundled source 則另有每次 `k_max_new = 256` 的**文字輸出 generation budget**，也實作 caller-supplied language hint。長音訊因此可能通過 input-length gate，卻因轉錄文字超過 256 tokens 而回 `OUTPUT_TRUNCATED`；upstream 文件沒有列出 forced hint，也不能反推 pinned crate 會拒絕 hint。Claro 目前仍把最長五分鐘音訊單次送入，尚未完成以靜音分段、截斷重試與文字合併，所以 Qwen 必須先鎖為離線候選；未來升級 crate 時也要重新跑相同 corpus，不能沿用 0.1.1 的結論。

## 解碼與 Context 策略

1. **production 暫留 `zh`，評測同時跑 `auto`／`zh`**：現行 Whisper 與 Qwen backend 都可接收 language hint；四案合成 smoke 未見內容差異，沒有真人證據支持全面改成 auto。評測工具會把 requested/effective profile 寫進報告，避免混比。
2. **Whisper prompt 只放詞彙，不放指令句**：`initial_prompt` 是解碼前文，不是 LLM system prompt。無詞彙時傳 `None`；有詞彙時只給經預算限制的字典／Context canonical terms。
3. **Whisper 正式可用，Qwen 僅作候選**：完整 Whisper large-v3 是目前精準度推薦，並可使用 lexical bias；Qwen 仍可由離線 runner 做 A/B，但 production resolve、下載與啟用都 fail closed。現有 Qwen family 不接受 Whisper run extension，不能假裝 Context prompt 已送進去。
4. **CLEAN 不負責猜回錯字**：專有詞仍由 STT Context、個人字典的確定性替換與模型選擇處理。ORGANIZE 才能使用 bounded ScreenContext 做格式判斷，且仍受 Meaning Lock 保護。
5. **內建 LLM 同時有 generation 與 total cap**：generation 最多 6 秒，整次內建整理總額度 8 秒；忙碌時不排隊，準備前後都檢查取消與總額度。現行 Metal load 是阻塞呼叫，若 load 回來才發現超時，模型保留供下次 warm call但本文立即 fallback；正式 bundle 仍須量 cold p50／p95。高記憶體 profile 的 `keep_models_warm` 尚未接到 LLM prewarm，不能宣稱 Mac 資源優化已經極致。

## 與 Typeless 的可觀察對標

Typeless 的[隱私政策](https://www.typeless.com/privacy)明示會把 voice audio 與有限 Context（使用中的 app、相關文字）一起處理，以提供 context-aware transcription；其[官方 onboarding](https://www.typeless.com/help/installation-and-setup)展示清單／email 格式化，以及使用者改口後只保留最後時間。這些只能作為產品行為的驗收目標；依 SPEC D14，不推測 Typeless 的內部模型、prompt 或私有管線。

Claro 的差異是音訊與 Context 預設全本地，Context 可稽核且受限；但要對標「感覺很準」，不能只看通順度。評測必須把底層轉錄正確率與潤飾格式品質分開：

- STT：normalized CER、英文／數字／專有詞 exact-match、漏字／幻覺率、靜音 false positive。
- CLEAN／ORGANIZE：Meaning Lock 通過率、填充詞／改口／列舉格式正確率、fallback 原因。
- 體感：release-to-text p50／p95、cold／warm、不同硬體與麥克風距離。

## Release gate 與剩餘工作

在以下項目完成前，不宣稱「準確率問題已解決」或「優於 Typeless」：

1. 建立可重跑、經同意錄製的台灣真人語音集，至少分成：短句、長句改口、中英 code-switch、姓名／產品名、數字日期、輕噪音與遠講。音訊不得進一般 history；測試資料的保存／公開範圍要另行同意。
2. 同一批音訊先跑 Whisper large-v3／Turbo；Qwen3-ASR 0.6B／1.7B 必須先完成長音訊分段與截斷重試再加入正式比較。固定 normalizer，輸出逐 case diff、content CER、formatting accuracy、term accuracy、p50／p95與峰值 RSS。
3. 對 Context on/off 做成對測試；Whisper 技術詞模式必須證明 bias 的增益，並同時計算誤偏置率。
4. 用正式 `.app` 驗證 cold start、warm run、閒置卸載後重載，以及 builtin CLEAN 的 8 秒 total cap／warm-next-call 行為；阻塞式 cold load 若超過總額度，不得再追加完整 generation 時間。
5. 由至少三種使用者情境做盲測：一般繁中聊天、開發者技術聽寫、跨 app 的文件／email 整理。記錄錯字與操作中斷，不只收「順不順」主觀分數。

## 主要來源

- [Qwen3-ASR 官方 repo（模型說明與 publisher benchmark）](https://github.com/QwenLM/Qwen3-ASR)
- [Qwen3-ASR Technical Report](https://arxiv.org/abs/2601.21337)
- [transcribe.cpp v0.1.1：Qwen3-ASR family](https://github.com/handy-computer/transcribe.cpp/blob/d89ecb75062e8457681c563994675dc60e31db80/docs/models/qwen3-asr.md)
- [transcribe.cpp v0.1.1：Qwen3-ASR 0.6B port](https://github.com/handy-computer/transcribe.cpp/blob/d89ecb75062e8457681c563994675dc60e31db80/docs/models/qwen3-asr-0.6b.md)
- [transcribe.cpp v0.1.1：Qwen3-ASR 1.7B port](https://github.com/handy-computer/transcribe.cpp/blob/d89ecb75062e8457681c563994675dc60e31db80/docs/models/qwen3-asr-1.7b.md)
- [transcribe.cpp current `main` family 文件（只用來追蹤 upstream 漂移）](https://github.com/handy-computer/transcribe.cpp/blob/main/docs/models/qwen3-asr.md)
- [OpenAI Whisper large-v3-turbo model card](https://huggingface.co/openai/whisper-large-v3-turbo)
- [Typeless Privacy Policy](https://www.typeless.com/privacy)
- [Typeless Installation & Setup](https://www.typeless.com/help/installation-and-setup)
