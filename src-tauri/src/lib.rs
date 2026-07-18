use axum::{
    extract::State as AxumState,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use reqwest::header::{ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, RANGE};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use url::Url;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DownloadStatus {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DownloadItem {
    id: String,
    url: String,
    file_name: String,
    destination: String,
    total_bytes: u64,
    downloaded_bytes: u64,
    speed: u64,
    status: DownloadStatus,
    error: Option<String>,
    created_at: u64,
    completed_at: Option<u64>,
    category: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AppSettings {
    download_dir: String,
    concurrent_downloads: u8,
    connections_per_download: u8,
    speed_limit_kbps: u64,
    start_minimized: bool,
    theme: String,
    language: String,
    intercept_browser_downloads: bool,
    min_file_size_mb: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        let dir = dirs_download().to_string_lossy().to_string();
        Self {
            download_dir: dir,
            concurrent_downloads: 3,
            connections_per_download: 8,
            speed_limit_kbps: 0,
            start_minimized: false,
            theme: "system".into(),
            language: "zh-CN".into(),
            intercept_browser_downloads: true,
            min_file_size_mb: 1,
        }
    }
}

struct DownloadManager {
    items: Mutex<HashMap<String, DownloadItem>>,
    controls: Mutex<HashMap<String, CancellationToken>>,
    settings: Mutex<AppSettings>,
    data_dir: PathBuf,
    client: reqwest::Client,
}
type Shared = Arc<DownloadManager>;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn dirs_download() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads")
}
fn safe_name(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches([' ', '.']);
    if trimmed.is_empty() {
        "download".into()
    } else {
        trimmed.chars().take(180).collect()
    }
}
fn infer_name(url: &Url, disposition: Option<&str>) -> String {
    if let Some(cd) = disposition {
        if let Some(value) = cd
            .split(';')
            .find_map(|p| p.trim().strip_prefix("filename="))
        {
            return safe_name(value.trim_matches(['\"', '\'']));
        }
    }
    safe_name(
        url.path_segments()
            .and_then(|mut s| s.next_back())
            .filter(|s| !s.is_empty())
            .unwrap_or("download"),
    )
}
fn category(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "mkv" | "mov" | "webm" | "m3u8" => "video",
        "mp3" | "wav" | "flac" | "aac" | "m4a" => "audio",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" => "images",
        "zip" | "rar" | "7z" | "tar" | "gz" => "archives",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" => "documents",
        "exe" | "msi" | "dmg" | "pkg" | "appimage" => "apps",
        _ => "other",
    }
    .into()
}

impl DownloadManager {
    async fn load(data_dir: PathBuf) -> Shared {
        let _ = fs::create_dir_all(&data_dir).await;
        let items = fs::read(data_dir.join("downloads.json"))
            .await
            .ok()
            .and_then(|x| serde_json::from_slice(&x).ok())
            .unwrap_or_default();
        let settings = fs::read(data_dir.join("settings.json"))
            .await
            .ok()
            .and_then(|x| serde_json::from_slice(&x).ok())
            .unwrap_or_default();
        Arc::new(Self {
            items: Mutex::new(items),
            controls: Mutex::new(HashMap::new()),
            settings: Mutex::new(settings),
            data_dir,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::limited(10))
                .user_agent("LumaGet/0.1")
                .build()
                .unwrap(),
        })
    }
    async fn persist_items(&self) {
        if let Ok(bytes) = serde_json::to_vec_pretty(&*self.items.lock().await) {
            let _ = fs::write(self.data_dir.join("downloads.json"), bytes).await;
        }
    }
    async fn persist_settings(&self) {
        if let Ok(bytes) = serde_json::to_vec_pretty(&*self.settings.lock().await) {
            let _ = fs::write(self.data_dir.join("settings.json"), bytes).await;
        }
    }
    async fn pause(&self, id: &str) {
        if let Some(t) = self.controls.lock().await.remove(id) {
            t.cancel();
        }
        if let Some(i) = self.items.lock().await.get_mut(id) {
            i.status = DownloadStatus::Paused;
            i.speed = 0;
        }
        self.persist_items().await;
    }
    async fn start(self: &Shared, id: String) {
        let token = CancellationToken::new();
        self.controls.lock().await.insert(id.clone(), token.clone());
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = manager.download(&id, token.clone()).await;
            manager.controls.lock().await.remove(&id);
            if let Err(error) = result {
                let mut items = manager.items.lock().await;
                if let Some(item) = items.get_mut(&id) {
                    if !token.is_cancelled() {
                        item.status = DownloadStatus::Failed;
                        item.error = Some(error);
                    }
                    item.speed = 0;
                }
            }
            manager.persist_items().await;
        });
    }
    async fn download(&self, id: &str, token: CancellationToken) -> Result<(), String> {
        let (url, path, existing, limit, connections) = {
            let settings = self.settings.lock().await.clone();
            let mut items = self.items.lock().await;
            let item = items.get_mut(id).ok_or("Task not found")?;
            item.status = DownloadStatus::Downloading;
            item.error = None;
            let path = PathBuf::from(&item.destination).join(&item.file_name);
            let existing = fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
            item.downloaded_bytes = existing;
            (
                item.url.clone(),
                path,
                existing,
                settings.speed_limit_kbps * 1024,
                settings.connections_per_download.clamp(1, 16),
            )
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        let probe = self.client.head(&url).send().await.ok();
        let probe_total = probe.as_ref().and_then(|r| r.content_length()).unwrap_or(0);
        let accepts_ranges = probe
            .as_ref()
            .and_then(|r| r.headers().get(ACCEPT_RANGES))
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("bytes"));
        if existing == 0 && connections > 1 && accepts_ranges && probe_total >= 4 * 1024 * 1024 {
            return self
                .download_segmented(id, &url, &path, probe_total, connections, token)
                .await;
        }

        let mut req = self.client.get(&url);
        if existing > 0 {
            req = req.header(RANGE, format!("bytes={existing}-"));
        }
        let response = req.send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success()
            && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
        {
            return Err(format!("HTTP {}", response.status()));
        }
        let append = existing > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let total = response.content_length().unwrap_or(0) + if append { existing } else { 0 };
        {
            let mut items = self.items.lock().await;
            if let Some(i) = items.get_mut(id) {
                i.total_bytes = total.max(i.total_bytes);
                if !append {
                    i.downloaded_bytes = 0;
                }
            }
        }
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&path)
            .await
            .map_err(|e| e.to_string())?;
        let mut stream = response.bytes_stream();
        let mut sampled_at = Instant::now();
        let mut sampled_bytes = 0u64;
        let mut total_written = if append { existing } else { 0 };
        while let Some(chunk) = stream.next().await {
            if token.is_cancelled() {
                return Ok(());
            }
            let chunk = chunk.map_err(|e| e.to_string())?;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            total_written += chunk.len() as u64;
            sampled_bytes += chunk.len() as u64;
            let elapsed = sampled_at.elapsed();
            if elapsed >= Duration::from_millis(350) {
                let speed = (sampled_bytes as f64 / elapsed.as_secs_f64()) as u64;
                {
                    let mut items = self.items.lock().await;
                    if let Some(i) = items.get_mut(id) {
                        i.downloaded_bytes = total_written;
                        i.speed = speed;
                    }
                }
                if limit > 0 && speed > limit {
                    tokio::time::sleep(Duration::from_secs_f64(
                        (sampled_bytes as f64 / limit as f64 - elapsed.as_secs_f64()).max(0.0),
                    ))
                    .await;
                }
                sampled_at = Instant::now();
                sampled_bytes = 0;
            }
        }
        file.flush().await.map_err(|e| e.to_string())?;
        let mut items = self.items.lock().await;
        if let Some(i) = items.get_mut(id) {
            i.downloaded_bytes = total_written;
            i.total_bytes = i.total_bytes.max(total_written);
            i.status = DownloadStatus::Completed;
            i.speed = 0;
            i.completed_at = Some(now());
        }
        Ok(())
    }

    async fn download_segmented(
        &self,
        id: &str,
        url: &str,
        output: &Path,
        total: u64,
        connections: u8,
        token: CancellationToken,
    ) -> Result<(), String> {
        let segment_size = total.div_ceil(connections as u64);
        let mut initial = 0u64;
        let mut parts = Vec::new();
        for index in 0..connections {
            let part = output.with_extension(format!("lumaget-{id}.part{index}"));
            initial += fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
            parts.push(part);
        }
        if let Some(item) = self.items.lock().await.get_mut(id) {
            item.total_bytes = total;
            item.downloaded_bytes = initial.min(total);
        }

        let sampler = Arc::new(Mutex::new((Instant::now(), 0u64)));
        let mut jobs = futures_util::stream::FuturesUnordered::new();
        for (index, part) in parts.iter().cloned().enumerate() {
            let start = index as u64 * segment_size;
            if start >= total {
                continue;
            }
            let end = ((index as u64 + 1) * segment_size - 1).min(total - 1);
            let already = fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
            if start + already > end {
                continue;
            }
            let request_start = start + already;
            let job_token = token.clone();
            let sampler = sampler.clone();
            let url = url.to_owned();
            jobs.push(async move {
                let response = self
                    .client
                    .get(&url)
                    .header(RANGE, format!("bytes={request_start}-{end}"))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                    return Err("Server stopped supporting segmented ranges".to_string());
                }
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&part)
                    .await
                    .map_err(|e| e.to_string())?;
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    if job_token.is_cancelled() {
                        return Ok(());
                    }
                    let chunk = chunk.map_err(|e| e.to_string())?;
                    file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                    let chunk_len = chunk.len() as u64;
                    if let Some(item) = self.items.lock().await.get_mut(id) {
                        item.downloaded_bytes = (item.downloaded_bytes + chunk_len).min(total);
                    }
                    let mut sample = sampler.lock().await;
                    sample.1 += chunk_len;
                    if sample.0.elapsed() >= Duration::from_millis(350) {
                        let speed = (sample.1 as f64 / sample.0.elapsed().as_secs_f64()) as u64;
                        if let Some(item) = self.items.lock().await.get_mut(id) {
                            item.speed = speed;
                        }
                        *sample = (Instant::now(), 0);
                    }
                }
                file.flush().await.map_err(|e| e.to_string())?;
                Ok::<(), String>(())
            });
        }
        while let Some(result) = jobs.next().await {
            result?;
        }
        if token.is_cancelled() {
            return Ok(());
        }

        let mut merged = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(output)
            .await
            .map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 1024 * 1024];
        for part in &parts {
            let mut source = fs::File::open(part).await.map_err(|e| e.to_string())?;
            loop {
                let read = source.read(&mut buffer).await.map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                merged
                    .write_all(&buffer[..read])
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        merged.flush().await.map_err(|e| e.to_string())?;
        for part in parts {
            let _ = fs::remove_file(part).await;
        }
        if let Some(item) = self.items.lock().await.get_mut(id) {
            item.downloaded_bytes = total;
            item.status = DownloadStatus::Completed;
            item.speed = 0;
            item.completed_at = Some(now());
        }
        Ok(())
    }
}

#[tauri::command]
async fn list_downloads(manager: State<'_, Shared>) -> Result<Vec<DownloadItem>, String> {
    let mut x: Vec<_> = manager.items.lock().await.values().cloned().collect();
    x.sort_by_key(|i| std::cmp::Reverse(i.created_at));
    Ok(x)
}
#[tauri::command]
async fn get_settings(manager: State<'_, Shared>) -> Result<AppSettings, String> {
    Ok(manager.settings.lock().await.clone())
}
#[tauri::command]
async fn save_settings(settings: AppSettings, manager: State<'_, Shared>) -> Result<(), String> {
    *manager.settings.lock().await = settings;
    manager.persist_settings().await;
    Ok(())
}

async fn create_download(
    url: String,
    file_name: Option<String>,
    manager: &Shared,
) -> Result<DownloadItem, String> {
    let parsed = Url::parse(&url).map_err(|_| "请输入有效的 HTTP/HTTPS 链接".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("只支持 HTTP/HTTPS 链接".into());
    }
    let head = manager.client.head(parsed.clone()).send().await.ok();
    let name = file_name.map(|x| safe_name(&x)).unwrap_or_else(|| {
        infer_name(
            &parsed,
            head.as_ref()
                .and_then(|r| r.headers().get(CONTENT_DISPOSITION))
                .and_then(|v| v.to_str().ok()),
        )
    });
    let total = head
        .as_ref()
        .and_then(|r| r.headers().get(CONTENT_LENGTH))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let destination = manager.settings.lock().await.download_dir.clone();
    let item = DownloadItem {
        id: Uuid::new_v4().to_string(),
        url,
        file_name: name.clone(),
        destination,
        total_bytes: total,
        downloaded_bytes: 0,
        speed: 0,
        status: DownloadStatus::Queued,
        error: None,
        created_at: now(),
        completed_at: None,
        category: category(&name),
    };
    manager
        .items
        .lock()
        .await
        .insert(item.id.clone(), item.clone());
    manager.persist_items().await;
    manager.start(item.id.clone()).await;
    Ok(item)
}
#[tauri::command]
async fn add_download(
    url: String,
    file_name: Option<String>,
    manager: State<'_, Shared>,
) -> Result<DownloadItem, String> {
    create_download(url, file_name, manager.inner()).await
}
#[tauri::command]
async fn pause_download(id: String, manager: State<'_, Shared>) -> Result<(), String> {
    manager.pause(&id).await;
    Ok(())
}
#[tauri::command]
async fn resume_download(id: String, manager: State<'_, Shared>) -> Result<(), String> {
    manager.start(id).await;
    Ok(())
}
#[tauri::command]
async fn retry_download(id: String, manager: State<'_, Shared>) -> Result<(), String> {
    manager.start(id).await;
    Ok(())
}
#[tauri::command]
async fn cancel_download(id: String, manager: State<'_, Shared>) -> Result<(), String> {
    manager.pause(&id).await;
    if let Some(i) = manager.items.lock().await.get_mut(&id) {
        i.status = DownloadStatus::Cancelled;
    }
    manager.persist_items().await;
    Ok(())
}
#[tauri::command]
async fn remove_download(
    id: String,
    delete_file: bool,
    manager: State<'_, Shared>,
) -> Result<(), String> {
    manager.pause(&id).await;
    if let Some(i) = manager.items.lock().await.remove(&id) {
        if delete_file {
            let _ = fs::remove_file(PathBuf::from(i.destination).join(i.file_name)).await;
        }
    }
    manager.persist_items().await;
    Ok(())
}
#[derive(Deserialize)]
struct ExtensionRequest {
    url: String,
    #[serde(rename = "fileName")]
    file_name: Option<String>,
}
async fn health() -> &'static str {
    "LumaGet is ready"
}
async fn extension_add(
    AxumState(manager): AxumState<Shared>,
    Json(req): Json<ExtensionRequest>,
) -> Result<Json<DownloadItem>, (StatusCode, String)> {
    create_download(req.url, req.file_name, &manager)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}
async fn run_bridge(manager: Shared) {
    let app = Router::new()
        .route("/health", get(health))
        .route("/downloads", post(extension_add))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods(Any),
        )
        .with_state(manager);
    if let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:17433").await {
        let _ = axum::serve(listener, app).await;
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let manager = tauri::async_runtime::block_on(DownloadManager::load(data_dir));
            app.manage(manager.clone());
            tauri::async_runtime::spawn(run_bridge(manager));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_downloads,
            get_settings,
            save_settings,
            add_download,
            pause_download,
            resume_download,
            retry_download,
            cancel_download,
            remove_download
        ])
        .run(tauri::generate_context!())
        .expect("error while running LumaGet");
}
