export interface Status {
  model_id: string;
  model_label: string;
  model_present: boolean;
  model_approx_mb: number;
  accessibility: boolean;
  hotkey_active: boolean;
  input_device: string | null;
  default_input: string | null;
  input_devices: string[];
  dictation_state: "idle" | "recording" | "processing";
  context_enabled: boolean;
}

export interface ModelInfo {
  id: string;
  label: string;
  desc: string;
  size_mb: number;
  recommended: boolean;
  downloaded: boolean;
  active: boolean;
  downloading: boolean;
}

export interface LlmConfig {
  provider: string;
  model: string;
  base_url: string;
  has_key: boolean;
  /** Apple Intelligence 可用性：0=可用 1=裝置不支援 2=未開啟 3=模型下載中 4=系統過舊 5=其他 */
  apple_status: number;
}

export interface DictEntry {
  from: string;
  to: string;
}

export interface DownloadProgress {
  model_id: string;
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
