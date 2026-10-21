use app::config::Logging;
use tracing_subscriber::{
    EnvFilter,
    fmt::writer::{BoxMakeWriter, MakeWriterExt},
    prelude::*,
};

pub fn init(config: &Logging) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>, ()> {
    let filter = EnvFilter::try_new(&config.filter)
        .map_err(|_| ())?
        .add_directive("reqwest=warn".parse().map_err(|_| ())?)
        .add_directive("hyper=warn".parse().map_err(|_| ())?)
        .add_directive("teloxide=warn".parse().map_err(|_| ())?);
    let (writer, guard): (BoxMakeWriter, _) = if let Some(directory) = &config.directory {
        std::fs::create_dir_all(directory).map_err(|_| ())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| ())?;
        }
        let file = tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(if config.rotation == "hourly" {
                tracing_appender::rolling::Rotation::HOURLY
            } else {
                tracing_appender::rolling::Rotation::DAILY
            })
            .max_log_files(config.retained_files)
            .filename_prefix("pocketpilot.log")
            .build(directory)
            .map_err(|_| ())?;
        let (writer, guard) = tracing_appender::non_blocking(file);
        let writer = if config.console {
            BoxMakeWriter::new(writer.and(std::io::stderr))
        } else {
            BoxMakeWriter::new(writer)
        };
        (writer, Some(guard))
    } else {
        (BoxMakeWriter::new(std::io::stderr), None)
    };
    // Transport/SQL trace events can contain credential URLs or raw message bodies.
    // This independent filter also applies to more specific user log directives.
    let redacted = tracing_subscriber::filter::filter_fn(|metadata| {
        ![
            "reqwest",
            "hyper",
            "h2",
            "teloxide_core",
            "tungstenite",
            "tokio_tungstenite",
            "rustls",
            "sqlx",
            "ssh2_config",
            "serenity",
            "slack_morphism",
            "openlark_client",
            "openlark_core",
            "openlark_auth",
            "lark_websocket_protobuf",
        ]
        .iter()
        .any(|target| metadata.target().starts_with(target))
    });
    if config.format == "json" {
        tracing_subscriber::registry()
            .with(filter)
            .with(redacted)
            .with(tracing_subscriber::fmt::layer().with_writer(writer).json())
            .try_init()
            .map_err(|_| ())?;
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(redacted)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(writer)
                    .with_ansi(false),
            )
            .try_init()
            .map_err(|_| ())?;
    }
    Ok(guard)
}
