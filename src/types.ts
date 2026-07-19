export type TaskStatus = "queued" | "downloading" | "paused" | "completed" | "failed" | "cancelled" | "scheduled" | "verifying";
export type CollisionPolicy = "overwrite" | "skip" | "rename";

export interface MediaSelection {
  extractor?: string;
  format_id?: string;
  format_label?: string;
  subtitles: string[];
  thumbnail?: string;
}

export interface DownloadTask {
  id: string;
  url: string;
  file_name: string;
  destination: string;
  total_bytes: number;
  downloaded_bytes: number;
  speed: number;
  eta_seconds?: number;
  status: TaskStatus;
  error?: string;
  created_at: number;
  completed_at?: number;
  scheduled_at?: number;
  category: string;
  queue_position: number;
  priority: number;
  retry_count: number;
  max_retries: number;
  checksum_sha256?: string;
  expected_checksum?: string;
  source: string;
  etag?: string;
  last_modified?: string;
  headers: Record<string, string>;
  media?: MediaSelection;
  per_task_speed_limit: number;
  collision_policy: CollisionPolicy;
  connection_count: number;
  active_connections: number;
  segments: DownloadSegment[];
}

export interface DownloadSegment { index: number; start_byte: number; end_byte: number; downloaded_bytes: number; status: string; }

export interface NewTaskRequest {
  url: string;
  file_name?: string;
  destination?: string;
  headers: Record<string, string>;
  scheduled_at?: number;
  priority: number;
  expected_checksum?: string;
  source?: string;
  per_task_speed_limit: number;
  collision_policy: CollisionPolicy;
  media?: MediaSelection;
  connection_count?: number;
}

export interface AppSettings {
  download_dir: string;
  concurrent_downloads: number;
  connections_per_download: number;
  speed_limit_kbps: number;
  start_minimized: boolean;
  minimize_to_tray: boolean;
  close_to_tray: boolean;
  notifications: boolean;
  auto_start: boolean;
  theme: "system" | "light" | "dark";
  language: string;
  intercept_browser_downloads: boolean;
  min_file_size_mb: number;
  clipboard_monitor: boolean;
  proxy_mode: "system" | "none" | "manual";
  proxy_url: string;
  proxy_username: string;
  proxy_password: string;
  user_agent: string;
  default_collision_policy: CollisionPolicy;
  max_retries: number;
  retry_base_seconds: number;
  verify_after_download: boolean;
  media_tool_auto_update: boolean;
  window_width?: number;
  window_height?: number;
  auto_scale_ui?: boolean;
}

export interface PairingInfo { code: string; expires_at: number; paired_extension?: string; }
export type ToolPhase = "missing" | "downloading" | "verifying" | "extracting" | "ready" | "failed";
export interface ToolStatus {
  state: ToolPhase;
  version: string;
  downloaded_bytes: number;
  total_bytes: number;
  installed_bytes: number;
  error?: string;
  yt_dlp_available: boolean;
  ffmpeg_available: boolean;
}
export interface MediaFormat { id: string; label: string; extension?: string; width?: number; height?: number; file_size?: number; has_video: boolean; has_audio: boolean; }
export interface MediaProbeResult { title: string; thumbnail?: string; extractor?: string; duration?: number; formats: MediaFormat[]; subtitles: string[]; drm: boolean; }
export interface TaskEvent { task: DownloadTask; event: string; }
export type FilterKey = "all" | TaskStatus | "images" | "video" | "audio" | "documents" | "archives" | "apps";
