use super::*;

fn fixture(kind: i32, batch_size: i32) -> String {
    let payload = Payload {
        accounts: vec![Parameters {
            secret: b"12345678901234567890".to_vec(),
            name: "fixture@example.invalid".into(),
            issuer: "fixture".into(),
            algorithm: 1,
            digits: 1,
            kind,
        }],
        version: 2,
        batch_size,
        batch_index: 0,
    };
    let mut url = url::Url::parse("otpauth-migration://offline").unwrap();
    url.query_pairs_mut()
        .append_pair("data", &STANDARD.encode(payload.encode_to_vec()));
    url.to_string()
}

#[test]
fn imports_typed_totp_and_redacts_debug() {
    let accounts = decode(&fixture(2, 1)).unwrap();
    assert_eq!(accounts.len(), 1);
    let a = &accounts[0];
    assert_eq!(a.totp.algorithm, "SHA1");
    assert_eq!(a.totp.digits, 6);
    assert_eq!(a.totp.period_seconds, 30);
    assert_eq!(
        a.totp.secret_base32,
        totp_rs::Secret::from(b"12345678901234567890".as_slice()).to_base32()
    );
    assert!(!format!("{a:?}").contains(&a.totp.secret_base32));
}

#[test]
fn rejects_hotp_incomplete_exports_and_malformed_uris() {
    for uri in [
        fixture(1, 1),
        fixture(2, 2),
        "otpauth-migration://offline?data=invalid".into(),
        "https://offline?data=invalid".into(),
        format!("{}&data=duplicate", fixture(2, 1)),
    ] {
        let e = decode(&uri).unwrap_err();
        assert!(!e.to_string().contains(&uri));
    }
}
