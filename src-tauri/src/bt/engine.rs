//! 单个进程内 librqbit 会话、持久化与进程级状态。
use crate::models::AppSettings;
use librqbit::{ManagedTorrent, Session, SessionOptions};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
/// 纯 Rust BT 引擎：单例管理 librqbit Session、速度采样与任务绑定。
#[derive(Default)]
pub struct BtEngine {
    pub(super) session: Mutex<Option<Arc<Session>>>,
    pub(super) bindings: Mutex<HashMap<String, Arc<ManagedTorrent>>>,
    pub(super) speed_samples: Mutex<HashMap<String, SpeedSample>>,
    data_dir: std::sync::RwLock<Option<PathBuf>>,
    pub(super) last_settings: Mutex<Option<AppSettings>>,
    pub(super) seed_monitors: Mutex<HashMap<String, (String, CancellationToken)>>,
    pub(super) download_budget: std::sync::atomic::AtomicU64,
    pub(super) upload_budget: std::sync::atomic::AtomicU64,
    pub(super) registration: Mutex<()>,
    pub(super) seed_sync: Mutex<()>,
    pub(super) seed_preparation: std::sync::Mutex<Option<super::preparation::PreparationState>>,
}

pub(super) struct SpeedSample {
    pub(super) last_uploaded: u64,
    pub(super) total_uploaded: u64,
}

impl BtEngine {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
            bindings: Mutex::new(HashMap::new()),
            speed_samples: Mutex::new(HashMap::new()),
            data_dir: std::sync::RwLock::new(None),
            last_settings: Mutex::new(None),
            seed_monitors: Mutex::new(HashMap::new()),
            download_budget: std::sync::atomic::AtomicU64::new(0),
            upload_budget: std::sync::atomic::AtomicU64::new(0),
            registration: Mutex::new(()),
            seed_sync: Mutex::new(()),
            seed_preparation: std::sync::Mutex::new(None),
        }
    }

    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self {
            session: Mutex::new(None),
            bindings: Mutex::new(HashMap::new()),
            speed_samples: Mutex::new(HashMap::new()),
            data_dir: std::sync::RwLock::new(Some(data_dir)),
            last_settings: Mutex::new(None),
            seed_monitors: Mutex::new(HashMap::new()),
            download_budget: std::sync::atomic::AtomicU64::new(0),
            upload_budget: std::sync::atomic::AtomicU64::new(0),
            registration: Mutex::new(()),
            seed_sync: Mutex::new(()),
            seed_preparation: std::sync::Mutex::new(None),
        }
    }

    pub fn set_data_dir(&self, dir: PathBuf) {
        if let Ok(mut guard) = self.data_dir.write() {
            *guard = Some(dir);
        }
    }

    pub fn resolve_data_dir(&self) -> PathBuf {
        if let Ok(guard) = self.data_dir.read() {
            if let Some(dir) = guard.as_ref() {
                return dir.clone();
            }
        }
        if let Some(env_dir) = std::env::var_os("MAOBU_FETCH_DATA_DIR")
            .or_else(|| std::env::var_os("LUMAGET_DATA_DIR"))
            .map(PathBuf::from)
        {
            return env_dir.join("session");
        }
        #[cfg(windows)]
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("maobu-fetch").join("session");
        }
        if let Some(base) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
            return base.join("maobu-fetch/session");
        }
        if let Some(base) = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
        {
            return base.join(".local/share/maobu-fetch/session");
        }
        // 生产 manager 总是显式传入 app_data_dir；不回退到易失临时目录。
        PathBuf::from(".maobu-fetch/session")
    }

    pub(super) async fn get_or_init_session(
        &self,
        settings_override: Option<&AppSettings>,
    ) -> Result<Arc<Session>, String> {
        let settings_to_apply = if let Some(settings) = settings_override {
            let mut last_settings = self.last_settings.lock().await;
            *last_settings = Some(settings.clone());
            Some(settings.clone())
        } else {
            let last_settings = self.last_settings.lock().await;
            last_settings.clone()
        };

        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_ref() {
            if let Some(settings) = settings_to_apply.as_ref() {
                self.apply_session_limits(session, settings);
            }
            return Ok(session.clone());
        }

        let default_dir = self.resolve_data_dir();
        tokio::fs::create_dir_all(&default_dir)
            .await
            .map_err(|e| format!("创建 BT 会话目录失败：{e}"))?;
        let clean_marker = default_dir.join("maobu-clean-shutdown");
        let restore_recheck = default_dir.join("session.json").is_file() && !clean_marker.is_file();
        match tokio::fs::remove_file(&clean_marker).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("更新 BT 会话启动标记失败：{error}")),
        }
        let dht_opts = librqbit::DhtSessionConfig {
            persistence: Some(librqbit::dht::DhtPersistenceConfig {
                config_filename: Some(default_dir.join("dht.json")),
                ..Default::default()
            }),
            ..Default::default()
        };
        let opts = SessionOptions {
            dht: if cfg!(test) { None } else { Some(dht_opts) },
            fastresume: true,
            persistence: Some(librqbit::SessionPersistenceConfig::Json {
                folder: Some(default_dir.clone()),
            }),
            restore_paused: true,
            restore_recheck,
            disable_local_service_discovery: true,
            ..Default::default()
        };
        let session = Session::new_with_opts(default_dir, opts)
            .await
            .map_err(|e| format!("初始化纯 Rust BT 引擎失败: {e}"))?;
        if let Some(settings) = settings_to_apply.as_ref() {
            self.apply_session_limits(&session, settings);
        }
        *guard = Some(session.clone());
        tracing::info!("纯 Rust BT 引擎 (librqbit) 初始化成功");
        Ok(session)
    }

    pub async fn ensure_started(
        &self,
        _app: &tauri::AppHandle,
        settings: &AppSettings,
    ) -> Result<Arc<Session>, String> {
        self.get_or_init_session(Some(settings)).await
    }

    pub async fn is_running(&self) -> bool {
        let guard = self.session.lock().await;
        guard.is_some()
    }
}
