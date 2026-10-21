use super::*;
use crate::bindings::{Manager, State, Target};

#[tokio::test]
async fn binding_controls_pause_polling_keep_input_enabled_and_confirm_detach() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(6000, 0, "/open workspace thread"))
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(6001, 0, "/bindings"))
        .await
        .unwrap();
    fixture.choose(6002, "1").await;
    let detail = fixture.outputs().await.pop().unwrap();
    assert!(menu_labels(&detail).contains("暂停自动观察"));
    fixture.choose(6003, "1").await;
    let snapshot = fixture.service.config.snapshot();
    let manager = Manager::new(&fixture.service.store, fixture.service.binding_scope(10));
    let target = Target::Telegram { topic: 11 };
    let entry = manager
        .entry(&snapshot.config, &target, "en")
        .await
        .unwrap();
    assert_eq!(entry.state, State::Paused);
    let binding = fixture
        .service
        .store
        .binding(1, 10, 11)
        .await
        .unwrap()
        .unwrap();
    let before = fixture.state.lock().await.requests.clone();
    fixture.service.refresh_binding(&binding).await.unwrap();
    assert_eq!(
        fixture.state.lock().await.requests,
        before,
        "paused polling performs no runtime requests"
    );
    fixture
        .service
        .handle(&input(6004, 11, "continue with the fixture task"))
        .await
        .unwrap();
    assert!(fixture.state.lock().await.requests.contains(&"turn/steer"));
    fixture.choose(6005, "2").await;
    assert!(fixture.outputs().await.pop().unwrap().text.contains("解除"));
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .is_some()
    );
    fixture.choose(6006, "1").await;
    assert_eq!(
        manager
            .entry(&snapshot.config, &target, "en")
            .await
            .unwrap()
            .state,
        State::Detached
    );
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .is_none()
    );
}
