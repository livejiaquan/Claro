import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MicLevel, ModelInfo, Status } from "../types";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import Onboarding from "./Onboarding";

afterEach(cleanup);

const model = (overrides: Partial<ModelInfo> = {}): ModelInfo => ({
  id: "large-v3-q5_0",
  label: "Large v3（量化）",
  desc: "接近最高品質的省空間選擇",
  size_mb: 1024,
  recommended: true,
  available: true,
  preview: false,
  downloaded: false,
  active: false,
  downloading: false,
  ...overrides,
});

const status = (overrides: Partial<Status> = {}): Status => ({
  model_id: "large-v3-q5_0",
  model_label: "Large v3（量化）",
  model_present: false,
  model_approx_mb: 1024,
  accessibility: false,
  hotkey_active: false,
  input_device: null,
  default_input: "MacBook Pro Microphone",
  input_devices: [],
  dictation_state: "idle",
  context_enabled: true,
  hotkey: "Opt+Shift+C",
  setup_completed: false,
  successful_pastes_this_launch: 0,
  history_enabled: true,
  mic_test_passed_this_launch: false,
  ...overrides,
});

const mic: MicLevel = {
  level: 0,
  active: false,
  generation: 0,
  passed: false,
  timed_out: false,
};

function configureInvoke({ ready = false }: { ready?: boolean } = {}) {
  invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "list_models":
        return [model({ downloaded: ready, active: ready })];
      case "get_llm_config":
        return ready
          ? {
              provider: "apple",
              model: "",
              base_url: "",
              has_key: false,
              apple_status: 0,
              polish_mode: "clean",
              effective_mode: "clean",
            }
          : {
              provider: "off",
              model: "",
              base_url: "",
              has_key: false,
              apple_status: 1,
              polish_mode: "clean",
              effective_mode: "raw",
            };
      case "list_builtin_llms":
        return [];
      case "get_hardware_profile":
        return {
          architecture: "arm64",
          memory_gb: 8,
          tier: "balanced",
          low_memory_mode: false,
          keep_models_warm: false,
          recommended_stt: "large-v3-q5_0",
          recommended_llm_provider: "builtin",
          recommended_llm_model: "qwen3-4b",
          reason: "測試",
        };
      default:
        return null;
    }
  });
}

function renderOnboarding(currentStatus: Status) {
  return render(
    <Onboarding
      status={currentStatus}
      mic={mic}
      progress={null}
      llmProgress={null}
      onDownloadStart={vi.fn()}
      refresh={vi.fn()}
      onToast={vi.fn()}
      onDone={vi.fn()}
      onOpenSettings={vi.fn()}
    />,
  );
}

describe("Onboarding mission clarity", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("keeps text organization optional while explaining the three required checks", async () => {
    configureInvoke();
    renderOnboarding(status());

    await waitFor(() => expect(screen.getByText("下載前先知道")).toBeTruthy());

    expect(screen.getByText("0/3")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "下一步：做第一次聽寫" })).toBeTruthy();
    expect(screen.getByText("先完成上方 3 項必要檢查；完成後回到這裡做一次貼上測試。")).toBeTruthy();
    expect(screen.getByText(/需要網路.*時間取決於網路速度.*可續傳/)).toBeTruthy();
    expect(screen.getByText("先完成 3 項必要檢查；接著做一次真正的聽寫，確認文字能貼到游標處。")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "文字整理（選用）" })).toBeTruthy();
    expect(screen.getByText(/不會修正語音辨識錯字.*不是完成設定的必要步驟/)).toBeTruthy();
  });

  it("turns the next card into an actionable final check after the three requirements pass", async () => {
    configureInvoke({ ready: true });
    renderOnboarding(
      status({
        model_present: true,
        accessibility: true,
        hotkey_active: true,
        mic_test_passed_this_launch: true,
      }),
    );

    await waitFor(() => expect(screen.getByText("3/3")).toBeTruthy());

    expect(screen.getByText("下一步：第一次聽寫")).toBeTruthy();
    expect(screen.getByText("這是前三項檢查後的最後確認，不計入上方檢查分數；做完後回到 Claro 完成設定。")).toBeTruthy();
    expect(screen.getByRole("list")).toBeTruthy();
    expect(screen.getByRole("button", { name: "完成首次設定" })).toHaveProperty("disabled", true);
    expect(screen.getByRole("heading", { name: "下一步：做第一次聽寫" })).toBeTruthy();
  });
});
