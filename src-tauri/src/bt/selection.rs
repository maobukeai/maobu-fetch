//! 合法的1基文件选择转换与真实引擎更新。
use super::BtEngine;

impl BtEngine {
    pub async fn select_files_for(
        &self,
        task: &crate::models::DownloadTask,
        indices: &[u32],
    ) -> Result<(), String> {
        if task.status == crate::models::TaskStatus::Completed {
            let files = self.files_for_task(task).await?;
            if indices.iter().any(|index| {
                files
                    .iter()
                    .find(|file| file.index == *index)
                    .is_some_and(|file| file.downloaded_bytes < file.length_bytes)
            }) {
                return Err(
                    "所选文件尚未完整下载；请明确重新下载任务，不能把已完成的做种任务转为后台下载"
                        .into(),
                );
            }
        }
        self.select_files(&task.id, indices).await
    }

    pub async fn select_files(&self, task_id: &str, indices: &[u32]) -> Result<(), String> {
        let _registration = self.registration.lock().await;
        if indices.is_empty() {
            return Err("请至少选择一个 BT 文件".into());
        }
        let session = self.session.lock().await.clone().ok_or("BT 会话尚未启动")?;
        let handle = self
            .bindings
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or("BT 任务尚未注册")?;
        let count = handle
            .with_metadata(|m| m.file_infos.len())
            .map_err(|_| "BT_METADATA_PENDING: 请等待元数据获取完成")?;
        let selected = selected_indices(indices, count)?
            .unwrap_or_default()
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        let stats = handle.stats();
        if stats.finished {
            let missing = handle
                .with_metadata(|metadata| {
                    selected.iter().any(|index| {
                        stats.file_progress.get(*index).copied().unwrap_or(0)
                            < metadata.file_infos[*index].len
                    })
                })
                .map_err(|e| format!("读取 BT 校验状态失败：{e}"))?;
            if missing {
                return Err("新增文件尚未下载，请明确重新下载任务后再选择；已完成的做种任务不能隐式开始补下载".into());
            }
        }
        session
            .update_only_files(&handle, &selected)
            .await
            .map_err(|e| format!("更新 BT 文件选择失败：{e}"))
    }
}

pub(super) fn selected_indices(
    indices: &[u32],
    count: usize,
) -> Result<Option<Vec<usize>>, String> {
    if indices.is_empty() {
        return Ok(None);
    }
    let mut selected = Vec::new();
    for &index in indices {
        if index == 0 || index as usize > count {
            return Err("BT 文件索引超出种子清单范围".into());
        }
        let index = index as usize - 1;
        if !selected.contains(&index) {
            selected.push(index);
        }
    }
    Ok(Some(selected))
}
