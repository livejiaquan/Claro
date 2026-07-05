import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import type { DownloadProgress, MicLevel, Status } from "../types";
import { Hotkey, LevelBar, Row, Section } from "../ui";

export default function Settings({
  status,
  mic,
  progress,
  refresh,
}: {
  status: Status;
  mic: MicLevel;
  progress: DownloadProgress | null;
  refresh: () => void;
}) {
  const [error, setError] = useState<string | null>(null);

  const call = (cmd: string, args?: Record<string, unknown>) => {
    setError(null);
    invoke(cmd, args)
      .then(refresh)
      .catch((e) => setError(String(e)));
  };

  const permsOk = status.accessibility && status.hotkey_active;
  const pct =
    progress && progress.total_mb
      ? Math.min(100, (progress.downloaded_mb / progress.total_mb) * 100)
      : null;

  return (
    <div className="page-in">
      <h1 className="text-[24px] font-bold tracking-tight mb-6">設定</h1>

      <Section title="聽寫">
        <Row label="快捷鍵" sub="按住說話、放開出字；快點一下＝免持模式">
          <Hotkey />
        </Row>
      </Section>

      <Section title="聲音">
        <Row label="輸入裝置" sub={!status.input_device ? "跟隨系統預設" : "已固定，不隨系統切換"}>
          <select
            className="select no-drag"
            value={status.input_device ?? ""}
            onChange={(e) => {
              if (mic.active) invoke("mic_test_stop").catch(() => {});
              call("set_input_device", { name: e.target.value });
            }}
          >
            <option value="">
              系統預設{status.default_input ? `（${status.default_input}）` : ""}
            </option>
            {status.input_devices.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
        </Row>
        <Row
          label="麥克風測試"
          alignTop
          sub={
            mic.active
              ? mic.level > 0.01
                ? "收音正常——這就是聽寫時的音量。"
                : "太安靜——對著麥克風說幾句話，或換一個輸入裝置。"
              : "確認 Claro 聽得到你。"
          }
        >
          <div className="flex items-center gap-3 w-[260px] pt-1">
            <button
              className={`btn no-drag ${mic.active ? "recording" : ""}`}
              onClick={() => call(mic.active ? "mic_test_stop" : "mic_test_start")}
            >
              {mic.active ? "停止" : "測試"}
            </button>
            <LevelBar level={mic.level} active={mic.active} />
          </div>
        </Row>
      </Section>

      <Section title="語音模型">
        <Row
          label={`Whisper ${status.model_id}`}
          sub={
            status.model_present
              ? "已就緒・完全離線辨識"
              : `約 ${(status.model_approx_mb / 1024).toFixed(1)} GB，下載一次後完全離線`
          }
        >
          {status.model_present ? (
            <span className="pill green">
              <span className="dot" />
              已就緒
            </span>
          ) : progress ? (
            <div className="w-[180px]">
              <div className="progress-track">
                <div className="progress-fill" style={{ width: pct !== null ? `${pct}%` : "30%" }} />
              </div>
              <div className="text-[11px] mt-1 text-right" style={{ color: "var(--muted)" }}>
                {progress.downloaded_mb}/{progress.total_mb ?? "?"} MB
              </div>
            </div>
          ) : (
            <button className="btn-primary no-drag" onClick={() => call("download_model")}>
              下載模型
            </button>
          )}
        </Row>
      </Section>

      <Section title="權限">
        <Row
          label="輔助使用"
          sub={
            permsOk
              ? "已授權——全域快捷鍵與貼上功能正常。"
              : "系統設定 → 隱私與安全性 → 輔助使用，勾選 Claro 後重新啟動。"
          }
        >
          <span className={`pill ${permsOk ? "green" : "amber"}`}>
            <span className="dot" />
            {permsOk ? "已授權" : "需要授權"}
          </span>
        </Row>
      </Section>

      <Section title="關於">
        <Row label="Claro" sub="全程本地處理，語音與文字不離開這台機器。">
          <span className="text-[12.5px]" style={{ color: "var(--faint)" }}>
            v0.1.0
          </span>
        </Row>
      </Section>

      {error && (
        <p className="text-[13px] -mt-2" style={{ color: "var(--red)" }}>
          {error}
        </p>
      )}
    </div>
  );
}
