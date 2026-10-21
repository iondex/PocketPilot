use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;

async fn proxy(target: std::net::SocketAddr) -> (Proxy, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = Proxy::try_from(format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(client.read_u8().await.unwrap());
        }
        assert!(
            String::from_utf8(header)
                .unwrap()
                .starts_with("CONNECT unreachable.invalid:80 HTTP/1.1\r\n")
        );
        let mut remote = TcpStream::connect(target).await.unwrap();
        client
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
    });
    (proxy, task)
}

#[test]
fn config_is_explicit_and_redacts_invalid_urls() {
    for value in ["", "  "] {
        assert!(!Proxy::try_from(value.to_owned()).unwrap().enabled());
    }
    let proxy = Proxy::try_from("http://192.0.2.10:7890".to_owned()).unwrap();
    assert!(!format!("{proxy:?}").contains("192.0.2.10"));
    for value in [
        "192.0.2.10:7890",
        "socks5://192.0.2.10:7890",
        "http://user:secret@example.com",
        "http://example.com/path",
        "http://example.com/?token=secret",
    ] {
        let error = Proxy::try_from(value.to_owned()).unwrap_err();
        assert!(!error.contains("secret"));
    }
}

#[tokio::test]
#[allow(clippy::result_large_err)] // tungstenite's callback signature uses an HTTP response.
async fn websocket_relay_uses_connect_proxy_and_preserves_sdk_query() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (proxy, proxy_task) = proxy(upstream.local_addr().unwrap()).await;
        let upstream_task = tokio::spawn(async move {
            let (stream, _) = upstream.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |req: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(req.uri().query(), Some("v=10&encoding=json"));
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let message = socket.next().await.unwrap().unwrap();
            assert_eq!(message.to_text().unwrap(), "fixture");
            socket.send(message).await.unwrap();
        });
        let relay = Relay::start(proxy, "ws://unreachable.invalid:80/".into())
            .await
            .unwrap();
        let endpoint = format!("{}?v=10&encoding=json", relay.endpoint);
        let (mut socket, _) = tokio_tungstenite::connect_async(endpoint).await.unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                "fixture".into(),
            ))
            .await
            .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap().to_text().unwrap(),
            "fixture"
        );
        drop(socket);
        drop(relay);
        upstream_task.await.unwrap();
        proxy_task.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn http_proxy_routes_an_unresolvable_destination() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = Proxy::try_from(format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
        }
        assert!(
            String::from_utf8(header)
                .unwrap()
                .starts_with("GET http://unreachable.invalid/probe HTTP/1.1")
        );
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let response = proxy
        .http_builder()
        .unwrap()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .get("http://unreachable.invalid/probe")
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
    server.await.unwrap();
}

#[tokio::test]
async fn direct_http_ignores_proxy_environment() {
    if std::env::var_os("POCKETPILOT_DIRECT_FIXTURE").is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::direct_http_ignores_proxy_environment"])
            .env("POCKETPILOT_DIRECT_FIXTURE", "1")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("http_proxy", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("all_proxy", "http://127.0.0.1:1")
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    assert!(
        Proxy::default()
            .http_builder()
            .unwrap()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(endpoint)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn connect_rejection_never_falls_back_to_direct() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = Proxy::try_from(format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
        }
        stream
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
            .await
            .unwrap();
    });
    assert!(proxy.tcp("unreachable.invalid", 443).await.is_err());
    task.await.unwrap();
}

#[tokio::test]
async fn attachment_download_is_bounded_and_never_forwards_credentials_on_redirects() {
    for (response, accepted) in [
        ("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndata", true),
        ("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nlarge", false),
        (
            "HTTP/1.1 302 Found\r\nLocation: http://unreachable.invalid/private\r\nContent-Length: 0\r\n\r\n",
            false,
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
            }
            assert!(
                String::from_utf8_lossy(&header)
                    .to_ascii_lowercase()
                    .contains("authorization: bearer fixture-token")
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let result =
            super::download::bytes(&Proxy::default(), &url, Some("fixture-token"), 4).await;
        assert_eq!(result.is_ok(), accepted);
        if accepted {
            assert_eq!(result.unwrap(), b"data");
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn websocket_relay_rejects_clients_without_its_random_access_path() {
    let relay = Relay::start(Proxy::default(), "ws://127.0.0.1:9/".into())
        .await
        .unwrap();
    let mut address = url::Url::parse(&relay.endpoint).unwrap();
    address.set_path("/");
    let result = tokio_tungstenite::connect_async(address.as_str()).await;
    assert!(
        matches!(result,Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status()==403)
    );
}
