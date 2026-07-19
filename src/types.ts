export type TaskStatus = "queued" | "downloading" | "paused" | "completed" | "failed" | "cancelled" | "scheduled" | "verifying" | "waiting-network";
export type CollisionPolicy = "overwrite" | "skip" | "rename";
export type CompletionAction = "none" | "open-folder" | "run-file";

export interface MediaSelection {
  extractor?: string;
  format_id?: string;
  format_label?: string;
  subtitles: string[];
  thumbnail?: string;
  requires_ffmpeg?: boolean;
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
  completion_action: CompletionAction;
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
  completion_action: CompletionAction;
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
  frosted_glass: boolean;
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
  default_completion_action: CompletionAction;
  max_retries: number;
  retry_base_seconds: number;
  verify_after_download: boolean;
  media_tool_auto_update: boolean;
  yt_dlp_path: string;
  ffmpeg_path: string;
  ffprobe_path: string;
  low_memory_mode: boolean;
  window_width?: number;
  window_height?: number;
  auto_scale_ui?: boolean;
}

export interface PairingInfo { code: string; expires_at: number; paired_extension?: string; }
export type ToolPhase = "missing" | "downloading" | "verifying" | "extracting" | "ready" | "failed";
export type ToolComponent = "yt-dlp" | "ffmpeg";
export interface ToolStatus {
  state: ToolPhase;
  version: string;
  downloaded_bytes: number;
  total_bytes: number;
  installed_bytes: number;
  error?: string;
  yt_dlp_available: boolean;
  ffmpeg_available: boolean;
  active_component?: ToolComponent;
  yt_dlp_version: string;
  ffmpeg_version: string;
  yt_dlp_download_bytes: number;
  ffmpeg_download_bytes: number;
  yt_dlp_installed_bytes: number;
  ffmpeg_installed_bytes: number;
  yt_dlp_source: "missing" | "custom" | "bundled" | "system";
  ffmpeg_source: "missing" | "custom" | "bundled" | "system";
  yt_dlp_resolved_path?: string;
  ffmpeg_resolved_path?: string;
}
export interface DetectedMediaTools {
  yt_dlp_path?: string;
  ffmpeg_path?: string;
  ffprobe_path?: string;
}
export interface MediaFormat { id: string; label: string; extension?: string; width?: number; height?: number; file_size?: number; has_video: boolean; has_audio: boolean; requires_ffmpeg: boolean; }
export interface MediaProbeResult { title: string; thumbnail?: string; extractor?: string; duration?: number; formats: MediaFormat[]; subtitles: string[]; drm: boolean; }
export interface TaskEvent { task: DownloadTask; event: string; }
export type FilterKey = "all" | TaskStatus | "images" | "video" | "audio" | "documents" | "archives" | "apps";
