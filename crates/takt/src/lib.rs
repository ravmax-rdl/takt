//! takt: a desktop client for Google Calendar and Google Tasks.

pub mod agenda;
pub mod config;
pub mod detail;
pub mod keys;
pub mod logging;
pub mod model;
pub mod ui;

use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext as _, WindowOptions};

/// The OS time zone, used for every displayed time. Falls back to UTC.
pub fn display_zone() -> chrono_tz::Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::UTC)
}

/// Starts takt: logging, config, the database, then the window.
pub fn run() {
    let paths = config::Paths::discover().expect("cannot find the user's app-data directories");
    let _log_guard = logging::init(&paths.log_dir);
    let config = config::load(&paths.config_file);
    // Create and migrate the database before any read-only connection opens it.
    if let Err(error) = takt_store::Store::open(&paths.db_file) {
        tracing::error!(%error, "cannot open the database");
        eprintln!(
            "takt cannot open its database at {}: {error}",
            paths.db_file.display()
        );
        std::process::exit(1);
    }
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        keys::bind(cx);
        let options = WindowOptions {
            titlebar: Some(TitleBar::title_bar_options()),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ui::AppView::new(paths.clone(), config.clone(), window, cx))
        })
        .expect("cannot open the main window");
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
