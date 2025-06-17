use std::path::Path;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};

#[allow(dead_code)]
pub struct LoggerGuard(WorkerGuard);

pub fn init_logging(log_dir: impl AsRef<Path>, prefix: &str) -> LoggerGuard {
    let builder = EnvFilter::builder()
        .with_default_directive("CiRCLE_sat_bot=info".parse().unwrap());

    let console_filter = builder.clone().parse_lossy(&std::env::var("RUST_LOG").unwrap_or_default());
    let file_filter = builder.parse_lossy(&std::env::var("RUST_LOG").unwrap_or_default());

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(prefix)
        .filename_suffix("log")  // Suffix for log files
        .build(log_dir)
        .expect("Failed to create file appender");
    let (non_blocking, guard) = NonBlocking::new(file_appender);

    let file_layer = fmt::layer()
        .with_writer(non_blocking.clone())
        .with_ansi(false)
        .with_filter(file_filter);
    let stdout_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_ansi(true)
        .with_filter(console_filter);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stdout_layer)
        .init();

    LoggerGuard(guard)
}