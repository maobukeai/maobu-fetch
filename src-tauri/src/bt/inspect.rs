//! 无写入的磁力元数据预览。
use super::{BtEngine, DEFAULT_PUBLIC_TRACKERS};
use crate::models::BtFileEntry;

impl BtEngine {
    pub async fn inspect_magnet(
        &self,
        magnet_url: &str,
        timeout_secs: u64,
    ) -> Result<crate::models::BtTorrentInspectResult, String> {
        let session = self.get_or_init_session(None).await?;

        let mut magnet_with_trackers = magnet_url.to_string();
        for tr in DEFAULT_PUBLIC_TRACKERS {
            let encoded: String = url::form_urlencoded::byte_serialize(tr.as_bytes()).collect();
            if !magnet_with_trackers.contains(&encoded) {
                magnet_with_trackers.push_str("&tr=");
                magnet_with_trackers.push_str(&encoded);
            }
        }

        let add_torrent = librqbit::AddTorrent::from_url(&magnet_with_trackers);
        let opts = librqbit::AddTorrentOptions {
            list_only: true,
            ..Default::default()
        };

        let fut = session.add_torrent(add_torrent, Some(opts));
        let response = match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(format!("获取磁力元数据失败: {e}")),
            Err(_) => return Err("未能即时获取到磁力元数据（网络较慢或节点较少），建议直接点击【开始下载】，任务会在后台全速获取并开始下载".into()),
        };

        match response {
            librqbit::AddTorrentResponse::ListOnly(resp) => {
                let name = resp
                    .info
                    .name()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "未命名种子".into());
                let mut files = Vec::new();
                let mut total_bytes = 0u64;
                for (idx, f) in resp.info.iter_file_details().enumerate() {
                    total_bytes += f.len;
                    files.push(BtFileEntry {
                        index: (idx + 1) as u32,
                        path: f.filename.to_string(),
                        length_bytes: f.len,
                        selected: true,
                        downloaded_bytes: 0,
                    });
                }
                Ok(crate::models::BtTorrentInspectResult {
                    info_hash: resp.info_hash.as_string(),
                    name,
                    total_bytes,
                    files,
                })
            }
            _ => Err("未能解析磁力元数据响应".into()),
        }
    }
}
