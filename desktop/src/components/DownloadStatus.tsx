import type { DownloadUiState } from "../downloadState";

export default function DownloadStatus({
  state,
  label,
  onCancel,
  className = "",
}: {
  state: DownloadUiState;
  label: string;
  onCancel: () => void;
  className?: string;
}) {
  if (state.phase === "failed") {
    return (
      <div className="config-error mt-2" role="alert">
        下載失敗：{state.error}。請檢查網路或儲存空間後重試；已完成的部分會保留。
      </div>
    );
  }

  if (state.phase === "cancelled") {
    return (
      <div className="setup-inline-state mt-2" role="status">
        已取消下載。已完成的部分會保留，下次可從中斷處續傳。
      </div>
    );
  }

  if (!state.active) return null;

  const detail =
    state.phase === "cancelling"
      ? "正在取消…"
      : state.phase === "preparing"
        ? "正在準備下載，可能需要先釋放模型記憶體…"
        : state.progress
          ? `${state.progress.downloaded_mb}/${state.progress.total_mb ?? "?"} MB`
          : "正在下載…";
  const valueText =
    state.phase === "cancelling"
      ? `${label}下載正在取消`
      : state.phase === "preparing"
        ? `${label}正在準備下載`
        : state.progress
          ? `${label}已下載 ${state.progress.downloaded_mb} MB，共 ${state.progress.total_mb ?? "未知"} MB`
          : `${label}正在下載`;
  const announcement =
    state.phase === "cancelling"
      ? `${label}下載正在取消`
      : state.phase === "preparing"
        ? `${label}正在準備下載`
        : `${label}正在下載；你可以隨時取消`;

  return (
    <div className={`mt-2 ${className}`} aria-busy="true">
      <div
        className="progress-track"
        role="progressbar"
        aria-label={`下載 ${label}`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={state.percent === null ? undefined : Math.round(state.percent)}
        aria-valuetext={valueText}
      >
        <div
          className="progress-fill"
          style={{ width: state.percent === null ? "30%" : `${state.percent}%` }}
        />
      </div>
      <div
        className="text-[11px] mt-1 flex items-center gap-2"
        style={{ color: "var(--muted)" }}
      >
        <span>{detail}</span>
        <button
          className="btn danger-quiet no-drag"
          disabled={!state.canCancel}
          aria-label={`取消 ${label}下載`}
          onClick={onCancel}
        >
          {state.phase === "cancelling" ? "取消中…" : "取消"}
        </button>
      </div>
      <span className="sr-only" role="status" aria-live="polite">
        {announcement}
      </span>
    </div>
  );
}
