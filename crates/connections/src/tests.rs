use super::*;

fn remote() -> Host {
    Host::SSH(ssh::Options {
        alias: "fixture".into(),
        config: None,
        connect_timeout: Duration::from_secs(1),
        keepalive_seconds: 5,
        keepalive_missed: 2,
        askpass: None,
        max_auth_attempts: 1,
        authentication_methods: Vec::new(),
    })
}

#[tokio::test]
async fn local_and_remote_commands_preserve_literal_arguments_and_multiline_scripts() {
    let input = "spaces ' quotes $(printf injected);\nsecond line";
    let program = Program {
        binary: "sh".into(),
        arguments: vec![
            "-c".into(),
            "printf '%s' \"$1\"\n".into(),
            "fixture".into(),
            input.into(),
        ],
    };
    let local = Host::Local
        .command(&program)
        .unwrap()
        .output()
        .await
        .unwrap();

    // Execute only the generated remote command in a local shell: no SSH account is contacted.
    let command = remote().command(&program).unwrap();
    let script = command.as_std().get_args().last().unwrap();
    let remote = Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .await
        .unwrap();
    assert!(local.status.success() && remote.status.success());
    assert_eq!(local.stdout, input.as_bytes());
    assert_eq!(remote.stdout, local.stdout);
}

#[test]
fn invalid_programs_are_rejected_before_a_process_is_started() {
    for host in [Host::Local, remote()] {
        for program in [
            Program {
                binary: "".into(),
                arguments: Vec::new(),
            },
            Program {
                binary: "sh\n".into(),
                arguments: Vec::new(),
            },
            Program {
                binary: "sh".into(),
                arguments: vec!["a\0b".into()],
            },
        ] {
            assert!(matches!(host.command(&program), Err(Error::Endpoint)));
        }
    }
}
