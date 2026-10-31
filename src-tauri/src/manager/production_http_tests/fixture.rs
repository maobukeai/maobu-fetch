use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::Response,
    routing::any,
    Router,
};
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub const LENGTH: usize = 8 * 1024 * 1024 + 137;

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Good,
    Ignore,
    WrongRange,
    Range416,
    Unknown,
    SilentHeaders,
    SilentBody,
    LateExtra,
    NoValidatorBody,
}

#[derive(Clone, Debug)]
pub struct RequestedRange {
    pub start: u64,
    pub end: u64,
    pub probe: bool,
    pub status: u16,
}

pub struct FixtureState {
    pub mode: Mode,
    pub data: Arc<Vec<u8>>,
    pub requests: Mutex<Vec<RequestedRange>>,
    pub hits: AtomicUsize,
    pub slow: AtomicBool,
    pub version: AtomicU64,
    pub active: AtomicUsize,
    pub peak: AtomicUsize,
    pub expected_parallel: AtomicUsize,
    pub sent: AtomicU64,
    cancelled: CancellationToken,
}

pub struct Fixture {
    pub url: String,
    pub state: Arc<FixtureState>,
    handle: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn new(mode: Mode) -> Self {
        let state = Arc::new(FixtureState {
            mode,
            data: Arc::new(
                (0..LENGTH)
                    .map(|at| ((at * 31 + (at >> 7) + 17) % 251) as u8)
                    .collect(),
            ),
            requests: Mutex::new(Vec::new()),
            hits: AtomicUsize::new(0),
            slow: AtomicBool::new(matches!(
                mode,
                Mode::SilentHeaders | Mode::SilentBody | Mode::NoValidatorBody
            )),
            version: AtomicU64::new(1),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            expected_parallel: AtomicUsize::new(1),
            sent: AtomicU64::new(0),
            cancelled: CancellationToken::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/fixture.bin", any(respond))
            .with_state(state.clone());
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url: format!("http://{address}/fixture.bin"),
            state,
            handle,
        }
    }
    pub fn expected(&self) -> Vec<u8> {
        let mask = if self.state.version.load(Ordering::Acquire) == 1 {
            0
        } else {
            0x5a
        };
        self.state.data.iter().map(|byte| byte ^ mask).collect()
    }
    pub fn resume(&self) {
        self.state.slow.store(false, Ordering::Release);
    }
    pub fn stop_serving(&self) {
        self.handle.abort();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.state.cancelled.cancel();
        self.handle.abort();
    }
}

struct ActiveBody(Arc<FixtureState>);
impl ActiveBody {
    fn new(state: Arc<FixtureState>) -> Self {
        let active = state.active.fetch_add(1, Ordering::AcqRel) + 1;
        state.peak.fetch_max(active, Ordering::AcqRel);
        Self(state)
    }
}
impl Drop for ActiveBody {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

async fn respond(
    State(state): State<Arc<FixtureState>>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    state.hits.fetch_add(1, Ordering::AcqRel);
    if matches!(state.mode, Mode::SilentHeaders) && state.slow.load(Ordering::Acquire) {
        tokio::select! { _ = state.cancelled.cancelled() => {}, _ = tokio::time::sleep(Duration::from_secs(60)) => {} }
    }
    let total = state.data.len() as u64;
    let parsed = headers
        .get("range")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_range);
    let probe = parsed == Some((0, Some(0)));
    let ignore = matches!(state.mode, Mode::Ignore | Mode::Unknown);
    let (start, end, partial) = if ignore {
        (0, total - 1, false)
    } else if let Some((start, end)) = parsed {
        (start, end.unwrap_or(total - 1).min(total - 1), true)
    } else {
        (0, total - 1, false)
    };
    let status = if start >= total || (!probe && matches!(state.mode, Mode::Range416)) {
        416
    } else if partial {
        206
    } else {
        200
    };
    if method != Method::HEAD {
        state.requests.lock().unwrap().push(RequestedRange {
            start,
            end,
            probe: probe && status == 206,
            status,
        });
    }
    let version = state.version.load(Ordering::Acquire);
    let mut response = Response::builder().status(StatusCode::from_u16(status).unwrap());
    if !matches!(state.mode, Mode::NoValidatorBody) {
        response = response
            .header("etag", if version == 1 { "\"ABC\"" } else { "\"abc\"" })
            .header("last-modified", "Wed, 21 Oct 2015 07:28:00 GMT");
    }
    if status == 416 {
        return response
            .header("content-range", format!("bytes */{total}"))
            .body(Body::empty())
            .unwrap();
    }
    if partial {
        let declared_start = if !probe && matches!(state.mode, Mode::WrongRange) {
            start + 1
        } else {
            start
        };
        response = response.header(
            "content-range",
            format!("bytes {declared_start}-{end}/{total}"),
        );
    }
    let length = end - start + 1;
    if !matches!(state.mode, Mode::Unknown) && (probe || !matches!(state.mode, Mode::LateExtra)) {
        response = response.header("content-length", length);
    }
    if method == Method::HEAD {
        return response.body(Body::empty()).unwrap();
    }
    let mask = if version == 1 { 0 } else { 0x5a };
    let slow = !probe
        && matches!(state.mode, Mode::SilentBody | Mode::NoValidatorBody)
        && state.slow.load(Ordering::Acquire);
    let guard = if probe {
        None
    } else {
        Some(ActiveBody::new(state.clone()))
    };
    // Hold successful data responses until all configured connections arrive.
    // A sequential implementation cannot pass the bounded concurrency assertion.
    if !probe && matches!(state.mode, Mode::Good) {
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while state.peak.load(Ordering::Acquire)
                < state.expected_parallel.load(Ordering::Acquire)
            {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await;
    }
    let stream = futures_util::stream::unfold(
        (state, start as usize, end as usize, slow, guard),
        move |(state, position, end, slow, guard)| async move {
            if position > end {
                if !probe && matches!(state.mode, Mode::LateExtra) && position == end + 1 {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    return Some((
                        Ok::<_, Infallible>(vec![0xa5; 17]),
                        (state, end + 2, end, slow, guard),
                    ));
                }
                return None;
            }
            if slow && position > start as usize {
                tokio::select! { _ = state.cancelled.cancelled() => return None, _ = tokio::time::sleep(Duration::from_secs(60)) => {} }
            }
            let next = (position + if slow { 64 * 1024 } else { 256 * 1024 }).min(end + 1);
            let bytes: Vec<u8> = state.data[position..next]
                .iter()
                .map(|byte| byte ^ mask)
                .collect();
            state.sent.fetch_add(bytes.len() as u64, Ordering::AcqRel);
            Some((Ok::<_, Infallible>(bytes), (state, next, end, slow, guard)))
        },
    );
    response.body(Body::from_stream(stream)).unwrap()
}

fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    Some((
        start.parse().ok()?,
        if end.is_empty() {
            None
        } else {
            Some(end.parse().ok()?)
        },
    ))
}
