//! Logs go to a daily file. Never log tokens; log event text only at `trace`.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Keep the returned guard alive for the life of the app, or buffered lines are lost.
pub fn init(log_dir: &Path) -> Option<WorkerGuard> {
    std::fs::create_dir_all(log_dir).ok()?;
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(log_dir, "takt.log"));
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_env("TAKT_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init()
        .ok()?;
    Some(guard)
}
