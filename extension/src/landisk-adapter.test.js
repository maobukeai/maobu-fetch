import test from "node:test";
import assert from "node:assert/strict";

// 加载 landisk-adapter.js (挂载到 globalThis.MaobuLanDisk)
import "./landisk-adapter.js";

const {
  init,
  isTailscaleOrLanHost,
  extractLeafName,
  resolveFolderName,
  buildBatchParams,
  deconstructLanDiskSelection,
  formatBytes,
} = globalThis.MaobuLanDisk;

test("LanDisk: Tailscale 与本地局域网主机名识别", () => {
  assert.equal(isTailscaleOrLanHost("100.100.1.5"), true);
  assert.equal(isTailscaleOrLanHost("100.64.0.1"), true);
  assert.equal(isTailscaleOrLanHost("100.127.255.254"), true);
  assert.equal(isTailscaleOrLanHost("my-server.ts.net"), true);
  assert.equal(isTailscaleOrLanHost("node.tailscale.net"), true);
  assert.equal(isTailscaleOrLanHost("ts.net"), true);
  assert.equal(isTailscaleOrLanHost("192.168.1.100"), true);
  assert.equal(isTailscaleOrLanHost("10.0.0.1"), true);
  assert.equal(isTailscaleOrLanHost("172.16.1.1"), true);
  assert.equal(isTailscaleOrLanHost("localhost"), true);
  assert.equal(isTailscaleOrLanHost("127.0.0.1"), true);

  // IPv6 支持 (Tailscale ULA / Link-Local / Loopback)
  assert.equal(isTailscaleOrLanHost("::1"), true);
  assert.equal(isTailscaleOrLanHost("[::1]"), true);
  assert.equal(isTailscaleOrLanHost("fe80::1ff:fe00:1"), true);
  assert.equal(isTailscaleOrLanHost("[fe80::1]"), true);
  assert.equal(isTailscaleOrLanHost("fd7a:115c:a1e0::1"), true);

  // 公网 IP 与普通公网域名（保证全网注入时不触发劫持与定时器）
  assert.equal(isTailscaleOrLanHost("8.8.8.8"), false);
  assert.equal(isTailscaleOrLanHost("1.1.1.1"), false);
  assert.equal(isTailscaleOrLanHost("example.com"), false);
  assert.equal(isTailscaleOrLanHost("github.com"), false);
  assert.equal(isTailscaleOrLanHost("google.com"), false);
  assert.equal(isTailscaleOrLanHost("www.baidu.com"), false);
  assert.equal(isTailscaleOrLanHost("114.114.114.114"), false);
});

test("LanDisk: init guard 在非 LAN / Tailscale 环境下直接 return 不执行任何钩子", () => {
  // 在当前 Node 测试环境下（非 Tailscale/LAN hostname），调用 init 应安全静默返回
  assert.doesNotThrow(() => {
    init();
  });
});

test("LanDisk: 叶子节点提取与文件夹名推断", () => {
  assert.equal(extractLeafName("/var/data/document.pdf"), "document.pdf");
  assert.equal(extractLeafName("D:\\Photos\\Summer\\pic.png"), "pic.png");
  assert.equal(extractLeafName("/var/shared_folder/"), "shared_folder");

  const files = ["D:\\Photos\\IMG_001.JPG", "D:\\Photos\\IMG_002.JPG"];
  const folder = resolveFolderName(files, "");
  assert.equal(folder, "batch_download_2_items");

  const singleDir = ["/home/user/workspace/project-alpha"];
  assert.equal(resolveFolderName(singleDir, ""), "project-alpha");
  assert.equal(resolveFolderName(singleDir, "custom_name"), "custom_name");
});

test("LanDisk: 打包下载表单参数序列化正确", () => {
  const files = ["D:\\Photos\\IMG_001.JPG", "D:\\Photos\\IMG_002.JPG"];
  const folder = resolveFolderName(files, "");
  const body = buildBatchParams(files, folder, "123456");
  const parsed = new URLSearchParams(body);
  assert.equal(parsed.get("folderName"), "batch_download_2_items");
  assert.deepEqual(parsed.getAll("files"), files);
  assert.equal(parsed.get("pin"), "123456");
});

test("LanDisk: 纯客户端递归解构文件树 (单目录及多层子目录展开)", async () => {
  // 模拟 LanDisk API 接口响应
  const mockApiData = {
    "/Photos": [
      { name: "trip.jpg", path: "/Photos/trip.jpg", size: 2048, isDirectory: false },
      { name: "2026", path: "/Photos/2026", size: 0, isDirectory: true },
    ],
    "/Photos/2026": [
      { name: "sub1.png", path: "/Photos/2026/sub1.png", size: 4096, isDirectory: false },
      { name: "nested", path: "/Photos/2026/nested", size: 0, isDirectory: true },
    ],
    "/Photos/2026/nested": [
      { name: "deep.txt", path: "/Photos/2026/nested/deep.txt", size: 512, isDirectory: false },
    ],
  };

  const mockFetch = async (url) => {
    const parsed = new URL(url, "http://192.168.1.100:3000");
    if (parsed.pathname === "/api/files") {
      const p = parsed.searchParams.get("path");
      if (mockApiData[p]) {
        return {
          ok: true,
          status: 200,
          json: async () => mockApiData[p],
        };
      }
      // 单文件返回 400 Bad Request
      return {
        ok: false,
        status: 400,
        json: async () => ({ error: "Not a directory" }),
      };
    }
    return { ok: false, status: 404 };
  };

  const deconstructed = await deconstructLanDiskSelection(
    ["/Photos"],
    "Photos",
    "pin777",
    mockFetch
  );

  assert.equal(deconstructed.length, 3);

  // 1. /Photos/trip.jpg
  assert.equal(deconstructed[0].name, "trip.jpg");
  assert.equal(deconstructed[0].remotePath, "/Photos/trip.jpg");
  assert.equal(deconstructed[0].relativeDir, "");
  assert.equal(deconstructed[0].size, 2048);
  assert.ok(deconstructed[0].downloadUrl.includes("/api/download?path=%2FPhotos%2Ftrip.jpg"));
  assert.ok(deconstructed[0].downloadUrl.includes("pin=pin777"));

  // 2. /Photos/2026/sub1.png
  assert.equal(deconstructed[1].name, "sub1.png");
  assert.equal(deconstructed[1].relativeDir, "2026");
  assert.equal(deconstructed[1].size, 4096);

  // 3. /Photos/2026/nested/deep.txt
  assert.equal(deconstructed[2].name, "deep.txt");
  assert.equal(deconstructed[2].relativeDir, "2026/nested");
  assert.equal(deconstructed[2].size, 512);
});

test("LanDisk: 多选混合项目（单文件 + 目录）解构且保留前缀", async () => {
  const mockApiData = {
    "/FolderA": [
      { name: "f1.pdf", path: "/FolderA/f1.pdf", size: 100, isDirectory: false },
    ],
  };

  const mockFetch = async (url) => {
    const parsed = new URL(url, "http://192.168.1.100:3000");
    const p = parsed.searchParams.get("path");
    if (mockApiData[p]) {
      return { ok: true, status: 200, json: async () => mockApiData[p] };
    }
    return { ok: false, status: 400, json: async () => ({}) };
  };

  const items = ["/FolderA", "/single_file.zip"];
  const deconstructed = await deconstructLanDiskSelection(
    items,
    "batch_download_2_items",
    "",
    mockFetch
  );

  assert.equal(deconstructed.length, 2);
  // 多选时目录内的相对路径带有自身目录名 FolderA 作为前缀
  assert.equal(deconstructed[0].name, "f1.pdf");
  assert.equal(deconstructed[0].relativeDir, "FolderA");

  // 单文件无子目录
  assert.equal(deconstructed[1].name, "single_file.zip");
  assert.equal(deconstructed[1].relativeDir, "");
});

test("LanDisk: formatBytes 字节容量格式化", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(1024), "1.0 KB");
  assert.equal(formatBytes(1024 * 1024), "1.0 MB");
  assert.equal(formatBytes(1536 * 1024 * 1024), "1.5 GB");
});

test("LanDisk: 递归解构计算总文件大小与批次元数据", async () => {
  const mockApiData = {
    "/Data": [
      { name: "part1.bin", path: "/Data/part1.bin", size: 1024 * 1024 * 10, isDirectory: false },
      { name: "part2.bin", path: "/Data/part2.bin", size: 1024 * 1024 * 20, isDirectory: false },
    ],
  };

  const mockFetch = async (url) => {
    const parsed = new URL(url, "http://192.168.1.100:3000");
    const p = parsed.searchParams.get("path");
    if (mockApiData[p]) {
      return { ok: true, status: 200, json: async () => mockApiData[p] };
    }
    return { ok: false, status: 400, json: async () => ({}) };
  };

  const deconstructed = await deconstructLanDiskSelection(
    ["/Data"],
    "Data",
    "",
    mockFetch
  );

  assert.equal(deconstructed.length, 2);
  const totalBytes = deconstructed.reduce((acc, f) => acc + (f.size || 0), 0);
  assert.equal(totalBytes, 30 * 1024 * 1024);
  assert.equal(formatBytes(totalBytes), "30.0 MB");
});
