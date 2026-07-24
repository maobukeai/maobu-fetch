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
      const mockDefaultRetryPolicy = {
        connection_timeout_secs: 15,
        task_timeout_secs: null,
        max_retries: 5,
        backoff: "exponential",
        initial_backoff_ms: 1000,
        max_backoff_ms: 30000
      };

      const mockSettings = {
        download_dir: "C:\\Users\\20269\\Downloads",
        concurrent_downloads: 5,
        connections_per_download: 16,
        speed_limit_kbps: 0,
        start_minimized: false,
        minimize_to_tray: true,
        close_to_tray: true,
        notifications: true,
        auto_start: false,
        theme: "system",
        accent_color: "blue",
        frosted_glass: true,
        language: "zh-CN",
        intercept_browser_downloads: true,
        min_file_size_mb: 1,
        clipboard_monitor: true,
        proxy_mode: "system",
        proxy_url: "",
        proxy_username: "",
        proxy_password: "",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
        default_collision_policy: "rename",
        default_completion_action: "none",
        max_retries: 5,
        retry_base_seconds: 2,
        verify_after_download: true,
        media_tool_auto_update: true,
        yt_dlp_path: "",
        ffmpeg_path: "",
        ffprobe_path: "",
        low_memory_mode: false,
        auto_scale_ui: true,
        default_retry_policy: mockDefaultRetryPolicy,
        row_compact: false,
        detail_default_collapsed: false,
        color_scheme: "system",
        archive_days: 30,
        archive_threshold: 100,
        notify_on_complete: true,
        notify_on_failure: true,
        notify_sound_enabled: true,
        notify_failure_sound_enabled: false
      };

      const mockTasks = [
        {
          id: "task-ubuntu-001",
          url: "https://releases.ubuntu.com/26.04/ubuntu-26.04-desktop-amd64.iso",
          file_name: "ubuntu-26.04-desktop-amd64.iso",
          destination: "C:\\Users\\20269\\Downloads\\ubuntu-26.04-desktop-amd64.iso",
          total_bytes: 6144000000,
          downloaded_bytes: 4026531840,
          speed: 29777216,
          eta_seconds: 71,
          status: "downloading",
          created_at: 1784801700000,
          category: "system",
          queue_position: 1,
          priority: 0,
          retry_count: 0,
          max_retries: 5,
          source: "direct",
          etag: '"6600a12b-16e000000"',
          last_modified: "Wed, 15 Jul 2026 12:00:00 GMT",
          accepts_ranges: true,
          headers: {
            "User-Agent": "MaobuFetch/0.6.3",
            "Accept": "*/*"
          },
          per_task_speed_limit: 0,
          collision_policy: "rename",
          completion_action: "none",
          connection_count: 16,
          active_connections: 16,
          segments: [
            { index: 0, start_byte: 0, end_byte: 384000000, downloaded_bytes: 384000000, status: "completed" },
            { index: 1, start_byte: 384000001, end_byte: 768000000, downloaded_bytes: 384000000, status: "completed" },
            { index: 2, start_byte: 768000001, end_byte: 1152000000, downloaded_bytes: 384000000, status: "completed" },
            { index: 3, start_byte: 1152000001, end_byte: 1536000000, downloaded_bytes: 384000000, status: "completed" },
            { index: 4, start_byte: 1536000001, end_byte: 1920000000, downloaded_bytes: 384000000, status: "completed" },
            { index: 5, start_byte: 1920000001, end_byte: 2304000000, downloaded_bytes: 2304000000, status: "completed" },
            { index: 6, start_byte: 2304000001, end_byte: 2688000000, downloaded_bytes: 2688000000, status: "completed" },
            { index: 7, start_byte: 2704000001, end: 3072000000, downloaded_bytes: 3072000000, status: "completed" },
            { index: 8, start_byte: 3072000001, end: 3456000000, downloaded_bytes: 3456000000, status: "completed" },
            { index: 9, start_byte: 3456000001, end: 3840000000, downloaded_bytes: 3840000000, status: "completed" },
            { index: 10, start_byte: 3840000001, end: 4224000000, downloaded_bytes: 186531840, status: "downloading" },
            { index: 11, start_byte: 4224000001, end: 4608000000, downloaded_bytes: 0, status: "downloading" },
            { index: 12, start_byte: 4608000001, end: 4992000000, downloaded_bytes: 0, status: "downloading" },
            { index: 13, start_byte: 4992000001, end: 5376000000, downloaded_bytes: 0, status: "downloading" },
            { index: 14, start_byte: 5376000001, end: 5760000000, downloaded_bytes: 0, status: "downloading" },
            { index: 15, start_byte: 5760000001, end: 6144000000, downloaded_bytes: 0, status: "downloading" }
          ]
        },
        {
          id: "task-bilibili-002",
          url: "https://www.bilibili.com/video/BV1xx411c7m9",
          file_name: "bilibili_BV1xx411c7m9_1080P_UltraHD.mp4",
          destination: "C:\\Users\\20269\\Videos\\bilibili_BV1xx411c7m9_1080P_UltraHD.mp4",
          total_bytes: 922746880,
          downloaded_bytes: 461373440,
          speed: 14155776,
          eta_seconds: 32,
          status: "downloading",
          created_at: 1784801800000,
          category: "video",
          queue_position: 2,
          priority: 0,
          retry_count: 0,
          max_retries: 5,
          source: "bilibili",
          headers: {},
          per_task_speed_limit: 0,
          collision_policy: "rename",
          completion_action: "none",
          connection_count: 8,
          active_connections: 8,
          segments: []
        },
        {
          id: "task-yt-003",
          url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.06.09/yt-dlp.exe",
          file_name: "yt-dlp_2026.06.09_x64.exe",
          destination: "C:\\Users\\20269\\Downloads\\yt-dlp_2026.06.09_x64.exe",
          total_bytes: 18202192,
          downloaded_bytes: 18202192,
          speed: 0,
          eta_seconds: 0,
          status: "completed",
          created_at: 1784800000000,
          completed_at: 1784800500000,
          category: "apps",
          queue_position: 3,
          priority: 0,
          retry_count: 0,
          max_retries: 5,
          source: "direct",
          checksum_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
          headers: {},
          per_task_speed_limit: 0,
          collision_policy: "rename",
          completion_action: "none",
          connection_count: 4,
          active_connections: 0,
          segments: []
        },
        {
          id: "task-node-004",
          url: "https://nodejs.org/dist/v24.14.0/node-v24.14.0-x64.msi",
          file_name: "node-v24.14.0-x64.msi",
          destination: "C:\\Users\\20269\\Downloads\\node-v24.14.0-x64.msi",
          total_bytes: 47185920,
          downloaded_bytes: 33554432,
          speed: 0,
          eta_seconds: 0,
          status: "paused",
          created_at: 1784795000000,
          category: "apps",
          queue_position: 4,
          priority: 0,
          retry_count: 0,
          max_retries: 5,
          source: "direct",
          headers: {},
          per_task_speed_limit: 0,
          collision_policy: "rename",
          completion_action: "none",
          connection_count: 8,
          active_connections: 0,
          segments: []
        },
        {
          id: "task-kernel-005",
          url: "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.10.tar.xz",
          file_name: "linux-6.10.tar.xz",
          destination: "C:\\Users\\20269\\Downloads\\linux-6.10.tar.xz",
          total_bytes: 146800640,
          downloaded_bytes: 0,
          speed: 0,
          eta_seconds: 0,
          status: "scheduled",
          created_at: 1784802000000,
          scheduled_at: 1784850000000,
          category: "archives",
          queue_position: 5,
          priority: 0,
          retry_count: 0,
          max_retries: 5,
          source: "direct",
          headers: {},
          per_task_speed_limit: 0,
          collision_policy: "rename",
          completion_action: "none",
          connection_count: 8,
          active_connections: 0,
          segments: []
        }
      ];

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
          if (cmd === "app_get_info") return { version: "0.6.5", portable_mode: false, data_dir: "C:\\Users\\20269\\AppData\\Roaming\\maobu-fetch" };
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
  await new Promise(r => setTimeout(r, 1500));
  await page1.waitForSelector(".task-row");
  await page1.screenshot({ path: path.join(outputDir, "01_main_dashboard.webp"), type: "webp", quality: 85 });
  console.log("Saved: 01_main_dashboard.webp");

  // 2. 选中任务展开切片与详情 (02_slice_visualization.webp)
  await page1.evaluate(() => {
    const row = document.querySelector(".task-row");
    if (row) row.click();
    const detailsToggle = document.querySelector(".details-toggle");
    if (detailsToggle) detailsToggle.click();
  });
  await new Promise(r => setTimeout(r, 1000));
  await page1.screenshot({ path: path.join(outputDir, "02_slice_visualization.webp"), type: "webp", quality: 85 });
  console.log("Saved: 02_slice_visualization.webp");
  await page1.close();

  // 3. 新建任务弹窗 (03_new_task_modal.webp)
  const page2 = await browser.newPage();
  await page2.setViewport({ width: 1280, height: 800, deviceScaleFactor: 2 });
  await setupMockData(page2);
  await page2.goto("http://localhost:1420", { waitUntil: "networkidle0" });
  await new Promise(r => setTimeout(r, 1500));
  await page2.evaluate(() => {
    const btn = document.querySelector(".new-button") || document.querySelector(".action-btn-standalone");
    if (btn) btn.click();
  });
  await new Promise(r => setTimeout(r, 1000));
  await page2.screenshot({ path: path.join(outputDir, "03_new_task_modal.webp"), type: "webp", quality: 85 });
  console.log("Saved: 03_new_task_modal.webp");
  await page2.close();

  // 4. 设置页面与各个 Tab (04 ~ 10)
  const settingsTabs = [
    { text: "常规", file: "04_settings_general.webp" },
    { text: "下载", file: "05_settings_download.webp" },
    { text: "网络", file: "06_settings_network.webp" },
    { text: "浏览器", file: "07_settings_browser.webp" },
    { text: "媒体", file: "08_settings_media.webp" },
    { text: "外观", file: "09_settings_appearance.webp" },
    { text: "关于", file: "10_settings_about_matrix.webp" }
  ];

  for (const item of settingsTabs) {
    const pageTab = await browser.newPage();
    await pageTab.setViewport({ width: 1280, height: 800, deviceScaleFactor: 2 });
    await setupMockData(pageTab);
    await pageTab.goto("http://localhost:1420", { waitUntil: "networkidle0" });
    await new Promise(r => setTimeout(r, 1500));
    await pageTab.waitForSelector(".nav-settings");
    await pageTab.evaluate(() => { document.querySelector(".nav-settings")?.click(); });
    await pageTab.waitForSelector(".settings-page", { timeout: 5000 });

    if (item.text !== "常规") {
      await pageTab.evaluate((targetText) => {
        const btns = Array.from(document.querySelectorAll(".settings-nav-list .nav-item"));
        const btn = btns.find(b => b.textContent.trim() === targetText);
        if (btn) btn.click();
      }, item.text);
      await new Promise(r => setTimeout(r, 800));
    } else {
      await new Promise(r => setTimeout(r, 600));
    }

    await pageTab.screenshot({ path: path.join(outputDir, item.file), type: "webp", quality: 85 });
    console.log(`Saved: ${item.file} (Tab text: ${item.text})`);
    await pageTab.close();
  }

  // 5. 浏览器扩展 Popup 截图 (11_extension_popup.webp) - 使用真实的 extension/src/popup.js 渲染
  const pageExt = await browser.newPage();
  await pageExt.setViewport({ width: 380, height: 720, deviceScaleFactor: 2 });
  await pageExt.evaluateOnNewDocument(() => {
    window.chrome = {
      runtime: {
        lastError: null,
        getURL: (path) => path,
        sendMessage: (msg, cb) => {
          if (msg.type === "health") {
            cb({ ok: true, paired: true });
          } else if (msg.type === "recent-tasks") {
            cb({
              ok: true,
              result: {
                tasks: [
                  {
                    id: "task-1",
                    file_name: "Maobu Fetch 0.6.3 x64-setup (8).exe",
                    status: "downloading",
                    progress: 0.94,
                    speed: 16389734
                  }
                ]
              }
            });
          } else {
            cb({ ok: true });
          }
        }
      },
      storage: {
        local: {
          get: (keys, cb) => cb({ intercept: true, minSizeMb: 1 }),
          set: (data, cb) => cb && cb()
        },
        session: {
          get: (key, cb) => cb({ [key]: [] })
        }
      },
      cookies: {
        getAll: (details, cb) => cb([{ name: "SESSDATA", value: "abcdef123" }])
      },
      tabs: {
        query: (details, cb) => cb([{ id: 1, url: "https://www.bilibili.com/video/BV1xx411c7m9", title: "哔哩哔哩视频" }])
      }
    };
  });
  const popupPath = path.join(__dirname, "..", "extension", "src", "popup.html");
  await pageExt.goto(`file:///${popupPath.replace(/\\/g, "/")}`);
  await new Promise(r => setTimeout(r, 1200));
  await pageExt.screenshot({ path: path.join(outputDir, "11_extension_popup.webp"), type: "webp", quality: 90 });
  console.log("Saved REAL extension popup: 11_extension_popup.webp");
  await pageExt.close();

  await browser.close();
  console.log("All 11 crisp WebP screenshots captured successfully!");
}

run().catch((err) => {
  console.error("Error capturing screenshots:", err);
  process.exit(1);
});
