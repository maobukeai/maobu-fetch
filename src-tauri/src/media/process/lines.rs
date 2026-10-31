use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::mpsc,
};

pub(crate) async fn pump<R: AsyncRead + Unpin>(
    mut reader: R,
    sender: mpsc::Sender<Result<String, String>>,
) {
    let mut buffer = [0; 4096];
    let mut line = Vec::new();
    loop {
        let length = match reader.read(&mut buffer).await {
            Ok(length) => length,
            Err(_) => {
                let _ = sender.send(Err("读取媒体工具输出失败".into())).await;
                return;
            }
        };
        if length == 0 {
            if !line.is_empty() {
                let _ = sender
                    .send(Ok(String::from_utf8_lossy(&line).into_owned()))
                    .await;
            }
            return;
        }
        for byte in &buffer[..length] {
            if *byte == b'\n' {
                let text = String::from_utf8_lossy(&line)
                    .trim_end_matches('\r')
                    .to_owned();
                if sender.send(Ok(text)).await.is_err() {
                    return;
                }
                line.clear();
            } else if line.len() >= 2 * 1024 * 1024 {
                let _ = sender
                    .send(Err("媒体工具单行输出超过安全上限".into()))
                    .await;
                return;
            } else {
                line.push(*byte);
            }
        }
    }
}

pub(crate) struct Readers(pub Vec<tokio::task::JoinHandle<()>>);
impl Drop for Readers {
    fn drop(&mut self) {
        for reader in &self.0 {
            reader.abort();
        }
    }
}
