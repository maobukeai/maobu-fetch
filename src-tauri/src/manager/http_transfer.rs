use super::http_response::ResponsePlan;
use futures_util::StreamExt;
use std::{future::Future, path::Path};
use tokio::{fs::OpenOptions, io::{AsyncWriteExt, BufWriter}};
use tokio_util::sync::CancellationToken;

pub(super) trait StreamObserver: Send {
    fn before_write(&mut self, bytes: u64) -> impl Future<Output = Result<(), String>> + Send;
    fn progress(&mut self, total: u64) -> impl Future<Output = Result<(), String>> + Send;
}

/// 普通流的唯一生产写入循环；测试直接调用本函数，覆盖实际取消/长度/落盘语义。
pub(super) async fn receive_response<O: StreamObserver>(
    response: reqwest::Response, plan: &ResponsePlan, target: &Path,
    token: &CancellationToken, buffer_size: usize, observer: &mut O,
) -> Result<u64, String> {
    if token.is_cancelled() { return Err("任务已暂停".into()); }
    let file = OpenOptions::new().create(true).write(true).append(plan.append)
        .truncate(!plan.append).open(target).await
        .map_err(|error| format!("无法写入下载临时文件：{error}"))?;
    let mut file = BufWriter::with_capacity(buffer_size, file);
    let mut received = 0u64;
    let mut stream = response.bytes_stream();
    let result = async {
        loop {
            let chunk = tokio::select! {
                biased;
                _ = token.cancelled() => return Err("任务已暂停".to_string()),
                next = stream.next() => next,
            };
            let Some(chunk) = chunk else { break; };
            let chunk = chunk.map_err(super::friendly_body_error)?;
            let next = received.checked_add(chunk.len() as u64).ok_or("响应长度溢出")?;
            if plan.response_bytes.is_some_and(|expected| next > expected) {
                return Err("响应数据超出声明范围，已停止写入并保留分片".into());
            }
            observer.before_write(chunk.len() as u64).await?;
            if token.is_cancelled() { return Err("任务已暂停".into()); }
            file.write_all(&chunk).await.map_err(|error| format!("写入下载文件失败：{error}"))?;
            received = next;
            observer.progress(plan.initial_bytes + received).await?;
        }
        if plan.response_bytes.is_some_and(|expected| received != expected) {
            return Err("下载响应不完整，已保留分片供重试".into());
        }
        let total = plan.initial_bytes + received;
        if plan.total_bytes > 0 && total != plan.total_bytes {
            return Err(format!("下载提前结束（已接收 {total} 字节 / 共 {} 字节），已保留分片", plan.total_bytes));
        }
        Ok(total)
    }.await;
    // 取消、网络断开、长度校验失败都必须完成 flush；失败不能被当作已暂停成功。
    file.flush().await.map_err(|error| format!("保存下载分片失败：{error}"))?;
    result
}
