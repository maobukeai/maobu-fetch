use super::*;

pub(crate) fn cloud_request_context(settings: &AppSettings, task: Option<&DownloadTask>,
    cancel: CancellationToken) -> Result<crate::cloud_request::CloudRequestContext, String> {
    // 私网CDN的自动直连不能扩散到公网网盘API；只有显式任务覆盖继承到这里。
    let (client, no_redirect) = if let Some(task) = task.filter(|task|task.proxy_override.is_some()) {
        (build_task_client(settings, task)?, build_task_client_with_redirect_policy(settings, task, reqwest::redirect::Policy::none())?)
    } else {
        (build_client(settings)?, build_client_with_redirect_policy(settings, reqwest::redirect::Policy::none())?)
    };
    Ok(crate::cloud_request::CloudRequestContext::new(client, no_redirect, cancel))
}
