use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Scheduled,
    Verifying,
    #[serde(rename = "waiting-network")]
    WaitingNetwork,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Scheduled => "scheduled",
            Self::Verifying => "verifying",
            Self::WaitingNetwork => "waiting-network",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "downloading" => Self::Downloading,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "scheduled" => Self::Scheduled,
            "verifying" => Self::Verifying,
            "waiting-network" => Self::WaitingNetwork,
            _ => Self::Queued,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub file_name: String,
    pub destination: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub speed: u64,
    pub eta_seconds: Option<u64>,
    pub status: TaskStatus,
    pub error: Option<String>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
    pub scheduled_at: Option<u64>,
    pub category: String,
    pub queue_position: i64,
    pub priority: i32,
    pub retry_count: u32,
    pub max_retries: u32,
    pub checksum_sha256: Option<String>,
    pub expected_checksum: Option<String>,
    pub source: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    #[serde(default)]
    pub final_url: Option<String>,
    #[serde(default)]
    pub response_status: Option<u16>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub accepts_ranges: Option<bool>,
    pub headers: HashMap<String, String>,
    pub media: Option<MediaSelection>,
    pub per_task_speed_limit: u64,
    pub collision_policy: CollisionPolicy,
    #[serde(default)]
    pub completion_action: CompletionAction,
    pub connection_count: u8,
    #[serde(default)]
    pub active_connections: u8,
    #[serde(default)]
    pub segments: Vec<DownloadSegment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadSegment {
    pub index: u8,
    pub start_byte: u64,
    pub end_byte: u64,
    pub downloaded_bytes: u64,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MediaSelection {
    pub extractor: Option<String>,
    pub format_id: Option<String>,
    pub format_label: Option<String>,
    pub subtitles: Vec<String>,
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub requires_ffmpeg: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CollisionPolicy {
    Overwrite,
    Skip,
    #[default]
    Rename,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CompletionAction {
    #[default]
    None,
    OpenFolder,
    RunFile,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PowerAction {
    #[default]
    None,
    Shutdown,
    Hibernate,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PowerActionPhase {
    #[default]
    Idle,
    Armed,
    Countdown,
    Blocked,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PowerActionState {
    pub action: PowerAction,
    pub phase: PowerActionPhase,
    pub remaining_seconds: u64,
    pub target_count: usize,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewTaskRequest {
    pub url: String,
    pub file_name: Option<String>,
    pub destination: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub scheduled_at: Option<u64>,
    #[serde(default)]
    pub priority: i32,
    pub expected_checksum: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub per_task_speed_limit: u64,
    #[serde(default)]
    pub collision_policy: CollisionPolicy,
    #[serde(default)]
    pub completion_action: CompletionAction,
    pub media: Option<MediaSelection>,
    pub connection_count: Option<u8>,
    #[serde(default)]
    pub start_paused: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchTaskRequest {
    pub urls: Vec<String>,
    pub destination: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub scheduled_at: Option<u64>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub per_task_speed_limit: u64,
    #[serde(default)]
    pub collision_policy: CollisionPolicy,
    #[serde(default)]
    pub completion_action: CompletionAction,
    pub connection_count: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskExportFile {
    pub schema_version: u32,
    pub exported_at: u64,
    pub tasks: Vec<TaskExportItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskExportItem {
    pub url: String,
    pub file_name: String,
    pub priority: i32,
    pub scheduled_at: Option<u64>,
    pub expected_checksum: Option<String>,
    pub per_task_speed_limit: u64,
    pub collision_policy: CollisionPolicy,
    pub completion_action: CompletionAction,
    pub media: Option<MediaSelection>,
    pub connection_count: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    pub download_dir: String,
    pub concurrent_downloads: u8,
    pub connections_per_download: u8,
    pub speed_limit_kbps: u64,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub notifications: bool,
    pub auto_start: bool,
    pub theme: String,
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    #[serde(default)]
    pub frosted_glass: bool,
    pub language: String,
    pub intercept_browser_downloads: bool,
    pub min_file_size_mb: u64,
    pub clipboard_monitor: bool,
    pub proxy_mode: String,
    pub proxy_url: String,
    pub proxy_username: String,
    pub proxy_password: String,
    pub user_agent: String,
    pub default_collision_policy: CollisionPolicy,
    #[serde(default)]
    pub default_completion_action: CompletionAction,
    pub max_retries: u32,
    pub retry_base_seconds: u64,
    pub verify_after_download: bool,
    pub media_tool_auto_update: bool,
    #[serde(default)]
    pub yt_dlp_path: String,
    #[serde(default)]
    pub ffmpeg_path: String,
    #[serde(default)]
    pub ffprobe_path: String,
    #[serde(default)]
    pub low_memory_mode: bool,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub auto_scale_ui: Option<bool>,
}

impl Default for AppSettings {
    fn default() -> Self {
        let download_dir = std::env::var_os("USERPROFILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("Downloads")
            .to_string_lossy()
            .to_string();
        Self {
            download_dir,
            concurrent_downloads: 3,
            connections_per_download: 8,
            speed_limit_kbps: 0,
            start_minimized: false,
            minimize_to_tray: true,
            close_to_tray: false,
            notifications: true,
            auto_start: false,
            theme: "system".into(),
            accent_color: default_accent_color(),
            frosted_glass: false,
            language: "zh-CN".into(),
            intercept_browser_downloads: true,
            min_file_size_mb: 1,
            clipboard_monitor: false,
            proxy_mode: "system".into(),
            proxy_url: String::new(),
            proxy_username: String::new(),
            proxy_password: String::new(),
            user_agent: "MaobuFetch/0.5".into(),
            default_collision_policy: CollisionPolicy::Rename,
            default_completion_action: CompletionAction::None,
            max_retries: 3,
            retry_base_seconds: 2,
            verify_after_download: false,
            media_tool_auto_update: true,
            yt_dlp_path: String::new(),
            ffmpeg_path: String::new(),
            ffprobe_path: String::new(),
            low_memory_mode: false,
            window_width: Some(1024),
            window_height: Some(720),
            auto_scale_ui: Some(false),
        }
    }
}

fn default_accent_color() -> String {
    "blue".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskProgressEvent {
    pub task: DownloadTask,
    pub event: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairingInfo {
    pub code: String,
    pub expires_at: u64,
    pub paired_extension: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolPhase {
    Missing,
    Downloading,
    Verifying,
    Extracting,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ToolComponent {
    YtDlp,
    Ffmpeg,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolStatus {
    pub state: ToolPhase,
    pub version: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub installed_bytes: u64,
    pub error: Option<String>,
    pub yt_dlp_available: bool,
    pub ffmpeg_available: bool,
    pub active_component: Option<ToolComponent>,
    pub yt_dlp_version: String,
    pub ffmpeg_version: String,
    pub yt_dlp_download_bytes: u64,
    pub ffmpeg_download_bytes: u64,
    pub yt_dlp_installed_bytes: u64,
    pub ffmpeg_installed_bytes: u64,
    pub yt_dlp_source: String,
    pub ffmpeg_source: String,
    pub yt_dlp_resolved_path: Option<String>,
    pub ffmpeg_resolved_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectedMediaTools {
    pub yt_dlp_path: Option<String>,
    pub ffmpeg_path: Option<String>,
    pub ffprobe_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaProbeResult {
    pub title: String,
    pub thumbnail: Option<String>,
    pub extractor: Option<String>,
    pub duration: Option<f64>,
    pub formats: Vec<MediaFormat>,
    pub subtitles: Vec<String>,
    pub drm: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaFormat {
    pub id: String,
    pub label: String,
    pub extension: Option<String>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub file_size: Option<u64>,
    pub has_video: bool,
    pub has_audio: bool,
    #[serde(default)]
    pub requires_ffmpeg: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_json_defaults_low_memory_mode_to_off() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("low_memory_mode");
        object.remove("yt_dlp_path");
        object.remove("ffmpeg_path");
        object.remove("ffprobe_path");
        object.remove("default_completion_action");
        object.remove("accent_color");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(!restored.low_memory_mode);
        assert!(restored.yt_dlp_path.is_empty());
        assert!(restored.ffmpeg_path.is_empty());
        assert!(restored.ffprobe_path.is_empty());
        assert_eq!(restored.default_completion_action, CompletionAction::None);
        assert_eq!(restored.accent_color, "blue");
    }

    #[test]
    fn old_settings_json_defaults_frosted_glass_to_off() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("frosted_glass");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(!restored.frosted_glass);
    }

    #[test]
    fn old_browser_media_request_defaults_ffmpeg_requirement_to_off() {
        let selection: MediaSelection = serde_json::from_value(serde_json::json!({
            "format_id": "18",
            "subtitles": []
        }))
        .unwrap();
        assert!(!selection.requires_ffmpeg);
    }

    #[test]
    fn old_extension_request_defaults_completion_action_to_none() {
        let request: NewTaskRequest = serde_json::from_value(serde_json::json!({
            "url": "https://example.com/file.zip"
        }))
        .unwrap();
        assert_eq!(request.completion_action, CompletionAction::None);
    }

    #[test]
    fn waiting_network_status_uses_stable_protocol_value() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::WaitingNetwork).unwrap(),
            "\"waiting-network\""
        );
        assert_eq!(
            TaskStatus::from_db("waiting-network"),
            TaskStatus::WaitingNetwork
        );
    }

    #[test]
    fn power_action_uses_stable_kebab_case_values() {
        assert_eq!(
            serde_json::to_string(&PowerAction::Hibernate).unwrap(),
            "\"hibernate\""
        );
        assert_eq!(
            serde_json::to_string(&PowerActionPhase::Countdown).unwrap(),
            "\"countdown\""
        );
    }
}
