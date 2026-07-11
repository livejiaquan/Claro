# Claro macOS 發行手冊

P0 的 release pipeline 是「候選檔 staging」，不會自動建立 GitHub Release，也沒有啟用自動更新。這是刻意的安全邊界：簽章、公證、sidecar 與最低系統版本都通過後，仍由 release environment 的人工審核者決定是否公開。

Workflow 內所有 GitHub Actions 都固定到完整 commit SHA；Apple ID、公證密碼與簽章 identity 只注入實際需要的檢查、憑證匯入或 build step，不放在 job-level environment。更新 action 版本時要重新查核 upstream commit，不得退回浮動 tag。

## 外部必要條件

- Apple Developer Program 帳號
- `Developer ID Application` 憑證（匯出為有密碼的 `.p12`）
- GitHub `release` environment，建議設定 required reviewers 並限制可部署的 tag
- 以下 GitHub Actions secrets：
  - `APPLE_CERTIFICATE`：`.p12` 的單行 base64
  - `APPLE_CERTIFICATE_PASSWORD`：匯出 `.p12` 時的密碼
  - `APPLE_SIGNING_IDENTITY`：完整的 Developer ID Application identity
  - `APPLE_ID`：公證使用的 Apple ID
  - `APPLE_PASSWORD`：該 Apple ID 的 app-specific password
  - `APPLE_TEAM_ID`：Apple Developer Team ID
  - `KEYCHAIN_PASSWORD`：只供 CI 暫存 keychain 使用的高強度隨機值

私鑰、憑證內容與密碼不得寫入 repo、workflow YAML、issue 或 build log。

## 產生候選檔

1. 確認 `desktop/src-tauri/tauri.conf.json`、`desktop/src-tauri/Cargo.toml` 與 `desktop/package.json` 的版本一致，並且 bundle identifier 維持為 `dev.claro.desktop`；公開後不得無遷移計畫更動，以免 macOS TCC 將它視為新 App。
2. 建立並推送完全相符的 tag，例如版本 `0.1.0` 只能使用 `v0.1.0`。
3. 從該 tag 手動執行 `release-candidate` workflow，選擇目標架構。
4. Workflow 會編譯對應架構的 `mic_indicator`、建立 `.app` 與 `.dmg`、完成 Developer ID 簽章與 Apple 公證，再驗證：
   - `Claro.app/Contents/MacOS/mic_indicator` 存在且可執行
   - sidecar 架構與 workflow 選項一致
   - app 與 sidecar 的最低 macOS 版本為 11.0
   - app 深層簽章、sidecar 簽章與 Gatekeeper assessment 通過
   - app 與 DMG 的 notarization ticket 可被 `stapler` 驗證
5. 下載 Actions artifact，在乾淨 Mac 完成 `docs/ROADMAP.md` 的 smoke checklist 後，才手動升格為公開 release。

## 自動更新仍未啟用

目前專案沒有 updater public key，也沒有更新 endpoint；workflow 因此不會產生 `latest.json` 或 updater signature。等實際的公開下載位置與 key rotation／外洩處理流程定案後，再一次加入真實 public key、HTTPS endpoint 與 updater CI。私鑰只應存在於 release secrets，不能提交到 repo。
