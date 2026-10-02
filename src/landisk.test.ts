/**
 * 局域网互联 Pro / LanDisk 解析单元测试（landisk.ts）。
 *
 * 遵循项目零外部测试框架规范（AGENTS.md §8），通过 `npx tsx src/landisk.test.ts` 运行。
 */

import {
  isTailscaleOrLanHost,
  isLanDiskUrl,
  parseLanDiskUrl,
  buildLanDiskDownloadUrl,
  extractLeafName,
} from "./services/landisk";
import { computeBatchSummaries } from "./components/common/BatchSummaryBar";
import type { DownloadTask } from "./types";
import { formatBytes } from "./formatters";

function assertEqual<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(
      `${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`
    );
  }
}

function assertTrue(value: unknown, message: string): void {
  if (!value) {
    throw new Error(`assertion failed (${message}): ${value}`);
  }
}

console.log("▶ 测试 isTailscaleOrLanHost...");
assertEqual(isTailscaleOrLanHost("100.64.0.1"), true, "Tailscale 起始 IP");
assertEqual(isTailscaleOrLanHost("100.100.50.25"), true, "Tailscale 常用 IP");
assertEqual(isTailscaleOrLanHost("100.127.255.254"), true, "Tailscale 结束 IP");
assertEqual(isTailscaleOrLanHost("my-nas.ts.net"), true, "Tailscale 节点域名");
assertEqual(isTailscaleOrLanHost("lan-disk.tailscale.net"), true, "Tailscale 官方域名");
assertEqual(isTailscaleOrLanHost("192.168.1.100"), true, "私网 C 段");
assertEqual(isTailscaleOrLanHost("10.0.0.1"), true, "私网 A 段");
assertEqual(isTailscaleOrLanHost("172.16.0.5"), true, "私网 B 段");
assertEqual(isTailscaleOrLanHost("localhost"), true, "localhost 本地环回");
assertEqual(isTailscaleOrLanHost("127.0.0.1"), true, "127.0.0.1 本地环回");
assertEqual(isTailscaleOrLanHost("8.8.8.8"), false, "公网 DNS IP");
assertEqual(isTailscaleOrLanHost("114.114.114.114"), false, "公网 IP");
assertEqual(isTailscaleOrLanHost("github.com"), false, "公网域名");
console.log("✔ isTailscaleOrLanHost 测试通过");

console.log("▶ 测试 isLanDiskUrl...");
assertEqual(
  isLanDiskUrl("http://192.168.1.50:3000/api/download/batch?folderName=photos"),
  true,
  "打包下载 API"
);
assertEqual(
  isLanDiskUrl("http://100.100.1.2:8080/api/download?path=%2FPhotos%2Fa.jpg"),
  true,
  "单文件直链下载 API"
);
assertEqual(
  isLanDiskUrl("http://my-node.ts.net:3000/?path=/Movies"),
  true,
  "带 path 查询参数的 Tailscale 页面"
);
assertEqual(
  isLanDiskUrl("http://127.0.0.1:3000/#/files?path=%2FDocs"),
  true,
  "带 hash path 的单页路由"
);
assertEqual(
  isLanDiskUrl("https://example.com/file.zip"),
  false,
  "普通公网直链"
);
console.log("✔ isLanDiskUrl 测试通过");

console.log("▶ 测试 parseLanDiskUrl...");
const url1 = "http://192.168.1.10:3000/api/download/batch?folderName=album&files=p1.jpg&files=p2.jpg&pin=8888";
const parsed1 = parseLanDiskUrl(url1);
assertTrue(parsed1 !== null, "parsed1 非空");
if (parsed1) {
  assertEqual(parsed1.baseUrl, "http://192.168.1.10:3000", "提取 baseUrl");
  assertEqual(parsed1.folderName, "album", "提取 folderName");
  assertEqual(parsed1.pin, "8888", "提取 pin");
  assertEqual(parsed1.files.length, 2, "提取 files 长度");
  assertEqual(parsed1.files[0], "p1.jpg", "提取 files[0]");
  assertEqual(parsed1.files[1], "p2.jpg", "提取 files[1]");
  assertEqual(parsed1.isBatch, true, "isBatch 标识");
}

const url2 = "http://100.64.1.2:5000/?path=%2FShared%2FVideos&pin=123456";
const parsed2 = parseLanDiskUrl(url2);
assertTrue(parsed2 !== null, "parsed2 非空");
if (parsed2) {
  assertEqual(parsed2.targetPath, "/Shared/Videos", "提取 targetPath");
  assertEqual(parsed2.pin, "123456", "提取 pin");
  assertEqual(parsed2.isBatch, false, "isBatch 标识为 false");
}
console.log("✔ parseLanDiskUrl 测试通过");

console.log("▶ 测试 buildLanDiskDownloadUrl 与 extractLeafName...");
const dlUrl = buildLanDiskDownloadUrl("http://100.64.1.2:5000", "/data/file.mp4", "123");
assertEqual(
  dlUrl,
  "http://100.64.1.2:5000/api/download?path=%2Fdata%2Ffile.mp4&pin=123",
  "直链构建并携带 pin"
);

assertEqual(extractLeafName("/data/sub/video.mp4"), "video.mp4", "Unix 文件路径");
assertEqual(extractLeafName("D:\\Photos\\Summer\\pic.png"), "pic.png", "Windows 文件路径");
assertEqual(extractLeafName("/var/shared_folder/"), "shared_folder", "目录路径");
assertEqual(extractLeafName(""), "download", "空路径回退默认名");
console.log("✔ buildLanDiskDownloadUrl 与 extractLeafName 测试通过");

console.log("▶ 测试 SPA Hash fragment 路由中的 LanDisk URL 解析...");
const hashUrl = "http://100.64.0.1:8080/#/files?path=%2FHome%2FDocuments&pin=secret99";
const parsedHash = parseLanDiskUrl(hashUrl);
assertTrue(parsedHash !== null, "parsedHash 非空");
if (parsedHash) {
  assertEqual(parsedHash.baseUrl, "http://100.64.0.1:8080", "SPA baseUrl");
  assertEqual(parsedHash.targetPath, "/Home/Documents", "SPA targetPath");
  assertEqual(parsedHash.pin, "secret99", "SPA pin");
}
console.log("✔ SPA Hash fragment 路由测试通过");

console.log("▶ 测试边界与异常 URL 过滤...");
assertEqual(isLanDiskUrl(""), false, "空字符串不是 LanDisk URL");
assertEqual(isLanDiskUrl("not-a-valid-url"), false, "非法 URL 返回 false");
assertEqual(isLanDiskUrl("https://example.com/api/other"), false, "非局域网非相关 API 返回 false");
assertEqual(parseLanDiskUrl("https://example.com/not-landisk"), null, "无 LanDisk 参数解析返回 null");
console.log("✔ 边界与异常 URL 过滤测试通过");

console.log("▶ 测试 computeBatchSummaries 批量聚合与预填字节计算...");
const mockTasks: DownloadTask[] = [
  {
    id: "task-1",
    url: "http://192.168.1.100:3000/api/download?path=/Docs/file1.pdf",
    file_name: "file1.pdf",
    destination: "D:/Downloads/Docs",
    total_bytes: 10485760, // 10 MB
    downloaded_bytes: 10485760,
    speed: 0,
    status: "completed",
    created_at: 1000,
    category: "document",
    queue_position: 0,
    priority: 0,
    retry_count: 0,
    max_retries: 3,
    source: "landisk_deconstructed",
    headers: {},
    per_task_speed_limit: 0,
    collision_policy: "rename",
    completion_action: "none",
    connection_count: 16,
    active_connections: 0,
    segments: [],
    batch_id: "Docs",
  },
  {
    id: "task-2",
    url: "http://192.168.1.100:3000/api/download?path=/Docs/file2.pdf",
    file_name: "file2.pdf",
    destination: "D:/Downloads/Docs",
    total_bytes: 20971520, // 20 MB
    downloaded_bytes: 5242880, // 5 MB
    speed: 5242880, // 5 MB/s
    status: "downloading",
    created_at: 1001,
    category: "document",
    queue_position: 1,
    priority: 0,
    retry_count: 0,
    max_retries: 3,
    source: "landisk_deconstructed",
    headers: {},
    per_task_speed_limit: 0,
    collision_policy: "rename",
    completion_action: "none",
    connection_count: 16,
    active_connections: 16,
    segments: [],
    batch_id: "Docs",
  },
  {
    id: "task-3",
    url: "http://192.168.1.100:3000/api/download?path=/Docs/file3.pdf",
    file_name: "file3.pdf",
    destination: "D:/Downloads/Docs",
    total_bytes: 31457280, // 30 MB (预填大小，queued 状态不显示破折号)
    downloaded_bytes: 0,
    speed: 0,
    status: "queued",
    created_at: 1002,
    category: "document",
    queue_position: 2,
    priority: 0,
    retry_count: 0,
    max_retries: 3,
    source: "landisk_deconstructed",
    headers: {},
    per_task_speed_limit: 0,
    collision_policy: "rename",
    completion_action: "none",
    connection_count: 16,
    active_connections: 0,
    segments: [],
    batch_id: "Docs",
  },
];

const summaries = computeBatchSummaries(mockTasks, new Set());
assertEqual(summaries.length, 1, "聚合为一个批次");
const s = summaries[0];
assertEqual(s.batchId, "Docs", "批次 ID 匹配");
assertEqual(s.totalTasks, 3, "总任务数 3");
assertEqual(s.completedTasks, 1, "已完成 1 项");
assertEqual(s.activeTasks, 1, "正在下载 1 项");
assertEqual(s.totalBytes, 62914560, "总大小 60 MB");
assertEqual(s.downloadedBytes, 15728640, "已下载 15 MB");
assertEqual(s.speed, 5242880, "聚合速度 5 MB/s");
assertTrue(s.etaSeconds !== undefined && s.etaSeconds > 0, "计算剩余时间 ETA");
assertEqual(formatBytes(s.totalBytes), "60.0 MB", "总容量格式化正确");
console.log("✔ computeBatchSummaries 批量聚合与预填字节测试通过");


