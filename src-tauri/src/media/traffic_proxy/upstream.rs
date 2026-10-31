use super::{request::read_header, socks};
use base64::{engine::general_purpose::STANDARD, Engine};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};

pub(super) trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
pub(super) type Connection = Box<dyn Stream>;

pub(super) async fn connect(
    host: &str,
    port: u16,
    proxy: Option<&url::Url>,
) -> Result<Connection, String> {
    let Some(proxy) = proxy else {
        return TcpStream::connect((host, port))
            .await
            .map(|stream| Box::new(stream) as Connection)
            .map_err(|_| "媒体目标连接失败，请检查网络或代理".into());
    };
    let proxy_host = proxy
        .host_str()
        .ok_or("媒体上游代理缺少主机")?
        .trim_matches(['[', ']']);
    let proxy_port =
        proxy
            .port_or_known_default()
            .unwrap_or(if proxy.scheme().starts_with("socks") {
                1080
            } else {
                8080
            });
    let stream = TcpStream::connect((proxy_host, proxy_port))
        .await
        .map_err(|_| "媒体上游代理连接失败")?;
    if matches!(proxy.scheme(), "socks5" | "socks5h") {
        return socks::connect(stream, host, port, proxy)
            .await
            .map(|stream| Box::new(stream) as Connection);
    }
    let mut stream: Connection = if proxy.scheme() == "https" {
        let connector = native_tls::TlsConnector::new().map_err(|_| "初始化媒体代理 TLS 失败")?;
        let tls = tokio_native_tls::TlsConnector::from(connector)
            .connect(proxy_host, stream)
            .await
            .map_err(|_| "媒体代理 TLS 验证失败，请检查代理证书或系统信任设置")?;
        Box::new(tls)
    } else {
        Box::new(stream)
    };
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let auth = if !proxy.username().is_empty() {
        let user = decode_component(proxy.username())?;
        let password = decode_component(proxy.password().unwrap_or(""))?;
        format!(
            "Proxy-Authorization: Basic {}\r\n",
            STANDARD.encode(format!("{user}:{password}"))
        )
    } else {
        String::new()
    };
    stream
        .write_all(
            format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n{auth}\r\n").as_bytes(),
        )
        .await
        .map_err(|_| "发送媒体代理隧道请求失败")?;
    let response = read_header(&mut stream).await?;
    let status = std::str::from_utf8(&response)
        .ok()
        .and_then(|line| line.lines().next())
        .and_then(|line| line.split_whitespace().nth(1));
    if status != Some("200") {
        return Err("媒体上游代理拒绝隧道，请检查代理权限与认证".into());
    }
    Ok(stream)
}

pub(super) fn decode_component(value: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(value.len());
    let source = value.as_bytes();
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let code = source
                .get(index + 1..index + 3)
                .and_then(|value| std::str::from_utf8(value).ok())
                .and_then(|text| u8::from_str_radix(text, 16).ok())
                .ok_or("媒体代理认证编码无效")?;
            bytes.push(code);
            index += 3;
        } else {
            bytes.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).map_err(|_| "媒体代理认证编码无效".into())
}
