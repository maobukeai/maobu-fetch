const puppeteer = require("puppeteer-core");
const path = require("path");
const fs = require("fs");

const executablePath = fs.existsSync("C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe")
  ? "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe"
  : "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";

const outputDir = path.join(__dirname, "..", "docs", "assets");
if (!fs.existsSync(outputDir)) {
  fs.mkdirSync(outputDir, { recursive: true });
}

async function run() {
  console.log("Launching Edge for README screenshots...");
  const browser = await puppeteer.launch({
    executablePath,
    headless: true,
    args: [
      "--no-sandbox",
      "--disable-setuid-sandbox",
      "--force-device-scale-factor=2",
      "--font-render-hinting=medium",
      "--enable-font-antialiasing",
    ],
  });

  const setupMockData = async (page) => {
    await page.evaluateOnNewDocument(() => {
      const mockTasks = [
        {
          id: "task-ubuntu-001",
          url: "https://releases.ubuntu.com/26.04/ubuntu-26.04-desktop-amd64.iso",
          filename: "ubuntu-26.04-desktop-amd64.iso",
          save_path: "C:\\Users\\20269\\Downloads\\ubuntu-26.04-desktop-amd64.iso",
          status: "downloading",
          size: 6144000000,
          downloaded: 4026531840,
          speed: 29777216,
          progress: 0.655,
          eta: 71,
          connections: 16,
          category: "系统镜像",
          created_at: "2026-07-23T10:15:00Z",
          etag: '"6600a12b-16e000000"',
          last_modified: "Wed, 15 Jul 2026 12:00:00 GMT",
          verify_sha256: true,
          priority: 10,
          slices: [
            { index: 0, start: 0, end: 384000000, downloaded: 384000000, speed: 0, status: "completed" },
            { index: 1, start: 384000001, end: 768000000, downloaded: 384000000, speed: 0, status: "completed" },
            { index: 2, start: 768000001, end: 1152000000, downloaded: 384000000, speed: 0, status: "completed" },
            { index: 3, start: 1152000001, end: 1536000000, downloaded: 384000000, speed: 0, status: "completed" },
            { index: 4, start: 1536000001, end: 1920000000, downloaded: 384000000, speed: 0, status: "completed" },
            { index: 5, start: 1920000001, end: 2304000000, downloaded: 2304000000, speed: 0, status: "completed" },
            { index: 6, start: 2304000001, end: 2688000000, downloaded: 2688000000, speed: 0, status: "completed" },
            { index: 7, start: 2704000001, end: 3072000000, downloaded: 3072000000, speed: 0, status: "completed" },
            { index: 8, start: 3072000001, end: 3456000000, downloaded: 3456000000, speed: 0, status: "completed" },
            { index: 9, start: 3456000001, end: 3840000000, downloaded: 3840000000, speed: 0, status: "completed" },
            { index: 10, start: 3840000001, end: 4224000000, downloaded: 186531840, speed: 3820000, status: "downloading" },
            { index: 11, start: 4224000001, end: 4608000000, downloaded: 0, speed: 3640000, status: "downloading" },
            { index: 12, start: 4608000001, end: 4992000000, downloaded: 0, speed: 4120000, status: "downloading" },
            { index: 13, start: 4992000001, end: 5376000000, downloaded: 0, speed: 3950000, status: "downloading" },
            { index: 14, start: 5376000001, end: 5760000000, downloaded: 0, speed: 4200000, status: "downloading" },
            { index: 15, start: 5760000001, end: 6144000000, downloaded: 0, speed: 4040000, status: "downloading" }
          ]
        },
        {
          id: "task-bilibili-002",
          url: "https://www.bilibili.com/video/BV1xx411c7m9",
          filename: "bilibili_BV1xx411c7m9_1080P_UltraHD.mp4",
          save_path: "C:\\Users\\20269\\Videos\\bilibili_BV1xx411c7m9_1080P_UltraHD.mp4",
          status: "downloading",
          size: 922746880,
          downloaded: 461373440,
          speed: 14155776,
          progress: 0.500,
          eta: 32,
          connections: 8,
          category: "影视媒体",
          created_at: "2026-07-23T10:18:22Z"
        },
        {
          id: "task-yt-003",
          url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.06.09/yt-dlp.exe",
          filename: "yt-dlp_2026.06.09_x64.exe",
          save_path: "C:\\Users\\20269\\Downloads\\yt-dlp_2026.06.09_x64.exe",
          status: "completed",
          size: 18202192,
          downloaded: 18202192,
          speed: 0,
          progress: 1.0,
          eta: 0,
          connections: 4,
          category: "常用软件",
          sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
          created_at: "2026-07-23T09:30:11Z"
        },
        {
          id: "task-node-004",
          url: "https://nodejs.org/dist/v24.14.0/node-v24.14.0-x64.msi",
          filename: "node-v24.14.0-x64.msi",
          save_path: "C:\\Users\\20269\\Downloads\\node-v24.14.0-x64.msi",
          status: "paused",
          size: 47185920,
          downloaded: 33554432,
          speed: 0,
          progress: 0.711,
          eta: 0,
          connections: 8,
          category: "常用软件",
          created_at: "2026-07-23T08:12:00Z"
        },
        {
          id: "task-kernel-005",
          url: "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.10.tar.xz",
          filename: "linux-6.10.tar.xz",
          save_path: "C:\\Users\\20269\\Downloads\\linux-6.10.tar.xz",
          status: "scheduled",
          scheduled_at: "2026-07-24T02:00:00Z",
          size: 146800640,
          downloaded: 0,
          speed: 0,
          progress: 0.0,
          eta: 0,
          connections: 8,
          category: "压缩文件",
          created_at: "2026-07-23T10:20:00Z"
        }
      ];

      const mockSettings = {
        download_dir: "C:\\Users\\20269\\Downloads",
        max_concurrent_tasks: 5,
        default_connections: 16,
        global_max_connections: 64,
        speed_limit_kbps: 0,
        collision_policy: "rename",
        verify_after_download: true,
        low_memory_mode: false,
        proxy_mode: "system",
        proxy_url: "",
        proxy_username: "",
        proxy_password: "",
        max_retries: 5,
        intercept_browser_downloads: true,
        min_file_size_mb: 1,
        theme: "system",
        accent_color: "blue",
        frosted_glass: true,
        auto_scale_ui: true,
        media_tool_auto_update: true,
        shortcut_keys: { toggle_window: "Alt+D", new_task: "Ctrl+N" }
      };

      const mockToolStatus = {
        state: "installed",
        version: "yt-dlp 2026.06.09 · FFmpeg 8.1.2",
        downloaded_bytes: 127930232,
        total_bytes: 127930232,
        installed_bytes: 127930232,
        yt_dlp_available: true,
        ffmpeg_available: true,
        yt_dlp_version: "2026.06.09",
        ffmpeg_version: "8.1.2 essentials",
        yt_dlp_download_bytes: 18202192,
        ffmpeg_download_bytes: 109728040,
        yt_dlp_installed_bytes: 18202192,
        ffmpeg_installed_bytes: 109728040,
        yt_dlp_source: "app_data",
        ffmpeg_source: "app_data"
      };

      const mockPairing = {
        code: "849201",
        expires_in: 580,
        paired_extension: "chrome-extension://abcdefghijklmnopqrstuvwxyz123456"
      };

      const mockPlatforms = [
        { platform: "bilibili", level: "verified", notes: "支持 1080P/4K 视频、高码率音频、弹幕与封面抓取；支持 Cookie 凭据同步。" },
        { platform: "youtube", level: "verified", notes: "支持 4K/8K Dash 流、多语言字幕提取及 FFmpeg 自动混流封装。" },
        { platform: "douyin", level: "experimental", notes: "支持单视频、无水印图集及直播流解析；支持网页 Cookie 授权导入。" },
        { platform: "tiktok", level: "experimental", notes: "支持高清无水印短视频提取与批量队列下载。" },
        { platform: "twitter", level: "experimental", notes: "支持 Tweet 媒体原图、GIF 与最高清晰度视频提取。" },
        { platform: "weibo", level: "experimental", notes: "支持微博视频与九宫格原图解析。" }
      ];

      let eventIdCounter = 1;

      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        plugins: {},
        invoke: async (cmd, args) => {
          if (cmd === "plugin:event|listen") return () => {};
          if (cmd === "plugin:event|unlisten") return null;
          if (cmd === "plugin:window|is_maximized") return false;
          if (cmd === "plugin:window|set_effects") return null;
          if (cmd === "plugin:window|set_size") return null;
          if (cmd === "tasks_list") return mockTasks;
          if (cmd === "settings_get") return mockSettings;
          if (cmd === "media_tool_status") return mockToolStatus;
          if (cmd === "pairing_info") return mockPairing;
          if (cmd === "platform_compatibility_list") return mockPlatforms;
          if (cmd === "url_history_list") return [];
          if (cmd === "category_rule_list") return [];
          if (cmd === "task_template_list") return [];
          if (cmd === "media_credential_list") return [];
          if (cmd === "filename_cleanup_rule_list") return [];
          if (cmd === "platform_naming_template_list") return [];
          if (cmd === "preset_list") return [];
          if (cmd === "tag_list") return [];
          if (cmd === "task_tags_list_all") return {};
          if (cmd === "cache_inspect") return { total_bytes: 29777216, file_count: 3 };
          if (cmd === "app_get_info") return { version: "0.6.3", portable_mode: false, data_dir: "C:\\Users\\20269\\AppData\\Roaming\\maobu-fetch" };
          if (cmd === "power_action_get") return { action: "none", phase: "idle", remaining_seconds: 0, target_count: 0 };
          return [];
        },
        transformCallback: (cb) => cb
      };
    });
  };

  // 1. 主界面 (01_main_dashboard.webp)
  const page1 = await browser.newPage();
  await page1.setViewport({ width: 1280, height: 800, deviceScaleFactor: 2 });
  await setupMockData(page1);
  await page1.goto("http://localhost:1420", { waitUntil: "networkidle0" });
  await new Promise(r => setTimeout(r, 1200));
  await page1.waitForSelector(".task-row");
  await page1.screenshot({ path: path.join(outputDir, "01_main_dashboard.webp"), type: "webp", quality: 85 });
  console.log("Saved: 01_main_dashboard.webp");

  // 2. 选中任务展开切片与详情 (02_slice_visualization.webp)
  await page1.evaluate(() => {
    if (window.__SHOW_TASK_DETAILS__) window.__SHOW_TASK_DETAILS__("task-ubuntu-001");
  });
  await new Promise(r => setTimeout(r, 800));
  await page1.screenshot({ path: path.join(outputDir, "02_slice_visualization.webp"), type: "webp", quality: 85 });
  console.log("Saved: 02_slice_visualization.webp");
  await page1.close();

  // 3. 新建任务弹窗 (03_new_task_modal.webp)
  const page2 = await browser.newPage();
  await page2.setViewport({ width: 1280, height: 800, deviceScaleFactor: 2 });
  await setupMockData(page2);
  await page2.goto("http://localhost:1420", { waitUntil: "networkidle0" });
  await new Promise(r => setTimeout(r, 1200));
  await page2.evaluate(() => {
    if (window.__OPEN_NEW_TASK__) window.__OPEN_NEW_TASK__();
  });
  await new Promise(r => setTimeout(r, 800));
  await page2.screenshot({ path: path.join(outputDir, "03_new_task_modal.webp"), type: "webp", quality: 85 });
  console.log("Saved: 03_new_task_modal.webp");
  await page2.close();

  // 4. 设置页面与各个 Tab (04 ~ 10)
  const settingsTabs = [
    { sec: "general", file: "04_settings_general.webp" },
    { sec: "download", file: "05_settings_download.webp" },
    { sec: "network", file: "06_settings_network.webp" },
    { sec: "browser", file: "07_settings_browser.webp" },
    { sec: "media", file: "08_settings_media.webp" },
    { sec: "appearance", file: "09_settings_appearance.webp" },
    { sec: "about", file: "10_settings_about_matrix.webp" }
  ];

  for (const item of settingsTabs) {
    const pageTab = await browser.newPage();
    await pageTab.setViewport({ width: 1280, height: 800, deviceScaleFactor: 2 });
    await setupMockData(pageTab);
    await pageTab.goto("http://localhost:1420", { waitUntil: "networkidle0" });
    await new Promise(r => setTimeout(r, 1200));
    await pageTab.evaluate(async (s) => {
      if (window.__OPEN_SETTINGS__) window.__OPEN_SETTINGS__();
      for (let i = 0; i < 20; i++) {
        if (window.__SET_SETTINGS_SECTION__) {
          window.__SET_SETTINGS_SECTION__(s);
          break;
        }
        await new Promise(r => setTimeout(r, 50));
      }
    }, item.sec);
    await new Promise(r => setTimeout(r, 800));

    await pageTab.screenshot({ path: path.join(outputDir, item.file), type: "webp", quality: 85 });
    console.log(`Saved: ${item.file}`);
    await pageTab.close();
  }

  await browser.close();
  console.log("All 10 crisp WebP screenshots captured successfully!");
}

run().catch((err) => {
  console.error("Error capturing screenshots:", err);
  process.exit(1);
});
