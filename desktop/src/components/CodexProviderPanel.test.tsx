import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  CodexCliStatus,
  CodexPreferences,
  CodexTestState,
} from "../types";
import CodexProviderPanel, {
  type CodexProviderPanelProps,
} from "./CodexProviderPanel";

afterEach(cleanup);

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

    expect(screen.getByRole("status").textContent).toContain(
      "不會送出文字或使用 Codex 額度",
    );
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

    expect(screen.getByRole("status").textContent).toContain(copy);
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
  it("saves a trimmed canonical spelling list and states that it will be sent", () => {
    const props = renderPanel();
    const textbox = screen.getByRole("textbox", {
      name: "正確拼法清單（選填）",
    });

    expect(textbox.getAttribute("maxlength")).toBe("1000");
    expect(screen.getByText(/清單會隨每次 Codex 校字送出/)).toBeTruthy();

    fireEvent.change(textbox, {
      target: { value: "  MLX、PyTorch  " },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "儲存正確拼法" }),
    );

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
});

describe("CodexProviderPanel test states", () => {
  it("starts an idle test through props", () => {
    const props = renderPanel();
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
      input: "我們用 Py Torch 跑訓練。",
      output: "我們用 PyTorch 跑訓練。",
    };
    renderPanel({ testState });

    const statuses = screen.getAllByRole("status");
    expect(
      statuses.some((status) =>
        status.textContent?.includes("受控校字測試已通過"),
      ),
    ).toBe(true);
    expect(screen.getByText(/校字結果：我們用 PyTorch/)).toBeTruthy();
  });

  it.each([
    ["timeout", "本次測試時間內完成"],
    ["rate_limited", "無法使用 Codex 額度"],
    ["auth_required", "Codex 登入已失效"],
    ["unavailable", "Codex 目前無法使用"],
    ["output_rejected", "未通過內容保護"],
    ["unknown", "無法完成受控校字測試"],
  ] as const)("renders the %s failure as an alert", (reason, copy) => {
    renderPanel({
      testState: { phase: "failed", reason },
    });

    expect(screen.getByRole("alert").textContent).toContain(copy);
  });
});
