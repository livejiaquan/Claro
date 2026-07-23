import { describe, expect, it } from "vitest";
import {
  codexConsentIsReady,
  codexProviderReducer,
  codexTestIsPending,
  consumeCancelledCodexTest,
  createCodexProviderUiState,
  nextCodexStatusRequestGeneration,
  nextCodexTestRequestGeneration,
  normalizeCodexStatus,
} from "./codexProviderState";
import {
  resolveLlmConfig,
  type CodexPreferences,
  type CodexStatus,
  type LlmConfig,
} from "./types";

const config = (overrides: Partial<LlmConfig> = {}): LlmConfig => ({
  provider: "off",
  model: "",
  base_url: "",
  has_key: false,
  apple_status: 0,
  ...overrides,
});

const preferences = (
  overrides: Partial<CodexPreferences> = {},
): CodexPreferences => ({
  correction_preferences: "",
  share_context_terms: false,
  consent_valid: true,
  context_consent_valid: false,
  correct_consent_valid: true,
  correct_mode_active: true,
  ...overrides,
});

const runnerStatus = (overrides: Partial<CodexStatus> = {}): CodexStatus => ({
  availability: "ready",
  version: "1.2.3",
  auth_mode: "chat_gpt",
  executable_path: "/private/path/codex",
  error_code: null,
  ...overrides,
});

describe("resolveLlmConfig Codex privacy gates", () => {
  it("forces Codex to cloud even if a stale backend labels it on-device", () => {
    const resolved = resolveLlmConfig(
      config({
        provider: "codex",
        polish_mode: "clean",
        execution_location: "on_device",
        endpoint_origin: "http://localhost:1234",
        local_only: false,
        codex_consent_valid: true,
      }),
    );

    expect(resolved.execution_location).toBe("cloud");
    expect(resolved.endpoint_origin).toBeNull();
    expect(resolved.destination_label).toBe("OpenAI Codex");
  });

  it("fails closed when Codex cloud consent is absent", () => {
    const resolved = resolveLlmConfig(
      config({
        provider: "codex",
        polish_mode: "clean",
        effective_mode: "clean",
        local_only: false,
      }),
    );

    expect(resolved.blocked_reason).toBe("codex_consent_required");
    expect(resolved.effective_mode).toBe("raw");
  });

  it("requires independent consent before correct may change tokens", () => {
    const resolved = resolveLlmConfig(
      config({
        provider: "codex",
        polish_mode: "correct",
        local_only: false,
        codex_consent_valid: true,
      }),
    );

    expect(resolved.blocked_reason).toBe("correct_consent_required");
    expect(resolved.effective_mode).toBe("raw");
  });

  it("requires context consent only when limited context terms are shared", () => {
    const blocked = resolveLlmConfig(
      config({
        provider: "codex",
        polish_mode: "correct",
        local_only: false,
        correct_consent_valid: true,
        codex_consent_valid: true,
        codex_share_context_terms: true,
      }),
    );
    const ready = resolveLlmConfig(
      config({
        provider: "codex",
        polish_mode: "correct",
        local_only: false,
        correct_consent_valid: true,
        codex_consent_valid: true,
        codex_share_context_terms: true,
        codex_context_consent_valid: true,
      }),
    );

    expect(blocked.blocked_reason).toBe("codex_context_consent_required");
    expect(ready.blocked_reason).toBeNull();
    expect(ready.effective_mode).toBe("correct");
  });
});

describe("Codex status normalization", () => {
  it("maps runner auth and availability into the safe UI contract", () => {
    expect(normalizeCodexStatus(runnerStatus())).toEqual({
      availability: "ready",
      version: "1.2.3",
      auth_mode: "chatgpt",
      error_code: null,
    });
    expect(
      normalizeCodexStatus(
        runnerStatus({
          availability: "not_authenticated",
          auth_mode: null,
        }),
      ),
    ).toMatchObject({
      availability: "auth_required",
      auth_mode: "unknown",
    });
  });

  it("collapses internal capability and probe failures into unavailable", () => {
    expect(
      normalizeCodexStatus(
        runnerStatus({ availability: "missing_capability" }),
      ).availability,
    ).toBe("unavailable");
    expect(
      normalizeCodexStatus(
        runnerStatus({ availability: "probe_failed" }),
      ).availability,
    ).toBe("unavailable");
  });
});

describe("codexProviderReducer", () => {
  it("ignores a stale status response after a newer refresh", () => {
    let state = createCodexProviderUiState();
    state = codexProviderReducer(state, {
      type: "status_requested",
      generation: nextCodexStatusRequestGeneration(state),
    });
    const staleGeneration = state.status_request_generation;
    state = codexProviderReducer(state, {
      type: "status_requested",
      generation: nextCodexStatusRequestGeneration(state),
    });
    state = codexProviderReducer(state, {
      type: "status_resolved",
      generation: staleGeneration,
      status: {
        availability: "ready",
        version: "old",
        auth_mode: "chatgpt",
        error_code: null,
      },
    });

    expect(state.status.availability).toBe("checking");
    expect(state.status.version).toBeNull();
  });

  it("prevents late success from overriding cancellation", () => {
    let state = createCodexProviderUiState();
    const generation = nextCodexTestRequestGeneration(state);
    state = codexProviderReducer(state, {
      type: "test_started",
      generation,
    });
    state = codexProviderReducer(state, {
      type: "test_cancel_requested",
      generation,
    });
    state = codexProviderReducer(state, {
      type: "test_succeeded",
      generation,
      input: "hyTorch",
      output: "PyTorch",
    });

    expect(state.test.phase).toBe("cancelling");
    expect(codexTestIsPending(state.test)).toBe(true);

    state = codexProviderReducer(state, {
      type: "test_cancelled",
      generation,
    });
    expect(state.test.phase).toBe("cancelled");
  });

  it("ignores a previous test result after a retry starts", () => {
    let state = createCodexProviderUiState();
    const first = nextCodexTestRequestGeneration(state);
    state = codexProviderReducer(state, {
      type: "test_started",
      generation: first,
    });
    const second = nextCodexTestRequestGeneration(state);
    state = codexProviderReducer(state, {
      type: "test_started",
      generation: second,
    });
    state = codexProviderReducer(state, {
      type: "test_failed",
      generation: first,
      reason: "timeout",
    });

    expect(state.test.phase).toBe("running");
    expect(state.test_request_generation).toBe(second);
  });
});

describe("Codex consent helpers", () => {
  it("allows transcript-only use without global screen context", () => {
    expect(codexConsentIsReady(preferences(), false)).toBe(true);
  });

  it("does not claim Codex is connected outside CORRECT mode", () => {
    expect(
      codexConsentIsReady(preferences({ correct_mode_active: false }), true),
    ).toBe(false);
  });

  it("falls back to transcript-only when global context is off", () => {
    const sharing = preferences({
      share_context_terms: true,
      context_consent_valid: true,
    });

    expect(codexConsentIsReady(sharing, false)).toBe(true);
    expect(codexConsentIsReady(sharing, true)).toBe(true);
    expect(
      codexConsentIsReady(
        { ...sharing, context_consent_valid: false },
        true,
      ),
    ).toBe(false);
  });
});

describe("queued Codex test cancellation", () => {
  it("consumes only the matching generation once", () => {
    const cancelled = new Set([3]);

    expect(consumeCancelledCodexTest(cancelled, 3)).toBe(true);
    expect(consumeCancelledCodexTest(cancelled, 3)).toBe(false);
    expect(consumeCancelledCodexTest(cancelled, 4)).toBe(false);
  });
});
