import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppSettings, DetectedMediaTools, DownloadTask, MediaProbeResult, NewTaskRequest, PairingInfo, TaskEvent, ToolComponent, ToolStatus } from "./types";

export const isDesktop = () => "__TAURI_INTERNALS__" in window;
const call = <T>(command: string, args?: Record<string, unknown>): Promise<T> => isDesktop() ? invoke<T>(command, args) : Promise.reject(new Error("请运行猫步下载器桌面应用"));

export const api = {
  list: () => isDesktop() ? call<DownloadTask[]>("tasks_list") : Promise.resolve([]),
  add: (request: NewTaskRequest) => call<DownloadTask>("task_add", { request }),
  addBatch: (urls: string[], template: Omit<NewTaskRequest, "url">) => call<DownloadTask[]>("tasks_add_batch", { request: { urls, destination: template.destination, headers: template.headers, scheduled_at: template.scheduled_at, priority: template.priority, collision_policy: template.collision_policy, connection_count: template.connection_count } }),
  action: (id: string, action: string) => call<void>("task_action", { id, action }),
  bulkAction: (ids: string[], action: string) => call<void>("tasks_bulk_action", { ids, action }),
  remove: (id: string, deleteFile: boolean) => call<void>("task_remove", { id, deleteFile }),
  reorder: (ids: string[]) => call<void>("queue_reorder", { ids }),
  settings: () => call<AppSettings>("settings_get"),
  saveSettings: (settings: AppSettings) => call<void>("settings_save", { settings }),
  openFile: (id: string) => call<void>("task_open_file", { id }),
  openFolder: (id: string) => call<void>("task_open_folder", { id }),
  verify: (id: string) => call<string>("task_verify", { id }),
  clearHistory: (includeCompleted: boolean) => call<void>("history_clear", { includeCompleted }),
  pairing: () => call<PairingInfo>("pairing_info"),
  rotatePairing: () => call<PairingInfo>("pairing_rotate"),
  revokePairing: () => call<void>("pairing_revoke"),
  probeMedia: (url: string) => call<MediaProbeResult>("media_probe", { url }),
  detectSystemMediaTools: () => call<DetectedMediaTools>("media_tools_detect_system"),
  toolStatus: () => isDesktop() ? call<ToolStatus>("media_tool_status") : Promise.resolve({ state: "missing", version: "yt-dlp 2026.06.09 · FFmpeg 8.1.2", downloaded_bytes: 0, total_bytes: 0, installed_bytes: 0, yt_dlp_available: false, ffmpeg_available: false, yt_dlp_version: "2026.06.09", ffmpeg_version: "8.1.2 essentials", yt_dlp_download_bytes: 18_202_192, ffmpeg_download_bytes: 109_728_040, yt_dlp_installed_bytes: 0, ffmpeg_installed_bytes: 0, yt_dlp_source: "missing", ffmpeg_source: "missing" } as ToolStatus),
  installMediaTool: (component: ToolComponent) => call<void>("media_tool_install", { component }),
  installMediaTools: () => call<void>("media_tools_install"),
  cancelMediaTools: () => call<void>("media_tools_cancel"),
  removeMediaTools: () => call<void>("media_tools_remove"),
  removeMediaTool: (component: ToolComponent) => call<void>("media_tool_remove", { component }),
  checkMediaToolsUpdate: () => call<ToolStatus>("media_tools_check_update"),
  subscribeMediaTools: async (handler: (status: ToolStatus) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<ToolStatus>("media-tools-progress", event => handler(event.payload)) : undefined,
  subscribeSettings: async (handler: (settings: AppSettings) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<AppSettings>("settings-changed", event => handler(event.payload)) : undefined,
  subscribe: async (handler: (event: TaskEvent | { removed: string }) => void): Promise<UnlistenFn[]> => {
    if (!isDesktop()) return [];
    return Promise.all([
      listen<TaskEvent>("task-created", event => handler(event.payload)),
      listen<TaskEvent>("task-updated", event => handler(event.payload)),
      listen<string>("task-removed", event => handler({ removed: event.payload }))
    ]);
  }
};
