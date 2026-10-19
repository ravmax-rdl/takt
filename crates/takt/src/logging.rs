//! Logs go to a daily file. Never log tokens; log event text only at `trace`.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

/// Keep the returned guard alive for the life of the app, or buffered lines are lost.
pub fn init(log_dir: &Path) -> Option<WorkerGuard> {
    std::fs::create_dir_all(log_dir).ok()?;
    // `rolling::daily` panics when the file can't be opened; the builder returns an error.
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("takt.log")
        .max_log_files(14)
        .build(log_dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unopenable_log_file_returns_none_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        // Occupy every plausible name for today's log file with a directory. This fails
        // before any global subscriber is installed, so other tests are unaffected.
        let today = chrono::Utc::now().date_naive();
        for offset in -1..=1 {
            let day = today + chrono::Duration::days(offset);
            std::fs::create_dir(dir.path().join(format!("takt.log.{day}"))).unwrap();
        }
        assert!(init(dir.path()).is_none());
    }
}
