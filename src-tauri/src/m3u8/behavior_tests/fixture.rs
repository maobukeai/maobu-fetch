use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

#[derive(Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
}
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub include_length: bool,
    pub header_delay: Duration,
    pub body_delay: Duration,
}
impl Reply {
    pub fn ok(body: impl AsRef<[u8]>) -> Self {
        Self {
            status: 200,
            body: body.as_ref().to_vec(),
            headers: vec![],
            include_length: true,
            header_delay: Duration::ZERO,
            body_delay: Duration::ZERO,
        }
    }
    pub fn etag(body: impl AsRef<[u8]>, etag: &str) -> Self {
        let mut reply = Self::ok(body);
        reply.headers.push(("ETag".into(), etag.into()));
        reply
    }
}

pub struct Fixture {
    pub url: String,
    pub requests: Arc<Mutex<Vec<Request>>>,
    server: JoinHandle<()>,
}
impl Fixture {
    pub async fn start(handler: impl Fn(Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let records = requests.clone();
        let handler = Arc::new(handler);
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let (handler, records) = (handler.clone(), records.clone());
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut buffer = [0u8; 2048];
                    while !data.windows(4).any(|part| part == b"\r\n\r\n") {
                        let Ok(read) = socket.read(&mut buffer).await else {
                            return;
                        };
                        if read == 0 || data.len() > 32768 {
                            return;
                        }
                        data.extend_from_slice(&buffer[..read]);
                    }
                    let text = String::from_utf8_lossy(&data);
                    let mut lines = text.lines();
                    let mut first = lines.next().unwrap().split_whitespace();
                    let request = Request {
                        method: first.next().unwrap().into(),
                        path: first.next().unwrap().into(),
                        headers: lines
                            .filter_map(|line| {
                                line.split_once(':').map(|(key, value)| {
                                    (key.to_ascii_lowercase(), value.trim().to_owned())
                                })
                            })
                            .collect(),
                    };
                    records.lock().unwrap().push(request.clone());
                    let head = request.method == "HEAD";
                    let reply = handler(request);
                    tokio::time::sleep(reply.header_delay).await;
                    let mut header =
                        format!("HTTP/1.1 {} Test\r\nConnection: close\r\n", reply.status);
                    if reply.include_length {
                        header.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
                    }
                    for (key, value) in reply.headers {
                        header.push_str(&format!("{key}: {value}\r\n"));
                    }
                    header.push_str("\r\n");
                    if socket.write_all(header.as_bytes()).await.is_err() {
                        return;
                    }
                    if !head {
                        tokio::time::sleep(reply.body_delay).await;
                        let _ = socket.write_all(&reply.body).await;
                    }
                });
            }
        });
        Self {
            url: format!("http://{address}/list.m3u8"),
            requests,
            server,
        }
    }

    pub fn count(&self, method: &str, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method == method && request.path == path)
            .count()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
