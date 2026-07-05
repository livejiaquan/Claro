# Handy 前端設計系統拆解（UI 參考）

日期：2026-07-05。方法：本地原始碼閱讀（Codex 執行）。用途：Claro UI 的結構參考（使用者指定要 Handy/Typeless 式產品級設計）。

## 版型

- Shell：`h-screen flex flex-col`，側欄＋內容＋固定 footer。側欄 `w-40`，右緣 `border-mid-gray/20`，logo 置頂。
- Nav active：品牌色填底（`bg-logo-primary/80`）；inactive `opacity-85` + hover 背景。
- 設定頁內容 `max-w-3xl mx-auto space-y-6`。
- Footer 狀態列：左＝模型選擇器（狀態圓點＋截斷名稱＋chevron），右＝更新檢查＋`v{version}`，`text-xs text-text/60`。有更新時變品牌色可點（開 GitHub releases）。

## 設定列/區塊元件（可直接對映我們的 Row/Section）

- `SettingContainer`：`flex items-center justify-between min-h-12 px-4 p-2`；grouped 模式去邊框由群組供框。標題 `text-sm font-medium`＋可選 info tooltip（w-4 h-4，hover 變品牌色，鍵盤可及）。
- `SettingsGroup`：標題 `text-xs font-medium uppercase tracking-wide text-mid-gray`；本體 `border rounded-lg` + `divide-y`。
- 快捷鍵 chip：`px-2 py-1 text-sm font-semibold bg-mid-gray/10 border rounded-md hover:border-logo-primary`。

## Tokens

Tailwind v4 CSS-first（無 config 檔）。`--color-text #0f0f0f`、`--color-background #fbfbfb`、accent 粉 `--color-logo-primary #faa2ca`、填色 `--color-background-ui #da5893`、`--color-mid-gray #808080`（一律配 /opacity 用）。根字級 15px/24px，系統字型。深色模式只換 text/background/logo 三色。

## 控制元件

- Toggle：hidden checkbox + peer，軌 `w-11 h-6`、球 `w-5 h-5`，checked 填 `background-ui`，focus ring 品牌色。
- Select：react-select 包裝，min-height 40px、radius 6px、粉色 focus。
- Dropdown：按鈕 `min-w-[200px] rounded-md`，選單 `absolute z-50 max-h-60` 有空狀態列。
- Slider：原生 range + inline linear-gradient 填色。

## Overlay（M5 重做膠囊時直接引用）

- 幾何 tokens：`--ov-rest-w: 172px`、`--ov-pill-w`、`--ov-work-w`、`--ov-open-w`、`--ov-base-h: 40px`。
- 形變動畫：寬度/圓角 morph `460ms cubic-bezier(0.22,1,0.36,1)`＋入場 `scard-pop`。
- 電平波形：9 根 bars，`prev*0.7 + target*0.3` 平滑，高度 `3 + pow(v,0.7)*15`（3–18px）。

## Onboarding

三態流：accessibility → model → done（回訪者跳過 model）。權限卡 `rounded-lg bg-white/5`＋圓形品牌色 icon 底＋CTA。模型選擇 `max-w-[600px]`，卡片 hover `shadow-lg scale-[1.01]`、active `scale-[0.99]`。

## 打磨細節

全面的 hover/focus transition；tooltip 是 portal＋視窗邊界感知（寬 200、留邊 12）；空狀態明確呈現。
