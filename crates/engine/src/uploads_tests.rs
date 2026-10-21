use super::*;

#[test]
fn files_are_scoped_to_the_selected_execution_account_and_source() {
    let provider = Provider {
        id: "fixture".into(),
        host_id: "host".into(),
        kind: "codex".into(),
        connection_mode: "existing_proxy".into(),
        compatibility_profile: "fixture".into(),
        control_verified: true,
        endpoint: Some(adapters::ServerEndpoint {
            binary: "/usr/bin/codex".into(),
            socket: "/private/account/.codex/app-server-control/app-server-control.sock".into(),
        }),
    };
    let (mutations, files) = prepare(
        &provider,
        "../source",
        "thread",
        vec![Downloaded {
            name: "../../report.txt".into(),
            bytes: b"content".to_vec(),
            image: false,
        }],
    )
    .unwrap();
    assert!(
        files[0]
            .path
            .starts_with("/private/account/.codex/pocketpilot/uploads/")
    );
    assert!(files[0].path.ends_with("/0/report.txt"));
    assert!(!files[0].path.contains(".."));
    assert!(matches!(&mutations[0], Mutation::Upload { bytes, .. } if bytes == b"content"));
    let mut changed = provider.clone();
    changed.endpoint = None;
    assert!(
        prepare(
            &changed,
            "source",
            "thread",
            vec![Downloaded {
                name: "file".into(),
                bytes: vec![],
                image: false
            }]
        )
        .is_err()
    );
}

#[test]
fn independent_runtime_files_stay_under_its_private_socket_directory() {
    let provider = Provider {
        id: "fixture".into(),
        host_id: "host".into(),
        kind: "claude_code".into(),
        connection_mode: "existing_proxy".into(),
        compatibility_profile: "pocketpilot-runtime-v2".into(),
        control_verified: false,
        endpoint: Some(adapters::ServerEndpoint {
            binary: "/home/operator/.pocketpilot/bin/agent-server-fixture".into(),
            socket: "/home/operator/.pocketpilot/claude-code/server.sock".into(),
        }),
    };
    let (_, files) = prepare(
        &provider,
        "source",
        "session",
        vec![Downloaded {
            name: "input.txt".into(),
            bytes: b"fixture".to_vec(),
            image: false,
        }],
    )
    .unwrap();
    assert!(
        files[0]
            .path
            .starts_with("/home/operator/.pocketpilot/claude-code/uploads/")
    );
}
