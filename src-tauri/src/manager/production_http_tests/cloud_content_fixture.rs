use axum::{body::Body, extract::State, http::HeaderMap, response::Response, Router};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Html,
    StreamHtml,
    RangeHtml,
    Binary,
    UnknownMime,
    Gzip,
}
struct Data {
    mode: Mode,
    body: Vec<u8>,
    requests: Mutex<Vec<Option<(usize, usize)>>>,
}
pub(super) struct Fixture {
    pub url: String,
    data: Arc<Data>,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn new(mode: Mode) -> Self {
        let length = if matches!(mode, Mode::RangeHtml) {
            4 * 1024 * 1024 + 128
        } else {
            8192
        };
        let mut body = vec![b' '; length];
        let prefix = if matches!(mode, Mode::Binary | Mode::UnknownMime) {
            b"MZ synthetic binary payload".as_slice()
        } else {
            b"<!DOCTYPE html><html><script>window.location='/download'</script></html>".as_slice()
        };
        body[..prefix.len()].copy_from_slice(prefix);
        if matches!(mode, Mode::Gzip) {
            // A valid gzip member containing HTML; the existing identity guard
            // must reject its encoding before any decompressed content is used.
            body = vec![
                31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 179, 201, 40, 201, 205, 177, 203, 204, 43, 73,
                45, 202, 77, 77, 201, 76, 44, 73, 181, 209, 7, 11, 1, 0, 122, 38, 189, 211, 25, 0,
                0, 0,
            ];
        }
        let data = Arc::new(Data {
            mode,
            body,
            requests: Mutex::new(vec![]),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().fallback(respond).with_state(data.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url: format!("http://{address}/selected-file"),
            data,
            server,
        }
    }
    pub fn expected(&self) -> &[u8] {
        &self.data.body
    }
    pub fn requested_start(&self, start: usize) -> bool {
        self.data
            .requests
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .any(|&(at, _)| at == start)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn respond(State(data): State<Arc<Data>>, headers: HeaderMap) -> Response {
    let parsed = headers
        .get("range")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.split_once('-'))
        .and_then(|(start, end)| {
            Some((
                start.parse::<usize>().ok()?,
                if end.is_empty() {
                    data.body.len() - 1
                } else {
                    end.parse::<usize>().ok()?
                },
            ))
        });
    data.requests.lock().unwrap().push(parsed);
    let is_probe = parsed == Some((0, 0));
    let changed_after_probe = matches!(data.mode, Mode::StreamHtml | Mode::RangeHtml);
    let partial = changed_after_probe && parsed.is_some();
    let (start, end) = if partial {
        parsed.unwrap()
    } else {
        (0, data.body.len() - 1)
    };
    assert!(start <= end && end < data.body.len());
    let mut response = Response::builder()
        .status(if partial { 206 } else { 200 })
        .header("content-length", end - start + 1)
        .header("etag", "\"same-resource\"")
        .header("last-modified", "Wed, 21 Oct 2015 07:28:00 GMT");
    if partial {
        response = response.header(
            "content-range",
            format!("bytes {start}-{end}/{}", data.body.len()),
        );
    }
    let mime = match data.mode {
        Mode::UnknownMime => None,
        Mode::Binary | Mode::Gzip => Some("application/octet-stream"),
        _ if changed_after_probe && is_probe => Some("application/octet-stream"),
        _ => Some("Text/Html; charset=utf-8"),
    };
    if let Some(mime) = mime {
        response = response.header("content-type", mime);
    }
    if matches!(data.mode, Mode::Gzip) {
        response = response.header("content-encoding", "gzip");
    }
    response
        .body(Body::from(data.body[start..=end].to_vec()))
        .unwrap()
}
