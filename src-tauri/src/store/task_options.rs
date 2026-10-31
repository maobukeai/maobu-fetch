use super::*;
use crate::models::{ProxyAuth, RetryPolicy};

impl Store {
    pub async fn update_retry_policy(
        &self,
        id: &str,
        policy: Option<RetryPolicy>,
    ) -> Result<DownloadTask, String> {
        self.edit_transport_options(id, |task| task.retry_policy_override = policy)
            .await
    }
    pub async fn update_proxy(
        &self,
        id: &str,
        proxy: Option<String>,
        auth: Option<ProxyAuth>,
    ) -> Result<DownloadTask, String> {
        self.edit_transport_options(id, |task| {
            task.proxy_override = proxy;
            task.proxy_auth = auth;
        })
        .await
    }
    async fn edit_transport_options(
        &self,
        id: &str,
        edit: impl FnOnce(&mut DownloadTask),
    ) -> Result<DownloadTask, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction()
            .map_err(|e| format!("保存传输选项失败：{e}"))?;
        let mut task = transaction
            .query_row("SELECT * FROM tasks WHERE id=?1", [id], task_from_row)
            .optional()
            .map_err(|e| format!("读取传输选项失败：{e}"))?
            .ok_or("任务不存在")?;
        edit(&mut task);
        Self::upsert_with(&transaction, &task)?;
        transaction
            .commit()
            .map_err(|e| format!("提交传输选项失败：{e}"))?;
        Ok(task)
    }
    pub async fn update_cloud_link(&self, task: &DownloadTask) -> Result<(), String> {
        let headers = serde_json::to_string(&task.headers).map_err(|_| "刷新认证信息序列化失败")?;
        let refresh =
            serde_json::to_string(&task.cloud_refresh).map_err(|_| "刷新元数据序列化失败")?;
        let affected=self.connection.lock().await.execute(
            "UPDATE tasks SET url=?1,headers_json=?2,cloud_refresh_json=?3,error=?4 WHERE id=?5 AND status='downloading'",
            params![task.url,headers,refresh,task.error.as_deref().map(crate::manager::redact_sensitive),task.id])
            .map_err(|e|format!("保存刷新链接失败：{e}"))?;
        if affected != 1 {
            return Err("任务已停止或被删除，未覆盖新的任务状态".into());
        }
        Ok(())
    }
    pub async fn update_task_output_name(
        &self,
        id: &str,
        name: &str,
        category: &str,
    ) -> Result<(), String> {
        let affected = self
            .connection
            .lock()
            .await
            .execute(
                "UPDATE tasks SET file_name=?1,category=?2 WHERE id=?3",
                params![name, category, id],
            )
            .map_err(|e| format!("保存目标文件名失败：{e}"))?;
        if affected != 1 {
            return Err("任务已被删除，未发布完成文件".into());
        }
        Ok(())
    }
    /// 只在当前行上修改用户选项，不把调用前的旧进度快照写回数据库。
    pub async fn update_runtime_options(
        &self,
        id: &str,
        priority: Option<i32>,
        limit: Option<u64>,
        action: Option<CompletionAction>,
    ) -> Result<DownloadTask, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction()
            .map_err(|e| format!("保存任务选项失败：{e}"))?;
        let mut task = transaction
            .query_row("SELECT * FROM tasks WHERE id=?1", [id], task_from_row)
            .optional()
            .map_err(|e| format!("读取任务选项失败：{e}"))?
            .ok_or("任务不存在")?;
        if let Some(action) = action {
            if action == CompletionAction::RunFile && task.source != "desktop" {
                return Err("只有桌面端手动创建的任务可以设置完成后运行文件".into());
            }
            task.completion_action = action;
        }
        if let Some(priority) = priority {
            task.priority = priority;
        }
        if let Some(limit) = limit {
            task.per_task_speed_limit = limit;
        }
        Self::upsert_with(&transaction, &task)?;
        transaction
            .commit()
            .map_err(|e| format!("提交任务选项失败：{e}"))?;
        Ok(task)
    }
}
