import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  CodexCliStatus,
  CodexPreferences,
  CodexTestState,
} from "../types";
import CodexProviderPanel, {
  type CodexProviderPanelProps,
} from "./CodexProviderPanel";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const readyStatus: CodexCliStatus = {
  availability: "ready",
  version: "1.2.3",
  auth_mode: "chatgpt",
  error_code: null,
};

const enabledPreferences: CodexPreferences = {
  correction_preferences: "",
  share_context_terms: false,
  consent_valid: true,
  context_consent_valid: true,
  correct_consent_valid: true,
  correct_mode_active: true,
};

function panelProps(
  overrides: Partial<CodexProviderPanelProps> = {},
): CodexProviderPanelProps {
  return {
    status: readyStatus,
    preferences: enabledPreferences,
    testState: { phase: "idle" },
    globalContextEnabled: true,
    onRefresh: vi.fn(),
    onEnable: vi.fn(),
    onSaveCorrectionPreferences: vi.fn(),
    onShareContextTermsChange: vi.fn(),
    onTest: vi.fn(),
    onCancelTest: vi.fn(),
    ...overrides,
  };
}

function renderPanel(overrides: Partial<CodexProviderPanelProps> = {}) {
  const props = panelProps(overrides);
  render(<CodexProviderPanel {...props} />);
  return props;
}

describe("CodexProviderPanel status states", () => {
  it("announces checking without implying that it consumes usage", () => {
    renderPanel({
      status: {
        availability: "checking",
        version: null,
        auth_mode: "unknown",
        error_code: null,
      },
    });

    expect(
      screen
        .getAllByRole("status")
        .some((status) =>
          status.textContent?.includes("不會送出文字或使用 Codex 額度"),
        ),
    ).toBe(true);
    expect(
      screen.getByRole("button", { name: "檢查中…" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it.each([
    ["not_installed", "沒有找到 Codex CLI"],
    ["auth_required", "尚未登入"],
    ["unsupported", "版本不支援"],
  ] as const)("renders the recoverable %s state", (availability, copy) => {
    renderPanel({
      status: {
        availability,
        version: "1.2.3",
        auth_mode: "unknown",
        error_code: null,
      },
    });

    expect(
      screen
        .getAllByRole("status")
        .some((status) => status.textContent?.includes(copy)),
    ).toBe(true);
    expect(
      screen.getByRole("button", { name: "重新檢查" }),
    ).toBeTruthy();
  });

  it("uses an alert for an unavailable probe without exposing diagnostics", () => {
    renderPanel({
      status: {
        availability: "unavailable",
        version: null,
        auth_mode: "unknown",
        error_code: "sensitive-internal-code",
      },
    });

    expect(screen.getByRole("alert").textContent).toContain(
      "暫時無法確認 Codex 狀態",
    );
    expect(screen.queryByText("sensitive-internal-code")).toBeNull();
  });

  it.each([
    ["chatgpt", "ChatGPT／Codex 登入與方案額度"],
    ["api_key", "可能依 API 用量計費"],
    ["unknown", "無法確認目前使用方案額度或 API 計費"],
  ] as const)("explains the %s auth mode honestly", (authMode, copy) => {
    renderPanel({
      status: { ...readyStatus, auth_mode: authMode },
    });

    expect(screen.getAllByRole("status")[0].textContent).toContain(copy);
  });
});

describe("CodexProviderPanel consent", () => {
  const pendingPreferences: CodexPreferences = {
    correction_preferences: "",
    share_context_terms: false,
    consent_valid: false,
    context_consent_valid: false,
    correct_consent_valid: false,
    correct_mode_active: false,
  };

  it("requires acknowledgement and defaults to transcript-only", () => {
    const props = renderPanel({
      preferences: pendingPreferences,
      globalContextEnabled: false,
    });
    const enable = screen.getByRole("button", {
      name: "同意並使用 Codex",
    });

    expect(enable.hasAttribute("disabled")).toBe(true);
    expect(
      screen
        .getByRole("checkbox", {
          name: "也允許送出本機萃取的有限畫面詞彙",
        })
        .hasAttribute("disabled"),
    ).toBe(true);

    fireEvent.click(
      screen.getByRole("checkbox", {
        name: /我了解上述文字資料會送到 OpenAI/,
      }),
    );
    fireEvent.click(enable);

    expect(props.onEnable).toHaveBeenCalledWith({
      share_context_terms: false,
    });
  });

  it("names the narrow capability and gives a counterexample before consent", () => {
    renderPanel({ preferences: pendingPreferences });

    expect(screen.getByText("Codex 專業拼法")).toBeTruthy();
    expect(screen.getByText(/Clau-de → Claude/)).toBeTruthy();
    expect(screen.getByText(/Py Torch → PyTorch/)).toBeTruthy();
    expect(screen.getByText(/不能證明語意一定正確/)).toBeTruthy();
    expect(screen.getByText(/三類候選詞合計最多 32 項/)).toBeTruthy();
    expect(screen.getByText(/App 名、視窗標題/)).toBeTruthy();
    expect(screen.getByText(/找不到可採用候選時完全不呼叫 Codex/)).toBeTruthy();
    expect(
      screen.getByText(/個人詞彙正確文字/),
    ).toBeTruthy();
  });

  it("lets a returning user inspect and clear stored spellings before re-consenting", async () => {
    const save = vi.fn().mockResolvedValue(undefined);
    renderPanel({
      preferences: {
        ...pendingPreferences,
        correction_preferences: "Claude\nInternalTerm",
      },
      onSaveCorrectionPreferences: save,
    });
    const textbox = screen.getByRole("textbox", {
      name: "正確拼法清單（選填）",
    }) as HTMLTextAreaElement;

    expect(textbox.value).toBe("Claude\nInternalTerm");
    expect(screen.getByText(/只儲存在本機/)).toBeTruthy();

    fireEvent.change(textbox, { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "立即儲存" }));
    await act(async () => {
      await Promise.resolve();
    });

    expect(save).toHaveBeenCalledWith("");
  });

  it("moves focus to the dynamically inserted primary consent heading", () => {
    renderPanel({ preferences: pendingPreferences });

    expect(document.activeElement).toBe(
      screen.getByText("允許 Claro 使用 Codex 統一專業拼法？"),
    );
  });

  it("hands focus to the connected content after primary consent succeeds", () => {
    const initial = panelProps({ preferences: pendingPreferences });
    const view = render(<CodexProviderPanel {...initial} />);

    view.rerender(
      <CodexProviderPanel
        {...initial}
        preferences={enabledPreferences}
      />,
    );

    expect(document.activeElement).toBe(screen.getByRole("note"));
    expect(screen.getByRole("note").textContent).toContain("資料界線");
  });

  it("keeps valid consent visible without claiming Codex is active in another mode", () => {
    renderPanel({
      preferences: {
        ...enabledPreferences,
        correct_mode_active: false,
      },
    });

    expect(screen.getByText("已同意・目前未使用")).toBeTruthy();
    expect(
      screen.queryByText("允許 Claro 使用 Codex 統一專業拼法？"),
    ).toBeNull();
    expect(screen.getByRole("note").textContent).toContain(
      "聽寫不會呼叫 Codex",
    );
    expect(
      screen.getByRole("button", { name: "測試" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("does not claim Codex is active when a returning user's probe is unavailable", () => {
    renderPanel({
      status: {
        availability: "unavailable",
        version: null,
        auth_mode: "unknown",
        error_code: "probe_failed",
      },
    });

    expect(screen.getByText("已同意・目前未使用")).toBeTruthy();
    expect(screen.getByRole("note").textContent).toContain(
      "聽寫不會呼叫 Codex",
    );
    expect(screen.queryByText("使用中")).toBeNull();
    expect(
      screen.getByRole("button", { name: "測試" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("clears a pending context opt-in when global context is revoked", () => {
    const initial = panelProps({ preferences: pendingPreferences });
    const view = render(<CodexProviderPanel {...initial} />);
    const contextOptIn = screen.getByRole("checkbox", {
      name: "也允許送出本機萃取的有限畫面詞彙",
    }) as HTMLInputElement;

    fireEvent.click(contextOptIn);
    expect(contextOptIn.checked).toBe(true);
    view.rerender(
      <CodexProviderPanel {...initial} globalContextEnabled={false} />,
    );
    view.rerender(
      <CodexProviderPanel {...initial} globalContextEnabled={true} />,
    );

    expect(
      (
        screen.getByRole("checkbox", {
          name: "也允許送出本機萃取的有限畫面詞彙",
        }) as HTMLInputElement
      ).checked,
    ).toBe(false);
  });

  it("closes and clears context confirmation after global context is revoked", () => {
    const initial = panelProps({
      preferences: {
        ...enabledPreferences,
        context_consent_valid: false,
      },
    });
    const view = render(<CodexProviderPanel {...initial} />);
    fireEvent.click(
      screen.getByRole("checkbox", { name: "有限畫面詞彙" }),
    );
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: "我了解這些詞彙會送到 OpenAI Codex。",
      }),
    );
    expect(
      screen.getByRole("button", { name: "允許有限畫面詞彙" }).hasAttribute(
        "disabled",
      ),
    ).toBe(false);

    view.rerender(
      <CodexProviderPanel {...initial} globalContextEnabled={false} />,
    );
    view.rerender(
      <CodexProviderPanel {...initial} globalContextEnabled={true} />,
    );

    expect(
      screen.queryByRole("button", { name: "允許有限畫面詞彙" }),
    ).toBeNull();
  });

  it("passes an explicit limited-terms opt-in when context is available", () => {
    const props = renderPanel({
      preferences: pendingPreferences,
      globalContextEnabled: true,
    });

    fireEvent.click(
      screen.getByRole("checkbox", {
        name: /我了解上述文字資料會送到 OpenAI/,
      }),
    );
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: "也允許送出本機萃取的有限畫面詞彙",
      }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "同意並使用 Codex" }),
    );

    expect(props.onEnable).toHaveBeenCalledWith({
      share_context_terms: true,
    });
  });

  it("shows a pending state and prevents duplicate enable requests", () => {
    renderPanel({
      preferences: pendingPreferences,
      enablePending: true,
    });

    const button = screen.getByRole("button", { name: "正在啟用…" });
    expect(button.hasAttribute("disabled")).toBe(true);
  });
});

describe("CodexProviderPanel preferences", () => {
  it("saves a trimmed canonical spelling list and states that it will be sent", async () => {
    const props = renderPanel();
    const textbox = screen.getByRole("textbox", {
      name: "正確拼法清單（選填）",
    });

    expect(textbox.getAttribute("maxlength")).toBe("1000");
    expect(
      screen.getByText(
        /左側至少四字母＋單一連字號＋右側兩字母.*內容保護可採用的項目才會送出/,
      ),
    ).toBeTruthy();

    fireEvent.change(textbox, {
      target: { value: "  MLX、PyTorch  " },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "立即儲存" }),
    );
    await act(async () => {
      await Promise.resolve();
    });

    expect(props.onSaveCorrectionPreferences).toHaveBeenCalledWith(
      "MLX、PyTorch",
    );
  });

  it("disables context terms when global screen context is off", () => {
    const props = renderPanel({ globalContextEnabled: false });
    const toggle = screen.getByRole("checkbox", {
      name: "有限畫面詞彙",
    });

    expect(toggle.hasAttribute("disabled")).toBe(true);
    fireEvent.click(toggle);
    expect(props.onShareContextTermsChange).not.toHaveBeenCalled();
    expect(screen.getByText(/不會送出任何畫面詞彙/)).toBeTruthy();
  });

  it("reports a context sharing change through props", () => {
    const props = renderPanel();

    fireEvent.click(
      screen.getByRole("checkbox", {
        name: "有限畫面詞彙",
      }),
    );

    expect(props.onShareContextTermsChange).toHaveBeenCalledWith(true);
  });

  it("requires a separate acknowledgement before first sharing context terms", () => {
    const props = renderPanel({
      preferences: {
        ...enabledPreferences,
        context_consent_valid: false,
      },
    });

    fireEvent.click(
      screen.getByRole("checkbox", {
        name: "有限畫面詞彙",
      }),
    );
    expect(document.activeElement).toBe(
      screen.getByText("允許 Codex 使用有限畫面詞彙？"),
    );
    const allow = screen.getByRole("button", {
      name: "允許有限畫面詞彙",
    });
    expect(allow.hasAttribute("disabled")).toBe(true);
    expect(props.onShareContextTermsChange).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("checkbox", {
        name: "我了解這些詞彙會送到 OpenAI Codex。",
      }),
    );
    fireEvent.click(allow);

    expect(props.onShareContextTermsChange).toHaveBeenCalledWith(true);
  });

  it("restores focus to the context toggle when confirmation is cancelled", () => {
    renderPanel({
      preferences: {
        ...enabledPreferences,
        context_consent_valid: false,
      },
    });
    const toggle = screen.getByRole("checkbox", {
      name: "有限畫面詞彙",
    });

    fireEvent.click(toggle);
    fireEvent.click(screen.getByRole("button", { name: "保持關閉" }));

    expect(document.activeElement).toBe(toggle);
  });

  it("disables secondary consent actions while saving", () => {
    const initial = panelProps({
      preferences: {
        ...enabledPreferences,
        context_consent_valid: false,
      },
    });
    const view = render(<CodexProviderPanel {...initial} />);
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: "有限畫面詞彙",
      }),
    );
    view.rerender(
      <CodexProviderPanel {...initial} preferencesSaving />,
    );

    expect(
      screen.getByRole("button", { name: "保持關閉" }).hasAttribute("disabled"),
    ).toBe(true);
    expect(
      screen
        .getByRole("checkbox", {
          name: "我了解這些詞彙會送到 OpenAI Codex。",
        })
        .hasAttribute("disabled"),
    ).toBe(true);
  });

  it("keeps an unsaved spelling draft when unrelated props refresh", () => {
    const initial = panelProps();
    const view = render(<CodexProviderPanel {...initial} />);
    const textbox = screen.getByRole("textbox", {
      name: "正確拼法清單（選填）",
    }) as HTMLTextAreaElement;

    fireEvent.change(textbox, { target: { value: "PyTorch" } });
    expect(screen.getByText("即將自動儲存")).toBeTruthy();
    view.rerender(
      <CodexProviderPanel
        {...initial}
        status={{ ...readyStatus, version: "1.2.4" }}
      />,
    );

    expect(textbox.value).toBe("PyTorch");
  });

  it("autosaves after 700 ms without disabling typing during the request", async () => {
    vi.useFakeTimers();
    let finishSave: (() => void) | undefined;
    const save = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finishSave = resolve;
        }),
    );
    renderPanel({ onSaveCorrectionPreferences: save });
    const textbox = screen.getByRole("textbox", {
      name: "正確拼法清單（選填）",
    }) as HTMLTextAreaElement;

    fireEvent.change(textbox, { target: { value: "PyTorch" } });
    act(() => vi.advanceTimersByTime(699));
    expect(save).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(1);
      await Promise.resolve();
    });
    expect(save).toHaveBeenCalledWith("PyTorch");
    expect(textbox.disabled).toBe(false);
    expect(screen.getByText("自動儲存中…")).toBeTruthy();

    fireEvent.change(textbox, { target: { value: "PyTorch\nWhisper" } });
    expect(textbox.value).toBe("PyTorch\nWhisper");
    expect(textbox.disabled).toBe(false);

    await act(async () => {
      finishSave?.();
      await Promise.resolve();
    });
    expect(textbox.value).toBe("PyTorch\nWhisper");
    expect(screen.getByText("即將自動儲存")).toBeTruthy();
  });

  it("flushes a pending spelling draft immediately on blur", async () => {
    vi.useFakeTimers();
    const save = vi.fn().mockResolvedValue(undefined);
    renderPanel({ onSaveCorrectionPreferences: save });
    const textbox = screen.getByRole("textbox", {
      name: "正確拼法清單（選填）",
    });

    fireEvent.change(textbox, { target: { value: "  MLX  " } });
    fireEvent.blur(textbox);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith("MLX");
  });

  it("retains the draft and offers an inline retry after autosave fails", async () => {
    const save = vi
      .fn()
      .mockRejectedValue(new Error("Codex 設定目前無法完成。"));
    renderPanel({ onSaveCorrectionPreferences: save });
    const textbox = screen.getByRole("textbox", {
      name: "正確拼法清單（選填）",
    }) as HTMLTextAreaElement;

    fireEvent.change(textbox, { target: { value: "MLX\nPyTorch" } });
    fireEvent.blur(textbox);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(textbox.value).toBe("MLX\nPyTorch");
    expect(screen.getByRole("alert").textContent).toContain(
      "Codex 設定目前無法完成。",
    );
    expect(screen.getByRole("alert").textContent).toContain("草稿仍保留");
    expect(
      screen.getByRole("button", { name: "重試儲存" }),
    ).toBeTruthy();
  });

  it("best-effort flushes a debounced draft when the panel unmounts", async () => {
    vi.useFakeTimers();
    const save = vi.fn().mockResolvedValue(undefined);
    const view = render(
      <CodexProviderPanel
        {...panelProps({ onSaveCorrectionPreferences: save })}
      />,
    );

    fireEvent.change(
      screen.getByRole("textbox", {
        name: "正確拼法清單（選填）",
      }),
      { target: { value: "Whisper" } },
    );
    view.unmount();
    await Promise.resolve();
    await Promise.resolve();

    expect(save).toHaveBeenCalledWith("Whisper");
  });
});

describe("CodexProviderPanel test states", () => {
  it("starts an idle test through props", () => {
    const props = renderPanel();
    expect(screen.getByText(/實際用量取決於目前的 CLI/)).toBeTruthy();
    expect(screen.queryByText(/少量 Codex 使用量/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "測試" }));
    expect(props.onTest).toHaveBeenCalledOnce();
  });

  it("offers cancellation while running and disables duplicate cancellation", () => {
    const running = renderPanel({
      testState: { phase: "running" },
    });
    expect(screen.getByText("正在測試 Codex 校字…")).toBeTruthy();
    fireEvent.click(
      screen.getByRole("button", { name: "取消測試" }),
    );
    expect(running.onCancelTest).toHaveBeenCalledOnce();

    cleanup();
    renderPanel({ testState: { phase: "cancelling" } });
    expect(
      screen.getByRole("button", { name: "取消中…" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("renders a successful synthetic input and output", () => {
    const testState: CodexTestState = {
      phase: "success",
      input: "我們用 Clau-de 跑訓練。",
      output: "我們用 Claude 跑訓練。",
    };
    renderPanel({ testState });

    const statuses = screen.getAllByRole("status");
    expect(
      statuses.some((status) =>
        status.textContent?.includes("受控校字測試已通過"),
      ),
    ).toBe(true);
    expect(screen.getByText(/校字結果：我們用 Claude/)).toBeTruthy();
  });

  it.each([
    ["timeout", "本次測試時間內完成"],
    ["rate_limited", "無法使用 Codex 額度"],
    ["auth_required", "Codex 登入已失效"],
    ["unavailable", "Codex 目前無法使用"],
    ["consent_changed", "受控能力已改變"],
    ["output_rejected", "未通過內容保護"],
    ["unknown", "無法完成受控校字測試"],
  ] as const)("renders the %s failure as an alert", (reason, copy) => {
    renderPanel({
      testState: { phase: "failed", reason },
    });

    expect(screen.getByRole("alert").textContent).toContain(copy);
  });
});
