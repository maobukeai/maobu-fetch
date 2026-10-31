use super::super::CloudRequestContext;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::Response,
    routing::any,
    Router,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub enum Mode {
    Normal,
    BaiduPcs,
    QuarkSaved,
    SilentHeaders,
    SilentBody,
    DirectOrigin,
    CookieRedirect,
}
pub struct StateData {
    pub seen: Mutex<HashMap<String, usize>>,
    pub rejected: AtomicUsize,
    pub mode: Mode,
    pub headers: Mutex<Vec<(String, HeaderMap)>>,
    downloads: AtomicUsize,
}
pub struct ProxyFixture {
    pub address: String,
    pub state: Arc<StateData>,
    handle: tokio::task::JoinHandle<()>,
}
impl ProxyFixture {
    pub fn settings(&self) -> crate::models::AppSettings {
        let mut settings = crate::models::AppSettings::default();
        settings.proxy_mode = "manual".into();
        settings.proxy_url = self.address.clone();
        settings.proxy_username = "fixture-user".into();
        settings.proxy_password = "fixture-password".into();
        settings
    }
    pub fn app_network(&self, cancel: CancellationToken) -> CloudRequestContext {
        crate::manager::cloud_request_context(&self.settings(), None, cancel)
            .unwrap()
            .with_test_endpoint("http://cloud-fixture.invalid")
    }
    pub async fn start(mode: Mode) -> Self {
        let state = Arc::new(StateData {
            seen: Mutex::new(HashMap::new()),
            rejected: AtomicUsize::new(0),
            mode,
            headers: Mutex::new(Vec::new()),
            downloads: AtomicUsize::new(0),
        });
        let router = Router::new()
            .fallback(any(respond))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            address: format!("http://{address}"),
            state,
            handle,
        }
    }
    pub fn network(&self, cancel: CancellationToken, password: &str) -> CloudRequestContext {
        let build = |policy| {
            reqwest::Client::builder()
                .no_proxy()
                .proxy(
                    reqwest::Proxy::all(&self.address)
                        .unwrap()
                        .basic_auth("fixture-user", password),
                )
                .redirect(policy)
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap()
        };
        CloudRequestContext::new(
            build(reqwest::redirect::Policy::limited(10)),
            build(reqwest::redirect::Policy::none()),
            cancel,
        )
        .with_test_endpoint("http://cloud-fixture.invalid")
    }
    pub fn require(&self, path: &str) {
        assert!(
            self.state.seen.lock().unwrap().contains_key(path),
            "required cloud phase did not traverse authenticated proxy"
        );
    }
}
impl Drop for ProxyFixture {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
fn body(value: Value) -> Response {
    Response::new(Body::from(value.to_string()))
}
fn redirect() -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header("Location", "https://download.example.invalid/fixture.bin")
        .body(Body::empty())
        .unwrap()
}
async fn respond(
    State(state): State<Arc<StateData>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    // Synthetic credentials only. Nothing from a real user's proxy or accounts is read.
    if matches!(state.mode, Mode::DirectOrigin) {
        assert!(
            headers.get("proxy-authorization").is_none(),
            "direct requests must not contain proxy credentials"
        );
    } else if headers
        .get("proxy-authorization")
        .and_then(|v| v.to_str().ok())
        != Some("Basic Zml4dHVyZS11c2VyOmZpeHR1cmUtcGFzc3dvcmQ=")
    {
        state.rejected.fetch_add(1, Ordering::AcqRel);
        return Response::builder()
            .status(StatusCode::PROXY_AUTHENTICATION_REQUIRED)
            .header("Proxy-Authenticate", "Basic realm=fixture")
            .body(Body::from("proxy authentication required"))
            .unwrap();
    }
    assert_ne!(
        method,
        Method::CONNECT,
        "all fixture endpoints must stay local HTTP; final redirects are not followed"
    );
    *state
        .seen
        .lock()
        .unwrap()
        .entry(uri.path().to_owned())
        .or_default() += 1;
    if matches!(state.mode, Mode::SilentHeaders) {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    if matches!(state.mode, Mode::SilentBody) {
        return Response::new(Body::from_stream(futures_util::stream::once(async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            Ok::<_, std::convert::Infallible>("{}".as_bytes())
        })));
    }
    route(&state, uri, &headers)
}
fn route(state: &StateData, uri: Uri, headers: &HeaderMap) -> Response {
    let path = uri.path();
    state
        .headers
        .lock()
        .unwrap()
        .push((path.into(), headers.clone()));
    match path{
        "/pan.baidu.com/share/verify"=>Response::builder().header("Set-Cookie","BDCLND=synthetic-sign; Path=/").body(Body::from(json!({"errno":0,"randsk":"synthetic-sign"}).to_string())).unwrap(),
        "/pan.baidu.com/s/1synthetic"=>Response::new(Body::from(r#"<script>{"share_uk":123,"shareid":456,"sign":"c2lnbg==","timestamp":123,"seckey":"c2lnbg=="}</script>"#)),
        "/pan.baidu.com/share/list"=>body(json!({"errno":0,"list":[{"fs_id":"1","server_filename":"fixture.bin","isdir":0,"size":3}],"total":1,"has_more":0})),
        "/pan.baidu.com/api/sharedownload"=>if matches!(state.mode,Mode::BaiduPcs){body(json!({"errno":-1}))}else{body(json!({"errno":0,"list":[{"dlink":"https://download.example.invalid/baidu"}]}))},
        "/pan.baidu.com/share/transfer"=>body(json!({"errno":0,"extra":{"list":[{"to_fs_id":"2","to":"/fixture.bin","to_server_filename":"fixture.bin"}]}})),
        "/d.pcs.baidu.com/rest/2.0/pcs/file"=>{
            let query:HashMap<_,_>=url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).into_owned().collect();
            if query.get("method").map(String::as_str)==Some("download"){redirect()}else{body(json!({"errno":-1}))}
        },
        "/drive.quark.cn/1/clouddrive/share/sharepage/token"=>if matches!(state.mode,Mode::CookieRedirect) {
            Response::builder().status(302).header("Location","/redirected").body(Body::empty()).unwrap()
        }else{body(json!({"code":0,"data":{"stoken":"synthetic-sign"}}))},
        "/drive.quark.cn/redirected"=>Response::builder().status(302).header("Location","https://cdn.example.invalid/token").body(Body::empty()).unwrap(),
        "/cdn.example.invalid/token"=>body(json!({"code":0,"data":{"stoken":"synthetic-sign"}})),
        "/drive.quark.cn/secure"=>Response::builder().status(302).header("Location","http://drive.quark.cn/plain").body(Body::empty()).unwrap(),
        "/drive.quark.cn/plain"=>body(json!({"ok":true})),
        "/drive.quark.cn/1/clouddrive/share/sharepage/detail"=>body(json!({"code":0,"data":{"list":[{"fid":"1","file_name":"fixture.bin","dir":false,"file":true,"size":3,"share_fid_token":"synthetic-sign"}],"total":1}})),
        "/drive-pc.quark.cn/1/clouddrive/file/download"=>{
            if matches!(state.mode,Mode::QuarkSaved)&&state.downloads.fetch_add(1,Ordering::AcqRel)==0{body(json!({"code":23018,"message":"size limit"}))}else{body(json!({"code":0,"data":{"download_url":"https://download.example.invalid/quark"}}))}
        },
        "/drive.quark.cn/1/clouddrive/share/sharepage/save"=>body(json!({"code":0,"data":{"task_id":"synthetic-save"}})),
        "/drive.quark.cn/1/clouddrive/task"=>body(json!({"code":0,"data":{"status":2,"save_as_top_fids":["saved"]}})),
        "/user.mypikpak.com/v1/shield/captcha/init"=>body(json!({"captcha_token":"synthetic-sign"})),
        "/api-drive.mypikpak.com/drive/v1/share"=>body(json!({"files":[{"id":"1","name":"fixture.bin","kind":"drive#file","size":"3"}],"next_page_token":""})),
        "/api-drive.mypikpak.com/drive/v1/share/file_info"=>body(json!({"file_info":{"web_content_link":"https://download.example.invalid/pikpak"}})),
        "/www.123684.com/api/share/get"=>body(json!({"code":0,"data":{"InfoList":[{"FileId":1,"FileName":"fixture.bin","Size":3,"Etag":"synthetic","S3KeyFlag":"synthetic","Type":0}],"Total":1,"Next":-1}})),
        "/www.123684.com/api/share/download/info"=>body(json!({"code":0,"data":{"DownloadURL":"https://download.example.invalid/pan123"}})),
        "/wwfixture.lanzouq.com/synthetic"=>{
            if headers.get("cookie").and_then(|v|v.to_str().ok()).is_none_or(|value|!value.contains("acw_sc__v2=")){Response::new(Body::from("<script>var arg1='0123456789ABCDEF0123456789ABCDEF01234567';</script>"))}
            else{Response::new(Body::from("<div class=\"n_box_3fn\">fixture.bin</div><iframe src=\"/fn?fixture=1\"></iframe>"))}
        },
        "/wwfixture.lanzouq.com/fn"=>Response::new(Body::from("<script>var sign='synthetic';url:'/ajaxm.php';data:{action:'downprocess',sign:sign,ves:1};</script>")),
        "/wwfixture.lanzouq.com/ajaxm.php"=>body(json!({"zt":1,"dom":"https://cdn.example.invalid","url":"synthetic"})),
        "/cdn.example.invalid/file/synthetic"=>redirect(),
        _=>body(json!({"errno":-1,"code":-1,"message":"synthetic unsupported branch"})),
    }
}
