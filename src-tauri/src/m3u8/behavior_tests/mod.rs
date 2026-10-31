//! Real HTTP/file regression tests of the same engine called by the desktop adapter.
use super::{
    engine::{download, HlsOptions},
    request::{HlsClient, TransferProgress},
    resume,
};
use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
mod crypto_and_parser;
mod fixture;
mod integrity;
mod live;
mod recovery;
mod redirects;
use fixture::{Fixture, Reply};

fn options(url: &str, cancel: CancellationToken) -> HlsOptions {
    HlsOptions {
        http: HlsClient {
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            source_url: url.into(),
            headers: HashMap::new(),
            cancel: cancel.child_token(),
            progress: Arc::new(TransferProgress::default()),
            limit: Arc::new(|_, _| Box::pin(async {})),
            retries: 0,
        },
        user_cancel: cancel,
        finalize_live: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        concurrency: 2,
        live_idle_timeout: Duration::from_secs(5),
    }
}

fn vod(paths: &[&str]) -> String {
    format!(
        "#EXTM3U\n{}#EXT-X-ENDLIST\n",
        paths
            .iter()
            .map(|path| format!("#EXTINF:1,\n{path}\n"))
            .collect::<String>()
    )
}

fn output(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("download.lumaget")
}

async fn read_output(path: &Path) -> Vec<u8> {
    tokio::fs::read(path).await.unwrap()
}
