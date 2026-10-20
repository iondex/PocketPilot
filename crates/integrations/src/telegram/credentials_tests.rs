use super::*;

#[test]
fn accepts_only_a_direct_configured_token() {
    assert!(
        read(&Telegram {
            proxy: Default::default(),
            default_locale: "zh-CN".into(),
            token: "123:abcdefghijklmnopqrstuvwxyz".into()
        })
        .is_ok()
    );
    assert!(
        read(&Telegram {
            proxy: Default::default(),
            default_locale: "zh-CN".into(),
            token: "$TELEGRAM_BOT_TOKEN".into()
        })
        .is_err()
    );
    assert!(
        read(&Telegram {
            proxy: Default::default(),
            default_locale: "zh-CN".into(),
            token: "".into()
        })
        .is_err()
    );
}
