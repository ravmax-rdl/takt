//! takt: a desktop client for Google Calendar and Google Tasks.

pub mod agenda;

/// The OS time zone, used for every displayed time. Falls back to UTC.
pub fn display_zone() -> chrono_tz::Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::UTC)
}
