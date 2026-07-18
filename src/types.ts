export type DownloadStatus = "queued" | "downloading" | "paused" | "completed" | "failed" | "cancelled";

export interface DownloadItem {
  id: string;
  url: string;
  file_name: string;
  destination: string;
  total_bytes: number;
  downloaded_bytes: number;
  speed: number;
  status: DownloadStatus;
  error?: string;
  created_at: number;
  completed_at?: number;
  category: string;
}

export interface AppSettings {
  download_dir: string;
  concurrent_downloads: number;
  connections_per_download: number;
  speed_limit_kbps: number;
  start_minimized: boolean;
  theme: "system" | "light" | "dark";
  language: "zh-CN" | "en";
  intercept_browser_downloads: boolean;
  min_file_size_mb: number;
}

export type FilterKey = "all" | DownloadStatus | "images" | "video" | "audio" | "documents" | "archives" | "apps";

