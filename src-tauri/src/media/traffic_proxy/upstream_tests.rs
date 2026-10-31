use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};

#[tokio::test]
async fn upstream_http_connect_auth_and_opaque_bytes_are_preserved() {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let header = String::from_utf8(request::read_header(&mut stream).await.unwrap()).unwrap();
        assert!(header.starts_with("CONNECT media.example.test:443 HTTP/1.1\r\n"));
        assert!(header.contains(&format!(
            "Proxy-Authorization: Basic {}",
            STANDARD.encode("user+name:synthetic+password")
        )));
        stream
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        let mut payload = [0; 5];
        stream.read_exact(&mut payload).await.unwrap();
        assert_eq!(payload, [0, 1, 2, 254, 255]);
        stream.write_all(&payload).await.unwrap();
    });
    let proxy =
        url::Url::parse(&format!("http://user+name:synthetic%2Bpassword@{address}")).unwrap();
    let mut stream = upstream::connect("media.example.test", 443, Some(&proxy))
        .await
        .unwrap();
    stream.write_all(&[0, 1, 2, 254, 255]).await.unwrap();
    let mut payload = [0; 5];
    stream.read_exact(&mut payload).await.unwrap();
    assert_eq!(payload, [0, 1, 2, 254, 255]);
    server.await.unwrap();
}

#[tokio::test]
async fn socks5_remote_dns_local_dns_and_auth_failure_follow_selected_policy() {
    for (scheme, host, expected_type, reject) in [
        ("socks5h", "unresolved.example.test", 3, false),
        ("socks5", "localhost", 0, false),
        ("socks5h", "example.test", 3, true),
    ] {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let host = host.to_owned();
        let expected_host = host.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 4];
            stream.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 2, 0, 2]);
            stream.write_all(&[5, 2]).await.unwrap();
            assert_eq!(stream.read_u8().await.unwrap(), 1);
            let length = stream.read_u8().await.unwrap() as usize;
            let mut user = vec![0; length];
            stream.read_exact(&mut user).await.unwrap();
            assert_eq!(user, b"user+");
            let length = stream.read_u8().await.unwrap() as usize;
            let mut password = vec![0; length];
            stream.read_exact(&mut password).await.unwrap();
            assert_eq!(password, b"synthetic+password");
            stream
                .write_all(&[1, if reject { 1 } else { 0 }])
                .await
                .unwrap();
            if reject {
                return;
            }
            let mut header = [0; 4];
            stream.read_exact(&mut header).await.unwrap();
            assert_eq!(&header[..3], &[5, 1, 0]);
            if expected_type == 3 {
                assert_eq!(header[3], 3);
                let length = stream.read_u8().await.unwrap() as usize;
                let mut domain = vec![0; length];
                stream.read_exact(&mut domain).await.unwrap();
                assert_eq!(domain, expected_host.as_bytes());
            } else {
                assert!(matches!(header[3], 1 | 4));
                let mut address = vec![0; if header[3] == 1 { 4 } else { 16 }];
                stream.read_exact(&mut address).await.unwrap();
            }
            assert_eq!(stream.read_u16().await.unwrap(), 443);
            stream
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .await
                .unwrap();
            let mut payload = [0; 3];
            stream.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"tls");
            stream.write_all(&payload).await.unwrap();
        });
        let proxy =
            url::Url::parse(&format!("{scheme}://user+:synthetic%2Bpassword@{address}")).unwrap();
        let result = upstream::connect(&host, 443, Some(&proxy)).await;
        if reject {
            assert!(result.is_err());
        } else {
            let mut stream = result.unwrap();
            stream.write_all(b"tls").await.unwrap();
            let mut payload = [0; 3];
            stream.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"tls");
        }
        server.await.unwrap();
    }
}
