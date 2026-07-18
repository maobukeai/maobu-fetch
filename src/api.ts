import { invoke } from "@tauri-apps/api/core";
import type { AppSettings, DownloadItem } from "./types";

const inTauri = () => "__TAURI_INTERNALS__" in window;

const demoItems: DownloadItem[] = [
  { id: "demo-1", url: "https://example.com/design-resources.zip", file_name: "design-resources.zip", destination: "Downloads", total_bytes: 1_840_000_000, downloaded_bytes: 1_214_000_000, speed: 8_420_000, status: "downloading", created_at: Date.now(), category: "archives" },
  { id: "demo-2", url: "https://example.com/product-film.mp4", file_name: "product-film.mp4", destination: "Downloads", total_bytes: 680_000_000, downloaded_bytes: 680_000_000, speed: 0, status: "completed", created_at: Date.now() - 3600000, completed_at: Date.now(), category: "video" }
];

const defaultSettings: AppSettings = { download_dir: "Downloads", concurrent_downloads: 3, connections_per_download: 8, speed_limit_kbps: 0, start_minimized: false, theme: "system", language: "zh-CN", intercept_browser_downloads: true, min_file_size_mb: 1 };

export const api = {
  list: (): Promise<DownloadItem[]> => inTauri() ? invoke("list_downloads") : Promise.resolve(demoItems),
  settings: (): Promise<AppSettings> => inTauri() ? invoke("get_settings") : Promise.resolve(defaultSettings),
  add: (url: string, fileName?: string): Promise<DownloadItem> => inTauri() ? invoke("add_download", { url, fileName }) : Promise.resolve({ ...demoItems[0], id: crypto.randomUUID(), url, file_name: fileName || url.split("/").pop() || "download" }),
  action: (action: string, id: string): Promise<void> => inTauri() ? invoke(`${action}_download`, { id }) : Promise.resolve(),
  remove: (id: string, deleteFile: boolean): Promise<void> => inTauri() ? invoke("remove_download", { id, deleteFile }) : Promise.resolve(),
  saveSettings: (settings: AppSettings): Promise<void> => inTauri() ? invoke("save_settings", { settings }) : Promise.resolve()
};

