use super::super::*;
use axum::{
    body::Body,
    http::{Method, Response},
    routing::any,
    Router,
};

pub(super) struct RecordingFixture {
    pub url: String,
    pub vod_url: String,
    pub waiting_url: String,
    pub bytes: Vec<u8>,
    pub playlists: Arc<AtomicU64>,
    pub silent_segments: Arc<AtomicU64>,
    server: tokio::task::JoinHandle<()>,
}
impl RecordingFixture {
    pub async fn new(bytes: Vec<u8>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let playlists = Arc::new(AtomicU64::new(0));
        let count = playlists.clone();
        let silent_segments = Arc::new(AtomicU64::new(0));
        let silent_requests = silent_segments.clone();
        let data = bytes.clone();
        let silent_size = bytes.len();
        let router = Router::new()
            .route(
                "/live",
                any(move |method: Method| {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::Relaxed);
                        playlist(method, false)
                    }
                }),
            )
            .route(
                "/vod.m3u8",
                any(|method: Method| async move { playlist(method, true) }),
            )
            .route("/waiting",any(|method:Method|async move {
                body(method,"application/vnd.apple.mpegurl",b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2,\n/silent.ts\n".to_vec())
            }))
            .route("/silent.ts",any(move |method:Method| {
              let silent_requests=silent_requests.clone();
              async move {
                if method!=Method::HEAD {silent_requests.fetch_add(1,Ordering::Release);}
                let stream=futures_util::stream::pending::<Result<Vec<u8>,std::convert::Infallible>>();
                Response::builder().header("content-type","video/mp2t").header("content-length",silent_size)
                    .body(if method==Method::HEAD{Body::empty()}else{Body::from_stream(stream)}).unwrap()
              }
            }))
            .route(
                "/recording.ts",
                any(move |method: Method| {
                    let data = data.clone();
                    async move { body(method, "video/mp2t", data) }
                }),
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url: format!("http://{address}/live"),
            vod_url: format!("http://{address}/vod.m3u8"),
            waiting_url: format!("http://{address}/waiting"),
            bytes,
            playlists,
            silent_segments,
            server,
        }
    }
}
impl Drop for RecordingFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
fn playlist(method: Method, vod: bool) -> Response<Body> {
    let mut text="#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2,\n/recording.ts\n".to_owned();
    if vod {
        text.push_str("#EXT-X-ENDLIST\n");
    }
    body(method, "application/vnd.apple.mpegurl", text.into_bytes())
}
fn body(method: Method, kind: &str, bytes: Vec<u8>) -> Response<Body> {
    Response::builder()
        .header("content-type", kind)
        .header("content-length", bytes.len())
        .header("etag", "\"recording-fixture-v1\"")
        .body(if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(bytes)
        })
        .unwrap()
}
