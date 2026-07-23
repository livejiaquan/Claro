import type { DownloadProgress } from "./types";

export type DownloadPhase =
  | "idle"
  | "preparing"
  | "downloading"
  | "cancelling"
  | "cancelled"
  | "failed"
  | "complete";

export type DownloadKind = "stt" | "llm";

export interface DownloadUiState {
  phase: DownloadPhase;
  active: boolean;
  canStart: boolean;
  canCancel: boolean;
  percent: number | null;
  progress: DownloadProgress | null;
  error: string | null;
}

export interface DownloadStateInput {
  modelId: string;
  downloaded: boolean;
  backendDownloading: boolean;
  progress: DownloadProgress | null;
  startRequested?: boolean;
  cancelRequested?: boolean;
}

export function isCancelledDownload(error: string | null | undefined): boolean {
  return Boolean(error?.includes("下載已取消"));
}

/**
 * 把事件與模型清單合併成唯一的前端下載狀態。
 *
 * progress 的終態優先於清單中的 downloading，因為事件抵達後清單可能仍是
 * 上一次查詢的快照。如此失敗或取消後，重試按鈕不會被 stale true 隱藏。
 */
export function resolveDownloadState({
  modelId,
  downloaded,
  backendDownloading,
  progress,
  startRequested = false,
  cancelRequested = false,
}: DownloadStateInput): DownloadUiState {
  const matchingProgress = progress?.model_id === modelId ? progress : null;
  const activationFinished =
    matchingProgress !== null && matchingProgress.activation_status !== "none";
  const completedByEvent =
    matchingProgress !== null && (matchingProgress.done || activationFinished);
  const eventError =
    matchingProgress !== null && !completedByEvent ? matchingProgress.error : null;

  if (eventError) {
    const phase = isCancelledDownload(eventError) ? "cancelled" : "failed";
    return {
      phase,
      active: false,
      canStart: !downloaded,
      canCancel: false,
      percent: null,
      progress: matchingProgress,
      error: phase === "failed" ? eventError : null,
    };
  }

  if (completedByEvent) {
    return {
      phase: "complete",
      active: false,
      canStart: false,
      canCancel: false,
      percent: 100,
      progress: matchingProgress,
      error: null,
    };
  }

  if (downloaded) {
    return {
      phase: "complete",
      active: false,
      canStart: false,
      canCancel: false,
      percent: 100,
      progress: matchingProgress,
      error: null,
    };
  }

  const eventActive = Boolean(matchingProgress);
  const active = eventActive || backendDownloading || startRequested;
  const total = matchingProgress?.total_mb ?? null;
  const percent =
    matchingProgress && total !== null && total > 0
      ? Math.min(100, Math.max(0, (matchingProgress.downloaded_mb / total) * 100))
      : null;

  if (cancelRequested && active) {
    return {
      phase: "cancelling",
      active: true,
      canStart: false,
      canCancel: false,
      percent,
      progress: matchingProgress,
      error: null,
    };
  }

  if (eventActive) {
    return {
      phase: "downloading",
      active: true,
      canStart: false,
      canCancel: true,
      percent,
      progress: matchingProgress,
      error: null,
    };
  }

  if (backendDownloading || startRequested) {
    return {
      phase: "preparing",
      active: true,
      canStart: false,
      canCancel: true,
      percent: null,
      progress: null,
      error: null,
    };
  }

  return {
    phase: "idle",
    active: false,
    canStart: true,
    canCancel: false,
    percent: null,
    progress: matchingProgress,
    error: null,
  };
}
