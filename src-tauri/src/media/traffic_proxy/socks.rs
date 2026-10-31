use super::upstream::decode_component;
use std::net::IpAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub(super) async fn connect(
    mut stream: TcpStream,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<TcpStream, String> {
    let authenticated = !proxy.username().is_empty();
    stream
        .write_all(if authenticated {
            &[5, 2, 0, 2]
        } else {
            &[5, 1, 0]
        })
        .await
        .map_err(|_| "SOCKS5 握手发送失败")?;
    let mut response = [0; 2];
    stream
        .read_exact(&mut response)
        .await
        .map_err(|_| "SOCKS5 握手响应不完整")?;
    if response[0] != 5 {
        return Err("SOCKS5 代理协议不匹配".into());
    }
    match response[1] {
        0 => {}
        2 if authenticated => {
            let user = decode_component(proxy.username())?;
            let password = decode_component(proxy.password().unwrap_or(""))?;
            if user.len() > 255 || password.len() > 255 {
                return Err("SOCKS5 认证字段过长".into());
            }
            let mut auth = vec![1, user.len() as u8];
            auth.extend_from_slice(user.as_bytes());
            auth.push(password.len() as u8);
            auth.extend_from_slice(password.as_bytes());
            stream
                .write_all(&auth)
                .await
                .map_err(|_| "SOCKS5 认证发送失败")?;
            stream
                .read_exact(&mut response)
                .await
                .map_err(|_| "SOCKS5 认证响应不完整")?;
            if response != [1, 0] {
                return Err("SOCKS5 代理认证失败".into());
            }
        }
        _ => return Err("SOCKS5 代理没有接受当前认证方式".into()),
    }
    let address = if proxy.scheme() == "socks5" {
        Some(
            tokio::net::lookup_host((host, port))
                .await
                .map_err(|_| "媒体目标 DNS 解析失败")?
                .next()
                .ok_or("媒体目标 DNS 没有返回地址")?
                .ip(),
        )
    } else {
        host.parse::<IpAddr>().ok()
    };
    let mut request = vec![5, 1, 0];
    match address {
        Some(IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Some(IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        None => {
            if host.len() > 255 {
                return Err("SOCKS5 目标主机名过长".into());
            }
            request.extend_from_slice(&[3, host.len() as u8]);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream
        .write_all(&request)
        .await
        .map_err(|_| "SOCKS5 目标请求发送失败")?;
    let mut header = [0; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|_| "SOCKS5 目标响应不完整")?;
    if header[0] != 5 || header[1] != 0 || header[2] != 0 {
        return Err("SOCKS5 代理无法连接媒体目标".into());
    }
    let address_length = match header[3] {
        1 => 4,
        4 => 16,
        3 => stream
            .read_u8()
            .await
            .map_err(|_| "SOCKS5 响应地址不完整")? as usize,
        _ => return Err("SOCKS5 响应地址类型无效".into()),
    };
    let mut remainder = vec![0; address_length + 2];
    stream
        .read_exact(&mut remainder)
        .await
        .map_err(|_| "SOCKS5 响应地址不完整")?;
    Ok(stream)
}
