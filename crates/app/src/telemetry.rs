use app::config::Logging;
use tracing_subscriber::{EnvFilter, fmt::writer::BoxMakeWriter, prelude::*};

pub fn init(config: &Logging) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>, ()> {
    let filter = EnvFilter::try_new(&config.filter)
        .map_err(|_| ())?
        .add_directive("reqwest=warn".parse().map_err(|_| ())?)
        .add_directive("hyper=warn".parse().map_err(|_| ())?)
        .add_directive("teloxide=warn".parse().map_err(|_| ())?);
    let (writer, guard): (BoxMakeWriter, _) = if let Some(directory) = &config.directory {
        std::fs::create_dir_all(directory).map_err(|_| ())?;
        let file = tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("pocketpilot.log")
            .build(directory)
            .map_err(|_| ())?;
        let (writer, guard) = tracing_appender::non_blocking(file);
        (BoxMakeWriter::new(writer), Some(guard))
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
