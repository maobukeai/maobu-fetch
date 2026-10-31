use base64::{engine::general_purpose::STANDARD, Engine};
use tokio::io::{AsyncRead, AsyncReadExt};

const HEADER_LIMIT: usize = 32 * 1024;

pub(super) struct Request {
    pub host: String,
    pub port: u16,
    pub connect: bool,
    pub forward_header: Vec<u8>,
}

pub(super) async fn read_header<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, String> {
    // Read exactly the header: do not consume and lose the first TLS/application bytes.
    let mut bytes = Vec::with_capacity(1024);
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= HEADER_LIMIT {
            return Err("媒体代理请求头过大".into());
        }
        let byte = reader.read_u8().await.map_err(|_| "媒体代理请求头不完整")?;
        bytes.push(byte);
    }
    Ok(bytes)
}

pub(super) fn parse_header(bytes: &[u8], secret: &str) -> Result<Request, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "媒体代理请求头编码无效")?;
    let mut lines = text.split("\r\n");
    let parts: Vec<_> = lines.next().unwrap_or("").split_whitespace().collect();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" && parts[2] != "HTTP/1.0" {
        return Err("媒体代理请求行无效".into());
    }
    let mut headers = Vec::new();
    let mut credentials = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("媒体代理请求头无效")?;
        if name.trim() != name
            || name.is_empty()
            || value
                .bytes()
                .any(|byte| byte < 32 && byte != b'\t' || byte == 127)
        {
            return Err("媒体代理请求头无效".into());
        }
        if name.eq_ignore_ascii_case("proxy-authorization") {
            credentials.push(value.trim());
        } else if !name.eq_ignore_ascii_case("proxy-connection")
            && !name.eq_ignore_ascii_case("connection")
            && !name.eq_ignore_ascii_case("host")
        {
            headers.push(line);
        }
    }
    let expected = format!("Basic {}", STANDARD.encode(format!("maobu:{secret}")));
    if credentials.len() != 1 || credentials[0] != expected {
        return Err("媒体代理认证失败".into());
    }
    let connect = parts[0] == "CONNECT";
    let target = if connect {
        format!("http://{}", parts[1])
    } else {
        parts[1].to_owned()
    };
    let url = url::Url::parse(&target).map_err(|_| "媒体代理目标地址无效")?;
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
        return Err("媒体代理仅接受 HTTP 或 CONNECT 请求".into());
    }
    let host = url
        .host_str()
        .ok_or("媒体代理缺少目标主机")?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = if connect {
        url.port().ok_or("CONNECT 缺少端口")?
    } else {
        url.port_or_known_default().ok_or("媒体代理缺少端口")?
    };
    if port == 0 {
        return Err("媒体代理目标端口无效".into());
    }
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let forward_header = if connect {
        Vec::new()
    } else {
        format!(
            "{} {} {}\r\nHost: {authority}\r\n{}\r\nConnection: close\r\n\r\n",
            parts[0],
            path,
            parts[2],
            headers.join("\r\n")
        )
        .into_bytes()
    };
    Ok(Request {
        host,
        port,
        connect,
        forward_header,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authentication_is_required_and_never_forwarded_to_origin() {
        let auth = STANDARD.encode("maobu:synthetic");
        let bytes = format!("GET http://example.test/a?q=1 HTTP/1.1\r\nHost: example.test\r\nProxy-Authorization: Basic {auth}\r\n\r\n");
        let request = parse_header(bytes.as_bytes(), "synthetic").unwrap();
        assert_eq!(request.host, "example.test");
        let forwarded = String::from_utf8(request.forward_header).unwrap();
        assert!(forwarded.starts_with("GET /a?q=1 HTTP/1.1"));
        assert!(!forwarded.to_lowercase().contains("proxy-authorization"));
        assert!(parse_header(bytes.as_bytes(), "wrong").is_err());
        assert!(parse_header(b"CONNECT example.test:443 HTTP/1.1\r\n\r\n", "synthetic").is_err());
    }
}
