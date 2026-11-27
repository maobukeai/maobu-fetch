import type { DownloadTask } from "../../types";
import {
  computeActiveConnDots,
  computeSparkHeights,
} from "./workspace-telemetry";

const assert = {
  equal(actual: unknown, expected: unknown, message?: string) {
    if (actual !== expected) {
      throw new Error(
        message ?? `Expected ${String(expected)}, got ${String(actual)}`
      );
    }
  },
  deepEqual(actual: readonly unknown[], expected: readonly unknown[]) {
    if (
      actual.length !== expected.length ||
      actual.some((val, index) => val !== expected[index])
    ) {
      throw new Error(
        `Expected [${expected.join(", ")}], got [${actual.join(", ")}]`
      );
    }
  },
};

function makeTask(overrides: Partial<DownloadTask>): DownloadTask {
  return {
    id: "task-1",
    url: "https://example.com/file.bin",
    file_name: "file.bin",
    destination: "C:\\Downloads",
    total_bytes: 1024,
    downloaded_bytes: 512,
    speed: 1024,
    status: "downloading",
    created_at: 1,
    category: "other",
    queue_position: 0,
    priority: 0,
    retry_count: 0,
    max_retries: 3,
    source: "manual",
    headers: {},
    per_task_speed_limit: 0,
    collision_policy: "rename",
    completion_action: "none",
    connection_count: 8,
    active_connections: 8,
    segments: [],
    ...overrides,
  };
}

// 1. 非下载状态返回 0
assert.equal(computeActiveConnDots(makeTask({ status: "paused" })), 0);
assert.equal(computeActiveConnDots(makeTask({ status: "completed" })), 0);
assert.equal(computeActiveConnDots(makeTask({ status: "queued" })), 0);

// 2. HTTP 多连接按比例映射
assert.equal(
  computeActiveConnDots(makeTask({ connection_count: 8, active_connections: 0 })),
  0
);
assert.equal(
  computeActiveConnDots(makeTask({ connection_count: 8, active_connections: 1 })),
  1
);
assert.equal(
  computeActiveConnDots(makeTask({ connection_count: 8, active_connections: 4 })),
  2
);
assert.equal(
  computeActiveConnDots(makeTask({ connection_count: 16, active_connections: 16 })),
  4
);

// 3. BT 任务按 peers 数量分档
assert.equal(
  computeActiveConnDots(
    makeTask({
      task_kind: "bt",
      active_connections: 0,
      bt_runtime: {
        num_peers: 0,
        num_seeds: 0,
        upload_speed: 0,
        fetching_metadata: false,
        uploaded_bytes: 0,
        seeding: false,
      },
    })
  ),
  0
);
assert.equal(
  computeActiveConnDots(
    makeTask({
      task_kind: "bt",
      bt_runtime: {
        num_peers: 2,
        num_seeds: 1,
        upload_speed: 0,
        fetching_metadata: false,
        uploaded_bytes: 0,
        seeding: false,
      },
    })
  ),
  1
);
assert.equal(
  computeActiveConnDots(
    makeTask({
      task_kind: "bt",
      bt_runtime: {
        num_peers: 8,
        num_seeds: 2,
        upload_speed: 0,
        fetching_metadata: false,
        uploaded_bytes: 0,
        seeding: false,
      },
    })
  ),
  2
);
assert.equal(
  computeActiveConnDots(
    makeTask({
      task_kind: "bt",
      bt_runtime: {
        num_peers: 20,
        num_seeds: 4,
        upload_speed: 0,
        fetching_metadata: false,
        uploaded_bytes: 0,
        seeding: false,
      },
    })
  ),
  3
);
assert.equal(
  computeActiveConnDots(
    makeTask({
      task_kind: "bt",
      bt_runtime: {
        num_peers: 30,
        num_seeds: 6,
        upload_speed: 0,
        fetching_metadata: false,
        uploaded_bytes: 0,
        seeding: false,
      },
    })
  ),
  4
);

// 4. 速率波形高度换算：全零返回静态基线，非零按峰值缩放
assert.deepEqual(computeSparkHeights([0, 0, 0, 0, 0, 0], 2, 10), [
  2, 2, 2, 2, 2, 2,
]);
assert.deepEqual(computeSparkHeights([0, 0, 0, 0, 0], 3, 11), [
  3, 3, 3, 3, 3,
]);
assert.deepEqual(computeSparkHeights([0, 50, 100, 25], 2, 10), [2, 6, 10, 4]);
assert.deepEqual(computeSparkHeights([0, 50, 100, 25, 75], 3, 11), [
  3, 7, 11, 5, 9,
]);
assert.deepEqual(computeSparkHeights([]), []);

console.log("PASS workspace-telemetry tests");
