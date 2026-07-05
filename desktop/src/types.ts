export interface Status {
  model_id: string;
  model_present: boolean;
  model_approx_mb: number;
  accessibility: boolean;
  hotkey_active: boolean;
  input_device: string | null;
  default_input: string | null;
  input_devices: string[];
  dictation_state: "idle" | "recording" | "processing";
}

export interface DownloadProgress {
  downloaded_mb: number;
  total_mb: number | null;
  done: boolean;
  error: string | null;
}

export interface MicLevel {
  level: number;
  active: boolean;
}

export interface HistoryEntry {
  ts: string;
  duration_s: number;
  raw: string;
  text: string;
  status: string;
  timings?: { stt_ms?: number };
}
