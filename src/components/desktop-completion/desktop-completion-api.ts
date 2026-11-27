import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface DesktopCompletionNotice {
  notice_id: number;
  task_id: string;
  title: string;
  body: string;
}

export interface DesktopCompletionSnapshot {
  notice: DesktopCompletionNotice | null;
  pending_count: number;
  duration_seconds: number;
  color_scheme: string;
  accent_color: string;
  language: string;
  revision: number;
}

export type DesktopCompletionAction = "file" | "folder" | "details" | "dismiss";

export const desktopCompletionApi = {
  snapshot: () => invoke<DesktopCompletionSnapshot>("desktop_completion_snapshot"),
  present: (notice: DesktopCompletionNotice, height: number) =>
    invoke<DesktopCompletionSnapshot>("desktop_completion_present", {
      taskId: notice.task_id, noticeId: notice.notice_id, height,
    }),
  act: (notice: DesktopCompletionNotice, action: DesktopCompletionAction) =>
    invoke<DesktopCompletionSnapshot>("desktop_completion_act", {
      taskId: notice.task_id, noticeId: notice.notice_id, action,
    }),
  subscribe: (callback: (snapshot: DesktopCompletionSnapshot) => void) =>
    listen<DesktopCompletionSnapshot>("desktop-completion-updated", (event) => callback(event.payload)),
};
