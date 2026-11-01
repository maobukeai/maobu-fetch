import { completedCount, completedTasks, newestSnapshot, nonNegative, summaryPercent, taskPercent, visibleCompleted, visibleTasks } from "./progress-model";
import { createRevealQueue } from "./reveal-queue";
import type { ProgressWindowSnapshot, ProgressWindowTask } from "./progress-api";

let checks = 0;
function equal(actual: unknown, expected: unknown, label: string) {
  checks++;
  if (actual !== expected) throw new Error(`${label}: expected ${expected}, got ${actual}`);
}

const task: ProgressWindowTask = {
  id: "local-task", file_name: "local.bin", status: "downloading",
  total_bytes: 100, downloaded_bytes: 25, speed_bps: 10,
  fetching_metadata: false, can_pause: true, can_resume: false,
};
const snapshot: ProgressWindowSnapshot = {
  revision: 10, tasks: Array.from({ length: 10 }, (_, i) => ({ ...task, id: `local-${i}` })),
  total_count: 10, total_speed_bps: 100, always_on_top: true, enabled: true,
  expanded: false, loading: false, theme: "system", color_scheme: "system", language: "zh-CN", accent_color: "blue",
};

equal(taskPercent(task), 25, "observed bytes determine progress");
equal(taskPercent({ ...task, total_bytes: 0 }), null, "unknown length has no invented percentage");
equal(taskPercent({ ...task, total_bytes: Number.NaN }), null, "invalid total is unknown");
equal(taskPercent({ ...task, fetching_metadata: true }), null, "metadata pending has no fabricated file progress");
equal(taskPercent({ ...task, downloaded_bytes: 110 }), 100, "over-count cannot draw outside the track");
equal(taskPercent({ ...task, downloaded_bytes: -5 }), 0, "negative telemetry never produces negative width");
equal(visibleTasks(snapshot).length, 3, "collapsed list remains compact");
equal(visibleTasks({ ...snapshot, expanded: true }).length, 6, "expanded list is bounded even with many tasks");
equal(visibleTasks({ ...snapshot, tasks: [] }).length, 0, "empty list never creates demo rows");
equal(newestSnapshot(snapshot, { ...snapshot, revision: 9, tasks: [] }), snapshot, "late initial invoke cannot erase a newer event");
equal(newestSnapshot(snapshot, { ...snapshot, revision: 11, always_on_top: false }).always_on_top, false, "newer pin preference reaches the UI");
equal(nonNegative(Number.POSITIVE_INFINITY), 0, "non-finite speed does not show an impossible rate");
equal(nonNegative(-1), 0, "negative speed does not reach formatting");
equal(summaryPercent(snapshot), null, "an older snapshot does not invent overall progress from preview rows");
equal(summaryPercent({ ...snapshot, summary_percent: 42.5 }), 42.5, "overall progress comes from the backend's whole queue");
equal(summaryPercent({ ...snapshot, summary_percent: null }), null, "an unknown member keeps the overall progress unknown");
equal(summaryPercent({ ...snapshot, summary_percent: 101 }), null, "invalid overall progress is not drawn as complete");
equal(summaryPercent({ ...snapshot, summary_percent: Number.NaN }), null, "non-finite overall progress stays unknown");
equal(completedCount(snapshot), 0, "older snapshots have no synthetic recent completions");
equal(completedTasks(snapshot).length, 0, "completed rows are optional for compatibility");
const recent = { ...snapshot, completed_tasks: Array.from({ length: 8 }, (_, i) => ({ ...task, id: `completed-${i}`, status: "completed" as const })), completed_count: 8 };
equal(completedCount(recent), 6, "recent count describes at most the six retained files");
equal(completedTasks(recent).length, 6, "completion preview cannot grow without bounds");
equal(visibleCompleted(recent).length, 3, "recent completions start with three rows");
equal(visibleCompleted({ ...recent, expanded: true }).length, 6, "recent completions expand to six rows");
equal(completedCount({ ...recent, completed_count: undefined }), 6, "rows supply a safe count when optional counter is absent");
equal(completedCount({ ...recent, completed_count: -1 }), 0, "negative recent telemetry is not shown");
equal(summaryPercent({ ...recent, summary_percent: 12.5 }), 12.5, "completion previews never distort active progress");

const tick = () => new Promise<void>(resolve => setTimeout(resolve, 0));
const pending: Array<{ value: boolean; resolve: (snapshot: boolean) => void; reject: () => void }> = [];
const accepted: boolean[] = [];
let failures = 0;
const queue = createRevealQueue<boolean>(value => new Promise((resolve, reject) => {
  pending.push({ value, resolve, reject: () => reject(new Error("synthetic reveal failure")) });
}), value => accepted.push(value), () => { failures++; });
queue.request(true);
queue.request(false);
equal(pending.length, 1, "a pending hover request is never duplicated");
pending[0].resolve(true);
await tick();
equal(pending[1]?.value, false, "leaving during a pending reveal still sends the latest collapse intent");
pending[1].resolve(false);
await tick();
equal(accepted.join(","), "true,false", "both acknowledged revisions reach the normal snapshot filter");
queue.request(true);
pending[2].reject();
await tick();
equal(failures, 1, "a failed reveal exposes one error rather than retrying indefinitely");
equal(pending.length, 3, "a failed unchanged intent does not spin requests");
queue.request(false);
pending[3].resolve(false);
await tick();
equal(accepted.at(-1), false, "a later user action remains available after a reveal failure");
queue.request(true);
queue.dispose();
pending[4].resolve(true);
await tick();
equal(accepted.length, 3, "unmounted windows ignore late reveal responses");

const deniedCalls: boolean[] = [];
const denied = createRevealQueue(value => { deniedCalls.push(value); return Promise.resolve(false); }, () => {}, () => {});
denied.request(true);
await tick();
equal(deniedCalls.length, 1, "a server denying an obsolete batch does not trigger a reveal retry loop");
denied.adopt(false);
await tick();
equal(deniedCalls.length, 1, "an observed collapsed server state alone is not new user intent");
denied.dispose();

const bootPending: Array<{ value: boolean; resolve: (value: boolean) => void }> = [];
let backendHold = true;
const boot = createRevealQueue<boolean>(value => new Promise(resolve => bootPending.push({ value, resolve })), value => { backendHold = value; }, () => {}, null);
boot.request(false);
equal(bootPending[0]?.value, false, "a reloaded interaction client explicitly releases an inherited backend hold");
boot.request(true);
equal(bootPending.length, 1, "new hover intent waits behind the initial release rather than racing it");
bootPending[0].resolve(false);
await tick();
equal(backendHold, false, "the acknowledged initial release clears the old backend hold");
equal(bootPending[1]?.value, true, "hover arriving during startup is serialized after the release");
bootPending[1].resolve(true);
await tick();
equal(backendHold, true, "the latest user interaction wins after startup release");
boot.request(false);
bootPending[2].resolve(false);
await tick();
boot.dispose();
console.log(`download progress display: ${checks} behavioral checks passed`);
