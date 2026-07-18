import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppSettings, DownloadTask, MediaProbeResult, NewTaskRequest, PairingInfo, TaskEvent, ToolStatus } from "./types";

export const isDesktop = () => "__TAURI_INTERNALS__" in window;
const call = <T>(command: string, args?: Record<string, unknown>): Promise<T> => isDesktop() ? invoke<T>(command, args) : Promise.reject(new Error("请运行 LumaGet 桌面应用"));

export const api = {
  list: () => isDesktop() ? call<DownloadTask[]>("tasks_list") : Promise.resolve([]),
  add: (request: NewTaskRequest) => call<DownloadTask>("task_add", { request }),
  addBatch: (urls: string[], template: Omit<NewTaskRequest, "url">) => call<DownloadTask[]>("tasks_add_batch", { request: { urls, destination: template.destination, headers: template.headers, scheduled_at: template.scheduled_at, priority: template.priority, collision_policy: template.collision_policy } }),
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
  toolStatus: () => call<ToolStatus[]>("media_tool_status"),
  subscribe: async (handler: (event: TaskEvent | { removed: string }) => void): Promise<UnlistenFn[]> => {
    if (!isDesktop()) return [];
    return Promise.all([
      listen<TaskEvent>("task-created", event => handler(event.payload)),
      listen<TaskEvent>("task-updated", event => handler(event.payload)),
      listen<string>("task-removed", event => handler({ removed: event.payload }))
    ]);
  }
};
