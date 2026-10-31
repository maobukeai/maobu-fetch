mod budget;
mod request;
mod socks;
#[cfg(test)]
mod tests;
mod upstream;
#[cfg(test)]
mod upstream_tests;
pub(crate) use budget::TrafficBudget;

use super::network_policy::NetworkPolicy;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

pub(super) fn configure_command(command: &mut tokio::process::Command, address: &str) {
    // An ambient NO_PROXY can otherwise bypass even yt-dlp's explicit --proxy.
    // Only the child's environment changes; credentials remain out of argv.
    for name in ["NO_PROXY", "no_proxy"] {
        command.env(name, "");
    }
    for name in [
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
    ] {
        command.env(name, address);
    }
}

#[derive(Clone, Default)]
pub(super) struct TrafficStats {
    active: Arc<AtomicUsize>,
    received: Arc<AtomicU64>,
}

impl TrafficStats {
    /// Actual connected TCP tunnels/HTTP streams, not configured fragment concurrency.
    pub fn active(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
}

struct ActiveConnection(TrafficStats);
impl ActiveConnection {
    fn new(stats: &TrafficStats) -> Self {
        stats.active.fetch_add(1, Ordering::Relaxed);
        Self(stats.clone())
    }
}
impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(super) struct TrafficProxy {
    address: String,
    cancel: CancellationToken,
    worker: Option<JoinHandle<()>>,
    pub stats: TrafficStats,
}

impl TrafficProxy {
    pub async fn start(
        policy: NetworkPolicy,
        budget: Option<TrafficBudget>,
        parent: &CancellationToken,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "无法创建本机媒体传输通道")?;
        let port = listener
            .local_addr()
            .map_err(|_| "无法读取本机媒体通道端口")?
            .port();
        let secret = uuid::Uuid::new_v4().simple().to_string();
        let address = format!("http://maobu:{secret}@127.0.0.1:{port}");
        let cancel = parent.child_token();
        let stats = TrafficStats::default();
        let worker = tokio::spawn(serve(
            listener,
            secret,
            policy,
            budget,
            stats.clone(),
            cancel.clone(),
        ));
        Ok(Self {
            address,
            cancel,
            stats,
            worker: Some(worker),
        })
    }

    /// This contains a short-lived credential: write only to the private process config.
    pub fn address(&self) -> &str {
        &self.address
    }

    pub async fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.await;
        }
    }
}

impl Drop for TrafficProxy {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            worker.abort();
        }
    }
}

async fn serve(
    listener: TcpListener,
    secret: String,
    policy: NetworkPolicy,
    budget: Option<TrafficBudget>,
    stats: TrafficStats,
    cancel: CancellationToken,
) {
    let capacity = Arc::new(Semaphore::new(64));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { break };
                if !peer.ip().is_loopback() { continue; }
                let Ok(permit) = capacity.clone().try_acquire_owned() else { continue };
                let (secret, policy, budget, stats, cancel) = (secret.clone(), policy.clone(), budget.clone(), stats.clone(), cancel.clone());
                connections.spawn(async move {
                    let _permit = permit;
                    let _ = handle_connection(stream, &secret, &policy, budget.as_ref(), &stats, &cancel).await;
                });
            }
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}

async fn handle_connection(
    mut client: TcpStream,
    secret: &str,
    policy: &NetworkPolicy,
    budget: Option<&TrafficBudget>,
    stats: &TrafficStats,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let setup = async {
        let header = request::read_header(&mut client).await?;
        let request = match request::parse_header(&header, secret) {
            Ok(request) => request,
            Err(error) => {
                let _ = client.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"Maobu\"\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await;
                return Err(error);
            }
        };
        let upstream =
            upstream::connect(&request.host, request.port, policy.upstream(&request.host)).await;
        match upstream {
            Ok(upstream) => Ok((request, upstream)),
            Err(error) => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await;
                Err(error)
            }
        }
    };
    let (request, mut remote) = tokio::select! {
        _ = cancel.cancelled() => return Err("任务已暂停".into()),
        result = tokio::time::timeout(std::time::Duration::from_secs(20), setup) => result.map_err(|_| "媒体代理连接超时")??,
    };
    let _active = ActiveConnection::new(stats);
    if request.connect {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .map_err(|_| "媒体代理响应失败")?;
    } else {
        remote
            .write_all(&request.forward_header)
            .await
            .map_err(|_| "媒体代理请求发送失败")?;
    }
    let (client_read, client_write) = client.split();
    let (remote_read, remote_write) = tokio::io::split(remote);
    tokio::select! {
        _ = cancel.cancelled() => Err("任务已暂停".into()),
        result = async {
            tokio::try_join!(pipe(client_read, remote_write, None, None, cancel), pipe(remote_read, client_write, budget, Some(stats), cancel))?;
            Ok(())
        } => result,
    }
}

async fn pipe<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut reader: R,
    mut writer: W,
    budget: Option<&TrafficBudget>,
    stats: Option<&TrafficStats>,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let mut buffer = [0; 16 * 1024];
    loop {
        let length = tokio::select! { _ = cancel.cancelled() => return Err("任务已暂停".into()), result = reader.read(&mut buffer) => result.map_err(|_| "媒体通道读取失败")? };
        if length == 0 {
            let _ = writer.shutdown().await;
            return Ok(());
        }
        if let Some(budget) = budget {
            budget.acquire(length, cancel).await?;
        }
        tokio::select! { _ = cancel.cancelled() => return Err("任务已暂停".into()), result = writer.write_all(&buffer[..length]) => result.map_err(|_| "媒体通道写入失败")? };
        if let Some(stats) = stats {
            stats.received.fetch_add(length as u64, Ordering::Relaxed);
        }
    }
}
