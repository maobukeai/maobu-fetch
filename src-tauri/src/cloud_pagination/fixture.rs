use super::*;
use axum::{
    body::Body,
    extract::State,
    http::{StatusCode, Uri},
    response::Response,
    routing::get,
    Router,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
#[derive(Clone, Copy, Debug)]
pub enum Style {
    Baidu,
    Quark,
    Pan123,
    PikPak,
}
impl Style {
    pub fn root(self) -> &'static str {
        match self {
            Self::Baidu => "/",
            Self::PikPak => "",
            _ => "0",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Scenario {
    Flat101,
    Flat201,
    Flat301,
    Duplicate,
    Deep80,
    DeepFailure,
    SizeOverflow,
    MetadataLimit,
    ApiError,
    InvalidTotal,
    InvalidCursor,
    ChangedTotal,
    Failure,
    Malformed,
    Repeated,
    Truncated,
    EntryLimit,
    FolderLimit,
    PageLimit,
    DepthLimit,
    Timeout,
    Cancelled,
}
impl Scenario {
    pub const ALL: [Self; 22] = [
        Self::Flat101,
        Self::Flat201,
        Self::Flat301,
        Self::Duplicate,
        Self::Deep80,
        Self::DeepFailure,
        Self::SizeOverflow,
        Self::MetadataLimit,
        Self::ApiError,
        Self::InvalidTotal,
        Self::InvalidCursor,
        Self::ChangedTotal,
        Self::Failure,
        Self::Malformed,
        Self::Repeated,
        Self::Truncated,
        Self::EntryLimit,
        Self::FolderLimit,
        Self::PageLimit,
        Self::DepthLimit,
        Self::Timeout,
        Self::Cancelled,
    ];
}
struct FixtureState {
    style: Style,
    case: Scenario,
    hits: AtomicUsize,
}
pub struct Fixture {
    pub base: String,
    style: Style,
    case: Scenario,
    state: Arc<FixtureState>,
    handle: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn start(style: Style, case: Scenario) -> Self {
        let state = Arc::new(FixtureState {
            style,
            case,
            hits: AtomicUsize::new(0),
        });
        let router = Router::new()
            .fallback(get(respond))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            base: format!("http://{address}"),
            style,
            case,
            state,
            handle,
        }
    }
    pub fn context(&self) -> Context {
        let mut context = Context::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            &self.base,
            CancellationToken::new(),
            "合成网盘",
        );
        match self.case {
            Scenario::EntryLimit => context.limits.entries = 50,
            Scenario::MetadataLimit => context.limits.metadata_bytes = 50,
            Scenario::FolderLimit => context.limits.folders = 5,
            Scenario::PageLimit => context.limits.pages = 1,
            Scenario::DepthLimit => context.limits.depth = 5,
            Scenario::Timeout => context.limits.duration = Duration::from_millis(20),
            Scenario::Cancelled => {
                let cancel = context.cancel.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    cancel.cancel();
                });
            }
            _ => {}
        }
        context
    }
    pub fn verify(&self, result: Result<Vec<Entry>, String>) {
        let expected = match self.case {
            Scenario::Flat101 | Scenario::Duplicate => Some(101),
            Scenario::Flat201 => Some(201),
            Scenario::Flat301 => Some(301),
            Scenario::Deep80 => Some(81),
            _ => None,
        };
        if let Some(expected) = expected {
            let entries = result
                .unwrap_or_else(|error| panic!("{:?}/{:?}: {}", self.style, self.case, error));
            assert_eq!(entries.len(), expected);
            assert_eq!(
                entries
                    .iter()
                    .map(|entry| &entry.id)
                    .collect::<HashSet<_>>()
                    .len(),
                expected
            );
            if matches!(self.case, Scenario::Deep80) {
                assert_eq!(entries.iter().filter(|entry| !entry.folder).count(), 1);
                assert!(entries.last().unwrap().path.matches('/').count() >= 80);
            }
        } else {
            let error = result
                .err()
                .expect("incomplete listing must be an error, never partial success");
            assert!(error.contains("未完整"), "{error}");
            for secret in [
                "synthetic-password",
                "synthetic-cookie",
                "synthetic-sign",
                "synthetic-share",
            ] {
                assert!(
                    !error.contains(secret),
                    "credentials must not appear in errors"
                );
            }
        }
        assert!(
            self.state.hits.load(Ordering::Acquire) < 100,
            "local fixtures must never spin indefinitely"
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn respond(State(state): State<Arc<FixtureState>>, uri: Uri) -> Response {
    state.hits.fetch_add(1, Ordering::AcqRel);
    let query: HashMap<_, _> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    if matches!(state.case, Scenario::Timeout | Scenario::Cancelled) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let page = query
        .get("page")
        .or_else(|| query.get("_page"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| {
            query
                .get("page_token")
                .and_then(|value| value.strip_prefix("next-"))
                .and_then(|v| v.parse::<usize>().ok())
                .map(|offset| offset / 100 + 1)
                .unwrap_or(1)
        });
    if page == 2 && matches!(state.case, Scenario::Failure) {
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("synthetic only"))
            .unwrap();
    }
    if page == 2 && matches!(state.case, Scenario::Malformed) {
        return Response::new(Body::from("not json"));
    }
    let parent = query
        .get("dir")
        .or_else(|| query.get("pdir_fid"))
        .or_else(|| query.get("parentFileId"))
        .or_else(|| query.get("parent_id"))
        .cloned()
        .unwrap_or_default();
    let depth = parent
        .rsplit("folder-")
        .next()
        .and_then(|v| v.parse::<usize>().ok())
        .or_else(|| parent.parse::<usize>().ok())
        .unwrap_or(0);
    let deep = matches!(
        state.case,
        Scenario::Deep80 | Scenario::DeepFailure | Scenario::FolderLimit | Scenario::DepthLimit
    );
    if depth == 1 && matches!(state.case, Scenario::DeepFailure) {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("synthetic folder missing"))
            .unwrap();
    }
    let count = if deep {
        1
    } else {
        match state.case {
            Scenario::Flat201 => 201,
            Scenario::Flat301 | Scenario::Repeated => 301,
            _ => 101,
        }
    };
    let offset = if page == 2 && matches!(state.case, Scenario::Repeated) {
        0
    } else {
        (page - 1) * 100
    };
    let end = (offset + 100).min(count);
    let mut records = Vec::new();
    if deep {
        let folder = depth < 80;
        let id = if folder {
            (depth + 1).to_string()
        } else {
            "leaf".into()
        };
        let name = if folder {
            format!("folder-{}", depth + 1)
        } else {
            "leaf.bin".into()
        };
        records.push(record(state.style, &id, &name, folder, depth + 1));
    } else if !matches!(state.case, Scenario::Truncated) {
        for index in offset..end {
            records.push(record(
                state.style,
                &(index + 1).to_string(),
                &format!("entry-{index}.bin"),
                false,
                index + 1,
            ));
        }
        if page == 1 && matches!(state.case, Scenario::Duplicate) {
            records.push(records[0].clone());
        }
    }
    if matches!(state.case, Scenario::SizeOverflow) {
        let key = if matches!(state.style, Style::Pan123) {
            "Size"
        } else {
            "size"
        };
        records[0][key] = json!(u64::MAX);
    }
    let more = !deep && end < count;
    let next = if more {
        format!("next-{end}")
    } else {
        String::new()
    };
    let mut value = match state.style {
        Style::Baidu => {
            json!({"errno":0,"list":records,"total":count,"has_more":if more{1}else{0}})
        }
        Style::Quark => json!({"code":0,"data":{"list":records},"metadata":{"_total":count}}),
        Style::Pan123 => {
            json!({"code":0,"data":{"InfoList":records,"Total":count,"Next":if more{end as i64}else{-1}}})
        }
        Style::PikPak => json!({"files":records,"total":count,"next_page_token":next}),
    };
    if matches!(state.case, Scenario::ApiError) {
        match state.style {
            Style::Baidu => value["errno"] = json!(-9),
            Style::Quark | Style::Pan123 => value["code"] = json!(500),
            Style::PikPak => value["error"] = json!("file_not_found"),
        };
    }
    if matches!(state.case, Scenario::InvalidTotal | Scenario::ChangedTotal) {
        let total = if matches!(state.case, Scenario::InvalidTotal) {
            json!("invalid")
        } else if page == 2 {
            json!(count + 1)
        } else {
            json!(count)
        };
        match state.style {
            Style::Baidu | Style::PikPak => value["total"] = total,
            Style::Quark => value["metadata"]["_total"] = total,
            Style::Pan123 => value["data"]["Total"] = total,
        };
    }
    if matches!(state.case, Scenario::InvalidCursor) {
        match state.style {
            Style::Baidu => value["has_more"] = json!({}),
            Style::Quark => value["data"]["has_more"] = json!({}),
            Style::Pan123 => value["data"]["Next"] = json!({}),
            Style::PikPak => value["next_page_token"] = json!({}),
        };
    }
    Response::new(Body::from(value.to_string()))
}
fn record(style: Style, id: &str, name: &str, folder: bool, size: usize) -> Value {
    match style {
        Style::Baidu => {
            json!({"fs_id":id,"server_filename":name,"isdir":if folder{1}else{0},"size":size,"path":format!("/{name}")})
        }
        Style::Quark => {
            json!({"fid":id,"file_name":name,"dir":folder,"file":!folder,"size":size,"share_fid_token":"synthetic-sign"})
        }
        Style::Pan123 => {
            json!({"FileId":id.parse::<i64>().unwrap_or(9_000_001),"FileName":name,"Type":if folder{1}else{0},"Size":size,"Etag":"synthetic","S3KeyFlag":"synthetic"})
        }
        Style::PikPak => {
            json!({"id":id,"name":name,"kind":if folder{"drive#folder"}else{"drive#file"},"size":size.to_string()})
        }
    }
}
