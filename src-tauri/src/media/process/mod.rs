pub(super) mod lines;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows_job;
use std::{
    process::{ExitStatus, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

pub(super) struct ManagedChild {
    pub child: Child,
    #[cfg(windows)]
    job: Option<windows_job::Job>,
}

impl ManagedChild {
    pub fn spawn(command: &mut Command) -> Result<Self, String> {
        command
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        windows_job::Job::prepare(command);
        let mut child = command
            .spawn()
            .map_err(|_| "无法启动媒体工具，请检查组件路径与权限")?;
        #[cfg(windows)]
        {
            let job = match windows_job::Job::attach_and_resume(&child) {
                Ok(job) => job,
                Err(error) => {
                    let _ = child.start_kill();
                    return Err(error);
                }
            };
            return Ok(Self {
                child,
                job: Some(job),
            });
        }
        #[cfg(not(windows))]
        Ok(Self { child })
    }

    pub async fn terminate(&mut self) {
        self.close_tree();
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }

    pub fn close_tree(&mut self) {
        #[cfg(windows)]
        {
            self.job.take();
        }
    }
}

pub(super) struct CapturedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub(super) async fn capture(
    command: &mut Command,
    cancel: &CancellationToken,
    timeout: Duration,
    stdout_limit: usize,
) -> Result<CapturedOutput, String> {
    if cancel.is_cancelled() {
        return Err("任务已暂停".into());
    }
    let mut managed = ManagedChild::spawn(command)?;
    let stdout = managed
        .child
        .stdout
        .take()
        .ok_or("无法读取媒体工具标准输出")?;
    let stderr = managed
        .child
        .stderr
        .take()
        .ok_or("无法读取媒体工具错误输出")?;
    let operation = async {
        let (stdout, stderr, status) = tokio::try_join!(
            read_bounded(stdout, stdout_limit),
            read_bounded(stderr, 128 * 1024),
            async {
                let status = managed
                    .child
                    .wait()
                    .await
                    .map_err(|_| "等待媒体工具退出失败".to_owned())?;
                managed.close_tree();
                Ok::<ExitStatus, String>(status)
            }
        )?;
        Ok(CapturedOutput {
            stdout,
            stderr,
            status,
        })
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("任务已暂停".into()),
        result = tokio::time::timeout(timeout, operation) => result.unwrap_or_else(|_| Err("媒体工具处理超时，请检查网络或代理后重试".into())),
    };
    if result.is_err() {
        managed.terminate().await;
    }
    result
}

pub(super) async fn read_bounded<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut buffer = [0; 4096];
    let mut bytes = Vec::new();
    loop {
        let length = reader
            .read(&mut buffer)
            .await
            .map_err(|_| "读取媒体工具输出失败")?;
        if length == 0 {
            return Ok(bytes);
        }
        if length > limit.saturating_sub(bytes.len()) {
            return Err("媒体工具输出超过安全上限，已停止处理".into());
        }
        bytes.extend_from_slice(&buffer[..length]);
    }
}
