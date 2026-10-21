use super::*;

#[test]
fn installer_rejects_truncated_wrong_architecture_and_dynamic_artifacts() {
    assert!(artifact_arch(b"not ELF").is_err());
    let mut bytes = vec![0u8; 120];
    bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
    bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(artifact_arch(&bytes).unwrap(), "x86_64");
    bytes[64..68].copy_from_slice(&3u32.to_le_bytes());
    assert!(artifact_arch(&bytes).is_err());
    bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
    bytes[18..20].copy_from_slice(&0u16.to_le_bytes());
    assert!(artifact_arch(&bytes).is_err());
    bytes[18..20].copy_from_slice(&183u16.to_le_bytes());
    assert_eq!(artifact_arch(&bytes).unwrap(), "aarch64");
    bytes[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(artifact_arch(&bytes).is_err());
}

#[tokio::test]
async fn uploaded_artifact_delivers_eof_before_waiting_for_the_remote_reply() {
    let mut child = tokio::process::Command::new("sh")
        .args(["-c", "cat > /dev/null; printf complete"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        send_artifact(input, b"fixture artifact").await.unwrap();
        let mut reply = Vec::new();
        output.take(64).read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, b"complete");
        assert!(child.wait().await.unwrap().success());
    })
    .await
    .unwrap();
}
