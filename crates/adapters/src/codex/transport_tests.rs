use super::*;

#[tokio::test]
async fn refuses_non_loopback_plain_websockets_before_connecting() {
    assert!(matches!(
        connect(Endpoint::Websocket {
            endpoint: "ws://192.0.2.1:1234".into()
        })
        .await,
        Err(Error::Endpoint)
    ));
}
