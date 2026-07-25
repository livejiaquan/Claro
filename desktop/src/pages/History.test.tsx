import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import History from "./History";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

afterEach(() => {
  cleanup();
  invokeMock.mockReset();
});

describe("History provider semantics", () => {
  it("labels the configured provider as selected instead of implying data was sent", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_history") {
        return Promise.resolve([
          {
            ts: "2026-07-26T10:00:00.000Z",
            duration_s: 1.2,
            raw: "我們用 Py Torch。",
            text: "我們用 PyTorch。",
            status: "pasted",
            polish: {
              mode: "correct",
              provider: "codex",
              changed: false,
              outcome: "fallback",
              fallback_reason: "local_only",
              codex_payload_started: false,
            },
          },
        ]);
      }
      if (command === "get_pending_result") return Promise.resolve(null);
      return Promise.resolve();
    });

    render(
      <History
        historyEnabled
        onCopied={vi.fn()}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText("選用：OpenAI Codex")).toBeTruthy();
    });
    const summary = screen.getByText("我們用 PyTorch。");
    const toggle = summary.closest("button");
    expect(toggle).not.toBeNull();
    fireEvent.click(toggle!);

    expect(screen.getByText("選用的整理引擎")).toBeTruthy();
    expect(screen.queryByText("文字整理位置")).toBeNull();
    expect(screen.getAllByText("保留本機結果").length).toBeGreaterThan(0);
    expect(screen.getByText("未送出文字・未用模型額度")).toBeTruthy();
    expect(screen.getByText("保留本機結果原因")).toBeTruthy();
    expect(
      screen.getByText(
        "未傳送；前置安全檢查在送出文字前停止，因此未使用模型額度",
      ),
    ).toBeTruthy();
    expect(screen.queryByText("安全退回原文")).toBeNull();
    expect(screen.getByText("僅限本機已阻擋雲端")).toBeTruthy();
  });

  it("distinguishes an empty candidate set from a blocked Codex request", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_history") {
        return Promise.resolve([
          {
            ts: "2026-07-26T10:05:00.000Z",
            duration_s: 0.8,
            raw: "今天用 Rust。",
            text: "今天用 Rust。",
            status: "pasted",
            polish: {
              mode: "correct",
              provider: "codex",
              changed: false,
              outcome: "unchanged",
              codex_payload_started: false,
            },
          },
        ]);
      }
      if (command === "get_pending_result") return Promise.resolve(null);
      return Promise.resolve();
    });

    render(
      <History
        historyEnabled
        onCopied={vi.fn()}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText("未送出文字・未用模型額度")).toBeTruthy();
    });
    const toggle = screen.getByText("今天用 Rust。").closest("button");
    expect(toggle).not.toBeNull();
    fireEvent.click(toggle!);

    expect(
      screen.getByText("未傳送；沒有合法候選，因此未使用模型額度"),
    ).toBeTruthy();
    expect(screen.queryByText("保留本機結果原因")).toBeNull();
  });
});
