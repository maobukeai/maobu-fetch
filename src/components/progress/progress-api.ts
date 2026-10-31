import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppSettings, ColorScheme, TaskStatus } from "../../types";

/** Deliberately excludes URLs, destination paths, headers and credentials. */
export interface ProgressWindowTask {
  id: string;
  file_name: string;
  status: TaskStatus;
  downloaded_bytes: number;
  total_bytes: number;
  speed_bps: number;
  eta_seconds?: number | null;
  fetching_metadata: boolean;
  can_pause: boolean;
  can_resume: boolean;
}

export interface ProgressWindowSnapshot {
  revision: number;
  tasks: ProgressWindowTask[];
  total_count: number;
  total_speed_bps: number;
  always_on_top: boolean;
  enabled: boolean;
  expanded: boolean;
  revealed?: boolean;
  /** Whole active queue, supplied by the backend; never calculated from the six preview rows. */
  summary_percent?: number | null;
  loading: boolean;
  theme: AppSettings["theme"];
  color_scheme: ColorScheme;
  language: string;
  accent_color: AppSettings["accent_color"];
  error?: string | null;
}

export const progressApi = {
  snapshot: () => invoke<ProgressWindowSnapshot>("progress_window_snapshot"),
  pinned: (pinned: boolean) => invoke<ProgressWindowSnapshot>("progress_window_set_pinned", { pinned }),
  expanded: (expanded: boolean) => invoke<ProgressWindowSnapshot>("progress_window_set_expanded", { expanded }),
  revealed: (revealed: boolean) => invoke<ProgressWindowSnapshot>("progress_window_set_revealed", { revealed }),
  action: (taskId: string, action: "pause" | "resume") => invoke<ProgressWindowSnapshot>("progress_window_task_action", { taskId, action }),
  showMain: (taskId?: string) => invoke<ProgressWindowSnapshot>("progress_window_show_main", { taskId }),
  dismiss: () => invoke<ProgressWindowSnapshot>("progress_window_dismiss"),
  restore: () => invoke<ProgressWindowSnapshot>("progress_window_restore"),
  subscribe: (callback: (snapshot: ProgressWindowSnapshot) => void) =>
    listen<ProgressWindowSnapshot>("download-progress-updated", (event) => callback(event.payload)),
};
