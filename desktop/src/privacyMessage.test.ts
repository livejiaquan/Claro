import { describe, expect, it } from "vitest";
import { polishReadinessLabel, privacyMessage } from "./privacyMessage";
import { resolveLlmConfig, type LlmConfig } from "./types";

const config = (overrides: Partial<LlmConfig> = {}): LlmConfig => ({
  provider: "codex",
  model: "",
  base_url: "",
  has_key: false,
  apple_status: 0,
  polish_mode: "correct",
  effective_mode: "correct",
  local_only: false,
  correct_consent_valid: true,
  cloud_consent_valid: true,
  codex_consent_valid: true,
  ...overrides,
});

describe("privacyMessage Codex disclosure", () => {
  it("lists every non-screen text category sent to Codex", () => {
    const message = privacyMessage(resolveLlmConfig(config()), false, true);

    expect(message.title).toBe("雲端整理・專業校字");
    expect(message.detail).toContain("轉錄文字");
    expect(message.detail).toContain("正確拼法清單");
    expect(message.detail).toContain("個人詞彙中");
    expect(message.detail).toContain("最多 32 項");
    expect(message.detail).not.toContain("有限畫面候選詞");
    expect(message.detail).toContain("音訊不傳送");
  });

  it("adds limited screen candidates only when both sharing gates are active", () => {
    const message = privacyMessage(
      resolveLlmConfig(
        config({
          codex_share_context_terms: true,
          codex_context_consent_valid: true,
        }),
      ),
      false,
      true,
    );

    expect(message.detail).toContain("有限畫面候選詞");
    expect(message.detail).toContain("不含 App 名或視窗標題");
  });

  it("does not claim local processing when config loading fails", () => {
    const message = privacyMessage(null, true, true);

    expect(message.tone).toBe("warning");
    expect(message.title).toBe("無法確認資料處理位置");
  });

  it("labels an enabled Codex setup as OpenAI cloud instead of local", () => {
    const llm = resolveLlmConfig(config());

    expect(polishReadinessLabel(llm, true)).toBe(
      "OpenAI Codex 雲端校字已就緒",
    );
    expect(polishReadinessLabel(llm, true)).not.toContain("本機整理");
  });
});
