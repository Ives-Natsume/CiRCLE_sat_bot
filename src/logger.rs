use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tokio::task;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};

use crate::i18n;

#[allow(dead_code)]
pub struct LoggerGuard(WorkerGuard);

pub fn init_logging(log_dir: impl AsRef<Path>, prefix: &str) -> LoggerGuard {
    let log_dir = log_dir.as_ref().to_path_buf();
    let builder = EnvFilter::builder()
        .with_default_directive("CiRCLE_sat_bot_server=info".parse().unwrap());

    let console_filter = builder.clone().parse_lossy(&std::env::var("RUST_LOG").unwrap_or_default());
    let file_filter = builder.parse_lossy(&std::env::var("RUST_LOG").unwrap_or_default());

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(prefix)
        .filename_suffix("log")  // Suffix for log files
        .build(&log_dir)
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

    start_log_cleanup_task(log_dir, prefix.to_string());

    LoggerGuard(guard)
}

fn start_log_cleanup_task(log_dir: PathBuf, prefix: String) {
    const MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24 * 3);
    const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

    task::spawn(async move {
        loop {
            if let Err(e) = cleanup_old_logs(&log_dir, &prefix, MAX_AGE) {
                tracing::warn!("{}: {}", i18n::text("log_file_deleted_error"), e);
            }
            tokio::time::sleep(CLEANUP_INTERVAL).await;
        }
    });
}

fn cleanup_old_logs(log_dir: &Path, prefix: &str, max_age: Duration) -> std::io::Result<()> {
    let now = SystemTime::now();

    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();

        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if file_name.starts_with(prefix) && file_name.ends_with(".log") {
                let metadata = fs::metadata(&path)?;
                if let Ok(modified) = metadata.modified() {
                    if now.duration_since(modified).unwrap_or_default() > max_age {
                        fs::remove_file(&path)?;
                        tracing::info!("Deleted old log file: {}", file_name);
                    }
                }
            }
        }
    }
    Ok(())
}
