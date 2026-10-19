use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::*;
use takt_sync::SyncState;

use crate::model::CalendarModel;

pub fn status_text(state: &SyncState, now: DateTime<Utc>, tz: Tz) -> String {
    match state {
        SyncState::Synced(at) => {
            let seconds = (now - *at).num_seconds().max(0);
            if seconds < 60 {
                format!("synced {seconds}s ago")
            } else if seconds < 3600 {
                format!("synced {}m ago", seconds / 60)
            } else {
                format!("synced at {}", at.with_timezone(&tz).format("%H:%M"))
            }
        }
        SyncState::Syncing => "syncing…".into(),
        SyncState::Offline(since) => {
            format!("offline since {}", since.with_timezone(&tz).format("%H:%M"))
        }
        SyncState::Error(message) => format!("error: {message}"),
        SyncState::NeedsReauth => "sign-in needed".into(),
    }
}

pub fn render(model: &CalendarModel, cx: &App) -> AnyElement {
    let now = Utc::now();
    h_flex()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .px_3()
        .py_1()
        .gap_4()
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .children(model.snapshot.accounts.iter().map(|a| {
            let state = model
                .statuses
                .get(&a.account_id)
                .map(|s| status_text(s, now, model.tz))
                .unwrap_or_else(|| "waiting".into());
            div()
                .min_w_0()
                .truncate()
                .child(format!("{} · {state}", a.email))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro, which shadows `#[test]`.
    use chrono::{TimeZone, Utc};
    use takt_sync::SyncState;

    use super::status_text;

    #[test]
    fn status_texts() {
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 14, 0, 30).unwrap();
        let tz = chrono_tz::UTC;
        assert_eq!(
            status_text(
                &SyncState::Synced(Utc.with_ymd_and_hms(2026, 10, 17, 14, 0, 18).unwrap()),
                now,
                tz
            ),
            "synced 12s ago"
        );
        assert_eq!(
            status_text(
                &SyncState::Synced(Utc.with_ymd_and_hms(2026, 10, 17, 13, 55, 0).unwrap()),
                now,
                tz
            ),
            "synced 5m ago"
        );
        assert_eq!(status_text(&SyncState::Syncing, now, tz), "syncing…");
        assert_eq!(
            status_text(
                &SyncState::Offline(Utc.with_ymd_and_hms(2026, 10, 17, 13, 2, 0).unwrap()),
                now,
                tz
            ),
            "offline since 13:02"
        );
        assert_eq!(
            status_text(&SyncState::NeedsReauth, now, tz),
            "sign-in needed"
        );
        assert_eq!(
            status_text(&SyncState::Error("HTTP 500".into()), now, tz),
            "error: HTTP 500"
        );
    }
}
