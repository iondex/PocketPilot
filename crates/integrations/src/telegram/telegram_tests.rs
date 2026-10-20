use super::*;

#[test]
fn pairing_never_authorizes_a_group_or_another_private_peer() {
    assert_eq!(access(10, -100, false, true), Access::Denied);
    assert_eq!(access(10, 11, true, true), Access::Denied);
    assert_eq!(access(10, 10, true, false), Access::PairingRequired);
    assert_eq!(access(10, 10, true, true), Access::Authorized);
}

#[test]
fn diagnostic_reasons_only_expose_fixed_categories() {
    for (message, expected) in [
        (
            "Bad Request: CUSTOM_EMOJI_ID_INVALID",
            "invalid_custom_emoji",
        ),
        (
            "Bad Request: not enough rights to manage topics",
            "topic_permission_denied",
        ),
        ("Internal Server Error", "server_unavailable"),
        (
            "private-response-body-with-sensitive-content",
            "api_rejected",
        ),
    ] {
        let error = RequestError::Api(ApiError::Unknown(message.into()));
        assert_eq!(error_reason(&error), expected);
        assert!(!safe_error(error).to_string().contains(message));
    }
}
