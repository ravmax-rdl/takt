# takt Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build takt's read-only foundation: multi-account Google sign-in, Calendar and Tasks clients, a SQLite cache with local recurrence expansion, a polling sync engine, and a GPUI agenda window.

**Architecture:** A Cargo workspace of five crates with dependencies pointing downward. `takt-recur` expands recurring series with no I/O. `takt-google` speaks OAuth and REST. `takt-store` keeps Google's raw JSON in SQLite plus index columns. `takt-sync` runs one tokio task per account on its own thread. `takt` is the gpui-kit binary that only reads the store and sends commands to sync.

**Tech Stack:** Rust 2024 (rustc 1.99), gpui-kit 0.7, reqwest 0.13 (rustls), rusqlite 0.40 (bundled) + rusqlite_migration 2, rrule 0.14, chrono + chrono-tz, keyring 4 (`v1`), tokio, async-channel, wiremock 0.6, proptest.

**Spec:** `docs/superpowers/specs/2026-10-17-takt-foundation-design.md`

## Global Constraints

- Edition 2024 for every crate; workspace resolver 3.
- Crates and paths: `crates/takt-recur`, `crates/takt-google`, `crates/takt-store`, `crates/takt-sync`, `crates/takt`.
- Dependency direction: `takt-recur` depends on no takt crate; `takt-google` depends on no takt crate; `takt-store` depends on `takt-recur` and `takt-google` (model types only); `takt-sync` on `takt-google` and `takt-store`; `takt` on all four. Only `takt` depends on `gpui-kit`.
- Sub-project 1 never writes to Google. Only GET requests to Calendar and Tasks, and POST to the OAuth token endpoint.
- Every stored Google resource keeps its JSON exactly as received in `raw_json`. Extracted columns must be derivable from `raw_json`.
- Instance IDs: `<masterId>_<YYYYMMDDTHHMMSSZ>` (UTC) for timed series, `<masterId>_<YYYYMMDD>` for all-day.
- OAuth scopes, requested together: `https://www.googleapis.com/auth/calendar`, `https://www.googleapis.com/auth/tasks`, `openid`, `email`.
- Refresh tokens live in the OS keychain: service `takt`, user `<account-email>`. Access tokens live in memory only and refresh 60 s before expiry.
- Retries: 429, 5xx, and 403 with reason `rateLimitExceeded` or `userRateLimitExceeded` back off exponentially with jitter, honour `Retry-After`, at most 5 attempts.
- Polling: 60 s focused, 5 min unfocused, immediately on focus gain and `SyncNow`. Settings and colours on startup then hourly. Offline backoff capped at 5 min. At most 4 calendars sync in parallel per account.
- Expansion stops at 10,000 occurrences per series per call.
- Never log tokens. Log event titles and descriptions only at `trace`.
- Commit messages: `type(scope): message`, scope = crate name or area (`workspace`, `spec`, `plan`, `docs`).
- Each task ends with its crate's tests passing. The final task runs `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`.
- If a pinned crate version fails to resolve, use the newest release with the same major version (`cargo search <name>`) and note it in the commit body.

## Review Focus

1. A calendar ID containing `#` and `@` (holiday and shared calendars) must reach Google percent-encoded, or every request for that calendar 404s. Test: Task 7, `events_page_percent_encodes_calendar_id`.
2. A cancelled series that arrives in an incremental page as only `{id, status}` must delete the series and all its exceptions. Test: Task 9, `minimal_cancelled_master_removes_series_and_exceptions`.
3. A daily series that started twenty years ago must still expand for today's range. Test: Task 1, `twenty_year_old_daily_series_still_expands_today`.
4. An all-day event on a date whose local midnight does not exist (America/Santiago, 2026-09-06) must still land on that day. Test: Task 10, `all_day_event_on_skipped_midnight_lands_on_its_day`.
5. A timed event that crosses midnight must appear under both days in the agenda. Test: Task 15, `overnight_event_appears_on_both_days`.

## File Structure

```
Cargo.toml                                  workspace manifest, shared dependency versions
README.md                                   Google Cloud setup, running, capturing fixtures
crates/takt-recur/
  src/lib.rs                                When, Range, Master, Exception, Occurrence, expand, series_end, instance_id
  src/normalize.rs                          Google recurrence lines -> text rrule parses
  tests/common/mod.rs                       time helpers for tests
  tests/expand_timed.rs                     timed series
  tests/expand_all_day.rs                   all-day series, UNTIL/EXDATE forms
  tests/exceptions.rs                       exceptions, caps, errors, series_end
  tests/properties.rs                       proptest invariants
  tests/golden.rs                           local expansion == Google's events.instances
  tests/fixtures/*.json                     captured series (Task 14)
crates/takt-google/
  src/lib.rs                                module wiring, Endpoints
  src/error.rs                              GoogleError
  src/model.rs                              Item<T>, EventFields, CalendarListEntry, TaskFields, Colors, Setting, Page
  src/auth.rs                               PKCE, sign-in loopback, code exchange, Auth (refresh + cache)
  src/token_store.rs                        TokenStore, KeyringTokenStore, MemoryTokenStore
  src/client.rs                             GoogleClient core: bearer auth, retries, error classification
  src/api.rs                                endpoint methods
  examples/capture.rs                       fixture capture tool (Task 14)
  tests/{model,auth,client,api}.rs
crates/takt-store/
  migrations/001_init.sql                   schema
  src/lib.rs                                Store open/migrate, module wiring
  src/error.rs                              StoreError
  src/types.rs                              rows, ChangeSet, TimeSpan, Occurrence, TaskItem, day_start_utc
  src/convert.rs                            EventFields -> index columns, Master/Exception builders
  src/accounts.rs                           accounts, settings, colours, calendars
  src/events.rs                             apply_events_page, resets
  src/query.rs                              occurrences
  src/tasks.rs                              task lists, tasks, tasks_due
  tests/{accounts,events,query,tasks}.rs
crates/takt-sync/
  src/lib.rs                                module wiring
  src/messages.rs                           Command, SyncEvent, SyncState
  src/engine.rs                             AccountEngine: one sync round and its recovery rules
  src/runtime.rs                            start(), supervisor, account loops, panic restart
  tests/common/mod.rs                       fake Google helpers
  tests/{engine,runtime}.rs
crates/takt/
  src/lib.rs                                module wiring, run(), display_zone()
  src/main.rs                               calls takt::run()
  src/config.rs                             Paths, config.toml loading
  src/logging.rs                            tracing to rolling file
  src/agenda.rs                             pure agenda builder
  src/detail.rs                             pure detail builder, rule describer
  src/model.rs                              CalendarModel entity
  src/keys.rs                               actions and key bindings
  src/ui/{mod,app_view,main_view,agenda_list,sidebar,status_bar,detail_panel,first_run}.rs
  tests/smoke.rs                            gpui test-support render
```

---

### Task 1: Workspace and recurrence expansion

**Files:**
- Create: `Cargo.toml` (replace), `crates/takt-recur/Cargo.toml`, `crates/takt-recur/src/lib.rs`, `crates/takt-recur/src/normalize.rs`, `crates/takt-recur/tests/common/mod.rs`, `crates/takt-recur/tests/expand_timed.rs`, `crates/takt-recur/tests/expand_all_day.rs`
- Create: `crates/takt-google/Cargo.toml`, `crates/takt-google/src/lib.rs`, `crates/takt-store/Cargo.toml`, `crates/takt-store/src/lib.rs`, `crates/takt-sync/Cargo.toml`, `crates/takt-sync/src/lib.rs`, `crates/takt/Cargo.toml`
- Move: `src/main.rs` → `crates/takt/src/main.rs`

**Interfaces:**
- Produces (`takt_recur`): `enum When { At(DateTime<Utc>), Date(NaiveDate) }` with `as_utc()`, `is_all_day()`; `struct Range { start, end }` (half-open) with `overlaps(&When, &When) -> bool`; `struct Master { id: String, start: When, end: When, tz: chrono_tz::Tz, recurrence: Vec<String> }`; `struct Exception { id: String, original_start: When, cancelled: bool, start: When, end: When }`; `enum Source { Generated, Exception }`; `struct Occurrence { instance_id, source, original_start, start, end }`; `enum RecurError { Parse(String), MixedStartEnd }`; `const MAX_OCCURRENCES: u16 = 10_000`; `fn instance_id(&str, &When) -> String`; `fn expand(&Master, &[Exception], &Range) -> Result<Vec<Occurrence>, RecurError>`.

- [ ] **Step 1: Replace the root manifest with a workspace**

`Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
publish = false

[workspace.dependencies]
takt-google = { path = "crates/takt-google" }
takt-recur = { path = "crates/takt-recur" }
takt-store = { path = "crates/takt-store" }
takt-sync = { path = "crates/takt-sync" }

async-channel = "2"
base64 = "0.22"
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.10"
directories = "6"
futures = "0.3"
gpui-kit = "0.7"
iana-time-zone = "0.1"
keyring = { version = "4", features = ["v1"] }
open = "5"
proptest = "1"
rand = "0.9"
reqwest = { version = "0.13", default-features = false, features = ["json", "form", "query", "rustls"] }
rrule = "0.14"
rusqlite = { version = "0.40", features = ["bundled"] }
rusqlite_migration = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tempfile = "3"
thiserror = "2"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "time", "sync", "io-util"] }
toml = "0.9"
tracing = "0.1"
tracing-appender = "0.2"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
wiremock = "0.6"
```

- [ ] **Step 2: Create the crate skeletons**

For each of `takt-google`, `takt-store`, `takt-sync`, create `crates/<name>/Cargo.toml`:

```toml
[package]
name = "<name>"
version.workspace = true
edition.workspace = true
publish.workspace = true

[dependencies]
```

and `crates/<name>/src/lib.rs` containing one doc line:

- `takt-google`: `//! Google Calendar v3 and Tasks v1 clients for takt.`
- `takt-store`: `//! SQLite cache of Google data for takt.`
- `takt-sync`: `//! Background sync engine for takt.`

Move the binary:

```bash
mkdir -p crates/takt/src
git mv src/main.rs crates/takt/src/main.rs
```

`crates/takt/Cargo.toml`:

```toml
[package]
name = "takt"
version.workspace = true
edition.workspace = true
publish.workspace = true

[dependencies]
```

`crates/takt-recur/Cargo.toml`:

```toml
[package]
name = "takt-recur"
version.workspace = true
edition.workspace = true
publish.workspace = true

[dependencies]
chrono.workspace = true
chrono-tz.workspace = true
rrule.workspace = true
thiserror.workspace = true

[dev-dependencies]
proptest.workspace = true
serde_json.workspace = true
```

- [ ] **Step 3: Write the test helpers and failing tests**

`crates/takt-recur/tests/common/mod.rs`:

```rust
#![allow(dead_code)]

use chrono::{DateTime, NaiveDate, Utc};
use takt_recur::{Range, When};

pub fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

pub fn at(s: &str) -> When {
    When::At(utc(s))
}

pub fn day(s: &str) -> When {
    When::Date(NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap())
}

pub fn range(start: &str, end: &str) -> Range {
    Range { start: utc(start), end: utc(end) }
}
```

`crates/takt-recur/tests/expand_timed.rs`:

```rust
mod common;

use common::*;
use takt_recur::{Master, Source, expand};

fn london_weekly() -> Master {
    Master {
        id: "abc".into(),
        start: at("2026-10-05T08:00:00Z"),
        end: at("2026-10-05T09:00:00Z"),
        tz: chrono_tz::Europe::London,
        recurrence: vec!["RRULE:FREQ=WEEKLY;COUNT=3".into()],
    }
}

#[test]
fn weekly_count_yields_google_instance_ids() {
    let got = expand(&london_weekly(), &[], &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z")).unwrap();
    let ids: Vec<_> = got.iter().map(|o| o.instance_id.as_str()).collect();
    assert_eq!(ids, ["abc_20261005T080000Z", "abc_20261012T080000Z", "abc_20261019T080000Z"]);
    assert!(got.iter().all(|o| o.source == Source::Generated));
    assert_eq!(got[1].end, at("2026-10-12T09:00:00Z"));
}

#[test]
fn occurrence_starting_before_range_but_overlapping_is_included() {
    let m = Master {
        id: "late".into(),
        start: at("2026-10-01T23:00:00Z"),
        end: at("2026-10-02T01:00:00Z"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=DAILY".into()],
    };
    let got = expand(&m, &[], &range("2026-10-10T00:00:00Z", "2026-10-11T00:00:00Z")).unwrap();
    let starts: Vec<_> = got.iter().map(|o| o.start).collect();
    assert_eq!(starts, [at("2026-10-09T23:00:00Z"), at("2026-10-10T23:00:00Z")]);
}

#[test]
fn wall_clock_time_is_kept_across_dst_change() {
    // 09:00 New York: EDT (UTC-4) on 26 Oct, EST (UTC-5) on 2 Nov 2026.
    let m = Master {
        id: "ny".into(),
        start: at("2026-10-26T13:00:00Z"),
        end: at("2026-10-26T13:30:00Z"),
        tz: chrono_tz::America::New_York,
        recurrence: vec!["RRULE:FREQ=WEEKLY;BYDAY=MO".into()],
    };
    let got = expand(&m, &[], &range("2026-10-26T00:00:00Z", "2026-11-03T00:00:00Z")).unwrap();
    let starts: Vec<_> = got.iter().map(|o| o.start).collect();
    assert_eq!(starts, [at("2026-10-26T13:00:00Z"), at("2026-11-02T14:00:00Z")]);
}

#[test]
fn twenty_year_old_daily_series_still_expands_today() {
    let m = Master {
        id: "old".into(),
        start: at("2005-01-01T09:00:00Z"),
        end: at("2005-01-01T09:15:00Z"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=DAILY".into()],
    };
    let got = expand(&m, &[], &range("2026-10-17T00:00:00Z", "2026-10-18T00:00:00Z")).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].instance_id, "old_20261017T090000Z");
}
```

`crates/takt-recur/tests/expand_all_day.rs`:

```rust
mod common;

use common::*;
use takt_recur::{Master, expand};

fn all_day(id: &str, start: &str, end: &str, rules: &[&str]) -> Master {
    Master {
        id: id.into(),
        start: day(start),
        end: day(end),
        tz: chrono_tz::UTC,
        recurrence: rules.iter().map(|r| r.to_string()).collect(),
    }
}

fn ids(got: &[takt_recur::Occurrence]) -> Vec<&str> {
    got.iter().map(|o| o.instance_id.as_str()).collect()
}

#[test]
fn yearly_all_day_uses_date_instance_ids() {
    let m = all_day("bday", "2000-03-15", "2000-03-16", &["RRULE:FREQ=YEARLY"]);
    let got = expand(&m, &[], &range("2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z")).unwrap();
    assert_eq!(ids(&got), ["bday_20260315"]);
    assert_eq!(got[0].start, day("2026-03-15"));
    assert_eq!(got[0].end, day("2026-03-16"));
}

#[test]
fn exdate_value_date_removes_occurrence() {
    let m = all_day("d", "2026-10-01", "2026-10-02", &["RRULE:FREQ=DAILY;COUNT=5", "EXDATE;VALUE=DATE:20261003"]);
    let got = expand(&m, &[], &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z")).unwrap();
    assert_eq!(ids(&got), ["d_20261001", "d_20261002", "d_20261004", "d_20261005"]);
}

#[test]
fn date_only_until_is_inclusive_for_all_day() {
    let m = all_day("w", "2026-10-05", "2026-10-06", &["RRULE:FREQ=WEEKLY;UNTIL=20261019"]);
    let got = expand(&m, &[], &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z")).unwrap();
    assert_eq!(ids(&got), ["w_20261005", "w_20261012", "w_20261019"]);
}

#[test]
fn date_only_until_on_timed_series_reads_in_series_zone() {
    // 18:00 Los Angeles (PDT, UTC-7) daily until 12 Oct local: last start is 13 Oct 01:00 UTC.
    let m = Master {
        id: "la".into(),
        start: at("2026-10-10T01:00:00Z"),
        end: at("2026-10-10T02:00:00Z"),
        tz: chrono_tz::America::Los_Angeles,
        recurrence: vec!["RRULE:FREQ=DAILY;UNTIL=20261012".into()],
    };
    let got = expand(&m, &[], &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z")).unwrap();
    assert_eq!(got.len(), 4);
    assert_eq!(got.last().unwrap().start, at("2026-10-13T01:00:00Z"));
}

#[test]
fn multi_day_all_day_keeps_its_length() {
    let m = all_day("trip", "2026-10-01", "2026-10-04", &["RRULE:FREQ=WEEKLY;COUNT=2"]);
    let got = expand(&m, &[], &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z")).unwrap();
    assert_eq!(got[1].start, day("2026-10-08"));
    assert_eq!(got[1].end, day("2026-10-11"));
}
```

- [ ] **Step 4: Run the tests to see them fail**

Run: `cargo test -p takt-recur`
Expected: compile errors, `expand`, `Master` and the rest are not defined.

- [ ] **Step 5: Implement `normalize.rs`**

`crates/takt-recur/src/normalize.rs`:

```rust
//! Rewrites Google's recurrence lines into the RFC 5545 text `rrule` accepts.

use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::{Master, When};

/// A DTSTART line built from the series' first event, followed by its
/// recurrence lines with UNTIL, EXDATE and RDATE values made explicit.
pub(crate) fn rruleset_text(master: &Master, all_day: bool) -> String {
    let mut lines = vec![dtstart_line(master)];
    lines.extend(
        master
            .recurrence
            .iter()
            .map(|line| normalize_line(line.trim(), all_day, master.tz)),
    );
    lines.join("\n")
}

fn dtstart_line(master: &Master) -> String {
    match master.start {
        When::At(t) => format!(
            "DTSTART;TZID={}:{}",
            master.tz.name(),
            t.with_timezone(&master.tz).format("%Y%m%dT%H%M%S")
        ),
        // All-day series run in UTC so generated instants map straight back to dates.
        When::Date(d) => format!("DTSTART:{}T000000Z", d.format("%Y%m%d")),
    }
}

fn normalize_line(line: &str, all_day: bool, tz: Tz) -> String {
    let Some((head, value)) = line.split_once(':') else {
        return line.to_string();
    };
    let name = head.split(';').next().unwrap_or_default().to_ascii_uppercase();
    match name.as_str() {
        "RRULE" | "EXRULE" => format!("{name}:{}", normalize_until(value, all_day, tz)),
        "EXDATE" | "RDATE" => {
            let values: Vec<&str> = value.split(',').map(str::trim).collect();
            if values.iter().all(|v| is_date(v)) {
                let stamps: Vec<String> = values.iter().map(|v| format!("{v}T000000Z")).collect();
                format!("{name}:{}", stamps.join(","))
            } else {
                line.to_string()
            }
        }
        _ => line.to_string(),
    }
}

fn normalize_until(rule: &str, all_day: bool, tz: Tz) -> String {
    rule.split(';')
        .map(|part| match part.split_once('=') {
            Some((key, value)) if key.eq_ignore_ascii_case("UNTIL") => {
                format!("UNTIL={}", until_value(value, all_day, tz))
            }
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Google sometimes writes UNTIL as a bare date or a floating local time.
/// RFC 5545 wants UTC when DTSTART carries a zone, so convert.
fn until_value(value: &str, all_day: bool, tz: Tz) -> String {
    if is_date(value) {
        if all_day {
            return format!("{value}T000000Z");
        }
        if let Ok(d) = NaiveDate::parse_from_str(value, "%Y%m%d") {
            return utc_stamp(d.and_hms_opt(23, 59, 59).expect("valid time"), tz);
        }
    } else if value.len() == 15 && !value.ends_with('Z') {
        if let Ok(local) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S") {
            return if all_day { format!("{value}Z") } else { utc_stamp(local, tz) };
        }
    }
    value.to_string()
}

fn utc_stamp(local: NaiveDateTime, tz: Tz) -> String {
    let utc = tz
        .from_local_datetime(&local)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| local.and_utc());
    utc.format("%Y%m%dT%H%M%SZ").to_string()
}

fn is_date(value: &str) -> bool {
    value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_day_exdate_dates_become_utc_midnight() {
        assert_eq!(
            normalize_line("EXDATE;VALUE=DATE:20261003,20261005", true, Tz::UTC),
            "EXDATE:20261003T000000Z,20261005T000000Z"
        );
    }

    #[test]
    fn timed_date_only_until_is_end_of_local_day() {
        assert_eq!(
            normalize_line("RRULE:FREQ=DAILY;UNTIL=20261012", false, chrono_tz::America::Los_Angeles),
            "RRULE:FREQ=DAILY;UNTIL=20261013T065959Z"
        );
    }

    #[test]
    fn floating_until_is_converted_from_series_zone() {
        assert_eq!(
            normalize_line("RRULE:FREQ=WEEKLY;UNTIL=20261012T090000", false, chrono_tz::Europe::London),
            "RRULE:FREQ=WEEKLY;UNTIL=20261012T080000Z"
        );
    }

    #[test]
    fn timezone_exdate_is_left_alone() {
        let line = "EXDATE;TZID=Europe/London:20261012T090000";
        assert_eq!(normalize_line(line, false, chrono_tz::Europe::London), line);
    }
}
```

- [ ] **Step 6: Implement `lib.rs`**

`crates/takt-recur/src/lib.rs`:

```rust
//! Expands Google Calendar recurring series into occurrences.
//!
//! Pure functions: no I/O and no clock. All-day dates compare as UTC midnight
//! here. Callers that need display-time-zone precision pad the range by a day
//! and filter the result.

mod normalize;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use chrono_tz::Tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum When {
    At(DateTime<Utc>),
    Date(NaiveDate),
}

impl When {
    pub fn as_utc(&self) -> DateTime<Utc> {
        match self {
            When::At(t) => *t,
            When::Date(d) => d.and_time(NaiveTime::MIN).and_utc(),
        }
    }

    pub fn is_all_day(&self) -> bool {
        matches!(self, When::Date(_))
    }
}

/// Half-open interval `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Range {
    /// True when `[start, end)` intersects the range. A zero-length item counts
    /// when it sits inside the range.
    pub fn overlaps(&self, start: &When, end: &When) -> bool {
        let (s, e) = (start.as_utc(), end.as_utc());
        s < self.end && (e > self.start || (s == e && s >= self.start))
    }
}

/// The first event of a series, as Google stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct Master {
    pub id: String,
    pub start: When,
    pub end: When,
    /// IANA zone from `start.timeZone`. Unused for all-day series.
    pub tz: Tz,
    /// `recurrence` lines exactly as Google sent them.
    pub recurrence: Vec<String>,
}

/// A changed or cancelled occurrence of a series.
#[derive(Debug, Clone, PartialEq)]
pub struct Exception {
    pub id: String,
    pub original_start: When,
    pub cancelled: bool,
    pub start: When,
    pub end: When,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Generated,
    Exception,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub instance_id: String,
    pub source: Source,
    pub original_start: When,
    pub start: When,
    pub end: When,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecurError {
    #[error("cannot parse recurrence: {0}")]
    Parse(String),
    #[error("series start and end disagree on all-day vs timed")]
    MixedStartEnd,
}

pub const MAX_OCCURRENCES: u16 = 10_000;

/// Google's ID for one occurrence of a series.
pub fn instance_id(master_id: &str, original_start: &When) -> String {
    match original_start {
        When::At(t) => format!("{master_id}_{}", t.format("%Y%m%dT%H%M%SZ")),
        When::Date(d) => format!("{master_id}_{}", d.format("%Y%m%d")),
    }
}

pub fn expand(
    master: &Master,
    _exceptions: &[Exception],
    range: &Range,
) -> Result<Vec<Occurrence>, RecurError> {
    let all_day = series_kind(master)?;
    let duration = master.end.as_utc() - master.start.as_utc();
    let set = parse_set(master, all_day)?;
    let after = to_rrule_tz(range.start - duration - TimeDelta::seconds(1));
    let before = to_rrule_tz(range.end);
    let mut out: Vec<Occurrence> = set
        .after(after)
        .before(before)
        .all(MAX_OCCURRENCES)
        .dates
        .into_iter()
        .map(|dt| original_of(dt.with_timezone(&Utc), all_day))
        .map(|original| Occurrence {
            instance_id: instance_id(&master.id, &original),
            source: Source::Generated,
            original_start: original,
            start: original,
            end: shift(&original, duration),
        })
        .filter(|o| range.overlaps(&o.start, &o.end))
        .collect();
    sort(&mut out);
    Ok(out)
}

fn series_kind(master: &Master) -> Result<bool, RecurError> {
    match (&master.start, &master.end) {
        (When::At(_), When::At(_)) => Ok(false),
        (When::Date(_), When::Date(_)) => Ok(true),
        _ => Err(RecurError::MixedStartEnd),
    }
}

fn parse_set(master: &Master, all_day: bool) -> Result<rrule::RRuleSet, RecurError> {
    normalize::rruleset_text(master, all_day)
        .parse::<rrule::RRuleSet>()
        .map_err(|e| RecurError::Parse(e.to_string()))
}

fn to_rrule_tz(t: DateTime<Utc>) -> DateTime<rrule::Tz> {
    t.with_timezone(&rrule::Tz::UTC)
}

fn original_of(t: DateTime<Utc>, all_day: bool) -> When {
    if all_day { When::Date(t.date_naive()) } else { When::At(t) }
}

fn shift(start: &When, duration: TimeDelta) -> When {
    match start {
        When::At(t) => When::At(*t + duration),
        When::Date(d) => When::Date(*d + TimeDelta::days(duration.num_days())),
    }
}

fn sort(occurrences: &mut [Occurrence]) {
    occurrences.sort_by(|a, b| {
        a.start
            .as_utc()
            .cmp(&b.start.as_utc())
            .then_with(|| a.instance_id.cmp(&b.instance_id))
    });
}
```

- [ ] **Step 7: Run the tests to see them pass**

Run: `cargo test -p takt-recur`
Expected: all tests in `normalize::tests`, `expand_timed`, `expand_all_day` pass. If `rrule` rejects a form, read the error text, extend `normalize_line` for that form, and add a unit test for it in `normalize::tests`.

- [ ] **Step 8: Check the whole workspace builds**

Run: `cargo build --workspace`
Expected: success; `crates/takt` still prints "Hello, world!".

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock crates
git commit -m "feat(takt-recur): expand timed and all-day series with Google instance IDs" -m "Also converts the repo into a Cargo workspace with empty crates for google, store, sync and the app."
```

---

### Task 2: Exceptions, caps, errors and series end

**Files:**
- Modify: `crates/takt-recur/src/lib.rs` (`expand`, add `series_end`)
- Create: `crates/takt-recur/tests/exceptions.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces: `expand` now applies exceptions; `fn series_end(&Master) -> Result<Option<When>, RecurError>` returns the end of the last occurrence, or `None` when the series has no end.

- [ ] **Step 1: Write the failing tests**

`crates/takt-recur/tests/exceptions.rs`:

```rust
mod common;

use common::*;
use takt_recur::{Exception, MAX_OCCURRENCES, Master, RecurError, Source, When, expand, series_end};

// Mondays 5, 12, 19, 26 October 2026, 09:00-10:00 UTC.
fn weekly() -> Master {
    Master {
        id: "w".into(),
        start: at("2026-10-05T09:00:00Z"),
        end: at("2026-10-05T10:00:00Z"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=WEEKLY;COUNT=4".into()],
    }
}

fn october() -> takt_recur::Range {
    range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z")
}

fn moved(original: When, start: &str, end: &str) -> Exception {
    Exception { id: "w_20261012T090000Z".into(), original_start: original, cancelled: false, start: at(start), end: at(end) }
}

#[test]
fn cancelled_exception_hides_occurrence() {
    let original = at("2026-10-12T09:00:00Z");
    let cancelled = Exception { id: "w_20261012T090000Z".into(), original_start: original, cancelled: true, start: original, end: original };
    let got = expand(&weekly(), &[cancelled], &october()).unwrap();
    let ids: Vec<_> = got.iter().map(|o| o.instance_id.as_str()).collect();
    assert_eq!(ids, ["w_20261005T090000Z", "w_20261019T090000Z", "w_20261026T090000Z"]);
}

#[test]
fn modified_exception_replaces_occurrence() {
    let e = moved(at("2026-10-12T09:00:00Z"), "2026-10-13T15:00:00Z", "2026-10-13T16:00:00Z");
    let got = expand(&weekly(), &[e], &october()).unwrap();
    assert_eq!(got.len(), 4);
    assert_eq!(got[1].start, at("2026-10-13T15:00:00Z"));
    assert_eq!(got[1].source, Source::Exception);
    assert_eq!(got[1].original_start, at("2026-10-12T09:00:00Z"));
    assert_eq!(got[1].instance_id, "w_20261012T090000Z");
}

#[test]
fn exception_moved_into_range_from_outside_is_included() {
    let e = moved(at("2026-10-12T09:00:00Z"), "2026-10-20T12:00:00Z", "2026-10-20T13:00:00Z");
    let got = expand(&weekly(), &[e], &range("2026-10-20T00:00:00Z", "2026-10-21T00:00:00Z")).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].start, at("2026-10-20T12:00:00Z"));
}

#[test]
fn exception_moved_out_of_range_hides_its_original() {
    let e = moved(at("2026-10-12T09:00:00Z"), "2026-10-30T09:00:00Z", "2026-10-30T10:00:00Z");
    let got = expand(&weekly(), &[e], &range("2026-10-12T00:00:00Z", "2026-10-13T00:00:00Z")).unwrap();
    assert!(got.is_empty());
}

#[test]
fn unparseable_rule_is_an_error() {
    let mut m = weekly();
    m.recurrence = vec!["RRULE:FREQ=SOMETIMES".into()];
    assert!(matches!(expand(&m, &[], &october()), Err(RecurError::Parse(_))));
}

#[test]
fn start_and_end_kinds_must_match() {
    let mut m = weekly();
    m.end = day("2026-10-06");
    assert_eq!(expand(&m, &[], &october()), Err(RecurError::MixedStartEnd));
}

#[test]
fn expansion_stops_at_cap() {
    let m = Master {
        id: "d".into(),
        start: at("2000-01-01T09:00:00Z"),
        end: at("2000-01-01T09:30:00Z"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=DAILY".into()],
    };
    let got = expand(&m, &[], &range("2000-01-01T00:00:00Z", "2100-01-01T00:00:00Z")).unwrap();
    assert_eq!(got.len(), MAX_OCCURRENCES as usize);
}

#[test]
fn series_end_is_none_without_until_or_count() {
    let mut m = weekly();
    m.recurrence = vec!["RRULE:FREQ=WEEKLY".into()];
    assert_eq!(series_end(&m), Ok(None));
}

#[test]
fn series_end_for_count() {
    assert_eq!(series_end(&weekly()), Ok(Some(at("2026-10-26T10:00:00Z"))));
}

#[test]
fn series_end_for_all_day_until() {
    let m = Master {
        id: "a".into(),
        start: day("2026-10-05"),
        end: day("2026-10-06"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=WEEKLY;UNTIL=20261019".into()],
    };
    assert_eq!(series_end(&m), Ok(Some(day("2026-10-20"))));
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt-recur --test exceptions`
Expected: compile error, `series_end` is not defined.

- [ ] **Step 3: Apply exceptions in `expand` and add `series_end`**

In `crates/takt-recur/src/lib.rs`, replace `expand` with:

```rust
pub fn expand(
    master: &Master,
    exceptions: &[Exception],
    range: &Range,
) -> Result<Vec<Occurrence>, RecurError> {
    let all_day = series_kind(master)?;
    let duration = master.end.as_utc() - master.start.as_utc();
    let set = parse_set(master, all_day)?;
    let after = to_rrule_tz(range.start - duration - TimeDelta::seconds(1));
    let before = to_rrule_tz(range.end);
    let mut out: Vec<Occurrence> = set
        .after(after)
        .before(before)
        .all(MAX_OCCURRENCES)
        .dates
        .into_iter()
        .map(|dt| original_of(dt.with_timezone(&Utc), all_day))
        // Any exception, cancelled or moved, replaces the generated occurrence.
        .filter(|original| !exceptions.iter().any(|e| e.original_start == *original))
        .map(|original| Occurrence {
            instance_id: instance_id(&master.id, &original),
            source: Source::Generated,
            original_start: original,
            start: original,
            end: shift(&original, duration),
        })
        .filter(|o| range.overlaps(&o.start, &o.end))
        .collect();
    out.extend(
        exceptions
            .iter()
            .filter(|e| !e.cancelled && range.overlaps(&e.start, &e.end))
            .map(|e| Occurrence {
                instance_id: e.id.clone(),
                source: Source::Exception,
                original_start: e.original_start,
                start: e.start,
                end: e.end,
            }),
    );
    sort(&mut out);
    Ok(out)
}

/// End of the series' last occurrence, or `None` when it never ends (or has
/// more than [`MAX_OCCURRENCES`] occurrences, which callers treat the same way).
pub fn series_end(master: &Master) -> Result<Option<When>, RecurError> {
    let all_day = series_kind(master)?;
    let bounded = master
        .recurrence
        .iter()
        .map(|line| line.trim().to_ascii_uppercase())
        .filter(|line| line.starts_with("RRULE"))
        .all(|line| line.contains("UNTIL=") || line.contains("COUNT="));
    if !bounded {
        return Ok(None);
    }
    let duration = master.end.as_utc() - master.start.as_utc();
    let result = parse_set(master, all_day)?.all(MAX_OCCURRENCES);
    if result.limited {
        return Ok(None);
    }
    Ok(Some(match result.dates.last() {
        Some(dt) => shift(&original_of(dt.with_timezone(&Utc), all_day), duration),
        None => master.end,
    }))
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p takt-recur`
Expected: all pass, including Task 1's tests.

- [ ] **Step 5: Commit**

```bash
git add crates/takt-recur
git commit -m "feat(takt-recur): apply exceptions, cap expansion and compute series end"
```

---

### Task 3: Property tests for expansion

**Files:**
- Create: `crates/takt-recur/tests/properties.rs`
- Modify (only if Step 2 finds a panic): `crates/takt-recur/src/lib.rs`

**Interfaces:**
- Consumes: `expand`, `series_end`, `Master`, `Range`, `When`.

- [ ] **Step 1: Write the property tests**

`crates/takt-recur/tests/properties.rs`:

```rust
use std::collections::HashSet;

use chrono::{NaiveDate, TimeDelta, TimeZone, Utc};
use proptest::prelude::*;
use takt_recur::{Master, Range, Source, When, expand, series_end};

fn master(freq: &str, interval: u32, count: Option<u32>, start_day: i64, minute: i64, length_min: i64, all_day: bool) -> Master {
    let base = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap() + TimeDelta::days(start_day);
    let mut rule = format!("RRULE:FREQ={freq};INTERVAL={interval}");
    if let Some(c) = count {
        rule.push_str(&format!(";COUNT={c}"));
    }
    let (start, end) = if all_day {
        let days = (length_min / 1440).max(1);
        (When::Date(base), When::Date(base + TimeDelta::days(days)))
    } else {
        let s = Utc.from_utc_datetime(&base.and_hms_opt(0, 0, 0).unwrap()) + TimeDelta::minutes(minute);
        (When::At(s), When::At(s + TimeDelta::minutes(length_min)))
    };
    Master { id: "p".into(), start, end, tz: chrono_tz::Europe::Berlin, recurrence: vec![rule] }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn expansion_invariants(
        freq in prop::sample::select(vec!["DAILY", "WEEKLY", "MONTHLY", "YEARLY"]),
        interval in 1u32..5,
        count in prop::option::of(1u32..60),
        start_day in 0i64..11_000,
        minute in 0i64..1440,
        length_min in 0i64..4_000,
        range_offset_days in -400i64..400,
        range_len_days in 1i64..120,
        all_day in any::<bool>(),
    ) {
        let m = master(freq, interval, count, start_day, minute, length_min, all_day);
        let range_start = m.start.as_utc() + TimeDelta::days(range_offset_days);
        let range = Range { start: range_start, end: range_start + TimeDelta::days(range_len_days) };
        if let Ok(got) = expand(&m, &[], &range) {
            prop_assert!(got.windows(2).all(|w| w[0].start.as_utc() <= w[1].start.as_utc()));
            prop_assert!(got.iter().all(|o| range.overlaps(&o.start, &o.end)));
            let ids: HashSet<_> = got.iter().map(|o| o.instance_id.clone()).collect();
            prop_assert_eq!(ids.len(), got.len());
            prop_assert!(got.iter().all(|o| o.source == Source::Generated));
        }
    }

    #[test]
    fn arbitrary_rule_text_never_panics(text in "\\PC{0,80}") {
        let mut m = master("DAILY", 1, Some(3), 9000, 540, 30, false);
        m.recurrence = vec![format!("RRULE:{text}")];
        let range = Range { start: m.start.as_utc(), end: m.start.as_utc() + TimeDelta::days(30) };
        let _ = expand(&m, &[], &range);
        let _ = series_end(&m);
    }
}
```

- [ ] **Step 2: Run them**

Run: `cargo test -p takt-recur --test properties`
Expected: PASS. If proptest reports a panic that originates inside `rrule`, do Step 3; otherwise skip to Step 4.

- [ ] **Step 3 (only on an `rrule` panic): Contain it**

In `crates/takt-recur/src/lib.rs`, route both calls into `rrule` through this helper and use it in `expand` and `series_end`:

```rust
/// `rrule` can panic on some malformed rules; turn that into a parse error so
/// one bad series cannot take down the UI or the sync thread.
fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, RecurError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .map_err(|_| RecurError::Parse("rule rejected by the rrule crate".into()))
}
```

For example in `expand`: `let dates = guarded(|| set.after(after).before(before).all(MAX_OCCURRENCES))?.dates;` and in `series_end`: `let result = guarded(|| set.all(MAX_OCCURRENCES))?;`. Add the shrunk input proptest printed as a named unit test in `tests/exceptions.rs` asserting `Err(RecurError::Parse(_))`. Re-run Step 2.

- [ ] **Step 4: Commit**

```bash
git add crates/takt-recur
git commit -m "test(takt-recur): add property tests for expansion invariants"
```

---

### Task 4: Google resource models

**Files:**
- Modify: `crates/takt-google/Cargo.toml`, `crates/takt-google/src/lib.rs`
- Create: `crates/takt-google/src/error.rs`, `crates/takt-google/src/model.rs`, `crates/takt-google/tests/model.rs`

**Interfaces:**
- Produces (`takt_google`): `enum GoogleError { Network(String), Gone, NotFound, Forbidden { reason: String }, RateLimited, InvalidGrant, Http { status: u16, body: String }, Decode(String), AuthFlow(String), TokenStore(String) }`; `pub use reqwest::Url`.
- Produces (`takt_google::model`): `Item<T> { fields: T, raw: serde_json::Value }` with `Item::from_value(Value) -> Result<Item<T>, GoogleError>`; `Page<T> { items: Vec<Item<T>>, next_page_token: Option<String>, next_sync_token: Option<String> }` with `Page::from_value`; `EventDateTime { date: Option<NaiveDate>, date_time: Option<DateTime<FixedOffset>>, time_zone: Option<String> }`; `EventFields` (fields below) with `is_cancelled()`, `self_response()`; `Attendee`; `CalendarListEntry` with `display_name()`; `TaskListFields`; `TaskFields` with `due_date()`; `ColorDef`; `Colors { calendar, event: BTreeMap<String, ColorDef> }`; `Setting { id, value }`.

- [ ] **Step 1: Add dependencies**

`crates/takt-google/Cargo.toml`, below `[package]`:

```toml
[dependencies]
base64.workspace = true
chrono.workspace = true
keyring.workspace = true
rand.workspace = true
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
thiserror.workspace = true
tokio.workspace = true
tracing.workspace = true

[dev-dependencies]
toml.workspace = true
wiremock.workspace = true
```

- [ ] **Step 2: Write the failing tests**

`crates/takt-google/tests/model.rs`:

```rust
use chrono::NaiveDate;
use serde_json::json;
use takt_google::GoogleError;
use takt_google::model::*;

#[test]
fn event_keeps_unknown_fields_in_raw() {
    let raw = json!({
        "kind": "calendar#event", "id": "e1", "status": "confirmed", "summary": "Standup",
        "iCalUID": "e1@google.com",
        "start": {"dateTime": "2026-10-17T09:00:00+01:00", "timeZone": "Europe/London"},
        "end": {"dateTime": "2026-10-17T09:30:00+01:00", "timeZone": "Europe/London"},
        "conferenceData": {"entryPoints": [{"uri": "https://meet.google.com/abc"}]},
        "x-future-field": {"nested": [1, 2, 3]}
    });
    let item = Item::<EventFields>::from_value(raw.clone()).unwrap();
    assert_eq!(item.raw, raw);
    assert_eq!(item.fields.id, "e1");
    assert_eq!(item.fields.ical_uid.as_deref(), Some("e1@google.com"));
    let start = item.fields.start.as_ref().unwrap();
    assert_eq!(start.time_zone.as_deref(), Some("Europe/London"));
    assert_eq!(start.date_time.unwrap().to_rfc3339(), "2026-10-17T09:00:00+01:00");
}

#[test]
fn cancelled_instance_with_minimal_fields_parses() {
    let item = Item::<EventFields>::from_value(json!({
        "id": "e1_20261017T080000Z", "status": "cancelled", "recurringEventId": "e1",
        "originalStartTime": {"dateTime": "2026-10-17T09:00:00+01:00", "timeZone": "Europe/London"}
    }))
    .unwrap();
    assert!(item.fields.is_cancelled());
    assert_eq!(item.fields.recurring_event_id.as_deref(), Some("e1"));
    assert!(item.fields.start.is_none());
    assert!(item.fields.original_start_time.is_some());
}

#[test]
fn all_day_event_reads_date() {
    let item = Item::<EventFields>::from_value(json!({
        "id": "a", "start": {"date": "2026-10-20"}, "end": {"date": "2026-10-21"}
    }))
    .unwrap();
    assert_eq!(item.fields.start.unwrap().date, NaiveDate::from_ymd_opt(2026, 10, 20));
}

#[test]
fn self_response_comes_from_self_attendee() {
    let item = Item::<EventFields>::from_value(json!({
        "id": "m",
        "attendees": [
            {"email": "a@example.com", "responseStatus": "accepted"},
            {"email": "me@example.com", "self": true, "responseStatus": "declined"}
        ]
    }))
    .unwrap();
    assert_eq!(item.fields.self_response(), Some("declined"));
}

#[test]
fn calendar_list_entry_flags_default_to_false() {
    let item = Item::<CalendarListEntry>::from_value(json!({"id": "me@example.com", "summary": "Me", "accessRole": "owner"})).unwrap();
    let c = item.fields;
    assert!(!c.selected && !c.hidden && !c.primary && !c.deleted);
    assert_eq!(c.display_name(), "Me");
}

#[test]
fn summary_override_wins_for_display_name() {
    let item = Item::<CalendarListEntry>::from_value(json!({"id": "x", "summary": "Team", "summaryOverride": "Work team"})).unwrap();
    assert_eq!(item.fields.display_name(), "Work team");
}

#[test]
fn task_due_date_is_date_part() {
    let item = Item::<TaskFields>::from_value(json!({
        "id": "t1", "title": "Pay rent", "status": "needsAction",
        "due": "2026-10-20T00:00:00.000Z", "updated": "2026-10-17T10:00:00.000Z"
    }))
    .unwrap();
    assert_eq!(item.fields.due_date(), NaiveDate::from_ymd_opt(2026, 10, 20));
}

#[test]
fn colors_parse() {
    let colors: Colors = serde_json::from_value(json!({
        "calendar": {"1": {"background": "#ac725e", "foreground": "#1d1d1d"}},
        "event": {"1": {"background": "#a4bdfc", "foreground": "#1d1d1d"}}
    }))
    .unwrap();
    assert_eq!(colors.event["1"].background, "#a4bdfc");
}

#[test]
fn page_reads_tokens_and_items() {
    let page = Page::<EventFields>::from_value(json!({
        "items": [{"id": "a"}, {"id": "b"}], "nextPageToken": "p2"
    }))
    .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next_page_token.as_deref(), Some("p2"));
    assert_eq!(page.next_sync_token, None);
}

#[test]
fn page_without_items_is_empty() {
    let page = Page::<EventFields>::from_value(json!({"nextSyncToken": "s1"})).unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.next_sync_token.as_deref(), Some("s1"));
}

#[test]
fn missing_id_is_a_decode_error() {
    let err = Item::<EventFields>::from_value(json!({"summary": "no id"})).unwrap_err();
    assert!(matches!(err, GoogleError::Decode(_)));
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p takt-google --test model`
Expected: compile errors, `takt_google::model` does not exist.

- [ ] **Step 4: Implement `error.rs`**

`crates/takt-google/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum GoogleError {
    #[error("network error: {0}")]
    Network(String),
    #[error("sync token no longer valid (410)")]
    Gone,
    #[error("not found (404)")]
    NotFound,
    #[error("forbidden: {reason}")]
    Forbidden { reason: String },
    #[error("rate limited after retries")]
    RateLimited,
    #[error("authorization was revoked or has expired")]
    InvalidGrant,
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("unexpected response: {0}")]
    Decode(String),
    #[error("sign-in failed: {0}")]
    AuthFlow(String),
    #[error("keychain: {0}")]
    TokenStore(String),
}

impl GoogleError {
    pub(crate) fn from_reqwest(error: reqwest::Error) -> Self {
        GoogleError::Network(error.to_string())
    }
}
```

- [ ] **Step 5: Implement `model.rs` and `lib.rs`**

`crates/takt-google/src/model.rs`:

```rust
//! The parts of Google's resources takt reads. Every resource also keeps its
//! complete JSON in [`Item::raw`], so fields takt does not know survive.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::GoogleError;

#[derive(Debug, Clone, PartialEq)]
pub struct Item<T> {
    pub fields: T,
    pub raw: Value,
}

impl<T: DeserializeOwned> Item<T> {
    pub fn from_value(raw: Value) -> Result<Self, GoogleError> {
        let fields = T::deserialize(&raw).map_err(|e| GoogleError::Decode(e.to_string()))?;
        Ok(Self { fields, raw })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    pub items: Vec<Item<T>>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
}

impl<T: DeserializeOwned> Page<T> {
    pub fn from_value(mut body: Value) -> Result<Self, GoogleError> {
        let items = match body.get_mut("items").map(Value::take) {
            Some(Value::Array(items)) => items
                .into_iter()
                .map(Item::from_value)
                .collect::<Result<Vec<_>, _>>()?,
            Some(Value::Null) | None => Vec::new(),
            Some(other) => {
                return Err(GoogleError::Decode(format!("items is not an array: {other}")));
            }
        };
        Ok(Self {
            items,
            next_page_token: string_field(&body, "nextPageToken"),
            next_sync_token: string_field(&body, "nextSyncToken"),
        })
    }
}

fn string_field(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDateTime {
    pub date: Option<NaiveDate>,
    pub date_time: Option<DateTime<FixedOffset>>,
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    pub email: Option<String>,
    #[serde(rename = "self", default)]
    pub is_self: bool,
    pub response_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFields {
    pub id: String,
    pub status: Option<String>,
    pub etag: Option<String>,
    #[serde(rename = "iCalUID")]
    pub ical_uid: Option<String>,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub color_id: Option<String>,
    pub event_type: Option<String>,
    pub start: Option<EventDateTime>,
    pub end: Option<EventDateTime>,
    #[serde(default)]
    pub recurrence: Vec<String>,
    pub recurring_event_id: Option<String>,
    pub original_start_time: Option<EventDateTime>,
    pub updated: Option<DateTime<Utc>>,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
}

impl EventFields {
    pub fn is_cancelled(&self) -> bool {
        self.status.as_deref() == Some("cancelled")
    }

    /// The signed-in user's RSVP, when they are an attendee.
    pub fn self_response(&self) -> Option<&str> {
        self.attendees
            .iter()
            .find(|a| a.is_self)
            .and_then(|a| a.response_status.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarListEntry {
    pub id: String,
    pub summary: Option<String>,
    pub summary_override: Option<String>,
    pub time_zone: Option<String>,
    pub color_id: Option<String>,
    pub background_color: Option<String>,
    pub foreground_color: Option<String>,
    pub access_role: Option<String>,
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub deleted: bool,
}

impl CalendarListEntry {
    pub fn display_name(&self) -> String {
        self.summary_override
            .clone()
            .or_else(|| self.summary.clone())
            .unwrap_or_else(|| self.id.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListFields {
    pub id: String,
    pub title: Option<String>,
    pub updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFields {
    pub id: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub due: Option<DateTime<FixedOffset>>,
    pub parent: Option<String>,
    pub position: Option<String>,
    pub completed: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub hidden: bool,
    pub updated: Option<DateTime<Utc>>,
}

impl TaskFields {
    /// The Tasks API keeps only the date of `due`; the time part is always midnight UTC.
    pub fn due_date(&self) -> Option<NaiveDate> {
        self.due.map(|d| d.date_naive())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ColorDef {
    pub background: String,
    pub foreground: String,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Colors {
    #[serde(default)]
    pub calendar: BTreeMap<String, ColorDef>,
    #[serde(default)]
    pub event: BTreeMap<String, ColorDef>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Setting {
    pub id: String,
    pub value: String,
}
```

`crates/takt-google/src/lib.rs`:

```rust
//! Google Calendar v3 and Tasks v1 clients for takt.

mod error;
pub mod model;

pub use error::GoogleError;
pub use reqwest::Url;
```

- [ ] **Step 6: Run the tests to see them pass**

Run: `cargo test -p takt-google --test model`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/takt-google Cargo.lock
git commit -m "feat(takt-google): add resource models that keep raw JSON"
```

---

### Task 5: OAuth sign-in, token refresh and keychain storage

**Files:**
- Modify: `crates/takt-google/src/lib.rs`
- Create: `crates/takt-google/src/auth.rs`, `crates/takt-google/src/token_store.rs`, `crates/takt-google/tests/auth.rs`

**Interfaces:**
- Consumes: `GoogleError`.
- Produces: `Endpoints { auth, token, calendar, tasks: Url }` with `Endpoints::google()` and `Endpoints::rooted_at(&str)`; `SCOPES: &[&str]`; `OAuthClient { client_id, client_secret }` (Clone); `Pkce { verifier, challenge }` with `generate()`, `from_verifier(impl Into<String>)`; `authorization_url(&Endpoints, client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> Url`; `enum Redirect { Code { code, state }, Denied(String), Other }`; `parse_redirect_request_line(&str) -> Redirect`; `IdClaims { sub, email }`; `decode_id_token(&str) -> Result<IdClaims, GoogleError>`; `SignedIn { sub, email, refresh_token, access_token, expires_in: u64 }`; `async fn sign_in(&reqwest::Client, &Endpoints, &OAuthClient, open_browser: impl FnOnce(&Url)) -> Result<SignedIn, GoogleError>`; `Auth::new(reqwest::Client, token_url: Url, OAuthClient, refresh_token: impl Into<String>)`, `Auth::with_access_token(self, impl Into<String>, Duration) -> Auth`, `async Auth::access_token(&self, force_refresh: bool) -> Result<String, GoogleError>`; `trait TokenStore: Send + Sync { get, set, delete }` keyed by email; `KeyringTokenStore`; `MemoryTokenStore`.

- [ ] **Step 1: Write the failing tests**

`crates/takt-google/tests/auth.rs`:

```rust
use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use takt_google::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn oauth() -> OAuthClient {
    OAuthClient { client_id: "cid".into(), client_secret: "secret".into() }
}

fn id_token(sub: &str, email: &str) -> String {
    let payload = URL_SAFE_NO_PAD.encode(json!({"sub": sub, "email": email}).to_string());
    format!("eyJhbGciOiJSUzI1NiJ9.{payload}.signature")
}

#[test]
fn pkce_matches_rfc7636_example() {
    let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
    assert_eq!(pkce.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuZeKitWfFM");
}

#[test]
fn generated_verifier_is_43_chars() {
    assert_eq!(Pkce::generate().verifier.len(), 43);
}

#[test]
fn authorization_url_carries_pkce_scopes_and_offline_access() {
    let url = authorization_url(&Endpoints::google(), "cid", "http://127.0.0.1:5555", "chal", "st");
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["code_challenge"], "chal");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["access_type"], "offline");
    assert_eq!(q["scope"], SCOPES.join(" "));
    assert_eq!(q["redirect_uri"], "http://127.0.0.1:5555");
    assert_eq!(q["state"], "st");
    assert_eq!(q["response_type"], "code");
}

#[test]
fn redirect_request_lines() {
    assert_eq!(
        parse_redirect_request_line("GET /?state=s1&code=4%2Fabc HTTP/1.1"),
        Redirect::Code { code: "4/abc".into(), state: "s1".into() }
    );
    assert_eq!(
        parse_redirect_request_line("GET /?error=access_denied&state=s1 HTTP/1.1"),
        Redirect::Denied("access_denied".into())
    );
    assert_eq!(parse_redirect_request_line("GET /favicon.ico HTTP/1.1"), Redirect::Other);
    assert_eq!(parse_redirect_request_line(""), Redirect::Other);
}

#[test]
fn id_token_claims_decode() {
    let claims = decode_id_token(&id_token("123", "a@example.com")).unwrap();
    assert_eq!(claims, IdClaims { sub: "123".into(), email: "a@example.com".into() });
    assert!(decode_id_token("not-a-jwt").is_err());
}

async fn mount_code_exchange(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains("code_verifier="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "at", "expires_in": 3599, "refresh_token": "rt",
            "id_token": id_token("sub-1", "me@example.com")
        })))
        .mount(server)
        .await;
}

fn browser_that_returns(state_override: Option<&'static str>) -> impl FnOnce(&Url) {
    move |auth_url: &Url| {
        let q: HashMap<String, String> = auth_url.query_pairs().into_owned().collect();
        let state = state_override.map(str::to_owned).unwrap_or_else(|| q["state"].clone());
        let callback = format!("{}/?code=the-code&state={state}", q["redirect_uri"]);
        tokio::spawn(async move {
            let _ = reqwest::get(callback).await;
        });
    }
}

#[tokio::test]
async fn sign_in_round_trip() {
    let server = MockServer::start().await;
    mount_code_exchange(&server).await;
    let endpoints = Endpoints::rooted_at(&server.uri());
    let signed = sign_in(&reqwest::Client::new(), &endpoints, &oauth(), browser_that_returns(None))
        .await
        .unwrap();
    assert_eq!(signed.sub, "sub-1");
    assert_eq!(signed.email, "me@example.com");
    assert_eq!(signed.refresh_token, "rt");
    assert_eq!(signed.access_token, "at");
}

#[tokio::test]
async fn sign_in_rejects_state_mismatch() {
    let server = MockServer::start().await;
    mount_code_exchange(&server).await;
    let endpoints = Endpoints::rooted_at(&server.uri());
    let err = sign_in(&reqwest::Client::new(), &endpoints, &oauth(), browser_that_returns(Some("forged")))
        .await
        .unwrap_err();
    assert!(matches!(err, GoogleError::AuthFlow(_)));
}

async fn mount_refresh(server: &MockServer, token: &str, expect: u64) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=rt"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": token, "expires_in": 3600})))
        .expect(expect)
        .mount(server)
        .await;
}

fn auth_for(server: &MockServer) -> Auth {
    let endpoints = Endpoints::rooted_at(&server.uri());
    Auth::new(reqwest::Client::new(), endpoints.token, oauth(), "rt")
}

#[tokio::test]
async fn cached_access_token_is_reused() {
    let server = MockServer::start().await;
    mount_refresh(&server, "at1", 1).await;
    let auth = auth_for(&server);
    assert_eq!(auth.access_token(false).await.unwrap(), "at1");
    assert_eq!(auth.access_token(false).await.unwrap(), "at1");
}

#[tokio::test]
async fn force_refresh_hits_token_endpoint() {
    let server = MockServer::start().await;
    mount_refresh(&server, "new", 1).await;
    let auth = auth_for(&server).with_access_token("old", Duration::from_secs(3600));
    assert_eq!(auth.access_token(true).await.unwrap(), "new");
}

#[tokio::test]
async fn token_close_to_expiry_is_refreshed() {
    let server = MockServer::start().await;
    mount_refresh(&server, "new", 1).await;
    let auth = auth_for(&server).with_access_token("old", Duration::from_secs(30));
    assert_eq!(auth.access_token(false).await.unwrap(), "new");
}

#[tokio::test]
async fn invalid_grant_maps_to_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant", "error_description": "Token has been expired or revoked."
        })))
        .mount(&server)
        .await;
    let err = auth_for(&server).access_token(false).await.unwrap_err();
    assert!(matches!(err, GoogleError::InvalidGrant));
}

#[test]
fn memory_token_store_round_trip() {
    let store = MemoryTokenStore::default();
    assert_eq!(store.get("a@example.com").unwrap(), None);
    store.set("a@example.com", "rt").unwrap();
    assert_eq!(store.get("a@example.com").unwrap().as_deref(), Some("rt"));
    store.delete("a@example.com").unwrap();
    assert_eq!(store.get("a@example.com").unwrap(), None);
}

#[test]
#[ignore = "touches the real OS keychain; run with --ignored"]
fn keyring_token_store_round_trip() {
    let store = KeyringTokenStore;
    let email = "takt-test@example.invalid";
    store.set(email, "rt-test").unwrap();
    assert_eq!(store.get(email).unwrap().as_deref(), Some("rt-test"));
    store.delete(email).unwrap();
    assert_eq!(store.get(email).unwrap(), None);
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt-google --test auth`
Expected: compile errors for the missing items.

- [ ] **Step 3: Implement `auth.rs`**

`crates/takt-google/src/auth.rs`:

```rust
//! Google OAuth for installed apps: PKCE, a loopback redirect, and refresh.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::{Endpoints, GoogleError};

pub const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar",
    "https://www.googleapis.com/auth/tasks",
    "openid",
    "email",
];

const REFRESH_MARGIN: Duration = Duration::from_secs(60);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        Self::from_verifier(URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()))
    }

    pub fn from_verifier(verifier: impl Into<String>) -> Self {
        let verifier = verifier.into();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

pub fn authorization_url(
    endpoints: &Endpoints,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Url {
    let mut url = endpoints.auth.clone();
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &SCOPES.join(" "))
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("access_type", "offline")
        // `consent` makes Google return a refresh token even on a repeat sign-in.
        .append_pair("prompt", "select_account consent");
    url
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirect {
    Code { code: String, state: String },
    Denied(String),
    Other,
}

/// Reads the request line of the browser's redirect, e.g. `GET /?code=..&state=.. HTTP/1.1`.
pub fn parse_redirect_request_line(line: &str) -> Redirect {
    let Some(target) = line.split_whitespace().nth(1) else {
        return Redirect::Other;
    };
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Redirect::Other;
    };
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if let Some(error) = query.get("error") {
        return Redirect::Denied(error.clone());
    }
    match (query.get("code"), query.get("state")) {
        (Some(code), Some(state)) => Redirect::Code { code: code.clone(), state: state.clone() },
        _ => Redirect::Other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IdClaims {
    pub sub: String,
    pub email: String,
}

/// Reads the claims of an ID token without checking its signature. OpenID
/// Connect allows this for a token received directly from the token endpoint
/// over TLS, which is the only place takt gets one.
pub fn decode_id_token(jwt: &str) -> Result<IdClaims, GoogleError> {
    let payload = jwt
        .split('.')
        .nth(1)
        .ok_or_else(|| GoogleError::AuthFlow("malformed id_token".into()))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|e| GoogleError::AuthFlow(format!("id_token payload: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| GoogleError::AuthFlow(format!("id_token claims: {e}")))
}

#[derive(Debug, Clone)]
pub struct SignedIn {
    pub sub: String,
    pub email: String,
    pub refresh_token: String,
    pub access_token: String,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenErrorBody {
    error: String,
}

/// Runs the whole browser sign-in: loopback listener, consent page, code exchange.
pub async fn sign_in(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    client: &OAuthClient,
    open_browser: impl FnOnce(&Url),
) -> Result<SignedIn, GoogleError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| GoogleError::AuthFlow(format!("cannot open loopback port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| GoogleError::AuthFlow(e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let pkce = Pkce::generate();
    let state = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>());
    open_browser(&authorization_url(endpoints, &client.client_id, &redirect_uri, &pkce.challenge, &state));
    let code = tokio::time::timeout(SIGN_IN_TIMEOUT, receive_code(&listener, &state))
        .await
        .map_err(|_| GoogleError::AuthFlow("timed out waiting for the browser".into()))??;
    exchange_code(http, endpoints, client, &code, &pkce.verifier, &redirect_uri).await
}

async fn receive_code(listener: &TcpListener, expected_state: &str) -> Result<String, GoogleError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| GoogleError::AuthFlow(e.to_string()))?;
        let mut buf = vec![0u8; 8192];
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| GoogleError::AuthFlow(e.to_string()))?;
        let request = String::from_utf8_lossy(&buf[..n]);
        match parse_redirect_request_line(request.lines().next().unwrap_or_default()) {
            Redirect::Code { code, state } if state == expected_state => {
                respond(&mut stream, "200 OK", "Signed in. You can close this tab and return to takt.").await;
                return Ok(code);
            }
            Redirect::Code { .. } => {
                respond(&mut stream, "400 Bad Request", "Sign-in state did not match. Try again from takt.").await;
                return Err(GoogleError::AuthFlow("state mismatch".into()));
            }
            Redirect::Denied(reason) => {
                respond(&mut stream, "200 OK", "Sign-in was cancelled. You can close this tab.").await;
                return Err(GoogleError::AuthFlow(format!("consent denied: {reason}")));
            }
            // Browsers also ask for /favicon.ico; ignore anything that is not the redirect.
            Redirect::Other => respond(&mut stream, "404 Not Found", "").await,
        }
    }
}

async fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!("<!doctype html><meta charset=utf-8><title>takt</title><p>{message}</p>");
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes()).await;
}

pub async fn exchange_code(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    client: &OAuthClient,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<SignedIn, GoogleError> {
    let body = post_token_form(
        http,
        &endpoints.token,
        &[
            ("code", code),
            ("client_id", &client.client_id),
            ("client_secret", &client.client_secret),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ],
    )
    .await?;
    let token: TokenResponse = serde_json::from_str(&body).map_err(|e| GoogleError::Decode(e.to_string()))?;
    let id_token = token
        .id_token
        .ok_or_else(|| GoogleError::AuthFlow("Google returned no id_token".into()))?;
    let claims = decode_id_token(&id_token)?;
    let refresh_token = token.refresh_token.ok_or_else(|| {
        GoogleError::AuthFlow(
            "Google returned no refresh token; remove takt under Third-party access in your Google Account and sign in again".into(),
        )
    })?;
    Ok(SignedIn {
        sub: claims.sub,
        email: claims.email,
        refresh_token,
        access_token: token.access_token,
        expires_in: token.expires_in,
    })
}

async fn post_token_form(http: &reqwest::Client, url: &Url, form: &[(&str, &str)]) -> Result<String, GoogleError> {
    let response = http
        .post(url.clone())
        .form(form)
        .send()
        .await
        .map_err(GoogleError::from_reqwest)?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(GoogleError::from_reqwest)?;
    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(token_error(status, &body))
    }
}

fn token_error(status: u16, body: &str) -> GoogleError {
    match serde_json::from_str::<TokenErrorBody>(body) {
        Ok(e) if e.error == "invalid_grant" => GoogleError::InvalidGrant,
        _ => GoogleError::Http { status, body: body.to_owned() },
    }
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    expires_in: u64,
}

/// One account's credentials. Hands out access tokens, refreshing as needed.
pub struct Auth {
    http: reqwest::Client,
    token_url: Url,
    client: OAuthClient,
    refresh_token: String,
    cache: Mutex<Option<(String, Instant)>>,
}

impl Auth {
    pub fn new(http: reqwest::Client, token_url: Url, client: OAuthClient, refresh_token: impl Into<String>) -> Self {
        Self { http, token_url, client, refresh_token: refresh_token.into(), cache: Mutex::new(None) }
    }

    /// Seeds the cache, e.g. with the access token a sign-in just returned.
    pub fn with_access_token(mut self, token: impl Into<String>, expires_in: Duration) -> Self {
        *self.cache.get_mut() = Some((token.into(), Instant::now() + expires_in));
        self
    }

    pub async fn access_token(&self, force_refresh: bool) -> Result<String, GoogleError> {
        let mut cache = self.cache.lock().await;
        if !force_refresh {
            if let Some((token, expires_at)) = cache.as_ref() {
                if Instant::now() + REFRESH_MARGIN < *expires_at {
                    return Ok(token.clone());
                }
            }
        }
        let body = post_token_form(
            &self.http,
            &self.token_url,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &self.refresh_token),
                ("client_id", &self.client.client_id),
                ("client_secret", &self.client.client_secret),
            ],
        )
        .await?;
        let fresh: RefreshResponse = serde_json::from_str(&body).map_err(|e| GoogleError::Decode(e.to_string()))?;
        *cache = Some((fresh.access_token.clone(), Instant::now() + Duration::from_secs(fresh.expires_in)));
        Ok(fresh.access_token)
    }
}
```

- [ ] **Step 4: Implement `token_store.rs`**

`crates/takt-google/src/token_store.rs`:

```rust
//! Where refresh tokens live: the OS keychain, or memory in tests.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::GoogleError;

const SERVICE: &str = "takt";

pub trait TokenStore: Send + Sync {
    fn get(&self, email: &str) -> Result<Option<String>, GoogleError>;
    fn set(&self, email: &str, refresh_token: &str) -> Result<(), GoogleError>;
    fn delete(&self, email: &str) -> Result<(), GoogleError>;
}

/// Windows Credential Manager, macOS Keychain or Secret Service.
pub struct KeyringTokenStore;

fn entry(email: &str) -> Result<keyring::Entry, GoogleError> {
    keyring::Entry::new(SERVICE, email).map_err(|e| GoogleError::TokenStore(e.to_string()))
}

impl TokenStore for KeyringTokenStore {
    fn get(&self, email: &str) -> Result<Option<String>, GoogleError> {
        match entry(email)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(GoogleError::TokenStore(e.to_string())),
        }
    }

    fn set(&self, email: &str, refresh_token: &str) -> Result<(), GoogleError> {
        entry(email)?
            .set_password(refresh_token)
            .map_err(|e| GoogleError::TokenStore(e.to_string()))
    }

    fn delete(&self, email: &str) -> Result<(), GoogleError> {
        match entry(email)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(GoogleError::TokenStore(e.to_string())),
        }
    }
}

#[derive(Default)]
pub struct MemoryTokenStore(Mutex<HashMap<String, String>>);

impl TokenStore for MemoryTokenStore {
    fn get(&self, email: &str) -> Result<Option<String>, GoogleError> {
        Ok(self.0.lock().expect("token map poisoned").get(email).cloned())
    }

    fn set(&self, email: &str, refresh_token: &str) -> Result<(), GoogleError> {
        self.0.lock().expect("token map poisoned").insert(email.to_owned(), refresh_token.to_owned());
        Ok(())
    }

    fn delete(&self, email: &str) -> Result<(), GoogleError> {
        self.0.lock().expect("token map poisoned").remove(email);
        Ok(())
    }
}
```

- [ ] **Step 5: Wire up `lib.rs`**

`crates/takt-google/src/lib.rs`:

```rust
//! Google Calendar v3 and Tasks v1 clients for takt.

mod auth;
mod error;
pub mod model;
mod token_store;

pub use auth::{
    Auth, IdClaims, OAuthClient, Pkce, Redirect, SCOPES, SignedIn, authorization_url, decode_id_token,
    exchange_code, parse_redirect_request_line, sign_in,
};
pub use error::GoogleError;
pub use reqwest::Url;
pub use token_store::{KeyringTokenStore, MemoryTokenStore, TokenStore};

/// Base URLs for Google's OAuth, Calendar and Tasks APIs.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub auth: Url,
    pub token: Url,
    pub calendar: Url,
    pub tasks: Url,
}

impl Endpoints {
    pub fn google() -> Self {
        Self {
            auth: url("https://accounts.google.com/o/oauth2/v2/auth"),
            token: url("https://oauth2.googleapis.com/token"),
            calendar: url("https://www.googleapis.com/calendar/v3/"),
            tasks: url("https://tasks.googleapis.com/tasks/v1/"),
        }
    }

    /// Endpoints under one test server, e.g. a wiremock `MockServer::uri()`.
    pub fn rooted_at(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            auth: url(&format!("{base}/auth")),
            token: url(&format!("{base}/token")),
            calendar: url(&format!("{base}/calendar/v3/")),
            tasks: url(&format!("{base}/tasks/v1/")),
        }
    }
}

fn url(s: &str) -> Url {
    Url::parse(s).expect("static URL is valid")
}
```

- [ ] **Step 6: Run the tests to see them pass**

Run: `cargo test -p takt-google --test auth`
Expected: PASS (the keychain test is ignored). If `keyring::Error::NoEntry` does not resolve, use `keyring::v1::Error::NoEntry`. Then run the ignored test once on this machine: `cargo test -p takt-google --test auth -- --ignored keyring`, expecting PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/takt-google
git commit -m "feat(takt-google): add PKCE loopback sign-in, token refresh and keychain store"
```

---

### Task 6: HTTP client with retries

**Files:**
- Modify: `crates/takt-google/src/lib.rs`
- Create: `crates/takt-google/src/client.rs`, `crates/takt-google/tests/client.rs`

**Interfaces:**
- Consumes: `Auth`, `Endpoints`, `GoogleError`.
- Produces: `RetryPolicy { max_attempts: u32, base_delay: Duration, max_delay: Duration }` (Default 5 / 500 ms / 32 s); `GoogleClient::new(reqwest::Client, Endpoints, Auth)`, `GoogleClient::with_retry(self, RetryPolicy)`, `GoogleClient::endpoints(&self) -> &Endpoints`, `async GoogleClient::get_json(&self, Url) -> Result<serde_json::Value, GoogleError>`.

- [ ] **Step 1: Write the failing tests**

`crates/takt-google/tests/client.rs`:

```rust
use std::time::Duration;

use serde_json::json;
use takt_google::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn oauth() -> OAuthClient {
    OAuthClient { client_id: "cid".into(), client_secret: "secret".into() }
}

fn fast_retry() -> RetryPolicy {
    RetryPolicy { max_attempts: 5, base_delay: Duration::from_millis(1), max_delay: Duration::from_millis(4) }
}

fn client_at(base: &str) -> GoogleClient {
    let endpoints = Endpoints::rooted_at(base);
    let http = reqwest::Client::new();
    let auth = Auth::new(http.clone(), endpoints.token.clone(), oauth(), "rt")
        .with_access_token("t1", Duration::from_secs(3600));
    GoogleClient::new(http, endpoints, auth).with_retry(fast_retry())
}

fn colors_url(server: &MockServer) -> Url {
    Url::parse(&format!("{}/calendar/v3/colors", server.uri())).unwrap()
}

#[tokio::test]
async fn sends_bearer_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/colors"))
        .and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let body = client_at(&server.uri()).get_json(colors_url(&server)).await.unwrap();
    assert_eq!(body, json!({"ok": true}));
}

#[tokio::test]
async fn refreshes_once_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "t2", "expires_in": 3600})))
        .expect(1)
        .mount(&server).await;
    Mock::given(method("GET")).and(header("authorization", "Bearer t2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server).await;
    assert!(client_at(&server.uri()).get_json(colors_url(&server)).await.is_ok());
}

#[tokio::test]
async fn second_401_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    Mock::given(method("POST")).and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "t2", "expires_in": 3600})))
        .mount(&server).await;
    let err = client_at(&server.uri()).get_json(colors_url(&server)).await.unwrap_err();
    assert!(matches!(err, GoogleError::Http { status: 401, .. }));
}

#[tokio::test]
async fn honours_retry_after_on_429() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server).await;
    assert!(client_at(&server.uri()).get_json(colors_url(&server)).await.is_ok());
}

#[tokio::test]
async fn server_errors_retry_then_fail() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).expect(5).mount(&server).await;
    let err = client_at(&server.uri()).get_json(colors_url(&server)).await.unwrap_err();
    assert!(matches!(err, GoogleError::Http { status: 503, .. }));
}

fn error_body(code: u16, reason: &str) -> serde_json::Value {
    json!({"error": {"code": code, "message": reason, "errors": [{"reason": reason, "domain": "usageLimits"}]}})
}

#[tokio::test]
async fn rate_limit_403_retries_then_reports() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body(403, "rateLimitExceeded")))
        .expect(5)
        .mount(&server).await;
    let err = client_at(&server.uri()).get_json(colors_url(&server)).await.unwrap_err();
    assert!(matches!(err, GoogleError::RateLimited));
}

#[tokio::test]
async fn plain_403_is_forbidden_without_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body(403, "forbidden")))
        .expect(1)
        .mount(&server).await;
    let err = client_at(&server.uri()).get_json(colors_url(&server)).await.unwrap_err();
    assert!(matches!(err, GoogleError::Forbidden { ref reason } if reason == "forbidden"));
}

#[tokio::test]
async fn gone_and_not_found_are_typed() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/calendar/v3/gone"))
        .respond_with(ResponseTemplate::new(410)).mount(&server).await;
    Mock::given(method("GET")).and(path("/calendar/v3/missing"))
        .respond_with(ResponseTemplate::new(404)).mount(&server).await;
    let client = client_at(&server.uri());
    let gone = Url::parse(&format!("{}/calendar/v3/gone", server.uri())).unwrap();
    let missing = Url::parse(&format!("{}/calendar/v3/missing", server.uri())).unwrap();
    assert!(matches!(client.get_json(gone).await, Err(GoogleError::Gone)));
    assert!(matches!(client.get_json(missing).await, Err(GoogleError::NotFound)));
}

#[tokio::test]
async fn connection_refused_is_a_network_error() {
    // Nothing listens on the discard port locally.
    let client = client_at("http://127.0.0.1:9");
    let err = client.get_json(Url::parse("http://127.0.0.1:9/calendar/v3/colors").unwrap()).await.unwrap_err();
    assert!(matches!(err, GoogleError::Network(_)));
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt-google --test client`
Expected: compile errors, `GoogleClient` and `RetryPolicy` are not defined.

- [ ] **Step 3: Implement `client.rs`**

`crates/takt-google/src/client.rs`:

```rust
//! Authenticated GET with Google's retry rules.

use std::time::Duration;

use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{StatusCode, Url};
use serde_json::Value;

use crate::{Auth, Endpoints, GoogleError};

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 5, base_delay: Duration::from_millis(500), max_delay: Duration::from_secs(32) }
    }
}

impl RetryPolicy {
    fn delay(&self, attempt: u32) -> Duration {
        let exponential = self.base_delay.saturating_mul(1u32 << attempt.saturating_sub(1).min(16));
        let jitter = rand::random_range(0..=self.base_delay.as_millis() as u64);
        exponential.min(self.max_delay) + Duration::from_millis(jitter)
    }
}

pub struct GoogleClient {
    pub(crate) http: reqwest::Client,
    pub(crate) endpoints: Endpoints,
    auth: Auth,
    retry: RetryPolicy,
}

enum Outcome {
    Retry,
    RefreshAndRetry,
    Fail(GoogleError),
}

impl GoogleClient {
    pub fn new(http: reqwest::Client, endpoints: Endpoints, auth: Auth) -> Self {
        Self { http, endpoints, auth, retry: RetryPolicy::default() }
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    pub async fn get_json(&self, url: Url) -> Result<Value, GoogleError> {
        let mut attempt = 0;
        let mut refreshed = false;
        loop {
            attempt += 1;
            let token = self.auth.access_token(false).await?;
            let response = self
                .http
                .get(url.clone())
                .bearer_auth(&token)
                .send()
                .await
                .map_err(GoogleError::from_reqwest)?;
            let status = response.status();
            let retry_after = retry_after(response.headers());
            let body = response.text().await.map_err(GoogleError::from_reqwest)?;
            if status.is_success() {
                return serde_json::from_str(&body).map_err(|e| GoogleError::Decode(e.to_string()));
            }
            match classify(status, &body) {
                Outcome::RefreshAndRetry if !refreshed => {
                    refreshed = true;
                    attempt -= 1;
                    self.auth.access_token(true).await?;
                }
                Outcome::RefreshAndRetry => {
                    return Err(GoogleError::Http { status: status.as_u16(), body });
                }
                Outcome::Retry if attempt < self.retry.max_attempts => {
                    let wait = retry_after.unwrap_or_else(|| self.retry.delay(attempt));
                    tracing::debug!(%status, ?wait, attempt, "retrying Google request");
                    tokio::time::sleep(wait).await;
                }
                Outcome::Retry if status.is_server_error() => {
                    return Err(GoogleError::Http { status: status.as_u16(), body });
                }
                Outcome::Retry => return Err(GoogleError::RateLimited),
                Outcome::Fail(error) => return Err(error),
            }
        }
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds = headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds))
}

fn error_reason(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value.pointer("/error/errors/0/reason")?.as_str().map(str::to_owned)
}

fn classify(status: StatusCode, body: &str) -> Outcome {
    match status.as_u16() {
        401 => Outcome::RefreshAndRetry,
        404 => Outcome::Fail(GoogleError::NotFound),
        410 => Outcome::Fail(GoogleError::Gone),
        429 => Outcome::Retry,
        403 => match error_reason(body).as_deref() {
            Some("rateLimitExceeded" | "userRateLimitExceeded") => Outcome::Retry,
            reason => Outcome::Fail(GoogleError::Forbidden { reason: reason.unwrap_or("forbidden").to_owned() }),
        },
        500..=599 => Outcome::Retry,
        code => Outcome::Fail(GoogleError::Http { status: code, body: body.to_owned() }),
    }
}
```

In `crates/takt-google/src/lib.rs` add `mod client;` next to the other modules and `pub use client::{GoogleClient, RetryPolicy};` next to the other re-exports.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p takt-google`
Expected: PASS for `model`, `auth` and `client`.

- [ ] **Step 5: Commit**

```bash
git add crates/takt-google
git commit -m "feat(takt-google): add authenticated GET with retry and error classification"
```

---

### Task 7: Calendar and Tasks endpoints

**Files:**
- Modify: `crates/takt-google/src/lib.rs`
- Create: `crates/takt-google/src/api.rs`, `crates/takt-google/tests/api.rs`

**Interfaces:**
- Consumes: `GoogleClient::get_json`, `model::*`.
- Produces (methods on `GoogleClient`):
  - `calendar_list_page(&self, sync_token: Option<&str>, page_token: Option<&str>) -> Result<Page<CalendarListEntry>, GoogleError>`
  - `events_page(&self, calendar_id: &str, sync_token: Option<&str>, page_token: Option<&str>) -> Result<Page<EventFields>, GoogleError>`
  - `colors(&self) -> Result<Colors, GoogleError>`
  - `settings(&self) -> Result<Vec<Setting>, GoogleError>` (all pages)
  - `task_lists(&self) -> Result<Vec<Item<TaskListFields>>, GoogleError>` (all pages)
  - `tasks_page(&self, list_id: &str, updated_min: Option<DateTime<Utc>>, page_token: Option<&str>) -> Result<Page<TaskFields>, GoogleError>`
  - crate-internal `api::join(&Url, &[&str]) -> Result<Url, GoogleError>`

- [ ] **Step 1: Write the failing tests**

`crates/takt-google/tests/api.rs`:

```rust
use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde_json::json;
use takt_google::*;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client_at(server: &MockServer) -> GoogleClient {
    let endpoints = Endpoints::rooted_at(&server.uri());
    let http = reqwest::Client::new();
    let oauth = OAuthClient { client_id: "cid".into(), client_secret: "secret".into() };
    let auth = Auth::new(http.clone(), endpoints.token.clone(), oauth, "rt").with_access_token("t1", Duration::from_secs(3600));
    GoogleClient::new(http, endpoints, auth)
}

fn ok(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

#[tokio::test]
async fn calendar_list_sends_sync_token_and_shows_hidden() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/users/me/calendarList"))
        .and(query_param("syncToken", "cl1"))
        .and(query_param("showHidden", "true"))
        .and(query_param("showDeleted", "true"))
        .respond_with(ok(json!({"items": [{"id": "me@example.com", "deleted": true}], "nextSyncToken": "cl2"})))
        .expect(1)
        .mount(&server).await;
    let page = client_at(&server).calendar_list_page(Some("cl1"), None).await.unwrap();
    assert!(page.items[0].fields.deleted);
    assert_eq!(page.next_sync_token.as_deref(), Some("cl2"));
}

#[tokio::test]
async fn events_page_follows_page_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/primary/events"))
        .and(query_param_is_missing("pageToken"))
        .and(query_param("showDeleted", "true"))
        .and(query_param("maxResults", "2500"))
        .and(query_param_is_missing("singleEvents"))
        .respond_with(ok(json!({"items": [{"id": "e1"}], "nextPageToken": "p2"})))
        .mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/primary/events"))
        .and(query_param("pageToken", "p2"))
        .and(query_param("syncToken", "s0"))
        .respond_with(ok(json!({"items": [{"id": "e2"}], "nextSyncToken": "s1"})))
        .mount(&server).await;
    let client = client_at(&server);
    let first = client.events_page("primary", Some("s0"), None).await.unwrap();
    assert_eq!(first.next_page_token.as_deref(), Some("p2"));
    let second = client.events_page("primary", Some("s0"), first.next_page_token.as_deref()).await.unwrap();
    assert_eq!(second.items[0].fields.id, "e2");
    assert_eq!(second.next_sync_token.as_deref(), Some("s1"));
}

#[tokio::test]
async fn events_page_percent_encodes_calendar_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/en.usa%23holiday@group.v.calendar.google.com/events"))
        .respond_with(ok(json!({"items": [], "nextSyncToken": "s"})))
        .expect(1)
        .mount(&server).await;
    let page = client_at(&server)
        .events_page("en.usa#holiday@group.v.calendar.google.com", None, None)
        .await
        .unwrap();
    assert!(page.items.is_empty());
}

#[tokio::test]
async fn settings_collects_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/calendar/v3/users/me/settings")).and(query_param_is_missing("pageToken"))
        .respond_with(ok(json!({"items": [{"id": "timezone", "value": "Europe/London"}], "nextPageToken": "n"})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/calendar/v3/users/me/settings")).and(query_param("pageToken", "n"))
        .respond_with(ok(json!({"items": [{"id": "format24HourTime", "value": "true"}]})))
        .mount(&server).await;
    let settings = client_at(&server).settings().await.unwrap();
    let ids: Vec<_> = settings.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["timezone", "format24HourTime"]);
}

#[tokio::test]
async fn colors_reads_palette() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/calendar/v3/colors"))
        .respond_with(ok(json!({"event": {"11": {"background": "#dc2127", "foreground": "#1d1d1d"}}})))
        .mount(&server).await;
    let colors = client_at(&server).colors().await.unwrap();
    assert_eq!(colors.event["11"].background, "#dc2127");
}

#[tokio::test]
async fn task_lists_collects_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/tasks/v1/users/@me/lists")).and(query_param_is_missing("pageToken"))
        .respond_with(ok(json!({"items": [{"id": "L1", "title": "My Tasks"}], "nextPageToken": "n"})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/tasks/v1/users/@me/lists")).and(query_param("pageToken", "n"))
        .respond_with(ok(json!({"items": [{"id": "L2", "title": "Errands"}]})))
        .mount(&server).await;
    let lists = client_at(&server).task_lists().await.unwrap();
    assert_eq!(lists.len(), 2);
}

#[tokio::test]
async fn tasks_page_sends_updated_min_and_flags() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/lists/L1/tasks"))
        .and(query_param("updatedMin", "2026-10-17T10:00:00Z"))
        .and(query_param("showCompleted", "true"))
        .and(query_param("showDeleted", "true"))
        .and(query_param("showHidden", "true"))
        .respond_with(ok(json!({"items": [{"id": "t1", "title": "Pay rent"}]})))
        .expect(1)
        .mount(&server).await;
    let since = Utc.with_ymd_and_hms(2026, 10, 17, 10, 0, 0).unwrap();
    let page = client_at(&server).tasks_page("L1", Some(since), None).await.unwrap();
    assert_eq!(page.items[0].fields.title.as_deref(), Some("Pay rent"));
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt-google --test api`
Expected: compile errors, the endpoint methods do not exist.

- [ ] **Step 3: Implement `api.rs`**

`crates/takt-google/src/api.rs`:

```rust
//! The read endpoints sub-project 1 uses.

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::Url;

use crate::model::{CalendarListEntry, Colors, EventFields, Item, Page, Setting, TaskFields, TaskListFields};
use crate::{GoogleClient, GoogleError};

impl GoogleClient {
    pub async fn calendar_list_page(
        &self,
        sync_token: Option<&str>,
        page_token: Option<&str>,
    ) -> Result<Page<CalendarListEntry>, GoogleError> {
        let mut url = join(&self.endpoints.calendar, &["users", "me", "calendarList"])?;
        url.query_pairs_mut()
            .append_pair("maxResults", "250")
            .append_pair("showDeleted", "true")
            .append_pair("showHidden", "true");
        push_tokens(&mut url, sync_token, page_token);
        Page::from_value(self.get_json(url).await?)
    }

    pub async fn events_page(
        &self,
        calendar_id: &str,
        sync_token: Option<&str>,
        page_token: Option<&str>,
    ) -> Result<Page<EventFields>, GoogleError> {
        let mut url = join(&self.endpoints.calendar, &["calendars", calendar_id, "events"])?;
        url.query_pairs_mut()
            .append_pair("maxResults", "2500")
            .append_pair("showDeleted", "true");
        push_tokens(&mut url, sync_token, page_token);
        Page::from_value(self.get_json(url).await?)
    }

    pub async fn colors(&self) -> Result<Colors, GoogleError> {
        let url = join(&self.endpoints.calendar, &["colors"])?;
        serde_json::from_value(self.get_json(url).await?).map_err(|e| GoogleError::Decode(e.to_string()))
    }

    pub async fn settings(&self) -> Result<Vec<Setting>, GoogleError> {
        let mut settings = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = join(&self.endpoints.calendar, &["users", "me", "settings"])?;
            url.query_pairs_mut().append_pair("maxResults", "250");
            push_tokens(&mut url, None, page_token.as_deref());
            let page: Page<Setting> = Page::from_value(self.get_json(url).await?)?;
            settings.extend(page.items.into_iter().map(|item| item.fields));
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(settings),
            }
        }
    }

    pub async fn task_lists(&self) -> Result<Vec<Item<TaskListFields>>, GoogleError> {
        let mut lists = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = join(&self.endpoints.tasks, &["users", "@me", "lists"])?;
            url.query_pairs_mut().append_pair("maxResults", "100");
            push_tokens(&mut url, None, page_token.as_deref());
            let page: Page<TaskListFields> = Page::from_value(self.get_json(url).await?)?;
            lists.extend(page.items);
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(lists),
            }
        }
    }

    pub async fn tasks_page(
        &self,
        list_id: &str,
        updated_min: Option<DateTime<Utc>>,
        page_token: Option<&str>,
    ) -> Result<Page<TaskFields>, GoogleError> {
        let mut url = join(&self.endpoints.tasks, &["lists", list_id, "tasks"])?;
        url.query_pairs_mut()
            .append_pair("maxResults", "100")
            .append_pair("showCompleted", "true")
            .append_pair("showDeleted", "true")
            .append_pair("showHidden", "true");
        if let Some(since) = updated_min {
            url.query_pairs_mut()
                .append_pair("updatedMin", &since.to_rfc3339_opts(SecondsFormat::Secs, true));
        }
        push_tokens(&mut url, None, page_token);
        Page::from_value(self.get_json(url).await?)
    }
}

/// Appends path segments to a base ending in `/`. Segments are percent-encoded,
/// so calendar IDs such as `en.usa#holiday@group.v.calendar.google.com` are safe.
pub(crate) fn join(base: &Url, segments: &[&str]) -> Result<Url, GoogleError> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| GoogleError::Decode(format!("base URL cannot take a path: {base}")))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

fn push_tokens(url: &mut Url, sync_token: Option<&str>, page_token: Option<&str>) {
    let mut q = url.query_pairs_mut();
    if let Some(token) = sync_token {
        q.append_pair("syncToken", token);
    }
    if let Some(token) = page_token {
        q.append_pair("pageToken", token);
    }
}
```

In `crates/takt-google/src/lib.rs` add `mod api;`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p takt-google`
Expected: PASS. If `events_page_percent_encodes_calendar_id` fails because wiremock compares the decoded path, change only that matcher to `path_regex(r"^/calendar/v3/calendars/en\.usa(%23|#)holiday@group\.v\.calendar\.google\.com/events$")`, and add an assertion in a unit test inside `api.rs` that `join(&base, &["calendars", "en.usa#holiday@group.v.calendar.google.com", "events"])` produces a path containing `%23`.

- [ ] **Step 5: Commit**

```bash
git add crates/takt-google
git commit -m "feat(takt-google): add calendar list, events, colours, settings and tasks endpoints"
```

---

### Task 8: Store schema, accounts, calendars, settings and colours

**Files:**
- Modify: `crates/takt-store/Cargo.toml`, `crates/takt-store/src/lib.rs`
- Create: `crates/takt-store/migrations/001_init.sql`, `crates/takt-store/src/error.rs`, `crates/takt-store/src/types.rs`, `crates/takt-store/src/accounts.rs`, `crates/takt-store/tests/accounts.rs`

**Interfaces:**
- Consumes: `takt_google::model::{Item, CalendarListEntry, Setting, Colors}`, `takt_recur::{When, Range}`.
- Produces (`takt_store`):
  - `Store::open(&Path)`, `Store::open_in_memory()`, `Store::open_read_only(&Path)` → `Result<Store, StoreError>`
  - `enum StoreError { Sql(rusqlite::Error), Migration(rusqlite_migration::Error), Json(serde_json::Error), Corrupt(String), Io(String) }`
  - `enum AccountStatus { Ok, NeedsReauth }`; `AccountRow { account_id, email, status }`; `CalendarRow { account_id, calendar_id, summary, time_zone, background_color, foreground_color, access_role, selected, hidden, primary }`
  - `TimeSpan { start: DateTime<Utc>, end: Option<DateTime<Utc>> }` with `union`, `overlaps(start, end)`
  - `ChangeSet { calendars: BTreeSet<String>, task_lists: BTreeSet<String>, span: Option<TimeSpan>, everything: bool }` with `is_empty()`, `affects(start, end)`, `merge(other)`
  - `ColorPalette { calendar, event: BTreeMap<String, (String, String)> }` (id → (background, foreground))
  - `day_start_utc(NaiveDate, Tz) -> DateTime<Utc>`, `local_days(from, to, Tz) -> Range`, `display_instant(&When, Tz) -> DateTime<Utc>`
  - Methods: `upsert_account(&mut self, account_id, email)`, `accounts()`, `set_account_status(account_id, AccountStatus)`, `remove_account(account_id)`, `calendar_list_sync_token(account_id)`, `apply_calendar_list_page(account_id, &[Item<CalendarListEntry>], next_sync_token: Option<&str>) -> Result<ChangeSet>`, `calendars(account_id: Option<&str>)`, `remove_calendar(account_id, calendar_id)`, `reset_account_sync(account_id)`, `put_settings(account_id, &[Setting])`, `settings(account_id) -> BTreeMap<String, String>`, `put_colors(account_id, &Colors)`, `palette(account_id) -> ColorPalette`
  - Re-exports `takt_recur::{Range, When}`.

- [ ] **Step 1: Add dependencies**

`crates/takt-store/Cargo.toml`, below `[package]`:

```toml
[dependencies]
chrono.workspace = true
chrono-tz.workspace = true
rusqlite.workspace = true
rusqlite_migration.workspace = true
serde_json.workspace = true
takt-google.workspace = true
takt-recur.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: Write the schema**

`crates/takt-store/migrations/001_init.sql`:

```sql
CREATE TABLE accounts (
    account_id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'ok',
    calendar_list_sync_token TEXT,
    added_at TEXT NOT NULL
);

CREATE TABLE account_settings (
    account_id TEXT NOT NULL REFERENCES accounts (account_id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (account_id, key)
);

CREATE TABLE colors (
    account_id TEXT NOT NULL REFERENCES accounts (account_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('calendar', 'event')),
    color_id TEXT NOT NULL,
    background TEXT NOT NULL,
    foreground TEXT NOT NULL,
    PRIMARY KEY (account_id, kind, color_id)
);

CREATE TABLE calendars (
    account_id TEXT NOT NULL REFERENCES accounts (account_id) ON DELETE CASCADE,
    calendar_id TEXT NOT NULL,
    raw_json TEXT NOT NULL,
    summary TEXT NOT NULL,
    time_zone TEXT,
    background_color TEXT,
    foreground_color TEXT,
    access_role TEXT,
    selected INTEGER NOT NULL DEFAULT 0,
    hidden INTEGER NOT NULL DEFAULT 0,
    is_primary INTEGER NOT NULL DEFAULT 0,
    events_sync_token TEXT,
    PRIMARY KEY (account_id, calendar_id)
);

-- start_utc, end_utc, series_end_utc: unix seconds. All-day dates are stored as UTC midnight.
-- original_start: RFC 3339 UTC for timed series, YYYY-MM-DD for all-day.
CREATE TABLE events (
    account_id TEXT NOT NULL,
    calendar_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    raw_json TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('single', 'master', 'exception')),
    status TEXT,
    ical_uid TEXT,
    recurring_event_id TEXT,
    original_start TEXT,
    start_utc INTEGER,
    end_utc INTEGER,
    all_day INTEGER NOT NULL DEFAULT 0,
    series_end_utc INTEGER,
    etag TEXT,
    updated TEXT,
    PRIMARY KEY (account_id, calendar_id, event_id),
    FOREIGN KEY (account_id, calendar_id) REFERENCES calendars (account_id, calendar_id) ON DELETE CASCADE
);

CREATE INDEX events_range ON events (kind, start_utc, end_utc);
CREATE INDEX events_series ON events (kind, start_utc, series_end_utc);
CREATE INDEX events_exceptions ON events (account_id, calendar_id, recurring_event_id);

CREATE TABLE task_lists (
    account_id TEXT NOT NULL REFERENCES accounts (account_id) ON DELETE CASCADE,
    list_id TEXT NOT NULL,
    raw_json TEXT NOT NULL,
    title TEXT NOT NULL,
    updated TEXT,
    updated_min_checkpoint TEXT,
    PRIMARY KEY (account_id, list_id)
);

CREATE TABLE tasks (
    account_id TEXT NOT NULL,
    list_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    raw_json TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT,
    due_date TEXT,
    parent TEXT,
    position TEXT,
    completed TEXT,
    hidden INTEGER NOT NULL DEFAULT 0,
    updated TEXT,
    PRIMARY KEY (account_id, list_id, task_id),
    FOREIGN KEY (account_id, list_id) REFERENCES task_lists (account_id, list_id) ON DELETE CASCADE
);

CREATE INDEX tasks_due ON tasks (due_date);
```

- [ ] **Step 3: Write the failing tests**

`crates/takt-store/tests/accounts.rs`:

```rust
use serde_json::json;
use takt_google::model::{CalendarListEntry, Colors, Item, Setting};
use takt_store::*;

fn cal(value: serde_json::Value) -> Item<CalendarListEntry> {
    Item::from_value(value).unwrap()
}

fn with_account() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    store
}

#[test]
fn fresh_store_has_no_accounts() {
    assert!(Store::open_in_memory().unwrap().accounts().unwrap().is_empty());
}

#[test]
fn on_disk_store_is_readable_from_a_read_only_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("takt.sqlite3");
    let mut writer = Store::open(&path).unwrap();
    writer.upsert_account("acc", "me@example.com").unwrap();
    let reader = Store::open_read_only(&path).unwrap();
    assert_eq!(reader.accounts().unwrap().len(), 1);
}

#[test]
fn upsert_account_updates_email_and_status() {
    let mut store = with_account();
    store.set_account_status("acc", AccountStatus::NeedsReauth).unwrap();
    assert_eq!(store.accounts().unwrap()[0].status, AccountStatus::NeedsReauth);
    store.upsert_account("acc", "renamed@example.com").unwrap();
    let accounts = store.accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].email, "renamed@example.com");
    assert_eq!(accounts[0].status, AccountStatus::Ok);
}

#[test]
fn calendar_list_page_upserts_deletes_and_stores_token() {
    let mut store = with_account();
    let changes = store
        .apply_calendar_list_page(
            "acc",
            &[
                cal(json!({"id": "me@example.com", "summary": "Me", "primary": true, "selected": true, "backgroundColor": "#9fe1e7", "timeZone": "Europe/London"})),
                cal(json!({"id": "team@group.calendar.google.com", "summary": "Team", "summaryOverride": "Work team"})),
            ],
            Some("cl1"),
        )
        .unwrap();
    assert!(changes.everything);
    assert_eq!(store.calendar_list_sync_token("acc").unwrap().as_deref(), Some("cl1"));
    let calendars = store.calendars(Some("acc")).unwrap();
    assert_eq!(calendars.len(), 2);
    assert_eq!(calendars[0].calendar_id, "me@example.com", "primary sorts first");
    assert!(calendars[0].selected && calendars[0].primary);
    assert_eq!(calendars[1].summary, "Work team");

    store
        .apply_calendar_list_page("acc", &[cal(json!({"id": "team@group.calendar.google.com", "deleted": true}))], Some("cl2"))
        .unwrap();
    assert_eq!(store.calendars(Some("acc")).unwrap().len(), 1);
    assert_eq!(store.calendar_list_sync_token("acc").unwrap().as_deref(), Some("cl2"));
}

#[test]
fn settings_and_palette_round_trip() {
    let mut store = with_account();
    store
        .put_settings("acc", &[Setting { id: "format24HourTime".into(), value: "false".into() }])
        .unwrap();
    assert_eq!(store.settings("acc").unwrap()["format24HourTime"], "false");
    let colors: Colors = serde_json::from_value(json!({"event": {"11": {"background": "#dc2127", "foreground": "#1d1d1d"}}})).unwrap();
    store.put_colors("acc", &colors).unwrap();
    assert_eq!(store.palette("acc").unwrap().event["11"].0, "#dc2127");
}

#[test]
fn removing_an_account_removes_its_calendars() {
    let mut store = with_account();
    store.apply_calendar_list_page("acc", &[cal(json!({"id": "c"}))], None).unwrap();
    store.remove_account("acc").unwrap();
    assert!(store.accounts().unwrap().is_empty());
    assert!(store.calendars(None).unwrap().is_empty());
}

#[test]
fn reset_account_sync_clears_calendars_and_token() {
    let mut store = with_account();
    store.apply_calendar_list_page("acc", &[cal(json!({"id": "c"}))], Some("cl1")).unwrap();
    store.reset_account_sync("acc").unwrap();
    assert!(store.calendars(Some("acc")).unwrap().is_empty());
    assert_eq!(store.calendar_list_sync_token("acc").unwrap(), None);
}

#[test]
fn day_start_utc_uses_local_midnight() {
    use chrono::{NaiveDate, TimeZone, Utc};
    let d = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    assert_eq!(day_start_utc(d, chrono_tz::Pacific::Honolulu), Utc.with_ymd_and_hms(2026, 10, 20, 10, 0, 0).unwrap());
}

#[test]
fn day_start_utc_skips_missing_midnight() {
    use chrono::{NaiveDate, TimeZone, Utc};
    // Chile moves clocks from 00:00 to 01:00 on 6 September 2026.
    let d = NaiveDate::from_ymd_opt(2026, 9, 6).unwrap();
    assert_eq!(day_start_utc(d, chrono_tz::America::Santiago), Utc.with_ymd_and_hms(2026, 9, 6, 4, 0, 0).unwrap());
}
```

- [ ] **Step 4: Run the tests to see them fail**

Run: `cargo test -p takt-store --test accounts`
Expected: compile errors for the missing items.

- [ ] **Step 5: Implement `error.rs` and `types.rs`**

`crates/takt-store/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Migration(#[from] rusqlite_migration::Error),
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("stored value is invalid: {0}")]
    Corrupt(String),
    #[error("cannot create the database directory: {0}")]
    Io(String),
}
```

`crates/takt-store/src/types.rs`:

```rust
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use takt_recur::{Range, When};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    Ok,
    NeedsReauth,
}

impl AccountStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            AccountStatus::Ok => "ok",
            AccountStatus::NeedsReauth => "needs_reauth",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        if value == "needs_reauth" { AccountStatus::NeedsReauth } else { AccountStatus::Ok }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub account_id: String,
    pub email: String,
    pub status: AccountStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarRow {
    pub account_id: String,
    pub calendar_id: String,
    pub summary: String,
    pub time_zone: Option<String>,
    pub background_color: Option<String>,
    pub foreground_color: Option<String>,
    pub access_role: Option<String>,
    pub selected: bool,
    pub hidden: bool,
    pub primary: bool,
}

/// A stretch of time. `end: None` runs forever (an endless series).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSpan {
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
}

impl TimeSpan {
    pub fn union(self, other: TimeSpan) -> TimeSpan {
        TimeSpan {
            start: self.start.min(other.start),
            end: match (self.end, other.end) {
                (Some(a), Some(b)) => Some(a.max(b)),
                _ => None,
            },
        }
    }

    pub fn overlaps(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        self.start < end && self.end.is_none_or(|e| e > start)
    }
}

/// What one write touched, so the UI knows whether to re-read its range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub calendars: BTreeSet<String>,
    pub task_lists: BTreeSet<String>,
    /// Union of the times that changed.
    pub span: Option<TimeSpan>,
    /// Something changed whose times are unknown (e.g. a calendar's colour): re-read every range.
    pub everything: bool,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.calendars.is_empty() && self.task_lists.is_empty() && !self.everything
    }

    pub fn affects(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        self.everything || self.span.is_some_and(|s| s.overlaps(start, end))
    }

    pub fn merge(&mut self, other: ChangeSet) {
        self.calendars.extend(other.calendars);
        self.task_lists.extend(other.task_lists);
        self.everything |= other.everything;
        if let Some(span) = other.span {
            self.add_span(span);
        }
    }

    pub(crate) fn touch_calendar(&mut self, calendar_id: &str, span: Option<TimeSpan>) {
        self.calendars.insert(calendar_id.to_owned());
        match span {
            Some(span) => self.add_span(span),
            None => self.everything = true,
        }
    }

    pub(crate) fn touch_task_list(&mut self, list_id: &str, span: Option<TimeSpan>) {
        self.task_lists.insert(list_id.to_owned());
        match span {
            Some(span) => self.add_span(span),
            None => self.everything = true,
        }
    }

    fn add_span(&mut self, span: TimeSpan) {
        self.span = Some(self.span.map_or(span, |current| current.union(span)));
    }
}

/// Google's colour palette for one account: colour ID -> (background, foreground).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColorPalette {
    pub calendar: BTreeMap<String, (String, String)>,
    pub event: BTreeMap<String, (String, String)>,
}

/// One occurrence ready for display. `event_id` names the row whose JSON
/// describes it: the series for generated occurrences, the exception otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub account_id: String,
    pub calendar_id: String,
    pub event_id: String,
    pub instance_id: String,
    pub start: When,
    pub end: When,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub color_id: Option<String>,
    pub self_response: Option<String>,
    pub recurrence_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItem {
    pub account_id: String,
    pub list_id: String,
    pub list_title: String,
    pub task_id: String,
    pub title: String,
    pub due_date: NaiveDate,
    pub completed: bool,
}

/// The UTC instant a local date begins in `tz`. When local midnight does not
/// exist (a DST jump at 00:00), the first valid local time that day.
pub fn day_start_utc(date: NaiveDate, tz: Tz) -> DateTime<Utc> {
    let midnight = date.and_time(NaiveTime::MIN);
    (0..=12)
        .map(|quarter| midnight + TimeDelta::minutes(15 * quarter))
        .find_map(|local| tz.from_local_datetime(&local).earliest())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| midnight.and_utc())
}

/// The range covering local dates `[from, to)` in `tz`.
pub fn local_days(from: NaiveDate, to: NaiveDate, tz: Tz) -> Range {
    Range { start: day_start_utc(from, tz), end: day_start_utc(to, tz) }
}

/// The instant a `When` begins as seen in `tz`. All-day dates start at local midnight.
pub fn display_instant(when: &When, tz: Tz) -> DateTime<Utc> {
    match when {
        When::At(t) => *t,
        When::Date(d) => day_start_utc(*d, tz),
    }
}
```

- [ ] **Step 6: Implement `lib.rs` and `accounts.rs`**

`crates/takt-store/src/lib.rs`:

```rust
//! SQLite cache of Google data for takt.
//!
//! Every row keeps Google's JSON in `raw_json`. The other columns are indexes
//! derived from it.

mod accounts;
mod error;
mod types;

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use rusqlite_migration::{M, Migrations};

pub use error::StoreError;
pub use takt_recur::{Range, When};
pub use types::*;

pub struct Store {
    pub(crate) conn: Connection,
}

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(include_str!("../migrations/001_init.sql"))])
}

impl Store {
    /// Opens (creating if needed) the database the sync engine writes to.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    /// A reader for the UI. The database must already exist and be migrated.
    pub fn open_read_only(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        migrations().to_latest(&mut conn)?;
        Ok(Self { conn })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn migrations_are_valid() {
        assert!(super::migrations().validate().is_ok());
    }
}
```

`crates/takt-store/src/accounts.rs`:

```rust
use std::collections::BTreeMap;

use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use takt_google::model::{CalendarListEntry, Colors, Item, Setting};

use crate::{AccountRow, AccountStatus, CalendarRow, ChangeSet, ColorPalette, Store, StoreError};

impl Store {
    pub fn upsert_account(&mut self, account_id: &str, email: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO accounts (account_id, email, status, added_at) VALUES (?1, ?2, 'ok', ?3)
             ON CONFLICT (account_id) DO UPDATE SET email = excluded.email, status = 'ok'",
            params![account_id, email, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn accounts(&self) -> Result<Vec<AccountRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT account_id, email, status FROM accounts ORDER BY added_at, email")?;
        let rows = stmt.query_map([], |row| {
            Ok(AccountRow {
                account_id: row.get(0)?,
                email: row.get(1)?,
                status: AccountStatus::parse(&row.get::<_, String>(2)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_account_status(&mut self, account_id: &str, status: AccountStatus) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE accounts SET status = ?2 WHERE account_id = ?1",
            params![account_id, status.as_str()],
        )?;
        Ok(())
    }

    pub fn remove_account(&mut self, account_id: &str) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM accounts WHERE account_id = ?1", params![account_id])?;
        Ok(())
    }

    pub fn calendar_list_sync_token(&self, account_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT calendar_list_sync_token FROM accounts WHERE account_id = ?1",
                params![account_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Applies one page of `calendarList.list`. Uses `ON CONFLICT DO UPDATE`,
    /// never `REPLACE`: a replace would delete the row and cascade to its events.
    pub fn apply_calendar_list_page(
        &mut self,
        account_id: &str,
        items: &[Item<CalendarListEntry>],
        next_sync_token: Option<&str>,
    ) -> Result<ChangeSet, StoreError> {
        let tx = self.conn.transaction()?;
        let mut changes = ChangeSet::default();
        for item in items {
            let c = &item.fields;
            changes.touch_calendar(&c.id, None);
            if c.deleted {
                tx.execute(
                    "DELETE FROM calendars WHERE account_id = ?1 AND calendar_id = ?2",
                    params![account_id, c.id],
                )?;
                continue;
            }
            tx.execute(
                "INSERT INTO calendars (account_id, calendar_id, raw_json, summary, time_zone, background_color,
                     foreground_color, access_role, selected, hidden, is_primary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (account_id, calendar_id) DO UPDATE SET
                     raw_json = excluded.raw_json, summary = excluded.summary, time_zone = excluded.time_zone,
                     background_color = excluded.background_color, foreground_color = excluded.foreground_color,
                     access_role = excluded.access_role, selected = excluded.selected, hidden = excluded.hidden,
                     is_primary = excluded.is_primary",
                params![
                    account_id,
                    c.id,
                    item.raw.to_string(),
                    c.display_name(),
                    c.time_zone,
                    c.background_color,
                    c.foreground_color,
                    c.access_role,
                    c.selected,
                    c.hidden,
                    c.primary,
                ],
            )?;
        }
        if let Some(token) = next_sync_token {
            tx.execute(
                "UPDATE accounts SET calendar_list_sync_token = ?2 WHERE account_id = ?1",
                params![account_id, token],
            )?;
        }
        tx.commit()?;
        Ok(changes)
    }

    /// Calendars for one account, or all accounts. Primary first, then by name.
    pub fn calendars(&self, account_id: Option<&str>) -> Result<Vec<CalendarRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.account_id, c.calendar_id, c.summary, c.time_zone, c.background_color, c.foreground_color,
                    c.access_role, c.selected, c.hidden, c.is_primary
             FROM calendars c JOIN accounts a ON a.account_id = c.account_id
             WHERE ?1 IS NULL OR c.account_id = ?1
             ORDER BY a.added_at, a.email, c.is_primary DESC, c.summary COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok(CalendarRow {
                account_id: row.get(0)?,
                calendar_id: row.get(1)?,
                summary: row.get(2)?,
                time_zone: row.get(3)?,
                background_color: row.get(4)?,
                foreground_color: row.get(5)?,
                access_role: row.get(6)?,
                selected: row.get(7)?,
                hidden: row.get(8)?,
                primary: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn remove_calendar(&mut self, account_id: &str, calendar_id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM calendars WHERE account_id = ?1 AND calendar_id = ?2",
            params![account_id, calendar_id],
        )?;
        Ok(())
    }

    /// After a 410 on the calendar list: drop the account's calendars (and
    /// their events) and its token, so the next round is a full sync.
    pub fn reset_account_sync(&mut self, account_id: &str) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM calendars WHERE account_id = ?1", params![account_id])?;
        tx.execute(
            "UPDATE accounts SET calendar_list_sync_token = NULL WHERE account_id = ?1",
            params![account_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn put_settings(&mut self, account_id: &str, settings: &[Setting]) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM account_settings WHERE account_id = ?1", params![account_id])?;
        for s in settings {
            tx.execute(
                "INSERT INTO account_settings (account_id, key, value) VALUES (?1, ?2, ?3)",
                params![account_id, s.id, s.value],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn settings(&self, account_id: &str) -> Result<BTreeMap<String, String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM account_settings WHERE account_id = ?1")?;
        let rows = stmt.query_map(params![account_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn put_colors(&mut self, account_id: &str, colors: &Colors) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM colors WHERE account_id = ?1", params![account_id])?;
        for (kind, map) in [("calendar", &colors.calendar), ("event", &colors.event)] {
            for (id, def) in map {
                tx.execute(
                    "INSERT INTO colors (account_id, kind, color_id, background, foreground) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![account_id, kind, id, def.background, def.foreground],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn palette(&self, account_id: &str) -> Result<ColorPalette, StoreError> {
        let mut palette = ColorPalette::default();
        let mut stmt = self
            .conn
            .prepare("SELECT kind, color_id, background, foreground FROM colors WHERE account_id = ?1")?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get(2)?, row.get(3)?))
        })?;
        for row in rows {
            let (kind, id, background, foreground) = row?;
            let map = if kind == "calendar" { &mut palette.calendar } else { &mut palette.event };
            map.insert(id, (background, foreground));
        }
        Ok(palette)
    }
}
```

Add `mod accounts;` is already listed in `lib.rs` above.

- [ ] **Step 7: Run the tests to see them pass**

Run: `cargo test -p takt-store`
Expected: PASS, including `tests::migrations_are_valid`.

- [ ] **Step 8: Commit**

```bash
git add crates/takt-store Cargo.lock
git commit -m "feat(takt-store): add schema, accounts, calendars, settings and colours"
```

---

### Task 9: Store event pages

**Files:**
- Modify: `crates/takt-store/src/lib.rs`
- Create: `crates/takt-store/src/convert.rs`, `crates/takt-store/src/events.rs`, `crates/takt-store/tests/events.rs`

**Interfaces:**
- Consumes: Task 8 `Store`, `ChangeSet`, `TimeSpan`; `takt_recur::{series_end, Master, Exception, When}`; `takt_google::model::{Item, EventFields, EventDateTime}`.
- Produces:
  - `Store::events_sync_token(&self, account_id, calendar_id) -> Result<Option<String>>`
  - `Store::apply_events_page(&mut self, account_id, calendar_id, &[Item<EventFields>], next_sync_token: Option<&str>) -> Result<ChangeSet>`
  - `Store::reset_calendar_events(&mut self, account_id, calendar_id) -> Result<()>`
  - `Store::event_raw(&self, account_id, calendar_id, event_id) -> Result<Option<serde_json::Value>>`
  - `Store::event_ids(&self, account_id, calendar_id) -> Result<Vec<String>>` (sorted)
  - crate-internal (`convert`): `when_of(&EventDateTime) -> Option<When>`, `encode_when`, `zone(Option<&str>) -> Option<Tz>`, `master_of(&EventFields, fallback: Tz) -> Option<Master>`, `exception_of(&EventFields) -> Option<Exception>`, `EventIndex::of(&EventFields, Tz)`, `span_of(kind, start, end, all_day, series_end) -> Option<TimeSpan>`

- [ ] **Step 1: Write the failing tests**

`crates/takt-store/tests/events.rs`:

```rust
use chrono::{TimeZone, Utc};
use serde_json::json;
use takt_google::model::{CalendarListEntry, EventFields, Item};
use takt_store::*;

fn ev(value: serde_json::Value) -> Item<EventFields> {
    Item::from_value(value).unwrap()
}

fn seeded() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    let cal: Item<CalendarListEntry> =
        Item::from_value(json!({"id": "cal", "summary": "Me", "selected": true, "timeZone": "Europe/London"})).unwrap();
    store.apply_calendar_list_page("acc", &[cal], None).unwrap();
    store
}

fn standup() -> serde_json::Value {
    json!({
        "id": "s1", "status": "confirmed", "summary": "Standup",
        "start": {"dateTime": "2026-10-19T09:00:00+01:00"}, "end": {"dateTime": "2026-10-19T09:30:00+01:00"}
    })
}

fn weekly_master() -> serde_json::Value {
    json!({
        "id": "m", "status": "confirmed", "summary": "1:1",
        "start": {"dateTime": "2026-10-05T10:00:00+01:00", "timeZone": "Europe/London"},
        "end": {"dateTime": "2026-10-05T10:30:00+01:00", "timeZone": "Europe/London"},
        "recurrence": ["RRULE:FREQ=WEEKLY"]
    })
}

#[test]
fn single_event_is_stored_with_token_and_reported() {
    let mut store = seeded();
    let changes = store.apply_events_page("acc", "cal", &[ev(standup())], Some("t1")).unwrap();
    assert_eq!(store.events_sync_token("acc", "cal").unwrap().as_deref(), Some("t1"));
    assert!(changes.calendars.contains("cal"));
    assert!(changes.affects(
        Utc.with_ymd_and_hms(2026, 10, 19, 8, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 10, 19, 8, 10, 0).unwrap()
    ));
    assert!(!changes.everything);
    assert_eq!(store.event_raw("acc", "cal", "s1").unwrap(), Some(standup()));
}

#[test]
fn page_without_token_keeps_previous_token() {
    let mut store = seeded();
    store.apply_events_page("acc", "cal", &[], Some("t0")).unwrap();
    store.apply_events_page("acc", "cal", &[ev(standup())], None).unwrap();
    assert_eq!(store.events_sync_token("acc", "cal").unwrap().as_deref(), Some("t0"));
}

#[test]
fn applying_a_page_twice_gives_the_same_state() {
    let mut store = seeded();
    let page = [ev(standup()), ev(weekly_master())];
    store.apply_events_page("acc", "cal", &page, None).unwrap();
    store.apply_events_page("acc", "cal", &page, Some("t1")).unwrap();
    assert_eq!(store.event_ids("acc", "cal").unwrap(), ["m", "s1"]);
}

#[test]
fn cancelled_single_is_deleted_and_old_time_reported() {
    let mut store = seeded();
    store.apply_events_page("acc", "cal", &[ev(standup())], None).unwrap();
    let changes = store
        .apply_events_page("acc", "cal", &[ev(json!({"id": "s1", "status": "cancelled"}))], None)
        .unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
    assert!(changes.affects(
        Utc.with_ymd_and_hms(2026, 10, 19, 8, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 10, 19, 8, 10, 0).unwrap()
    ));
}

#[test]
fn minimal_cancelled_master_removes_series_and_exceptions() {
    let mut store = seeded();
    let exception = json!({
        "id": "m_20261012T090000Z", "status": "confirmed", "summary": "1:1 (moved)", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"},
        "start": {"dateTime": "2026-10-13T10:00:00+01:00"}, "end": {"dateTime": "2026-10-13T10:30:00+01:00"}
    });
    store.apply_events_page("acc", "cal", &[ev(weekly_master()), ev(exception)], None).unwrap();
    assert_eq!(store.event_ids("acc", "cal").unwrap().len(), 2);
    store
        .apply_events_page("acc", "cal", &[ev(json!({"id": "m", "status": "cancelled"}))], None)
        .unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
}

#[test]
fn cancelled_exception_is_kept() {
    let mut store = seeded();
    let cancelled = json!({
        "id": "m_20261012T090000Z", "status": "cancelled", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"}
    });
    store.apply_events_page("acc", "cal", &[ev(weekly_master()), ev(cancelled)], None).unwrap();
    assert_eq!(store.event_ids("acc", "cal").unwrap(), ["m", "m_20261012T090000Z"]);
}

#[test]
fn endless_series_reports_an_unbounded_span() {
    let mut store = seeded();
    let changes = store.apply_events_page("acc", "cal", &[ev(weekly_master())], None).unwrap();
    assert_eq!(changes.span.unwrap().end, None);
}

#[test]
fn reset_calendar_events_clears_rows_and_token() {
    let mut store = seeded();
    store.apply_events_page("acc", "cal", &[ev(standup())], Some("t1")).unwrap();
    store.reset_calendar_events("acc", "cal").unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
    assert_eq!(store.events_sync_token("acc", "cal").unwrap(), None);
}

#[test]
fn calendar_list_update_keeps_events_and_token() {
    let mut store = seeded();
    store.apply_events_page("acc", "cal", &[ev(standup())], Some("t1")).unwrap();
    let recoloured: Item<CalendarListEntry> =
        Item::from_value(json!({"id": "cal", "summary": "Me", "selected": true, "backgroundColor": "#000000"})).unwrap();
    store.apply_calendar_list_page("acc", &[recoloured], None).unwrap();
    assert_eq!(store.event_ids("acc", "cal").unwrap(), ["s1"]);
    assert_eq!(store.events_sync_token("acc", "cal").unwrap().as_deref(), Some("t1"));
}

#[test]
fn removing_a_calendar_removes_its_events() {
    let mut store = seeded();
    store.apply_events_page("acc", "cal", &[ev(standup())], None).unwrap();
    store.remove_calendar("acc", "cal").unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt-store --test events`
Expected: compile errors for the missing methods.

- [ ] **Step 3: Implement `convert.rs`**

`crates/takt-store/src/convert.rs`:

```rust
//! Turning Google's event fields into index columns and recurrence inputs.

use chrono::{DateTime, NaiveDate, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use takt_google::model::{EventDateTime, EventFields};
use takt_recur::{Exception, Master, When};

use crate::{StoreError, TimeSpan};

const DAY: i64 = 86_400;

pub(crate) fn when_of(t: &EventDateTime) -> Option<When> {
    if let Some(date) = t.date {
        return Some(When::Date(date));
    }
    t.date_time.map(|dt| When::At(dt.with_timezone(&Utc)))
}

pub(crate) fn encode_when(when: &When) -> String {
    match when {
        When::At(t) => t.to_rfc3339_opts(SecondsFormat::Secs, true),
        When::Date(d) => d.format("%Y-%m-%d").to_string(),
    }
}

pub(crate) fn decode_when(text: &str) -> Result<When, StoreError> {
    if text.len() == 10 {
        return NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map(When::Date)
            .map_err(|e| StoreError::Corrupt(format!("date {text}: {e}")));
    }
    DateTime::parse_from_rfc3339(text)
        .map(|t| When::At(t.with_timezone(&Utc)))
        .map_err(|e| StoreError::Corrupt(format!("instant {text}: {e}")))
}

pub(crate) fn zone(name: Option<&str>) -> Option<Tz> {
    name.and_then(|n| n.parse().ok())
}

pub(crate) fn master_of(f: &EventFields, fallback: Tz) -> Option<Master> {
    let start = when_of(f.start.as_ref()?)?;
    let end = when_of(f.end.as_ref()?)?;
    let tz = zone(f.start.as_ref().and_then(|s| s.time_zone.as_deref())).unwrap_or(fallback);
    Some(Master { id: f.id.clone(), start, end, tz, recurrence: f.recurrence.clone() })
}

pub(crate) fn exception_of(f: &EventFields) -> Option<Exception> {
    let original_start = when_of(f.original_start_time.as_ref()?)?;
    let start = f.start.as_ref().and_then(when_of).unwrap_or(original_start);
    let end = f.end.as_ref().and_then(when_of).unwrap_or(start);
    Some(Exception { id: f.id.clone(), original_start, cancelled: f.is_cancelled(), start, end })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Single,
    Master,
    Exception,
}

impl Kind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Kind::Single => "single",
            Kind::Master => "master",
            Kind::Exception => "exception",
        }
    }

    pub(crate) fn parse(text: &str) -> Kind {
        match text {
            "master" => Kind::Master,
            "exception" => Kind::Exception,
            _ => Kind::Single,
        }
    }
}

/// The index columns of one `events` row.
pub(crate) struct EventIndex {
    pub kind: Kind,
    pub start_utc: Option<i64>,
    pub end_utc: Option<i64>,
    pub all_day: bool,
    pub series_end_utc: Option<i64>,
    pub original_start: Option<String>,
}

impl EventIndex {
    pub(crate) fn of(f: &EventFields, fallback: Tz) -> EventIndex {
        if f.recurring_event_id.is_some() {
            let e = exception_of(f);
            return EventIndex {
                kind: Kind::Exception,
                start_utc: e.as_ref().map(|e| e.start.as_utc().timestamp()),
                end_utc: e.as_ref().map(|e| e.end.as_utc().timestamp()),
                all_day: e.as_ref().is_some_and(|e| e.start.is_all_day()),
                series_end_utc: None,
                original_start: e.as_ref().map(|e| encode_when(&e.original_start)),
            };
        }
        let start = f.start.as_ref().and_then(when_of);
        let end = f.end.as_ref().and_then(when_of).or(start);
        let all_day = start.is_some_and(|s| s.is_all_day());
        let start_utc = start.map(|s| s.as_utc().timestamp());
        let end_utc = end.map(|e| e.as_utc().timestamp());
        if f.recurrence.is_empty() {
            return EventIndex { kind: Kind::Single, start_utc, end_utc, all_day, series_end_utc: None, original_start: None };
        }
        let series_end_utc = match master_of(f, fallback).map(|m| takt_recur::series_end(&m)) {
            Some(Ok(Some(last_end))) => Some(last_end.as_utc().timestamp()),
            Some(Ok(None)) => None,
            // Unparseable series: index it like a single event; queries flag it.
            Some(Err(_)) | None => end_utc,
        };
        EventIndex { kind: Kind::Master, start_utc, end_utc, all_day, series_end_utc, original_start: None }
    }

    pub(crate) fn span(&self) -> Option<TimeSpan> {
        span_of(self.kind, self.start_utc, self.end_utc, self.all_day, self.series_end_utc)
    }
}

/// The time a row covers. All-day rows are widened by a day on each side
/// because their real instants depend on the display time zone.
pub(crate) fn span_of(
    kind: Kind,
    start_utc: Option<i64>,
    end_utc: Option<i64>,
    all_day: bool,
    series_end_utc: Option<i64>,
) -> Option<TimeSpan> {
    let start = start_utc?;
    let end = match kind {
        Kind::Master => series_end_utc,
        _ => Some(end_utc.unwrap_or(start)),
    };
    let pad = if all_day { DAY } else { 0 };
    Some(TimeSpan { start: instant(start - pad), end: end.map(|e| instant(e + pad)) })
}

fn instant(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds, 0).single().unwrap_or(DateTime::<Utc>::MIN_UTC)
}
```

- [ ] **Step 4: Implement `events.rs`**

`crates/takt-store/src/events.rs`:

```rust
use chrono_tz::Tz;
use rusqlite::{OptionalExtension, Transaction, params};
use takt_google::model::{EventFields, Item};

use crate::convert::{EventIndex, Kind, span_of, zone};
use crate::{ChangeSet, Store, StoreError, TimeSpan};

const UPSERT_EVENT: &str = "
INSERT INTO events (account_id, calendar_id, event_id, raw_json, kind, status, ical_uid, recurring_event_id,
                    original_start, start_utc, end_utc, all_day, series_end_utc, etag, updated)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
ON CONFLICT (account_id, calendar_id, event_id) DO UPDATE SET
    raw_json = excluded.raw_json, kind = excluded.kind, status = excluded.status, ical_uid = excluded.ical_uid,
    recurring_event_id = excluded.recurring_event_id, original_start = excluded.original_start,
    start_utc = excluded.start_utc, end_utc = excluded.end_utc, all_day = excluded.all_day,
    series_end_utc = excluded.series_end_utc, etag = excluded.etag, updated = excluded.updated";

impl Store {
    pub fn events_sync_token(&self, account_id: &str, calendar_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT events_sync_token FROM calendars WHERE account_id = ?1 AND calendar_id = ?2",
                params![account_id, calendar_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Applies one page of `events.list` in a single transaction. The sync
    /// token is written only when the page carries one (the last page).
    pub fn apply_events_page(
        &mut self,
        account_id: &str,
        calendar_id: &str,
        items: &[Item<EventFields>],
        next_sync_token: Option<&str>,
    ) -> Result<ChangeSet, StoreError> {
        let tx = self.conn.transaction()?;
        let fallback = calendar_zone(&tx, account_id, calendar_id)?;
        let mut changes = ChangeSet::default();
        for item in items {
            let f = &item.fields;
            if let Some(old) = stored_span(&tx, account_id, calendar_id, &f.id)? {
                changes.touch_calendar(calendar_id, Some(old));
            }
            if f.is_cancelled() && f.recurring_event_id.is_none() {
                // A cancelled series may arrive as only {id, status}; remove its exceptions too.
                tx.execute(
                    "DELETE FROM events WHERE account_id = ?1 AND calendar_id = ?2
                       AND (event_id = ?3 OR recurring_event_id = ?3)",
                    params![account_id, calendar_id, f.id],
                )?;
                continue;
            }
            let index = EventIndex::of(f, fallback);
            if let Some(span) = index.span() {
                changes.touch_calendar(calendar_id, Some(span));
            }
            tx.execute(
                UPSERT_EVENT,
                params![
                    account_id,
                    calendar_id,
                    f.id,
                    item.raw.to_string(),
                    index.kind.as_str(),
                    f.status,
                    f.ical_uid,
                    f.recurring_event_id,
                    index.original_start,
                    index.start_utc,
                    index.end_utc,
                    index.all_day,
                    index.series_end_utc,
                    f.etag,
                    f.updated.map(|u| u.to_rfc3339()),
                ],
            )?;
        }
        if let Some(token) = next_sync_token {
            tx.execute(
                "UPDATE calendars SET events_sync_token = ?3 WHERE account_id = ?1 AND calendar_id = ?2",
                params![account_id, calendar_id, token],
            )?;
        }
        tx.commit()?;
        Ok(changes)
    }

    /// After a 410: forget the calendar's events and token so it fully re-syncs.
    pub fn reset_calendar_events(&mut self, account_id: &str, calendar_id: &str) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM events WHERE account_id = ?1 AND calendar_id = ?2",
            params![account_id, calendar_id],
        )?;
        tx.execute(
            "UPDATE calendars SET events_sync_token = NULL WHERE account_id = ?1 AND calendar_id = ?2",
            params![account_id, calendar_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn event_raw(
        &self,
        account_id: &str,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<Option<serde_json::Value>, StoreError> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT raw_json FROM events WHERE account_id = ?1 AND calendar_id = ?2 AND event_id = ?3",
                params![account_id, calendar_id, event_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw.map(|text| serde_json::from_str(&text)).transpose()?)
    }

    pub fn event_ids(&self, account_id: &str, calendar_id: &str) -> Result<Vec<String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id FROM events WHERE account_id = ?1 AND calendar_id = ?2 ORDER BY event_id",
        )?;
        let rows = stmt.query_map(params![account_id, calendar_id], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

pub(crate) fn calendar_zone(tx: &Transaction<'_>, account_id: &str, calendar_id: &str) -> Result<Tz, StoreError> {
    let name: Option<String> = tx
        .query_row(
            "SELECT time_zone FROM calendars WHERE account_id = ?1 AND calendar_id = ?2",
            params![account_id, calendar_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(zone(name.as_deref()).unwrap_or(Tz::UTC))
}

fn stored_span(
    tx: &Transaction<'_>,
    account_id: &str,
    calendar_id: &str,
    event_id: &str,
) -> Result<Option<TimeSpan>, StoreError> {
    let row = tx
        .query_row(
            "SELECT kind, start_utc, end_utc, all_day, series_end_utc FROM events
             WHERE account_id = ?1 AND calendar_id = ?2 AND event_id = ?3",
            params![account_id, calendar_id, event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(kind, start, end, all_day, series_end)| {
        span_of(Kind::parse(&kind), start, end, all_day, series_end)
    }))
}
```

In `crates/takt-store/src/lib.rs` add `mod convert;` and `mod events;` to the module list.

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p takt-store`
Expected: PASS for `accounts` and `events`.

- [ ] **Step 6: Commit**

```bash
git add crates/takt-store
git commit -m "feat(takt-store): apply event pages with cancellation rules and change spans"
```

---

### Task 10: Occurrence query

**Files:**
- Modify: `crates/takt-store/src/lib.rs`
- Create: `crates/takt-store/src/query.rs`, `crates/takt-store/tests/query.rs`

**Interfaces:**
- Consumes: Task 9 `convert` helpers, `takt_recur::expand`.
- Produces: `Store::occurrences(&self, range: &Range, tz: Tz) -> Result<Vec<Occurrence>, StoreError>`. Only calendars with `selected = 1 AND hidden = 0`. Sorted by display start, then summary. Rows from different accounts are never merged.

- [ ] **Step 1: Write the failing tests**

`crates/takt-store/tests/query.rs`:

```rust
use chrono::NaiveDate;
use chrono_tz::Tz;
use serde_json::json;
use takt_google::model::{CalendarListEntry, EventFields, Item};
use takt_store::*;

fn ev(value: serde_json::Value) -> Item<EventFields> {
    Item::from_value(value).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn store_with(account: &str, calendar: serde_json::Value) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    add(&mut store, account, calendar);
    store
}

fn add(store: &mut Store, account: &str, calendar: serde_json::Value) {
    store.upsert_account(account, &format!("{account}@example.com")).unwrap();
    let cal: Item<CalendarListEntry> = Item::from_value(calendar).unwrap();
    store.apply_calendar_list_page(account, &[cal], None).unwrap();
}

fn visible() -> serde_json::Value {
    json!({"id": "cal", "selected": true, "timeZone": "Europe/London"})
}

fn week_of_19_oct() -> Range {
    local_days(d(2026, 10, 19), d(2026, 10, 26), Tz::Europe__London)
}

#[test]
fn timed_single_inside_and_outside_range() {
    let mut store = store_with("acc", visible());
    store
        .apply_events_page("acc", "cal", &[
            ev(json!({"id": "in", "summary": "In", "start": {"dateTime": "2026-10-20T09:00:00Z"}, "end": {"dateTime": "2026-10-20T10:00:00Z"}})),
            ev(json!({"id": "out", "summary": "Out", "start": {"dateTime": "2026-11-20T09:00:00Z"}, "end": {"dateTime": "2026-11-20T10:00:00Z"}})),
        ], None)
        .unwrap();
    let got = store.occurrences(&week_of_19_oct(), Tz::Europe__London).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].summary.as_deref(), Some("In"));
    assert_eq!(got[0].instance_id, "in");
}

#[test]
fn unselected_or_hidden_calendars_are_excluded() {
    let mut store = store_with("acc", json!({"id": "cal", "selected": false}));
    add(&mut store, "acc", json!({"id": "hid", "selected": true, "hidden": true}));
    let e = json!({"id": "x", "start": {"dateTime": "2026-10-20T09:00:00Z"}, "end": {"dateTime": "2026-10-20T10:00:00Z"}});
    store.apply_events_page("acc", "cal", &[ev(e.clone())], None).unwrap();
    store.apply_events_page("acc", "hid", &[ev(e)], None).unwrap();
    assert!(store.occurrences(&week_of_19_oct(), Tz::UTC).unwrap().is_empty());
}

fn series() -> serde_json::Value {
    json!({
        "id": "m", "summary": "1:1", "location": "Room 4", "colorId": "11",
        "start": {"dateTime": "2026-10-05T10:00:00+01:00", "timeZone": "Europe/London"},
        "end": {"dateTime": "2026-10-05T10:30:00+01:00", "timeZone": "Europe/London"},
        "recurrence": ["RRULE:FREQ=WEEKLY"]
    })
}

#[test]
fn series_expands_with_master_fields() {
    let mut store = store_with("acc", visible());
    store.apply_events_page("acc", "cal", &[ev(series())], None).unwrap();
    let got = store.occurrences(&week_of_19_oct(), Tz::Europe__London).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].instance_id, "m_20261019T090000Z");
    assert_eq!(got[0].event_id, "m");
    assert_eq!(got[0].location.as_deref(), Some("Room 4"));
    assert_eq!(got[0].color_id.as_deref(), Some("11"));
}

#[test]
fn modified_exception_uses_its_own_fields() {
    let mut store = store_with("acc", visible());
    let moved = json!({
        "id": "m_20261019T090000Z", "summary": "1:1 (moved)", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-19T10:00:00+01:00", "timeZone": "Europe/London"},
        "start": {"dateTime": "2026-10-20T15:00:00+01:00"}, "end": {"dateTime": "2026-10-20T15:30:00+01:00"}
    });
    store.apply_events_page("acc", "cal", &[ev(series()), ev(moved)], None).unwrap();
    let got = store.occurrences(&week_of_19_oct(), Tz::Europe__London).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].summary.as_deref(), Some("1:1 (moved)"));
    assert_eq!(got[0].event_id, "m_20261019T090000Z");
}

#[test]
fn cancelled_exception_hides_its_occurrence() {
    let mut store = store_with("acc", visible());
    let cancelled = json!({
        "id": "m_20261019T090000Z", "status": "cancelled", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-19T10:00:00+01:00", "timeZone": "Europe/London"}
    });
    store.apply_events_page("acc", "cal", &[ev(series()), ev(cancelled)], None).unwrap();
    assert!(store.occurrences(&week_of_19_oct(), Tz::Europe__London).unwrap().is_empty());
}

#[test]
fn exception_moved_past_series_end_still_appears() {
    let mut store = store_with("acc", visible());
    let mut short = series();
    short["recurrence"] = json!(["RRULE:FREQ=WEEKLY;COUNT=2"]);
    let moved = json!({
        "id": "m_20261012T090000Z", "summary": "Late 1:1", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"},
        "start": {"dateTime": "2026-11-20T10:00:00Z"}, "end": {"dateTime": "2026-11-20T10:30:00Z"}
    });
    store.apply_events_page("acc", "cal", &[ev(short), ev(moved)], None).unwrap();
    let november = local_days(d(2026, 11, 1), d(2026, 12, 1), Tz::UTC);
    let got = store.occurrences(&november, Tz::UTC).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].summary.as_deref(), Some("Late 1:1"));
}

#[test]
fn unparseable_series_shows_once_with_a_flag() {
    let mut store = store_with("acc", visible());
    let mut broken = series();
    broken["recurrence"] = json!(["RRULE:FREQ=SOMETIMES"]);
    broken["start"] = json!({"dateTime": "2026-10-20T10:00:00+01:00", "timeZone": "Europe/London"});
    broken["end"] = json!({"dateTime": "2026-10-20T10:30:00+01:00", "timeZone": "Europe/London"});
    store.apply_events_page("acc", "cal", &[ev(broken)], None).unwrap();
    let got = store.occurrences(&week_of_19_oct(), Tz::Europe__London).unwrap();
    assert_eq!(got.len(), 1);
    assert!(got[0].recurrence_error);
}

fn all_day_on(date: &str) -> Item<EventFields> {
    let next = (NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap() + chrono::TimeDelta::days(1)).format("%Y-%m-%d").to_string();
    ev(json!({"id": "ad", "summary": "All day", "start": {"date": date}, "end": {"date": next}}))
}

#[test]
fn all_day_event_respects_display_zone() {
    let mut store = store_with("acc", visible());
    store.apply_events_page("acc", "cal", &[all_day_on("2026-10-20")], None).unwrap();
    for tz in [Tz::Pacific__Honolulu, Tz::Pacific__Tongatapu] {
        let on_day = store.occurrences(&local_days(d(2026, 10, 20), d(2026, 10, 21), tz), tz).unwrap();
        let before = store.occurrences(&local_days(d(2026, 10, 19), d(2026, 10, 20), tz), tz).unwrap();
        let after = store.occurrences(&local_days(d(2026, 10, 21), d(2026, 10, 22), tz), tz).unwrap();
        assert_eq!((on_day.len(), before.len(), after.len()), (1, 0, 0), "zone {tz}");
    }
}

#[test]
fn all_day_event_on_skipped_midnight_lands_on_its_day() {
    let tz = Tz::America__Santiago;
    let mut store = store_with("acc", visible());
    store.apply_events_page("acc", "cal", &[all_day_on("2026-09-06")], None).unwrap();
    assert_eq!(store.occurrences(&local_days(d(2026, 9, 6), d(2026, 9, 7), tz), tz).unwrap().len(), 1);
    assert_eq!(store.occurrences(&local_days(d(2026, 9, 5), d(2026, 9, 6), tz), tz).unwrap().len(), 0);
}

#[test]
fn same_event_in_two_accounts_appears_twice() {
    let mut store = store_with("work", visible());
    add(&mut store, "home", visible());
    let shared = json!({"id": "x", "iCalUID": "x@google.com", "summary": "Shared",
        "start": {"dateTime": "2026-10-20T09:00:00Z"}, "end": {"dateTime": "2026-10-20T10:00:00Z"}});
    store.apply_events_page("work", "cal", &[ev(shared.clone())], None).unwrap();
    store.apply_events_page("home", "cal", &[ev(shared)], None).unwrap();
    let got = store.occurrences(&week_of_19_oct(), Tz::UTC).unwrap();
    let accounts: Vec<_> = got.iter().map(|o| o.account_id.as_str()).collect();
    assert_eq!(got.len(), 2);
    assert!(accounts.contains(&"work") && accounts.contains(&"home"));
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt-store --test query`
Expected: compile error, `occurrences` is not defined.

- [ ] **Step 3: Implement `query.rs`**

`crates/takt-store/src/query.rs`:

```rust
use std::collections::{HashMap, HashSet};

use chrono::TimeDelta;
use chrono_tz::Tz;
use rusqlite::params;
use takt_google::model::EventFields;
use takt_recur::{Range, Source, When};

use crate::convert::{exception_of, master_of, when_of, zone};
use crate::{Occurrence, Store, StoreError, display_instant};

/// A visible `events` row with its calendar's zone.
struct Row {
    account_id: String,
    calendar_id: String,
    calendar_tz: Option<String>,
    fields: EventFields,
}

const VISIBLE: &str = "FROM events e JOIN calendars c
     ON c.account_id = e.account_id AND c.calendar_id = e.calendar_id
     WHERE c.selected = 1 AND c.hidden = 0";

impl Store {
    /// Everything visible in `range`, expanded and filtered exactly in `tz`.
    pub fn occurrences(&self, range: &Range, tz: Tz) -> Result<Vec<Occurrence>, StoreError> {
        // Index columns treat all-day dates as UTC midnight, so look a day wider
        // and filter precisely at the end.
        let padded = Range { start: range.start - TimeDelta::days(1), end: range.end + TimeDelta::days(1) };
        let (lo, hi) = (padded.start.timestamp(), padded.end.timestamp());
        let mut out = Vec::new();

        for row in self.rows(
            &format!("{VISIBLE} AND e.kind = 'single' AND e.start_utc < ?2 AND e.end_utc >= ?1"),
            lo,
            hi,
        )? {
            let f = &row.fields;
            let (Some(start), Some(end)) = (f.start.as_ref().and_then(when_of), f.end.as_ref().and_then(when_of)) else {
                continue;
            };
            out.push(occurrence(&row, &f.id, f.id.clone(), start, end, f, false));
        }

        let mut expanded = HashSet::new();
        for row in self.rows(
            &format!(
                "{VISIBLE} AND e.kind = 'master' AND e.start_utc < ?2
                 AND (e.series_end_utc IS NULL OR e.series_end_utc >= ?1)"
            ),
            lo,
            hi,
        )? {
            expanded.insert((row.account_id.clone(), row.calendar_id.clone(), row.fields.id.clone()));
            self.expand_series(&row, &padded, &mut out)?;
        }

        // Exceptions moved somewhere their series does not reach in this range.
        for row in self.rows(
            &format!(
                "{VISIBLE} AND e.kind = 'exception' AND (e.status IS NULL OR e.status != 'cancelled')
                 AND e.start_utc < ?2 AND e.end_utc >= ?1"
            ),
            lo,
            hi,
        )? {
            let master_id = row.fields.recurring_event_id.clone().unwrap_or_default();
            if expanded.contains(&(row.account_id.clone(), row.calendar_id.clone(), master_id)) {
                continue;
            }
            if let Some(e) = exception_of(&row.fields) {
                out.push(occurrence(&row, &row.fields.id, e.id.clone(), e.start, e.end, &row.fields, false));
            }
        }

        out.retain(|o| overlaps_in(range, &o.start, &o.end, tz));
        out.sort_by(|a, b| {
            display_instant(&a.start, tz)
                .cmp(&display_instant(&b.start, tz))
                .then_with(|| a.summary.cmp(&b.summary))
                .then_with(|| a.account_id.cmp(&b.account_id))
        });
        Ok(out)
    }

    fn expand_series(&self, master: &Row, range: &Range, out: &mut Vec<Occurrence>) -> Result<(), StoreError> {
        let exception_rows = self.exceptions_of(&master.account_id, &master.calendar_id, &master.fields.id)?;
        let exceptions: Vec<_> = exception_rows.iter().filter_map(exception_of).collect();
        let by_id: HashMap<&str, &EventFields> = exception_rows.iter().map(|f| (f.id.as_str(), f)).collect();
        let fallback = zone(master.calendar_tz.as_deref()).unwrap_or(Tz::UTC);
        let expanded = master_of(&master.fields, fallback)
            .ok_or_else(|| takt_recur::RecurError::Parse("series has no start or end".into()))
            .and_then(|m| takt_recur::expand(&m, &exceptions, range));
        match expanded {
            Ok(list) => {
                for o in list {
                    let source = match o.source {
                        Source::Exception => by_id.get(o.instance_id.as_str()).copied().unwrap_or(&master.fields),
                        Source::Generated => &master.fields,
                    };
                    out.push(occurrence(master, &source.id, o.instance_id, o.start, o.end, source, false));
                }
            }
            Err(error) => {
                tracing::warn!(event = %master.fields.id, %error, "showing series as a single event");
                let f = &master.fields;
                if let (Some(start), Some(end)) = (f.start.as_ref().and_then(when_of), f.end.as_ref().and_then(when_of)) {
                    out.push(occurrence(master, &f.id, f.id.clone(), start, end, f, true));
                }
            }
        }
        Ok(())
    }

    fn rows(&self, sql_tail: &str, lo: i64, hi: i64) -> Result<Vec<Row>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT e.account_id, e.calendar_id, c.time_zone, e.raw_json {sql_tail}"))?;
        let raw_rows = stmt.query_map(params![lo, hi], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?))
        })?;
        let mut rows = Vec::new();
        for raw_row in raw_rows {
            let (account_id, calendar_id, calendar_tz, raw) = raw_row?;
            let fields: EventFields = serde_json::from_str(&raw)?;
            rows.push(Row { account_id, calendar_id, calendar_tz, fields });
        }
        Ok(rows)
    }

    fn exceptions_of(&self, account_id: &str, calendar_id: &str, master_id: &str) -> Result<Vec<EventFields>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT raw_json FROM events WHERE account_id = ?1 AND calendar_id = ?2 AND recurring_event_id = ?3",
        )?;
        let raws = stmt.query_map(params![account_id, calendar_id, master_id], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for raw in raws {
            out.push(serde_json::from_str(&raw?)?);
        }
        Ok(out)
    }
}

fn occurrence(
    row: &Row,
    event_id: &str,
    instance_id: String,
    start: When,
    end: When,
    fields: &EventFields,
    recurrence_error: bool,
) -> Occurrence {
    Occurrence {
        account_id: row.account_id.clone(),
        calendar_id: row.calendar_id.clone(),
        event_id: event_id.to_owned(),
        instance_id,
        start,
        end,
        summary: fields.summary.clone(),
        location: fields.location.clone(),
        color_id: fields.color_id.clone(),
        self_response: fields.self_response().map(str::to_owned),
        recurrence_error,
    }
}

fn overlaps_in(range: &Range, start: &When, end: &When, tz: Tz) -> bool {
    let (s, e) = (display_instant(start, tz), display_instant(end, tz));
    s < range.end && (e > range.start || (s == e && s >= range.start))
}
```

`EventFields` must implement `serde::Deserialize` for `serde_json::from_str`; it does. In `crates/takt-store/src/lib.rs` add `mod query;`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p takt-store`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/takt-store
git commit -m "feat(takt-store): query visible occurrences with local expansion"
```

---

### Task 11: Task lists and tasks in the store

**Files:**
- Modify: `crates/takt-store/src/lib.rs`
- Create: `crates/takt-store/src/tasks.rs`, `crates/takt-store/tests/tasks.rs`

**Interfaces:**
- Consumes: `takt_google::model::{Item, TaskListFields, TaskFields}`, `ChangeSet`.
- Produces:
  - `Store::apply_task_lists(&mut self, account_id, &[Item<TaskListFields>]) -> Result<ChangeSet>` (full list: lists missing from it are deleted with their tasks)
  - `Store::task_checkpoint(&self, account_id, list_id) -> Result<Option<DateTime<Utc>>>`
  - `Store::apply_tasks_page(&mut self, account_id, list_id, &[Item<TaskFields>]) -> Result<ChangeSet>` (advances the checkpoint to the largest `updated` seen, never backwards)
  - `Store::tasks_due(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<TaskItem>>` (`[from, to)`, hidden tasks excluded, ordered by date then title)

- [ ] **Step 1: Write the failing tests**

`crates/takt-store/tests/tasks.rs`:

```rust
use chrono::{NaiveDate, TimeZone, Utc};
use serde_json::json;
use takt_google::model::{Item, TaskFields, TaskListFields};
use takt_store::*;

fn list(id: &str, title: &str) -> Item<TaskListFields> {
    Item::from_value(json!({"id": id, "title": title})).unwrap()
}

fn task(value: serde_json::Value) -> Item<TaskFields> {
    Item::from_value(value).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn seeded() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    store.apply_task_lists("acc", &[list("L1", "My Tasks")]).unwrap();
    store
}

#[test]
fn tasks_due_returns_dated_visible_tasks_in_range() {
    let mut store = seeded();
    store
        .apply_tasks_page("acc", "L1", &[
            task(json!({"id": "a", "title": "Pay rent", "status": "needsAction", "due": "2026-10-20T00:00:00.000Z", "updated": "2026-10-17T10:00:00.000Z"})),
            task(json!({"id": "b", "title": "Done", "status": "completed", "due": "2026-10-21T00:00:00.000Z", "updated": "2026-10-17T11:00:00.000Z"})),
            task(json!({"id": "c", "title": "Cleared", "status": "completed", "hidden": true, "due": "2026-10-21T00:00:00.000Z"})),
            task(json!({"id": "e", "title": "Undated", "status": "needsAction"})),
            task(json!({"id": "f", "title": "Later", "status": "needsAction", "due": "2026-12-01T00:00:00.000Z"})),
        ])
        .unwrap();
    let got = store.tasks_due(d(2026, 10, 19), d(2026, 10, 26)).unwrap();
    let titles: Vec<_> = got.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["Pay rent", "Done"]);
    assert!(got[1].completed);
    assert_eq!(got[0].list_title, "My Tasks");
}

#[test]
fn deleted_task_is_removed() {
    let mut store = seeded();
    store.apply_tasks_page("acc", "L1", &[task(json!({"id": "a", "title": "x", "due": "2026-10-20T00:00:00.000Z"}))]).unwrap();
    let changes = store.apply_tasks_page("acc", "L1", &[task(json!({"id": "a", "deleted": true}))]).unwrap();
    assert!(store.tasks_due(d(2026, 10, 1), d(2026, 11, 1)).unwrap().is_empty());
    assert!(changes.task_lists.contains("L1"));
}

#[test]
fn checkpoint_advances_and_never_goes_back() {
    let mut store = seeded();
    assert_eq!(store.task_checkpoint("acc", "L1").unwrap(), None);
    store.apply_tasks_page("acc", "L1", &[task(json!({"id": "a", "title": "x", "updated": "2026-10-17T11:00:00.000Z"}))]).unwrap();
    store.apply_tasks_page("acc", "L1", &[task(json!({"id": "b", "title": "y", "updated": "2026-10-17T09:00:00.000Z"}))]).unwrap();
    assert_eq!(store.task_checkpoint("acc", "L1").unwrap(), Some(Utc.with_ymd_and_hms(2026, 10, 17, 11, 0, 0).unwrap()));
}

#[test]
fn list_missing_from_full_list_is_removed_with_its_tasks() {
    let mut store = seeded();
    store.apply_tasks_page("acc", "L1", &[task(json!({"id": "a", "title": "x", "due": "2026-10-20T00:00:00.000Z"}))]).unwrap();
    store.apply_task_lists("acc", &[list("L2", "Errands")]).unwrap();
    assert!(store.tasks_due(d(2026, 10, 1), d(2026, 11, 1)).unwrap().is_empty());
    assert_eq!(store.task_checkpoint("acc", "L2").unwrap(), None);
}

#[test]
fn reapplying_task_lists_keeps_checkpoint() {
    let mut store = seeded();
    store.apply_tasks_page("acc", "L1", &[task(json!({"id": "a", "title": "x", "updated": "2026-10-17T11:00:00.000Z"}))]).unwrap();
    store.apply_task_lists("acc", &[list("L1", "Renamed")]).unwrap();
    assert!(store.task_checkpoint("acc", "L1").unwrap().is_some());
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt-store --test tasks`
Expected: compile errors for the missing methods.

- [ ] **Step 3: Implement `tasks.rs`**

`crates/takt-store/src/tasks.rs`:

```rust
use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use rusqlite::{OptionalExtension, params};
use takt_google::model::{Item, TaskFields, TaskListFields};

use crate::{ChangeSet, Store, StoreError, TaskItem, TimeSpan};

impl Store {
    /// Applies the complete list of task lists for an account.
    pub fn apply_task_lists(
        &mut self,
        account_id: &str,
        lists: &[Item<TaskListFields>],
    ) -> Result<ChangeSet, StoreError> {
        let tx = self.conn.transaction()?;
        let mut changes = ChangeSet::default();
        let keep: HashSet<&str> = lists.iter().map(|l| l.fields.id.as_str()).collect();
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT list_id FROM task_lists WHERE account_id = ?1")?;
            let rows = stmt.query_map(params![account_id], |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        for gone in existing.iter().filter(|id| !keep.contains(id.as_str())) {
            tx.execute(
                "DELETE FROM task_lists WHERE account_id = ?1 AND list_id = ?2",
                params![account_id, gone],
            )?;
            changes.touch_task_list(gone, None);
        }
        for l in lists {
            tx.execute(
                "INSERT INTO task_lists (account_id, list_id, raw_json, title, updated) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (account_id, list_id) DO UPDATE SET
                     raw_json = excluded.raw_json, title = excluded.title, updated = excluded.updated",
                params![
                    account_id,
                    l.fields.id,
                    l.raw.to_string(),
                    l.fields.title.clone().unwrap_or_default(),
                    l.fields.updated.map(|u| u.to_rfc3339()),
                ],
            )?;
        }
        tx.commit()?;
        Ok(changes)
    }

    pub fn task_checkpoint(&self, account_id: &str, list_id: &str) -> Result<Option<DateTime<Utc>>, StoreError> {
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT updated_min_checkpoint FROM task_lists WHERE account_id = ?1 AND list_id = ?2",
                params![account_id, list_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        text.map(|t| {
            DateTime::parse_from_rfc3339(&t)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| StoreError::Corrupt(format!("checkpoint {t}: {e}")))
        })
        .transpose()
    }

    pub fn apply_tasks_page(
        &mut self,
        account_id: &str,
        list_id: &str,
        items: &[Item<TaskFields>],
    ) -> Result<ChangeSet, StoreError> {
        let mut checkpoint = self.task_checkpoint(account_id, list_id)?;
        let tx = self.conn.transaction()?;
        let mut changes = ChangeSet::default();
        for item in items {
            let t = &item.fields;
            if let Some(updated) = t.updated {
                checkpoint = Some(checkpoint.map_or(updated, |c| c.max(updated)));
            }
            let old_due: Option<String> = tx
                .query_row(
                    "SELECT due_date FROM tasks WHERE account_id = ?1 AND list_id = ?2 AND task_id = ?3",
                    params![account_id, list_id, t.id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            if let Some(date) = old_due.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
                changes.touch_task_list(list_id, Some(day_span(date)));
            }
            if t.deleted {
                tx.execute(
                    "DELETE FROM tasks WHERE account_id = ?1 AND list_id = ?2 AND task_id = ?3",
                    params![account_id, list_id, t.id],
                )?;
                changes.task_lists.insert(list_id.to_owned());
                continue;
            }
            let due = t.due_date();
            if let Some(date) = due {
                changes.touch_task_list(list_id, Some(day_span(date)));
            }
            tx.execute(
                "INSERT INTO tasks (account_id, list_id, task_id, raw_json, title, status, due_date, parent, position,
                                    completed, hidden, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT (account_id, list_id, task_id) DO UPDATE SET
                     raw_json = excluded.raw_json, title = excluded.title, status = excluded.status,
                     due_date = excluded.due_date, parent = excluded.parent, position = excluded.position,
                     completed = excluded.completed, hidden = excluded.hidden, updated = excluded.updated",
                params![
                    account_id,
                    list_id,
                    t.id,
                    item.raw.to_string(),
                    t.title.clone().unwrap_or_default(),
                    t.status,
                    due.map(|d| d.format("%Y-%m-%d").to_string()),
                    t.parent,
                    t.position,
                    t.completed.map(|c| c.to_rfc3339()),
                    t.hidden,
                    t.updated.map(|u| u.to_rfc3339()),
                ],
            )?;
        }
        if let Some(c) = checkpoint {
            tx.execute(
                "UPDATE task_lists SET updated_min_checkpoint = ?3 WHERE account_id = ?1 AND list_id = ?2",
                params![account_id, list_id, c.to_rfc3339()],
            )?;
        }
        tx.commit()?;
        Ok(changes)
    }

    pub fn tasks_due(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<TaskItem>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT t.account_id, t.list_id, l.title, t.task_id, t.title, t.due_date, t.status
             FROM tasks t JOIN task_lists l ON l.account_id = t.account_id AND l.list_id = t.list_id
             WHERE t.hidden = 0 AND t.due_date >= ?1 AND t.due_date < ?2
             ORDER BY t.due_date, t.title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(
            params![from.format("%Y-%m-%d").to_string(), to.format("%Y-%m-%d").to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            let (account_id, list_id, list_title, task_id, title, due, status) = row?;
            let due_date = NaiveDate::parse_from_str(&due, "%Y-%m-%d")
                .map_err(|e| StoreError::Corrupt(format!("due date {due}: {e}")))?;
            out.push(TaskItem {
                account_id,
                list_id,
                list_title,
                task_id,
                title,
                due_date,
                completed: status.as_deref() == Some("completed"),
            });
        }
        Ok(out)
    }
}

/// A task's date as a span, a day wider each side for display zones.
fn day_span(date: NaiveDate) -> TimeSpan {
    let start = date.and_time(NaiveTime::MIN).and_utc();
    TimeSpan { start: start - TimeDelta::days(1), end: Some(start + TimeDelta::days(2)) }
}
```

In `crates/takt-store/src/lib.rs` add `mod tasks;`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p takt-store`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/takt-store
git commit -m "feat(takt-store): store task lists and tasks with update checkpoints"
```

---

### Task 12: Sync engine round and recovery

**Files:**
- Modify: `crates/takt-sync/Cargo.toml`, `crates/takt-sync/src/lib.rs`
- Create: `crates/takt-sync/src/messages.rs`, `crates/takt-sync/src/engine.rs`, `crates/takt-sync/tests/common/mod.rs`, `crates/takt-sync/tests/engine.rs`

**Interfaces:**
- Consumes: `GoogleClient` endpoint methods (Task 7), `Store` methods (Tasks 8–11), `ChangeSet`.
- Produces (`takt_sync`):
  - `enum Command { AddAccount, RemoveAccount { account_id: String }, SyncNow, WindowFocus(bool), Shutdown }`
  - `enum SyncEvent { Changed { account_id: String, changes: ChangeSet }, Status { account_id: String, state: SyncState }, AuthNeeded { account_id: String }, AccountsChanged, SignInFailed(String) }`
  - `enum SyncState { Synced(DateTime<Utc>), Syncing, Offline(DateTime<Utc>), Error(String), NeedsReauth }`
  - `enum SyncError { Google(GoogleError), Store(StoreError) }` with `is_offline()`, `needs_reauth()`
  - `const PARALLEL_CALENDARS: usize = 4`
  - `AccountEngine::new(account_id: impl Into<String>, GoogleClient, Arc<Mutex<Store>>, async_channel::Sender<SyncEvent>)`, `AccountEngine::account_id(&self) -> &str`, `async AccountEngine::round(&self, with_meta: bool) -> Result<(), SyncError>`

- [ ] **Step 1: Add dependencies**

`crates/takt-sync/Cargo.toml`, below `[package]`:

```toml
[dependencies]
async-channel.workspace = true
chrono.workspace = true
futures.workspace = true
reqwest.workspace = true
takt-google.workspace = true
takt-store.workspace = true
thiserror.workspace = true
tokio.workspace = true
tracing.workspace = true

[dev-dependencies]
serde_json.workspace = true
tempfile.workspace = true
wiremock.workspace = true
```

- [ ] **Step 2: Write the test helpers and failing tests**

`crates/takt-sync/tests/common/mod.rs`:

```rust
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use takt_google::{Auth, Endpoints, GoogleClient, OAuthClient, RetryPolicy};
use takt_store::Store;
use takt_sync::{AccountEngine, SyncEvent};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub fn oauth() -> OAuthClient {
    OAuthClient { client_id: "cid".into(), client_secret: "secret".into() }
}

pub fn ok(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

/// One attempt per request so failures surface immediately.
pub fn no_retry() -> RetryPolicy {
    RetryPolicy { max_attempts: 1, base_delay: Duration::from_millis(1), max_delay: Duration::from_millis(1) }
}

pub fn client_at(base: &str, auth: Option<Auth>) -> GoogleClient {
    let endpoints = Endpoints::rooted_at(base);
    let http = reqwest::Client::new();
    let auth = auth.unwrap_or_else(|| {
        Auth::new(http.clone(), endpoints.token.clone(), oauth(), "rt").with_access_token("t", Duration::from_secs(3600))
    });
    GoogleClient::new(http, endpoints, auth).with_retry(no_retry())
}

pub struct Fake {
    pub server: MockServer,
    pub store: Arc<Mutex<Store>>,
    pub events: async_channel::Receiver<SyncEvent>,
    pub engine: AccountEngine,
}

pub async fn fake() -> Fake {
    let server = MockServer::start().await;
    let engine_parts = engine_for(&server.uri(), None);
    Fake { server, store: engine_parts.0, events: engine_parts.1, engine: engine_parts.2 }
}

pub fn engine_for(
    base: &str,
    auth: Option<Auth>,
) -> (Arc<Mutex<Store>>, async_channel::Receiver<SyncEvent>, AccountEngine) {
    let mut store = Store::open_in_memory().unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    let store = Arc::new(Mutex::new(store));
    let (tx, rx) = async_channel::unbounded();
    let engine = AccountEngine::new("acc", client_at(base, auth), store.clone(), tx);
    (store, rx, engine)
}

pub async fn mount_meta(server: &MockServer) {
    Mock::given(method("GET")).and(path("/calendar/v3/colors"))
        .respond_with(ok(json!({"calendar": {}, "event": {}}))).mount(server).await;
    Mock::given(method("GET")).and(path("/calendar/v3/users/me/settings"))
        .respond_with(ok(json!({"items": [{"id": "format24HourTime", "value": "true"}]}))).mount(server).await;
}

pub async fn mount_no_tasks(server: &MockServer) {
    Mock::given(method("GET")).and(path("/tasks/v1/users/@me/lists"))
        .respond_with(ok(json!({"items": []}))).mount(server).await;
}

pub async fn mount_calendar_list(server: &MockServer) {
    Mock::given(method("GET")).and(path("/calendar/v3/users/me/calendarList"))
        .respond_with(ok(json!({
            "items": [{"id": "cal", "summary": "Me", "primary": true, "selected": true, "timeZone": "UTC"}],
            "nextSyncToken": "cl1"
        })))
        .mount(server).await;
}

pub fn event(id: &str, day: u32) -> Value {
    json!({
        "id": id, "summary": id,
        "start": {"dateTime": format!("2026-10-{day:02}T09:00:00Z")},
        "end": {"dateTime": format!("2026-10-{day:02}T10:00:00Z")}
    })
}

pub fn ids(store: &Arc<Mutex<Store>>) -> Vec<String> {
    store.lock().unwrap().event_ids("acc", "cal").unwrap()
}
```

`crates/takt-sync/tests/engine.rs`:

```rust
mod common;

use chrono::NaiveDate;
use common::*;
use serde_json::json;
use takt_google::{Auth, Endpoints};
use takt_sync::SyncEvent;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, ResponseTemplate};

const EVENTS: &str = "/calendar/v3/calendars/cal/events";

#[tokio::test]
async fn first_round_fills_the_store() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_meta(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS)).and(query_param_is_missing("syncToken"))
        .respond_with(ok(json!({"items": [event("e1", 20)], "nextSyncToken": "s1"})))
        .mount(&f.server).await;
    Mock::given(method("GET")).and(path("/tasks/v1/users/@me/lists"))
        .respond_with(ok(json!({"items": [{"id": "L1", "title": "My Tasks"}]}))).mount(&f.server).await;
    Mock::given(method("GET")).and(path("/tasks/v1/lists/L1/tasks")).and(query_param_is_missing("updatedMin"))
        .respond_with(ok(json!({"items": [{"id": "t1", "title": "Pay rent", "due": "2026-10-20T00:00:00.000Z", "updated": "2026-10-17T10:00:00.000Z"}]})))
        .mount(&f.server).await;

    f.engine.round(true).await.unwrap();

    assert_eq!(ids(&f.store), ["e1"]);
    let store = f.store.lock().unwrap();
    assert_eq!(store.events_sync_token("acc", "cal").unwrap().as_deref(), Some("s1"));
    assert_eq!(store.calendar_list_sync_token("acc").unwrap().as_deref(), Some("cl1"));
    assert_eq!(store.settings("acc").unwrap()["format24HourTime"], "true");
    let d = |day| NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
    assert_eq!(store.tasks_due(d(19), d(21)).unwrap().len(), 1);
    drop(store);
    let mut changed = 0;
    while let Ok(ev) = f.events.try_recv() {
        if matches!(ev, SyncEvent::Changed { .. }) {
            changed += 1;
        }
    }
    assert!(changed >= 3, "calendar list, meta, events and tasks each report a change");
}

#[tokio::test]
async fn incremental_round_sends_stored_tokens() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS)).and(query_param_is_missing("syncToken"))
        .respond_with(ok(json!({"items": [event("e1", 20)], "nextSyncToken": "s1"})))
        .mount(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS)).and(query_param("syncToken", "s1"))
        .respond_with(ok(json!({"items": [event("e2", 21)], "nextSyncToken": "s2"})))
        .mount(&f.server).await;
    f.engine.round(false).await.unwrap();
    f.engine.round(false).await.unwrap();
    assert_eq!(ids(&f.store), ["e1", "e2"]);
    assert_eq!(f.store.lock().unwrap().events_sync_token("acc", "cal").unwrap().as_deref(), Some("s2"));
}

#[tokio::test]
async fn gone_on_events_resets_and_full_syncs() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    {
        // The calendar-list upsert in the round keeps this token (ON CONFLICT DO UPDATE).
        let mut store = f.store.lock().unwrap();
        let cal = takt_google::model::Item::from_value(json!({"id": "cal", "selected": true})).unwrap();
        store.apply_calendar_list_page("acc", &[cal], None).unwrap();
        let old = takt_google::model::Item::from_value(event("old", 1)).unwrap();
        store.apply_events_page("acc", "cal", &[old], Some("stale")).unwrap();
    }
    Mock::given(method("GET")).and(path(EVENTS)).and(query_param("syncToken", "stale"))
        .respond_with(ResponseTemplate::new(410)).mount(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS)).and(query_param_is_missing("syncToken"))
        .respond_with(ok(json!({"items": [event("fresh", 22)], "nextSyncToken": "s9"})))
        .mount(&f.server).await;
    f.engine.round(false).await.unwrap();
    assert_eq!(ids(&f.store), ["fresh"]);
    assert_eq!(f.store.lock().unwrap().events_sync_token("acc", "cal").unwrap().as_deref(), Some("s9"));
}

#[tokio::test]
async fn calendar_that_404s_is_removed() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS)).respond_with(ResponseTemplate::new(404)).mount(&f.server).await;
    f.engine.round(false).await.unwrap();
    assert!(f.store.lock().unwrap().calendars(Some("acc")).unwrap().is_empty());
}

#[tokio::test]
async fn interrupted_paging_resumes_from_the_old_token() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS)).and(query_param_is_missing("pageToken"))
        .respond_with(ok(json!({"items": [event("e1", 20)], "nextPageToken": "p2"})))
        .mount(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS)).and(query_param("pageToken", "p2"))
        .respond_with(ResponseTemplate::new(500)).mount(&f.server).await;
    assert!(f.engine.round(false).await.is_err());
    assert_eq!(ids(&f.store), ["e1"]);
    assert_eq!(f.store.lock().unwrap().events_sync_token("acc", "cal").unwrap(), None);

    f.server.reset().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS)).and(query_param_is_missing("pageToken"))
        .respond_with(ok(json!({"items": [event("e1", 20)], "nextPageToken": "p2"})))
        .mount(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS)).and(query_param("pageToken", "p2"))
        .respond_with(ok(json!({"items": [event("e2", 21)], "nextSyncToken": "s1"})))
        .mount(&f.server).await;
    f.engine.round(false).await.unwrap();
    assert_eq!(ids(&f.store), ["e1", "e2"]);
    assert_eq!(f.store.lock().unwrap().events_sync_token("acc", "cal").unwrap().as_deref(), Some("s1"));
}

#[tokio::test]
async fn unreachable_google_is_offline() {
    let (_store, _events, engine) = engine_for("http://127.0.0.1:9", None);
    let err = engine.round(false).await.unwrap_err();
    assert!(err.is_offline(), "{err}");
}

#[tokio::test]
async fn revoked_refresh_token_needs_reauth() {
    let f = fake().await;
    Mock::given(method("POST")).and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .mount(&f.server).await;
    let endpoints = Endpoints::rooted_at(&f.server.uri());
    let auth = Auth::new(reqwest::Client::new(), endpoints.token, oauth(), "revoked");
    let (_store, _events, engine) = engine_for(&f.server.uri(), Some(auth));
    let err = engine.round(false).await.unwrap_err();
    assert!(err.needs_reauth(), "{err}");
}

#[tokio::test]
async fn forbidden_tasks_api_is_skipped() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    Mock::given(method("GET")).and(path(EVENTS))
        .respond_with(ok(json!({"items": [], "nextSyncToken": "s1"}))).mount(&f.server).await;
    Mock::given(method("GET")).and(path("/tasks/v1/users/@me/lists"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": {"errors": [{"reason": "forbidden"}]}})))
        .mount(&f.server).await;
    f.engine.round(false).await.unwrap();
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p takt-sync --test engine`
Expected: compile errors, `takt_sync::AccountEngine` does not exist.

- [ ] **Step 4: Implement `messages.rs`**

`crates/takt-sync/src/messages.rs`:

```rust
use chrono::{DateTime, Utc};
use takt_store::ChangeSet;

/// UI → sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    AddAccount,
    RemoveAccount { account_id: String },
    SyncNow,
    WindowFocus(bool),
    Shutdown,
}

/// Sync → UI. Never carries event data: the UI re-reads the store.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncEvent {
    Changed { account_id: String, changes: ChangeSet },
    Status { account_id: String, state: SyncState },
    AuthNeeded { account_id: String },
    AccountsChanged,
    SignInFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Synced(DateTime<Utc>),
    Syncing,
    Offline(DateTime<Utc>),
    Error(String),
    NeedsReauth,
}
```

- [ ] **Step 5: Implement `engine.rs`**

`crates/takt-sync/src/engine.rs`:

```rust
//! One sync round for one account, and what to do when Google says no.

use std::sync::{Arc, Mutex};

use chrono::TimeDelta;
use futures::stream::{self, StreamExt};
use takt_google::{GoogleClient, GoogleError};
use takt_store::{ChangeSet, Store, StoreError};

use crate::SyncEvent;

pub const PARALLEL_CALENDARS: usize = 4;

/// The Tasks API has no sync tokens; ask again for a minute before the
/// checkpoint so clock skew cannot lose an update. Upserts make repeats harmless.
const TASKS_OVERLAP: TimeDelta = TimeDelta::seconds(60);

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Google(#[from] GoogleError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl SyncError {
    pub fn is_offline(&self) -> bool {
        matches!(self, SyncError::Google(GoogleError::Network(_)))
    }

    pub fn needs_reauth(&self) -> bool {
        matches!(self, SyncError::Google(GoogleError::InvalidGrant))
    }
}

pub struct AccountEngine {
    account_id: String,
    client: GoogleClient,
    store: Arc<Mutex<Store>>,
    events: async_channel::Sender<SyncEvent>,
}

impl AccountEngine {
    pub fn new(
        account_id: impl Into<String>,
        client: GoogleClient,
        store: Arc<Mutex<Store>>,
        events: async_channel::Sender<SyncEvent>,
    ) -> Self {
        Self { account_id: account_id.into(), client, store, events }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// Calendar list, then settings and colours when asked, then every
    /// non-hidden calendar's events (primary first, four at a time), then tasks.
    pub async fn round(&self, with_meta: bool) -> Result<(), SyncError> {
        self.sync_calendar_list().await?;
        if with_meta {
            self.sync_meta().await?;
        }
        let calendars: Vec<String> = self
            .with_store(|s| s.calendars(Some(&self.account_id)))?
            .into_iter()
            .filter(|c| !c.hidden)
            .map(|c| c.calendar_id)
            .collect();
        let results: Vec<Result<(), SyncError>> = stream::iter(calendars.iter().map(|id| self.sync_calendar(id)))
            .buffer_unordered(PARALLEL_CALENDARS)
            .collect()
            .await;
        results.into_iter().collect::<Result<(), _>>()?;
        self.sync_tasks().await
    }

    async fn sync_calendar_list(&self) -> Result<(), SyncError> {
        let mut reset = false;
        'restart: loop {
            let token = self.with_store(|s| s.calendar_list_sync_token(&self.account_id))?;
            let mut page_token: Option<String> = None;
            loop {
                match self.client.calendar_list_page(token.as_deref(), page_token.as_deref()).await {
                    Ok(page) => {
                        let changes = self.with_store(|s| {
                            s.apply_calendar_list_page(&self.account_id, &page.items, page.next_sync_token.as_deref())
                        })?;
                        self.emit(changes);
                        match page.next_page_token {
                            Some(next) => page_token = Some(next),
                            None => return Ok(()),
                        }
                    }
                    Err(GoogleError::Gone) if !reset => {
                        reset = true;
                        tracing::info!(account = %self.account_id, "calendar list token expired; full re-sync");
                        self.with_store(|s| s.reset_account_sync(&self.account_id))?;
                        self.emit_everything();
                        continue 'restart;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }

    async fn sync_meta(&self) -> Result<(), SyncError> {
        let colors = self.client.colors().await?;
        let settings = self.client.settings().await?;
        self.with_store(|s| {
            s.put_colors(&self.account_id, &colors)?;
            s.put_settings(&self.account_id, &settings)
        })?;
        self.emit_everything();
        Ok(())
    }

    async fn sync_calendar(&self, calendar_id: &str) -> Result<(), SyncError> {
        let mut reset = false;
        'restart: loop {
            let token = self.with_store(|s| s.events_sync_token(&self.account_id, calendar_id))?;
            let mut page_token: Option<String> = None;
            loop {
                match self.client.events_page(calendar_id, token.as_deref(), page_token.as_deref()).await {
                    Ok(page) => {
                        let changes = self.with_store(|s| {
                            s.apply_events_page(&self.account_id, calendar_id, &page.items, page.next_sync_token.as_deref())
                        })?;
                        self.emit(changes);
                        match page.next_page_token {
                            Some(next) => page_token = Some(next),
                            None => return Ok(()),
                        }
                    }
                    Err(GoogleError::Gone) if !reset => {
                        reset = true;
                        tracing::info!(account = %self.account_id, calendar = %calendar_id, "events token expired; full re-sync");
                        self.with_store(|s| s.reset_calendar_events(&self.account_id, calendar_id))?;
                        self.emit_everything();
                        continue 'restart;
                    }
                    Err(GoogleError::NotFound | GoogleError::Forbidden { .. }) => {
                        tracing::info!(account = %self.account_id, calendar = %calendar_id, "lost access; removing calendar");
                        self.with_store(|s| s.remove_calendar(&self.account_id, calendar_id))?;
                        self.emit_everything();
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }

    async fn sync_tasks(&self) -> Result<(), SyncError> {
        let lists = match self.client.task_lists().await {
            Ok(lists) => lists,
            Err(GoogleError::Forbidden { reason }) => {
                tracing::warn!(account = %self.account_id, %reason, "Tasks API not available; skipping tasks");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let changes = self.with_store(|s| s.apply_task_lists(&self.account_id, &lists))?;
        self.emit(changes);
        for list in &lists {
            let list_id = list.fields.id.as_str();
            let since = self
                .with_store(|s| s.task_checkpoint(&self.account_id, list_id))?
                .map(|checkpoint| checkpoint - TASKS_OVERLAP);
            let mut page_token: Option<String> = None;
            loop {
                let page = match self.client.tasks_page(list_id, since, page_token.as_deref()).await {
                    Ok(page) => page,
                    Err(GoogleError::NotFound) => break,
                    Err(e) => return Err(e.into()),
                };
                let changes = self.with_store(|s| s.apply_tasks_page(&self.account_id, list_id, &page.items))?;
                self.emit(changes);
                match page.next_page_token {
                    Some(next) => page_token = Some(next),
                    None => break,
                }
            }
        }
        Ok(())
    }

    fn with_store<T>(&self, f: impl FnOnce(&mut Store) -> Result<T, StoreError>) -> Result<T, SyncError> {
        let mut store = self.store.lock().expect("store mutex poisoned");
        Ok(f(&mut store)?)
    }

    fn emit(&self, changes: ChangeSet) {
        if !changes.is_empty() {
            let _ = self.events.try_send(SyncEvent::Changed { account_id: self.account_id.clone(), changes });
        }
    }

    fn emit_everything(&self) {
        self.emit(ChangeSet { everything: true, ..ChangeSet::default() });
    }
}
```

`crates/takt-sync/src/lib.rs`:

```rust
//! Background sync engine for takt.

mod engine;
mod messages;

pub use engine::{AccountEngine, PARALLEL_CALENDARS, SyncError};
pub use messages::{Command, SyncEvent, SyncState};
```

- [ ] **Step 6: Run the tests to see them pass**

Run: `cargo test -p takt-sync --test engine`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/takt-sync Cargo.lock
git commit -m "feat(takt-sync): add per-account sync round with 410, 404 and offline handling"
```

---

### Task 13: Sync runtime, scheduling and account management

**Files:**
- Modify: `crates/takt-sync/src/lib.rs`
- Create: `crates/takt-sync/src/runtime.rs`, `crates/takt-sync/tests/runtime.rs`

**Interfaces:**
- Consumes: `AccountEngine`, `sign_in`, `TokenStore`, `Store`.
- Produces:
  - `Intervals { focused, unfocused, meta, offline_max, restart_max: Duration }` (Default 60 s / 300 s / 3600 s / 300 s / 300 s)
  - `SyncConfig { db_path: PathBuf, endpoints: Endpoints, oauth: OAuthClient, tokens: Arc<dyn TokenStore>, open_browser: Arc<dyn Fn(&Url) + Send + Sync>, intervals: Intervals, retry: RetryPolicy }`
  - `start(SyncConfig) -> std::io::Result<SyncHandle>`; `SyncHandle { commands: async_channel::Sender<Command>, events: async_channel::Receiver<SyncEvent> }` with `shutdown(self)`
  - crate-internal `supervise(make, on_panic, max_backoff)` and `AbortOnDrop<T>`

- [ ] **Step 1: Write the failing tests**

`crates/takt-sync/tests/runtime.rs`:

```rust
mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use common::*;
use serde_json::json;
use takt_google::{Endpoints, MemoryTokenStore, RetryPolicy, TokenStore, Url};
use takt_store::{AccountStatus, Store};
use takt_sync::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Rig {
    server: MockServer,
    dir: tempfile::TempDir,
    tokens: Arc<MemoryTokenStore>,
}

impl Rig {
    async fn new() -> Self {
        Self { server: MockServer::start().await, dir: tempfile::tempdir().unwrap(), tokens: Arc::new(MemoryTokenStore::default()) }
    }

    fn db(&self) -> std::path::PathBuf {
        self.dir.path().join("takt.sqlite3")
    }

    fn seed_account(&self) {
        Store::open(&self.db()).unwrap().upsert_account("acc", "me@example.com").unwrap();
        self.tokens.set("me@example.com", "rt").unwrap();
    }

    fn start(&self, open_browser: Arc<dyn Fn(&Url) + Send + Sync>) -> SyncHandle {
        start(SyncConfig {
            db_path: self.db(),
            endpoints: Endpoints::rooted_at(&self.server.uri()),
            oauth: oauth(),
            tokens: self.tokens.clone(),
            open_browser,
            intervals: Intervals { focused: Duration::from_secs(3600), unfocused: Duration::from_secs(3600), ..Intervals::default() },
            retry: RetryPolicy { max_attempts: 1, base_delay: Duration::from_millis(1), max_delay: Duration::from_millis(1) },
        })
        .unwrap()
    }

    async fn mount_happy_google(&self) {
        Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ok(json!({"access_token": "at", "expires_in": 3600}))).mount(&self.server).await;
        mount_calendar_list(&self.server).await;
        mount_meta(&self.server).await;
        mount_no_tasks(&self.server).await;
        Mock::given(method("GET")).and(path("/calendar/v3/calendars/cal/events"))
            .respond_with(ok(json!({"items": [], "nextSyncToken": "s"}))).mount(&self.server).await;
    }
}

fn no_browser() -> Arc<dyn Fn(&Url) + Send + Sync> {
    Arc::new(|_| {})
}

async fn wait_for(events: &async_channel::Receiver<SyncEvent>, wanted: impl Fn(&SyncEvent) -> bool) -> SyncEvent {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let ev = events.recv().await.expect("sync thread ended");
            if wanted(&ev) {
                return ev;
            }
        }
    })
    .await
    .expect("expected event did not arrive")
}

fn synced(account: &str) -> impl Fn(&SyncEvent) -> bool + '_ {
    move |ev| matches!(ev, SyncEvent::Status { account_id, state: SyncState::Synced(_) } if account_id == account)
}

#[tokio::test]
async fn existing_account_syncs_on_start_and_on_sync_now() {
    let rig = Rig::new().await;
    rig.seed_account();
    rig.mount_happy_google().await;
    let handle = rig.start(no_browser());
    wait_for(&handle.events, synced("acc")).await;
    handle.commands.send(Command::SyncNow).await.unwrap();
    wait_for(&handle.events, synced("acc")).await;
    let list_calls = rig.server.received_requests().await.unwrap().iter()
        .filter(|r| r.url.path() == "/calendar/v3/users/me/calendarList").count();
    assert!(list_calls >= 2);
    handle.shutdown();
}

#[tokio::test]
async fn revoked_token_marks_account_and_asks_for_sign_in() {
    let rig = Rig::new().await;
    rig.seed_account();
    Mock::given(method("POST")).and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .mount(&rig.server).await;
    let handle = rig.start(no_browser());
    wait_for(&handle.events, |ev| matches!(ev, SyncEvent::AuthNeeded { account_id } if account_id == "acc")).await;
    let accounts = Store::open_read_only(&rig.db()).unwrap().accounts().unwrap();
    assert_eq!(accounts[0].status, AccountStatus::NeedsReauth);
    handle.shutdown();
}

#[tokio::test]
async fn missing_refresh_token_asks_for_sign_in() {
    let rig = Rig::new().await;
    Store::open(&rig.db()).unwrap().upsert_account("acc", "me@example.com").unwrap();
    let handle = rig.start(no_browser());
    wait_for(&handle.events, |ev| matches!(ev, SyncEvent::AuthNeeded { .. })).await;
    handle.shutdown();
}

#[tokio::test]
async fn remove_account_deletes_rows_and_token() {
    let rig = Rig::new().await;
    rig.seed_account();
    rig.mount_happy_google().await;
    let handle = rig.start(no_browser());
    wait_for(&handle.events, synced("acc")).await;
    handle.commands.send(Command::RemoveAccount { account_id: "acc".into() }).await.unwrap();
    wait_for(&handle.events, |ev| matches!(ev, SyncEvent::AccountsChanged)).await;
    assert!(Store::open_read_only(&rig.db()).unwrap().accounts().unwrap().is_empty());
    assert_eq!(rig.tokens.get("me@example.com").unwrap(), None);
    handle.shutdown();
}

#[tokio::test]
async fn add_account_signs_in_and_starts_syncing() {
    let rig = Rig::new().await;
    rig.mount_happy_google().await;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({"sub": "sub-1", "email": "new@example.com"}).to_string());
    Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ok(json!({"access_token": "at", "expires_in": 3600, "refresh_token": "rt-new", "id_token": format!("h.{payload}.s")})))
        .mount(&rig.server).await;
    let browser: Arc<dyn Fn(&Url) + Send + Sync> = Arc::new(|url: &Url| {
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let callback = format!("{}/?code=c&state={}", q["redirect_uri"], q["state"]);
        tokio::spawn(async move {
            let _ = reqwest::get(callback).await;
        });
    });
    let handle = rig.start(browser);
    handle.commands.send(Command::AddAccount).await.unwrap();
    wait_for(&handle.events, |ev| matches!(ev, SyncEvent::AccountsChanged)).await;
    wait_for(&handle.events, synced("sub-1")).await;
    assert_eq!(rig.tokens.get("new@example.com").unwrap().as_deref(), Some("rt-new"));
    handle.shutdown();
}
```

Add `base64.workspace = true` to `[dev-dependencies]` of `crates/takt-sync/Cargo.toml`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt-sync --test runtime`
Expected: compile errors, `start`, `SyncConfig` and the rest are not defined.

- [ ] **Step 3: Implement `runtime.rs`**

`crates/takt-sync/src/runtime.rs`:

```rust
//! The sync thread: one tokio runtime, a supervisor that owns account loops,
//! and the polling schedule.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use takt_google::{Auth, Endpoints, GoogleClient, GoogleError, OAuthClient, RetryPolicy, SignedIn, TokenStore, Url};
use takt_store::{AccountStatus, Store};
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinError, JoinHandle};

use crate::{AccountEngine, Command, SyncEvent, SyncState};

const OFFLINE_FIRST_RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct Intervals {
    pub focused: Duration,
    pub unfocused: Duration,
    pub meta: Duration,
    pub offline_max: Duration,
    pub restart_max: Duration,
}

impl Default for Intervals {
    fn default() -> Self {
        Self {
            focused: Duration::from_secs(60),
            unfocused: Duration::from_secs(300),
            meta: Duration::from_secs(3600),
            offline_max: Duration::from_secs(300),
            restart_max: Duration::from_secs(300),
        }
    }
}

pub struct SyncConfig {
    pub db_path: PathBuf,
    pub endpoints: Endpoints,
    pub oauth: OAuthClient,
    pub tokens: Arc<dyn TokenStore>,
    pub open_browser: Arc<dyn Fn(&Url) + Send + Sync>,
    pub intervals: Intervals,
    pub retry: RetryPolicy,
}

pub struct SyncHandle {
    pub commands: async_channel::Sender<Command>,
    pub events: async_channel::Receiver<SyncEvent>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SyncHandle {
    /// Stops every account loop and waits for the sync thread to exit.
    pub fn shutdown(mut self) {
        let _ = self.commands.try_send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start(config: SyncConfig) -> std::io::Result<SyncHandle> {
    let (command_tx, command_rx) = async_channel::unbounded();
    let (event_tx, event_rx) = async_channel::unbounded();
    let thread = std::thread::Builder::new().name("takt-sync".into()).spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("cannot build the sync runtime");
        runtime.block_on(run(config, command_rx, event_tx));
    })?;
    Ok(SyncHandle { commands: command_tx, events: event_rx, thread: Some(thread) })
}

/// State every account loop reads.
struct Shared {
    store: Arc<Mutex<Store>>,
    http: reqwest::Client,
    endpoints: Endpoints,
    oauth: OAuthClient,
    tokens: Arc<dyn TokenStore>,
    open_browser: Arc<dyn Fn(&Url) + Send + Sync>,
    intervals: Intervals,
    retry: RetryPolicy,
    events: async_channel::Sender<SyncEvent>,
    focused: watch::Receiver<bool>,
    wake: watch::Receiver<u64>,
}

impl Shared {
    fn send(&self, event: SyncEvent) {
        let _ = self.events.try_send(event);
    }

    fn status(&self, account_id: &str, state: SyncState) {
        self.send(SyncEvent::Status { account_id: account_id.to_owned(), state });
    }

    fn poll_interval(&self) -> Duration {
        if *self.focused.borrow() { self.intervals.focused } else { self.intervals.unfocused }
    }

    fn mark_needs_reauth(&self, account_id: &str) {
        if let Err(error) = self.store.lock().expect("store mutex poisoned").set_account_status(account_id, AccountStatus::NeedsReauth) {
            tracing::error!(%error, "cannot record re-auth status");
        }
        self.status(account_id, SyncState::NeedsReauth);
        self.send(SyncEvent::AuthNeeded { account_id: account_id.to_owned() });
    }
}

async fn run(config: SyncConfig, commands: async_channel::Receiver<Command>, events: async_channel::Sender<SyncEvent>) {
    let store = match Store::open(&config.db_path) {
        Ok(store) => store,
        Err(error) => {
            tracing::error!(%error, "sync cannot open the database");
            let _ = events.try_send(SyncEvent::SignInFailed(format!("cannot open the database: {error}")));
            return;
        }
    };
    let (focus_tx, focused) = watch::channel(true);
    let (wake_tx, wake) = watch::channel(0u64);
    let shared = Arc::new(Shared {
        store: Arc::new(Mutex::new(store)),
        http: reqwest::Client::builder().timeout(Duration::from_secs(30)).build().expect("HTTP client"),
        endpoints: config.endpoints,
        oauth: config.oauth,
        tokens: config.tokens,
        open_browser: config.open_browser,
        intervals: config.intervals,
        retry: config.retry,
        events,
        focused,
        wake,
    });
    let (sign_in_tx, mut sign_ins) = mpsc::unbounded_channel::<Result<SignedIn, GoogleError>>();
    let mut loops: HashMap<String, AbortOnDrop<()>> = HashMap::new();

    let accounts = shared.store.lock().expect("store mutex poisoned").accounts().unwrap_or_default();
    for account in accounts {
        match account.status {
            AccountStatus::Ok => {
                loops.insert(account.account_id.clone(), spawn_account(&shared, account.account_id, account.email, None));
            }
            AccountStatus::NeedsReauth => {
                shared.status(&account.account_id, SyncState::NeedsReauth);
                shared.send(SyncEvent::AuthNeeded { account_id: account.account_id });
            }
        }
    }

    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Ok(Command::Shutdown) | Err(_) => break,
                Ok(Command::AddAccount) => {
                    let shared = shared.clone();
                    let sign_in_tx = sign_in_tx.clone();
                    tokio::spawn(async move {
                        let open = shared.open_browser.clone();
                        let result = takt_google::sign_in(&shared.http, &shared.endpoints, &shared.oauth, |url| open(url)).await;
                        let _ = sign_in_tx.send(result);
                    });
                }
                Ok(Command::RemoveAccount { account_id }) => {
                    loops.remove(&account_id);
                    remove_account(&shared, &account_id);
                }
                Ok(Command::SyncNow) => {
                    wake_tx.send_modify(|n| *n += 1);
                }
                Ok(Command::WindowFocus(focused)) => {
                    focus_tx.send_replace(focused);
                    if focused {
                        wake_tx.send_modify(|n| *n += 1);
                    }
                }
            },
            Some(result) = sign_ins.recv() => match result {
                Ok(signed) => {
                    if let Some(handle) = finish_sign_in(&shared, signed) {
                        loops.insert(handle.0, handle.1);
                    }
                }
                Err(error) => shared.send(SyncEvent::SignInFailed(error.to_string())),
            },
        }
    }
}

fn finish_sign_in(shared: &Arc<Shared>, signed: SignedIn) -> Option<(String, AbortOnDrop<()>)> {
    if let Err(error) = shared.tokens.set(&signed.email, &signed.refresh_token) {
        shared.send(SyncEvent::SignInFailed(format!("cannot save the sign-in to the keychain: {error}")));
        return None;
    }
    if let Err(error) = shared.store.lock().expect("store mutex poisoned").upsert_account(&signed.sub, &signed.email) {
        shared.send(SyncEvent::SignInFailed(format!("cannot save the account: {error}")));
        return None;
    }
    shared.send(SyncEvent::AccountsChanged);
    let seed = Some((signed.access_token, Duration::from_secs(signed.expires_in)));
    let handle = spawn_account(shared, signed.sub.clone(), signed.email, seed);
    Some((signed.sub, handle))
}

fn remove_account(shared: &Arc<Shared>, account_id: &str) {
    let mut store = shared.store.lock().expect("store mutex poisoned");
    let email = store
        .accounts()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a.account_id == account_id)
        .map(|a| a.email);
    if let Some(email) = email {
        if let Err(error) = shared.tokens.delete(&email) {
            tracing::warn!(%error, "cannot delete the keychain entry");
        }
    }
    if let Err(error) = store.remove_account(account_id) {
        tracing::error!(%error, "cannot remove the account");
    }
    drop(store);
    shared.send(SyncEvent::AccountsChanged);
}

fn spawn_account(
    shared: &Arc<Shared>,
    account_id: String,
    email: String,
    seed: Option<(String, Duration)>,
) -> AbortOnDrop<()> {
    let shared = shared.clone();
    AbortOnDrop(tokio::spawn(async move {
        let restart_max = shared.intervals.restart_max;
        let on_panic = {
            let shared = shared.clone();
            let account_id = account_id.clone();
            move || shared.status(&account_id, SyncState::Error("sync crashed; restarting".into()))
        };
        supervise(
            move || account_loop(shared.clone(), account_id.clone(), email.clone(), seed.clone()),
            on_panic,
            restart_max,
        )
        .await;
    }))
}

async fn account_loop(shared: Arc<Shared>, account_id: String, email: String, seed: Option<(String, Duration)>) {
    let refresh_token = match shared.tokens.get(&email) {
        Ok(Some(token)) => token,
        Ok(None) => return shared.mark_needs_reauth(&account_id),
        Err(error) => return shared.status(&account_id, SyncState::Error(error.to_string())),
    };
    let mut auth = Auth::new(shared.http.clone(), shared.endpoints.token.clone(), shared.oauth.clone(), refresh_token);
    if let Some((token, ttl)) = seed {
        auth = auth.with_access_token(token, ttl);
    }
    let client = GoogleClient::new(shared.http.clone(), shared.endpoints.clone(), auth).with_retry(shared.retry.clone());
    let engine = AccountEngine::new(account_id.clone(), client, shared.store.clone(), shared.events.clone());
    let mut wake = shared.wake.clone();
    let mut last_meta: Option<Instant> = None;
    let mut offline_since: Option<DateTime<Utc>> = None;
    let mut offline_delay = OFFLINE_FIRST_RETRY;
    loop {
        let with_meta = last_meta.is_none_or(|t| t.elapsed() >= shared.intervals.meta);
        shared.status(&account_id, SyncState::Syncing);
        let wait = match engine.round(with_meta).await {
            Ok(()) => {
                if with_meta {
                    last_meta = Some(Instant::now());
                }
                offline_since = None;
                offline_delay = OFFLINE_FIRST_RETRY;
                shared.status(&account_id, SyncState::Synced(Utc::now()));
                shared.poll_interval()
            }
            Err(error) if error.needs_reauth() => return shared.mark_needs_reauth(&account_id),
            Err(error) if error.is_offline() => {
                let since = *offline_since.get_or_insert_with(Utc::now);
                shared.status(&account_id, SyncState::Offline(since));
                let wait = offline_delay;
                offline_delay = (offline_delay * 2).min(shared.intervals.offline_max);
                wait
            }
            Err(error) => {
                tracing::warn!(account = %account_id, %error, "sync round failed");
                shared.status(&account_id, SyncState::Error(error.to_string()));
                shared.poll_interval()
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            changed = wake.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

/// A task handle that aborts the task when dropped.
pub(crate) struct AbortOnDrop<T>(pub(crate) JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs `make()` as a task and starts it again, with backoff, whenever it
/// panics. Returns when a run finishes normally.
pub(crate) async fn supervise<F, Fut>(mut make: F, on_panic: impl Fn(), max_backoff: Duration)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut delay = Duration::from_secs(1).min(max_backoff);
    loop {
        let mut task = AbortOnDrop(tokio::spawn(make()));
        let outcome: Result<(), JoinError> = (&mut task.0).await;
        match outcome {
            Ok(()) => return,
            Err(error) if error.is_panic() => {
                on_panic();
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_backoff);
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn panicking_task_is_restarted() {
        let runs = Arc::new(AtomicUsize::new(0));
        let panics = Arc::new(AtomicUsize::new(0));
        let make = {
            let runs = runs.clone();
            move || {
                let runs = runs.clone();
                async move {
                    if runs.fetch_add(1, SeqCst) == 0 {
                        panic!("first run fails");
                    }
                }
            }
        };
        let on_panic = {
            let panics = panics.clone();
            move || {
                panics.fetch_add(1, SeqCst);
            }
        };
        supervise(make, on_panic, Duration::from_millis(1)).await;
        assert_eq!(runs.load(SeqCst), 2);
        assert_eq!(panics.load(SeqCst), 1);
    }

    #[tokio::test]
    async fn dropping_the_handle_aborts_the_task() {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let handle = AbortOnDrop(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            flag.store(true, SeqCst);
        }));
        drop(handle);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!finished.load(SeqCst));
    }
}
```

`crates/takt-sync/src/lib.rs`:

```rust
//! Background sync engine for takt.

mod engine;
mod messages;
mod runtime;

pub use engine::{AccountEngine, PARALLEL_CALENDARS, SyncError};
pub use messages::{Command, SyncEvent, SyncState};
pub use runtime::{Intervals, SyncConfig, SyncHandle, start};
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p takt-sync`
Expected: PASS for the unit tests in `runtime`, `engine` and `runtime` integration tests. A panic message from `panicking_task_is_restarted` on stderr is expected.

- [ ] **Step 5: Commit**

```bash
git add crates/takt-sync
git commit -m "feat(takt-sync): run account loops on a sync thread with sign-in, removal and restart"
```

---

### Task 14: Golden recurrence fixtures

This task needs your human partner's Google account. Steps 1–3 are code; Step 4 is theirs.

**Files:**
- Modify: `crates/takt-google/src/api.rs`, `crates/takt-google/tests/api.rs`
- Create: `crates/takt-google/examples/capture.rs`, `crates/takt-recur/tests/golden.rs`, `crates/takt-recur/tests/fixtures/*.json`

**Interfaces:**
- Produces: `GoogleClient::instances(&self, calendar_id: &str, event_id: &str, time_min: DateTime<Utc>, time_max: DateTime<Utc>) -> Result<Vec<Item<EventFields>>, GoogleError>`; fixture files with shape `{"name", "range": {"start", "end"}, "master", "exceptions": [...], "instances": [...]}`.

- [ ] **Step 1: Add the `instances` endpoint, test first**

Append to `crates/takt-google/tests/api.rs`:

```rust
#[tokio::test]
async fn instances_collects_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/cal/events/m/instances"))
        .and(query_param("timeMin", "2026-01-01T00:00:00Z"))
        .and(query_param("timeMax", "2027-01-01T00:00:00Z"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ok(json!({"items": [{"id": "m_1"}], "nextPageToken": "n"})))
        .mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/cal/events/m/instances"))
        .and(query_param("pageToken", "n"))
        .respond_with(ok(json!({"items": [{"id": "m_2"}]})))
        .mount(&server).await;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
    let got = client_at(&server).instances("cal", "m", from, to).await.unwrap();
    assert_eq!(got.len(), 2);
}
```

Run `cargo test -p takt-google --test api instances`; expect a compile error. Then add to the `impl GoogleClient` block in `crates/takt-google/src/api.rs`:

```rust
    /// Google's own expansion of a series. Used only to capture test fixtures.
    pub async fn instances(
        &self,
        calendar_id: &str,
        event_id: &str,
        time_min: DateTime<Utc>,
        time_max: DateTime<Utc>,
    ) -> Result<Vec<Item<EventFields>>, GoogleError> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = join(&self.endpoints.calendar, &["calendars", calendar_id, "events", event_id, "instances"])?;
            url.query_pairs_mut()
                .append_pair("timeMin", &time_min.to_rfc3339_opts(SecondsFormat::Secs, true))
                .append_pair("timeMax", &time_max.to_rfc3339_opts(SecondsFormat::Secs, true))
                .append_pair("maxResults", "2500");
            push_tokens(&mut url, None, page_token.as_deref());
            let page: Page<EventFields> = Page::from_value(self.get_json(url).await?)?;
            out.extend(page.items);
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(out),
            }
        }
    }
```

Run `cargo test -p takt-google --test api`; expect PASS.

- [ ] **Step 2: Write the capture tool**

`crates/takt-google/examples/capture.rs`:

```rust
//! Captures recurring series and Google's expansion of them as test fixtures.
//!
//! cargo run -p takt-google --example capture -- \
//!     --config <path to config.toml> --email you@gmail.com --calendar takt-fixtures \
//!     --from 2026-01-01 --to 2028-01-01 --out crates/takt-recur/tests/fixtures

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use takt_google::model::{EventFields, Item};
use takt_google::{Auth, Endpoints, GoogleClient, KeyringTokenStore, OAuthClient, TokenStore};

#[derive(Deserialize)]
struct Config {
    google: GoogleSection,
}

#[derive(Deserialize)]
struct GoogleSection {
    client_id: String,
    client_secret: String,
}

fn arg(args: &HashMap<String, String>, name: &str) -> String {
    args.get(name).cloned().unwrap_or_else(|| panic!("missing --{name}"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args: HashMap<String, String> = raw
        .chunks(2)
        .map(|pair| (pair[0].trim_start_matches("--").to_owned(), pair.get(1).cloned().unwrap_or_default()))
        .collect();
    let config: Config = toml::from_str(&std::fs::read_to_string(arg(&args, "config"))?)?;
    let email = arg(&args, "email");
    let calendar_name = arg(&args, "calendar");
    let day = |name: &str| -> Result<_, Box<dyn std::error::Error>> {
        let d = NaiveDate::parse_from_str(&arg(&args, name), "%Y-%m-%d")?;
        Ok(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()))
    };
    let (from, to) = (day("from")?, day("to")?);
    let out = PathBuf::from(arg(&args, "out"));
    std::fs::create_dir_all(&out)?;

    let refresh = KeyringTokenStore.get(&email)?.ok_or("no refresh token for that email; sign in with takt first")?;
    let endpoints = Endpoints::google();
    let http = reqwest::Client::new();
    let oauth = OAuthClient { client_id: config.google.client_id, client_secret: config.google.client_secret };
    let client = GoogleClient::new(http.clone(), endpoints.clone(), Auth::new(http, endpoints.token, oauth, refresh));

    let mut calendar_id = None;
    let mut page_token: Option<String> = None;
    loop {
        let page = client.calendar_list_page(None, page_token.as_deref()).await?;
        if let Some(c) = page.items.iter().find(|c| c.fields.display_name() == calendar_name) {
            calendar_id = Some(c.fields.id.clone());
        }
        match page.next_page_token {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }
    let calendar_id = calendar_id.ok_or("calendar not found")?;

    let mut events: Vec<Item<EventFields>> = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let page = client.events_page(&calendar_id, None, page_token.as_deref()).await?;
        events.extend(page.items);
        match page.next_page_token {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }

    let mut scrubber = Scrubber::default();
    for master in events.iter().filter(|e| !e.fields.recurrence.is_empty() && !e.fields.is_cancelled()) {
        let exceptions: Vec<Value> = events
            .iter()
            .filter(|e| e.fields.recurring_event_id.as_deref() == Some(master.fields.id.as_str()))
            .map(|e| e.raw.clone())
            .collect();
        let instances: Vec<Value> = client
            .instances(&calendar_id, &master.fields.id, from, to)
            .await?
            .into_iter()
            .map(|i| i.raw)
            .collect();
        let name = master.fields.summary.clone().unwrap_or_else(|| master.fields.id.clone());
        let mut fixture = json!({
            "name": name,
            "range": {"start": from.to_rfc3339(), "end": to.to_rfc3339()},
            "master": master.raw,
            "exceptions": exceptions,
            "instances": instances,
        });
        scrubber.scrub(&mut fixture);
        let slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let file = out.join(format!("{}-{}.json", slug.trim_matches('-'), &master.fields.id[..8.min(master.fields.id.len())]));
        std::fs::write(&file, serde_json::to_string_pretty(&fixture)?)?;
        println!("wrote {}", file.display());
    }
    Ok(())
}

/// Replaces every email address with a stable placeholder and drops links
/// that encode the calendar's address.
#[derive(Default)]
struct Scrubber {
    emails: HashMap<String, String>,
}

impl Scrubber {
    fn scrub(&mut self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("htmlLink");
                for (key, v) in map.iter_mut() {
                    if key == "email" {
                        if let Value::String(email) = v {
                            let n = self.emails.len() + 1;
                            let placeholder = self.emails.entry(email.clone()).or_insert_with(|| format!("person{n}@example.com"));
                            *v = Value::String(placeholder.clone());
                        }
                    } else {
                        self.scrub(v);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|v| self.scrub(v)),
            _ => {}
        }
    }
}
```

Add `serde.workspace = true` is already a normal dependency; the example uses `toml` from `[dev-dependencies]`. Run `cargo build -p takt-google --example capture`; expect success.

- [ ] **Step 3: Write the golden test**

`crates/takt-recur/tests/golden.rs`:

```rust
//! Local expansion must match Google's `events.instances` for every captured series.

use std::path::Path;

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use serde_json::Value;
use takt_recur::{Exception, Master, Range, When, expand};

fn when(v: &Value) -> Option<When> {
    if let Some(date) = v.get("date").and_then(Value::as_str) {
        return NaiveDate::parse_from_str(date, "%Y-%m-%d").ok().map(When::Date);
    }
    let text = v.get("dateTime")?.as_str()?;
    DateTime::parse_from_rfc3339(text).ok().map(|t| When::At(t.with_timezone(&Utc)))
}

fn master_from(v: &Value) -> Master {
    Master {
        id: v["id"].as_str().unwrap().to_owned(),
        start: when(&v["start"]).unwrap(),
        end: when(&v["end"]).unwrap(),
        tz: v["start"]["timeZone"].as_str().and_then(|n| n.parse().ok()).unwrap_or(chrono_tz::UTC),
        recurrence: v["recurrence"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_owned()).collect(),
    }
}

fn exception_from(v: &Value) -> Option<Exception> {
    let original_start = when(&v["originalStartTime"])?;
    let start = when(&v["start"]).unwrap_or(original_start);
    let end = when(&v["end"]).unwrap_or(start);
    Some(Exception {
        id: v["id"].as_str()?.to_owned(),
        original_start,
        cancelled: v["status"] == "cancelled",
        start,
        end,
    })
}

fn instant(v: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(v.as_str().unwrap()).unwrap().with_timezone(&Utc)
}

#[test]
fn local_expansion_matches_google() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    assert!(!files.is_empty(), "no fixtures in {}; capture them as README.md describes", dir.display());

    let mut failures = Vec::new();
    for file in files {
        let fixture: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let range = Range { start: instant(&fixture["range"]["start"]), end: instant(&fixture["range"]["end"]) };
        // Google reads all-day dates in the calendar's zone at the range edges; compare away from them.
        let inner = Range { start: range.start + TimeDelta::days(1), end: range.end - TimeDelta::days(1) };
        let keep = |start: &When| start.as_utc() >= inner.start && start.as_utc() < inner.end;

        let master = master_from(&fixture["master"]);
        let exceptions: Vec<_> = fixture["exceptions"].as_array().unwrap().iter().filter_map(exception_from).collect();
        let mut ours: Vec<(String, When, When)> = expand(&master, &exceptions, &range)
            .unwrap()
            .into_iter()
            .filter(|o| keep(&o.start))
            .map(|o| (o.instance_id, o.start, o.end))
            .collect();
        let mut google: Vec<(String, When, When)> = fixture["instances"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["status"] != "cancelled")
            .map(|i| (i["id"].as_str().unwrap().to_owned(), when(&i["start"]).unwrap(), when(&i["end"]).unwrap()))
            .filter(|(_, start, _)| keep(start))
            .collect();
        ours.sort_by(|a, b| a.0.cmp(&b.0));
        google.sort_by(|a, b| a.0.cmp(&b.0));
        if ours != google {
            let only_ours: Vec<_> = ours.iter().filter(|o| !google.contains(o)).take(3).collect();
            let only_google: Vec<_> = google.iter().filter(|g| !ours.contains(g)).take(3).collect();
            failures.push(format!(
                "{} ({}): {} ours vs {} Google's\n  only ours: {only_ours:?}\n  only Google's: {only_google:?}",
                file.display(),
                fixture["name"],
                ours.len(),
                google.len()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
```

Run `cargo test -p takt-recur --test golden`; expect FAIL with "no fixtures in …".

- [ ] **Step 4 (your human partner): Create the series and capture them**

Stop and ask your human partner to do the following, then wait:

1. Finish Tasks 15–20 far enough to sign in with takt, or sign in once with any build that runs `Command::AddAccount`, so the keychain holds a refresh token for their email. If Task 20 is not done yet, do Step 4 after Task 20 and come back.
2. In Google Calendar on the web, create a calendar named `takt-fixtures` with these series:
   - weekly on Monday 09:00 in America/New_York starting 2026-10-19 (crosses the 1 Nov 2026 DST change)
   - monthly on the 3rd Tuesday; monthly on the 31st
   - weekly ending on a date; daily ending after 10 occurrences
   - a yearly all-day series
   - a weekly series with one occurrence moved to another day and one deleted
   - a weekly series edited with "this and following events" partway through
   - a daily series whose event time zone (set in the event's time zone picker) differs from the calendar's
3. Run:

```bash
cargo run -p takt-google --example capture -- --config "<path to config.toml>" --email "<their email>" --calendar takt-fixtures --from 2026-01-01 --to 2028-01-01 --out crates/takt-recur/tests/fixtures
```

- [ ] **Step 5: Make the golden test pass**

Run: `cargo test -p takt-recur --test golden`
Expected: PASS. For each failure, the message lists the first differing occurrences. Fix the cause in `crates/takt-recur/src/normalize.rs` or `lib.rs`, add a unit test reproducing it in `tests/exceptions.rs` or `normalize::tests`, and re-run. Do not edit fixtures to make them pass.

- [ ] **Step 6: Commit**

Check the fixtures contain no real email addresses (`grep -rE "@(gmail|googlemail)\.com" crates/takt-recur/tests/fixtures` prints nothing), then:

```bash
git add crates/takt-google crates/takt-recur
git commit -m "test(takt-recur): check local expansion against captured Google series"
```

---

### Task 15: Agenda builder

**Files:**
- Modify: `crates/takt/Cargo.toml`, `crates/takt/src/main.rs`
- Create: `crates/takt/src/lib.rs`, `crates/takt/src/agenda.rs`

**Interfaces:**
- Consumes: `takt_store::{AccountRow, CalendarRow, ColorPalette, Occurrence, TaskItem, When, display_instant}`.
- Produces (`takt::agenda`): `AgendaInput<'a> { occurrences, tasks, accounts, calendars, palettes: &HashMap<String, ColorPalette>, tz, today, from, to (exclusive), use_24h }`; `enum RowKey { Event { account_id, calendar_id, event_id, instance_id }, Task { account_id, list_id, task_id } }`; `AgendaRow { key, time_label, title, location, color: u32, account_badge, declined, is_task, completed, warning }`; `DaySection { date, header, rows }`; `build_agenda(&AgendaInput) -> Vec<DaySection>`; `parse_hex(&str) -> Option<u32>`; `const DEFAULT_COLOR: u32`, `const TASK_COLOR: u32`. For task rows, `location` holds the task list's title.
- Produces (`takt`): `display_zone() -> chrono_tz::Tz` (OS zone, UTC fallback).

The `takt` crate becomes a library plus a thin binary so integration tests can reach its modules. Its first build compiles GPUI and takes several minutes.

- [ ] **Step 1: Add dependencies and the library target**

`crates/takt/Cargo.toml`, below `[package]`:

```toml
[dependencies]
async-channel.workspace = true
chrono.workspace = true
chrono-tz.workspace = true
directories.workspace = true
gpui-kit.workspace = true
iana-time-zone.workspace = true
open.workspace = true
serde.workspace = true
serde_json.workspace = true
takt-google.workspace = true
takt-store.workspace = true
takt-sync.workspace = true
toml.workspace = true
tracing.workspace = true
tracing-appender.workspace = true
tracing-subscriber.workspace = true

[dev-dependencies]
gpui-kit = { workspace = true, features = ["test-support"] }
tempfile.workspace = true
```

`crates/takt/src/lib.rs`:

```rust
//! takt: a desktop client for Google Calendar and Google Tasks.

pub mod agenda;

/// The OS time zone, used for every displayed time. Falls back to UTC.
pub fn display_zone() -> chrono_tz::Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::UTC)
}
```

Leave `crates/takt/src/main.rs` printing "Hello, world!" until Task 18.

- [ ] **Step 2: Write the failing tests**

Create `crates/takt/src/agenda.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{DateTime, NaiveDate, Utc};
    use chrono_tz::Tz;
    use takt_store::{AccountRow, AccountStatus, CalendarRow, ColorPalette, Occurrence, TaskItem, When};

    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn at(s: &str) -> When {
        When::At(DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
    }

    fn account(id: &str) -> AccountRow {
        AccountRow { account_id: id.into(), email: format!("{id}@example.com"), status: AccountStatus::Ok }
    }

    fn calendar(account: &str, color: &str) -> CalendarRow {
        CalendarRow {
            account_id: account.into(),
            calendar_id: "cal".into(),
            summary: "Cal".into(),
            time_zone: None,
            background_color: Some(color.into()),
            foreground_color: None,
            access_role: Some("owner".into()),
            selected: true,
            hidden: false,
            primary: true,
        }
    }

    fn occ(account: &str, id: &str, title: Option<&str>, start: When, end: When) -> Occurrence {
        Occurrence {
            account_id: account.into(),
            calendar_id: "cal".into(),
            event_id: id.into(),
            instance_id: id.into(),
            start,
            end,
            summary: title.map(str::to_owned),
            location: None,
            color_id: None,
            self_response: None,
            recurrence_error: false,
        }
    }

    struct Fixture {
        occurrences: Vec<Occurrence>,
        tasks: Vec<TaskItem>,
        accounts: Vec<AccountRow>,
        calendars: Vec<CalendarRow>,
        palettes: HashMap<String, ColorPalette>,
        use_24h: bool,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                occurrences: vec![],
                tasks: vec![],
                accounts: vec![account("a")],
                calendars: vec![calendar("a", "#9fe1e7")],
                palettes: HashMap::new(),
                use_24h: true,
            }
        }

        fn build(&self) -> Vec<DaySection> {
            build_agenda(&AgendaInput {
                occurrences: &self.occurrences,
                tasks: &self.tasks,
                accounts: &self.accounts,
                calendars: &self.calendars,
                palettes: &self.palettes,
                tz: Tz::UTC,
                today: d(17),
                from: d(17),
                to: d(31),
                use_24h: self.use_24h,
            })
        }
    }

    fn titles(section: &DaySection) -> Vec<&str> {
        section.rows.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn orders_all_day_events_then_tasks_then_timed() {
        let mut f = Fixture::new();
        f.occurrences = vec![
            occ("a", "late", Some("Late"), at("2026-10-20T15:00:00Z"), at("2026-10-20T16:00:00Z")),
            occ("a", "early", Some("Early"), at("2026-10-20T09:00:00Z"), at("2026-10-20T10:00:00Z")),
            occ("a", "ad", Some("Holiday"), When::Date(d(20)), When::Date(d(21))),
        ];
        f.tasks = vec![TaskItem {
            account_id: "a".into(), list_id: "L".into(), list_title: "My Tasks".into(), task_id: "t".into(),
            title: "Pay rent".into(), due_date: d(20), completed: false,
        }];
        let sections = f.build();
        assert_eq!(sections.len(), 1);
        assert_eq!(titles(&sections[0]), ["Holiday", "Pay rent", "Early", "Late"]);
        assert_eq!(sections[0].rows[2].time_label, "09:00–10:00");
        assert_eq!(sections[0].rows[1].location.as_deref(), Some("My Tasks"));
        assert!(sections[0].rows[1].is_task);
    }

    #[test]
    fn overnight_event_appears_on_both_days() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ("a", "n", Some("Night"), at("2026-10-20T22:00:00Z"), at("2026-10-21T01:00:00Z"))];
        let sections = f.build();
        assert_eq!(sections.iter().map(|s| s.date).collect::<Vec<_>>(), [d(20), d(21)]);
        assert_eq!(sections[0].rows[0].time_label, "From 22:00");
        assert_eq!(sections[1].rows[0].time_label, "Until 01:00");
    }

    #[test]
    fn event_ending_at_midnight_stays_on_one_day() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ("a", "e", Some("Evening"), at("2026-10-20T20:00:00Z"), at("2026-10-21T00:00:00Z"))];
        assert_eq!(f.build().len(), 1);
    }

    #[test]
    fn multi_day_all_day_event_repeats_each_day() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ("a", "trip", Some("Trip"), When::Date(d(20)), When::Date(d(23)))];
        let sections = f.build();
        assert_eq!(sections.iter().map(|s| s.date).collect::<Vec<_>>(), [d(20), d(21), d(22)]);
        assert!(sections.iter().all(|s| s.rows[0].time_label == "All day"));
    }

    #[test]
    fn event_colour_beats_calendar_colour() {
        let mut f = Fixture::new();
        let mut palette = ColorPalette::default();
        palette.event.insert("11".into(), ("#dc2127".into(), "#1d1d1d".into()));
        f.palettes.insert("a".into(), palette);
        let mut coloured = occ("a", "c", Some("Coloured"), at("2026-10-20T09:00:00Z"), at("2026-10-20T10:00:00Z"));
        coloured.color_id = Some("11".into());
        f.occurrences = vec![coloured, occ("a", "p", Some("Plain"), at("2026-10-20T11:00:00Z"), at("2026-10-20T12:00:00Z"))];
        let rows = &f.build()[0].rows;
        assert_eq!(rows[0].color, 0xdc2127);
        assert_eq!(rows[1].color, 0x9fe1e7);
    }

    #[test]
    fn badges_appear_only_with_several_accounts_and_duplicates_stay_separate() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ("a", "x", Some("Shared"), at("2026-10-20T09:00:00Z"), at("2026-10-20T10:00:00Z"))];
        assert_eq!(f.build()[0].rows[0].account_badge, None);
        f.accounts.push(account("b"));
        f.calendars.push(calendar("b", "#ffad46"));
        f.occurrences.push(occ("b", "x", Some("Shared"), at("2026-10-20T09:00:00Z"), at("2026-10-20T10:00:00Z")));
        let rows = &f.build()[0].rows;
        assert_eq!(rows.len(), 2);
        let badges: Vec<_> = rows.iter().map(|r| r.account_badge.clone().unwrap()).collect();
        assert!(badges.contains(&"a@example.com".to_owned()) && badges.contains(&"b@example.com".to_owned()));
    }

    #[test]
    fn declined_and_untitled_events() {
        let mut f = Fixture::new();
        let mut declined = occ("a", "d", None, at("2026-10-20T09:00:00Z"), at("2026-10-20T10:00:00Z"));
        declined.self_response = Some("declined".into());
        f.occurrences = vec![declined];
        let row = &f.build()[0].rows[0];
        assert!(row.declined);
        assert_eq!(row.title, "(No title)");
    }

    #[test]
    fn twelve_hour_labels() {
        let mut f = Fixture::new();
        f.use_24h = false;
        f.occurrences = vec![occ("a", "m", Some("M"), at("2026-10-20T09:00:00Z"), at("2026-10-20T13:30:00Z"))];
        assert_eq!(f.build()[0].rows[0].time_label, "9:00 AM–1:30 PM");
    }

    #[test]
    fn headers_name_today_and_tomorrow_and_skip_empty_days() {
        let mut f = Fixture::new();
        f.occurrences = vec![
            occ("a", "1", Some("A"), at("2026-10-17T09:00:00Z"), at("2026-10-17T10:00:00Z")),
            occ("a", "2", Some("B"), at("2026-10-18T09:00:00Z"), at("2026-10-18T10:00:00Z")),
            occ("a", "3", Some("C"), at("2026-10-22T09:00:00Z"), at("2026-10-22T10:00:00Z")),
        ];
        let headers: Vec<_> = f.build().into_iter().map(|s| s.header).collect();
        assert_eq!(headers, ["Today · Sat 17 Oct", "Tomorrow · Sun 18 Oct", "Thu 22 Oct"]);
    }

    #[test]
    fn hex_colours_parse() {
        assert_eq!(parse_hex("#a4bdfc"), Some(0xa4bdfc));
        assert_eq!(parse_hex("a4bdfc"), Some(0xa4bdfc));
        assert_eq!(parse_hex("#zzz"), None);
    }
}
```

Add `pub mod agenda;` is already in `lib.rs`.

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p takt --lib agenda`
Expected: compile errors, `build_agenda` and the types do not exist. (The first build compiles GPUI.)

- [ ] **Step 4: Implement the builder above the test module**

Put this at the top of `crates/takt/src/agenda.rs`:

```rust
//! Turns store rows into the agenda's day sections. Pure: no GPUI types.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeDelta, Utc};
use chrono_tz::Tz;
use takt_store::{AccountRow, CalendarRow, ColorPalette, Occurrence, TaskItem, When, display_instant};

pub const DEFAULT_COLOR: u32 = 0x039be5;
pub const TASK_COLOR: u32 = 0x4285f4;

pub struct AgendaInput<'a> {
    pub occurrences: &'a [Occurrence],
    pub tasks: &'a [TaskItem],
    pub accounts: &'a [AccountRow],
    pub calendars: &'a [CalendarRow],
    pub palettes: &'a HashMap<String, ColorPalette>,
    pub tz: Tz,
    pub today: NaiveDate,
    pub from: NaiveDate,
    /// Exclusive.
    pub to: NaiveDate,
    pub use_24h: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKey {
    Event { account_id: String, calendar_id: String, event_id: String, instance_id: String },
    Task { account_id: String, list_id: String, task_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgendaRow {
    pub key: RowKey,
    pub time_label: String,
    pub title: String,
    /// The event's location, or the task list's title for a task.
    pub location: Option<String>,
    pub color: u32,
    pub account_badge: Option<String>,
    pub declined: bool,
    pub is_task: bool,
    pub completed: bool,
    pub warning: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DaySection {
    pub date: NaiveDate,
    pub header: String,
    pub rows: Vec<AgendaRow>,
}

/// Rows sort by group (all-day events, tasks, timed), then start, then title.
type SortKey = (u8, DateTime<Utc>, String);

pub fn build_agenda(input: &AgendaInput) -> Vec<DaySection> {
    let several_accounts = input.accounts.len() > 1;
    let emails: HashMap<&str, &str> = input.accounts.iter().map(|a| (a.account_id.as_str(), a.email.as_str())).collect();
    let calendar_colors: HashMap<(&str, &str), u32> = input
        .calendars
        .iter()
        .filter_map(|c| Some(((c.account_id.as_str(), c.calendar_id.as_str()), parse_hex(c.background_color.as_deref()?)?)))
        .collect();
    let badge = |account_id: &str| several_accounts.then(|| emails.get(account_id).copied().unwrap_or(account_id).to_owned());
    let mut days: BTreeMap<NaiveDate, Vec<(SortKey, AgendaRow)>> = BTreeMap::new();

    for o in input.occurrences {
        let color = o
            .color_id
            .as_deref()
            .and_then(|id| input.palettes.get(&o.account_id)?.event.get(id))
            .and_then(|(background, _)| parse_hex(background))
            .or_else(|| calendar_colors.get(&(o.account_id.as_str(), o.calendar_id.as_str())).copied())
            .unwrap_or(DEFAULT_COLOR);
        let covered = days_covered(&o.start, &o.end, input.tz);
        let (first, last) = (covered[0], *covered.last().expect("at least one day"));
        for date in covered.into_iter().filter(|d| *d >= input.from && *d < input.to) {
            let (group, label) = label_for(o, date, first, last, input.tz, input.use_24h);
            let row = AgendaRow {
                key: RowKey::Event {
                    account_id: o.account_id.clone(),
                    calendar_id: o.calendar_id.clone(),
                    event_id: o.event_id.clone(),
                    instance_id: o.instance_id.clone(),
                },
                time_label: label,
                title: o.summary.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "(No title)".into()),
                location: o.location.clone(),
                color,
                account_badge: badge(&o.account_id),
                declined: o.self_response.as_deref() == Some("declined"),
                is_task: false,
                completed: false,
                warning: o.recurrence_error,
            };
            let key = (group, display_instant(&o.start, input.tz), row.title.clone());
            days.entry(date).or_default().push((key, row));
        }
    }

    for t in input.tasks.iter().filter(|t| t.due_date >= input.from && t.due_date < input.to) {
        let row = AgendaRow {
            key: RowKey::Task { account_id: t.account_id.clone(), list_id: t.list_id.clone(), task_id: t.task_id.clone() },
            time_label: "All day".into(),
            title: if t.title.trim().is_empty() { "(No title)".into() } else { t.title.clone() },
            location: Some(t.list_title.clone()),
            color: TASK_COLOR,
            account_badge: badge(&t.account_id),
            declined: false,
            is_task: true,
            completed: t.completed,
            warning: false,
        };
        let key = (1, t.due_date.and_time(NaiveTime::MIN).and_utc(), row.title.clone());
        days.entry(t.due_date).or_default().push((key, row));
    }

    days.into_iter()
        .map(|(date, mut rows)| {
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            DaySection { date, header: header(date, input.today), rows: rows.into_iter().map(|(_, r)| r).collect() }
        })
        .collect()
}

/// Local dates an occurrence touches. An end at exactly local midnight does
/// not count as touching that day.
fn days_covered(start: &When, end: &When, tz: Tz) -> Vec<NaiveDate> {
    let (first, last) = match (start, end) {
        (When::Date(s), When::Date(e)) => (*s, if e > s { *e - TimeDelta::days(1) } else { *s }),
        _ => {
            let s = display_instant(start, tz).with_timezone(&tz);
            let e = display_instant(end, tz).with_timezone(&tz);
            let mut last = e.date_naive();
            if e > s && e.time() == NaiveTime::MIN {
                last -= TimeDelta::days(1);
            }
            (s.date_naive(), last.max(s.date_naive()))
        }
    };
    first.iter_days().take_while(|d| *d <= last).collect()
}

fn label_for(o: &Occurrence, date: NaiveDate, first: NaiveDate, last: NaiveDate, tz: Tz, use_24h: bool) -> (u8, String) {
    if o.start.is_all_day() {
        return (0, "All day".into());
    }
    let start = display_instant(&o.start, tz).with_timezone(&tz);
    let end = display_instant(&o.end, tz).with_timezone(&tz);
    let time = |t: DateTime<Tz>| if use_24h { t.format("%H:%M").to_string() } else { t.format("%-I:%M %p").to_string() };
    match (date == first, date == last) {
        (true, true) => (2, format!("{}–{}", time(start), time(end))),
        (true, false) => (2, format!("From {}", time(start))),
        (false, true) => (2, format!("Until {}", time(end))),
        (false, false) => (0, "All day".into()),
    }
}

fn header(date: NaiveDate, today: NaiveDate) -> String {
    let mut text = date.format("%a %-d %b").to_string();
    if date.year() != today.year() {
        text.push_str(&format!(" {}", date.year()));
    }
    if date == today {
        format!("Today · {text}")
    } else if date == today + TimeDelta::days(1) {
        format!("Tomorrow · {text}")
    } else {
        text
    }
}

pub fn parse_hex(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten()
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p takt --lib agenda`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/takt Cargo.lock
git commit -m "feat(takt): build agenda day sections from store rows"
```

---

### Task 16: Event detail builder

**Files:**
- Modify: `crates/takt/src/lib.rs`
- Create: `crates/takt/src/detail.rs`

**Interfaces:**
- Produces (`takt::detail`): `EventDetail { title, when, calendar, account, location, organizer, attendees: Vec<(String, String)>, conference: Vec<String>, reminders: Vec<String>, recurrence: Vec<String>, description: Option<String>, raw_json: String }`; `event_detail(raw: &serde_json::Value, when: String, calendar: String, account: String) -> EventDetail`; `describe_recurrence_line(&str) -> String`; `strip_html(&str) -> String`.

- [ ] **Step 1: Write the failing tests**

Create `crates/takt/src/detail.rs` with the tests:

```rust
#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn describes_common_rules() {
        assert_eq!(describe_recurrence_line("RRULE:FREQ=DAILY"), "Daily");
        assert_eq!(describe_recurrence_line("RRULE:FREQ=WEEKLY;BYDAY=MO,WE"), "Weekly on Mon, Wed");
        assert_eq!(describe_recurrence_line("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=FR"), "Every 2 weeks on Fri");
        assert_eq!(describe_recurrence_line("RRULE:FREQ=MONTHLY;BYDAY=3TU"), "Monthly on the third Tuesday");
        assert_eq!(describe_recurrence_line("RRULE:FREQ=MONTHLY;BYDAY=-1FR"), "Monthly on the last Friday");
        assert_eq!(describe_recurrence_line("RRULE:FREQ=MONTHLY;BYMONTHDAY=31"), "Monthly on day 31");
        assert_eq!(describe_recurrence_line("RRULE:FREQ=YEARLY;COUNT=5"), "Annually, 5 times");
        assert_eq!(describe_recurrence_line("RRULE:FREQ=WEEKLY;UNTIL=20261231T235959Z"), "Weekly, until 31 Dec 2026");
        assert_eq!(describe_recurrence_line("EXDATE;TZID=Europe/London:20261012T090000,20261019T090000"), "Except 2 dates");
        assert_eq!(describe_recurrence_line("RRULE:FREQ=MINUTELY"), "RRULE:FREQ=MINUTELY");
    }

    #[test]
    fn strips_html_descriptions() {
        assert_eq!(strip_html("Agenda:<br>1. Plans &amp; goals<br/><b>Bring</b> laptop"), "Agenda:\n1. Plans & goals\nBring laptop");
    }

    #[test]
    fn builds_detail_from_raw_json() {
        let raw = json!({
            "id": "e1", "summary": "Planning", "location": "Room 4", "description": "Bring <i>notes</i>",
            "organizer": {"email": "boss@example.com", "displayName": "Boss"},
            "attendees": [
                {"email": "a@example.com", "responseStatus": "accepted"},
                {"email": "me@example.com", "self": true, "responseStatus": "tentative"}
            ],
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "conferenceData": {"entryPoints": [{"entryPointType": "video", "uri": "https://meet.google.com/abc-defg-hij"}, {"entryPointType": "phone", "uri": "tel:+1-555-0100"}]},
            "reminders": {"useDefault": false, "overrides": [{"method": "popup", "minutes": 10}, {"method": "email", "minutes": 1440}]},
            "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=TU"],
            "x-future": true
        });
        let d = event_detail(&raw, "Tue 20 Oct, 09:00–10:00".into(), "Work".into(), "me@example.com".into());
        assert_eq!(d.title, "Planning");
        assert_eq!(d.organizer.as_deref(), Some("Boss"));
        assert_eq!(d.attendees, [("a@example.com".to_owned(), "Going".to_owned()), ("me@example.com".to_owned(), "Maybe".to_owned())]);
        assert_eq!(d.conference, ["https://meet.google.com/abc-defg-hij", "tel:+1-555-0100"]);
        assert_eq!(d.reminders, ["Notification 10 minutes before", "Email 1 day before"]);
        assert_eq!(d.recurrence, ["Weekly on Tue"]);
        assert_eq!(d.description.as_deref(), Some("Bring notes"));
        assert!(d.raw_json.contains("x-future"));
    }

    #[test]
    fn default_reminders_and_missing_fields() {
        let d = event_detail(&json!({"id": "e", "reminders": {"useDefault": true}}), String::new(), String::new(), String::new());
        assert_eq!(d.title, "(No title)");
        assert_eq!(d.reminders, ["Calendar's default reminders"]);
        assert!(d.attendees.is_empty() && d.conference.is_empty() && d.description.is_none());
    }
}
```

Add `pub mod detail;` to `crates/takt/src/lib.rs`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt --lib detail`
Expected: compile errors.

- [ ] **Step 3: Implement the builder above the test module**

```rust
//! The read-only detail panel's content, built from an event's raw JSON.

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct EventDetail {
    pub title: String,
    pub when: String,
    pub calendar: String,
    pub account: String,
    pub location: Option<String>,
    pub organizer: Option<String>,
    /// (who, response) pairs.
    pub attendees: Vec<(String, String)>,
    pub conference: Vec<String>,
    pub reminders: Vec<String>,
    pub recurrence: Vec<String>,
    pub description: Option<String>,
    pub raw_json: String,
}

pub fn event_detail(raw: &Value, when: String, calendar: String, account: String) -> EventDetail {
    let text = |v: &Value| v.as_str().map(str::to_owned).filter(|s| !s.trim().is_empty());
    let person = |p: &Value| text(&p["displayName"]).or_else(|| text(&p["email"]));
    let attendees = raw["attendees"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| Some((person(a)?, response_label(a["responseStatus"].as_str().unwrap_or("needsAction")).to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let mut conference: Vec<String> = raw["conferenceData"]["entryPoints"]
        .as_array()
        .map(|points| points.iter().filter_map(|p| text(&p["uri"])).collect())
        .unwrap_or_default();
    if let Some(link) = text(&raw["hangoutLink"]) {
        if !conference.contains(&link) {
            conference.insert(0, link);
        }
    }
    EventDetail {
        title: text(&raw["summary"]).unwrap_or_else(|| "(No title)".into()),
        when,
        calendar,
        account,
        location: text(&raw["location"]),
        organizer: person(&raw["organizer"]),
        attendees,
        conference,
        reminders: reminders(&raw["reminders"]),
        recurrence: raw["recurrence"]
            .as_array()
            .map(|lines| lines.iter().filter_map(Value::as_str).map(describe_recurrence_line).collect())
            .unwrap_or_default(),
        description: text(&raw["description"]).map(|d| strip_html(&d)).filter(|d| !d.is_empty()),
        raw_json: serde_json::to_string_pretty(raw).unwrap_or_default(),
    }
}

fn response_label(status: &str) -> &'static str {
    match status {
        "accepted" => "Going",
        "declined" => "Declined",
        "tentative" => "Maybe",
        _ => "Awaiting reply",
    }
}

fn reminders(value: &Value) -> Vec<String> {
    if value["useDefault"].as_bool() == Some(true) {
        return vec!["Calendar's default reminders".into()];
    }
    value["overrides"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|r| {
                    let method = if r["method"] == "email" { "Email" } else { "Notification" };
                    Some(format!("{method} {} before", duration(r["minutes"].as_u64()?)))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn duration(minutes: u64) -> String {
    let plural = |n: u64, unit: &str| if n == 1 { format!("1 {unit}") } else { format!("{n} {unit}s") };
    if minutes > 0 && minutes % 10_080 == 0 {
        plural(minutes / 10_080, "week")
    } else if minutes > 0 && minutes % 1_440 == 0 {
        plural(minutes / 1_440, "day")
    } else if minutes > 0 && minutes % 60 == 0 {
        plural(minutes / 60, "hour")
    } else {
        plural(minutes, "minute")
    }
}

/// Plain-language reading of one recurrence line. Lines it cannot read are returned as they are.
pub fn describe_recurrence_line(line: &str) -> String {
    let Some((head, value)) = line.split_once(':') else { return line.to_owned() };
    let name = head.split(';').next().unwrap_or_default().to_ascii_uppercase();
    match name.as_str() {
        "EXDATE" => {
            let n = value.split(',').count();
            if n == 1 { "Except 1 date".into() } else { format!("Except {n} dates") }
        }
        "RDATE" => {
            let n = value.split(',').count();
            if n == 1 { "Also 1 extra date".into() } else { format!("Also {n} extra dates") }
        }
        "RRULE" => describe_rule(value).unwrap_or_else(|| line.to_owned()),
        _ => line.to_owned(),
    }
}

fn describe_rule(rule: &str) -> Option<String> {
    let parts: std::collections::HashMap<String, String> = rule
        .split(';')
        .filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_ascii_uppercase(), v.to_owned())))
        .collect();
    let interval: u32 = parts.get("INTERVAL").and_then(|i| i.parse().ok()).unwrap_or(1);
    let (single, unit) = match parts.get("FREQ")?.as_str() {
        "DAILY" => ("Daily", "days"),
        "WEEKLY" => ("Weekly", "weeks"),
        "MONTHLY" => ("Monthly", "months"),
        "YEARLY" => ("Annually", "years"),
        _ => return None,
    };
    let mut text = if interval > 1 { format!("Every {interval} {unit}") } else { single.to_owned() };
    if let Some(days) = parts.get("BYDAY") {
        let names: Vec<String> = days.split(',').filter_map(describe_byday).collect();
        if let Some(first) = names.first().filter(|_| names.len() == 1 && first_is_ordinal(days)) {
            text.push_str(&format!(" on the {first}"));
        } else {
            text.push_str(&format!(" on {}", names.join(", ")));
        }
    } else if let Some(day) = parts.get("BYMONTHDAY") {
        text.push_str(&format!(" on day {day}"));
    }
    if let Some(count) = parts.get("COUNT") {
        text.push_str(&format!(", {count} times"));
    }
    if let Some(until) = parts.get("UNTIL") {
        let date = NaiveDateTime::parse_from_str(until.trim_end_matches('Z'), "%Y%m%dT%H%M%S")
            .map(|t| t.date())
            .or_else(|_| NaiveDate::parse_from_str(until, "%Y%m%d"))
            .ok()?;
        text.push_str(&format!(", until {}", date.format("%-d %b %Y")));
    }
    Some(text)
}

fn first_is_ordinal(days: &str) -> bool {
    days.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+')
}

fn describe_byday(token: &str) -> Option<String> {
    let token = token.trim();
    let split = token.len().checked_sub(2)?;
    let (ordinal, day) = token.split_at(split);
    let (short, long) = match day {
        "MO" => ("Mon", "Monday"),
        "TU" => ("Tue", "Tuesday"),
        "WE" => ("Wed", "Wednesday"),
        "TH" => ("Thu", "Thursday"),
        "FR" => ("Fri", "Friday"),
        "SA" => ("Sat", "Saturday"),
        "SU" => ("Sun", "Sunday"),
        _ => return None,
    };
    if ordinal.is_empty() {
        return Some(short.to_owned());
    }
    let which = match ordinal.trim_start_matches('+') {
        "1" => "first",
        "2" => "second",
        "3" => "third",
        "4" => "fourth",
        "5" => "fifth",
        "-1" => "last",
        "-2" => "second to last",
        _ => return Some(format!("{ordinal}{short}")),
    };
    Some(format!("{which} {long}"))
}

/// Google stores descriptions as light HTML. Keep line breaks, drop tags,
/// decode the common entities.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = rest[open + 1..open + close].trim().to_ascii_lowercase();
        if tag.starts_with("br") || tag == "/p" || tag == "/div" || tag == "/li" {
            out.push('\n');
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p takt --lib detail`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/takt
git commit -m "feat(takt): build read-only event details from raw JSON"
```

---

### Task 17: Config and logging

**Files:**
- Modify: `crates/takt/src/lib.rs`
- Create: `crates/takt/src/config.rs`, `crates/takt/src/logging.rs`

**Interfaces:**
- Produces (`takt::config`): `Paths { config_file, db_file, log_dir: PathBuf }` with `Paths::discover() -> Option<Paths>`; `enum ConfigState { Missing, Invalid(String), Ready(OAuthClient) }`; `load(&Path) -> ConfigState`; `const EXAMPLE: &str`.
- Produces (`takt::logging`): `init(&Path) -> Option<tracing_appender::non_blocking::WorkerGuard>`; level from `TAKT_LOG`, default `info`.

- [ ] **Step 1: Write the failing tests**

Create `crates/takt/src/config.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn missing_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("config.toml")), ConfigState::Missing);
    }

    #[test]
    fn valid_file_is_ready() {
        let (_dir, path) = write("[google]\nclient_id = \" id.apps.googleusercontent.com \"\nclient_secret = \"s3cret\"\n");
        assert_eq!(
            load(&path),
            ConfigState::Ready(OAuthClient { client_id: "id.apps.googleusercontent.com".into(), client_secret: "s3cret".into() })
        );
    }

    #[test]
    fn empty_values_are_invalid() {
        let (_dir, path) = write("[google]\nclient_id = \"\"\nclient_secret = \"x\"\n");
        assert!(matches!(load(&path), ConfigState::Invalid(_)));
    }

    #[test]
    fn malformed_toml_is_invalid() {
        let (_dir, path) = write("[google\nclient_id = ");
        assert!(matches!(load(&path), ConfigState::Invalid(_)));
    }

    #[test]
    fn example_parses_as_ready() {
        let (_dir, path) = write(EXAMPLE);
        assert!(matches!(load(&path), ConfigState::Ready(_)));
    }
}
```

Add `pub mod config;` and `pub mod logging;` to `crates/takt/src/lib.rs`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p takt --lib config`
Expected: compile errors.

- [ ] **Step 3: Implement `config.rs` (above the tests) and `logging.rs`**

```rust
//! Where takt keeps its files, and the OAuth client it reads from config.toml.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use takt_google::OAuthClient;

pub const EXAMPLE: &str = "[google]\nclient_id = \"<your id>.apps.googleusercontent.com\"\nclient_secret = \"<your secret>\"\n";

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub db_file: PathBuf,
    pub log_dir: PathBuf,
}

impl Paths {
    /// `%APPDATA%\takt\config\config.toml` and `%LOCALAPPDATA%\takt\data\...` on Windows.
    pub fn discover() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("", "", "takt")?;
        Some(Self {
            config_file: dirs.config_dir().join("config.toml"),
            db_file: dirs.data_local_dir().join("takt.sqlite3"),
            log_dir: dirs.data_local_dir().join("logs"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigState {
    Missing,
    Invalid(String),
    Ready(OAuthClient),
}

#[derive(Deserialize)]
struct File {
    google: GoogleSection,
}

#[derive(Deserialize)]
struct GoogleSection {
    client_id: String,
    client_secret: String,
}

pub fn load(path: &Path) -> ConfigState {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ConfigState::Missing,
        Err(e) => return ConfigState::Invalid(e.to_string()),
    };
    match toml::from_str::<File>(&text) {
        Ok(file) => {
            let (id, secret) = (file.google.client_id.trim(), file.google.client_secret.trim());
            if id.is_empty() || secret.is_empty() {
                ConfigState::Invalid("client_id and client_secret must both be set".into())
            } else {
                ConfigState::Ready(OAuthClient { client_id: id.to_owned(), client_secret: secret.to_owned() })
            }
        }
        Err(e) => ConfigState::Invalid(e.to_string()),
    }
}
```

`crates/takt/src/logging.rs`:

```rust
//! Logs go to a daily file. Never log tokens; log event text only at `trace`.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Keep the returned guard alive for the life of the app, or buffered lines are lost.
pub fn init(log_dir: &Path) -> Option<WorkerGuard> {
    std::fs::create_dir_all(log_dir).ok()?;
    let (writer, guard) = tracing_appender::non_blocking(tracing_appender::rolling::daily(log_dir, "takt.log"));
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_env("TAKT_LOG").unwrap_or_else(|_| EnvFilter::new("info")))
        .try_init()
        .ok()?;
    Some(guard)
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p takt --lib config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/takt
git commit -m "feat(takt): load OAuth client config and log to a daily file"
```

---

### Task 18: Calendar model, agenda window and bootstrap

GPUI names in this task and the next were read from gpui-kit 0.7.3 (`gpui-component-0.7.3/src/list/{delegate,list}.rs`, `button/button.rs`, `title_bar.rs`, `window_ext.rs`, `gpui-base-0.7.3/src/actions.rs`) and gpui-pre 0.3.9 (`app/context.rs`). If one does not compile, look it up in those files under `~/.cargo/registry/src/` and keep the structure; do not switch to a different widget.

**Files:**
- Modify: `crates/takt/src/lib.rs`, `crates/takt/src/main.rs`
- Create: `crates/takt/src/model.rs`, `crates/takt/src/ui/mod.rs`, `crates/takt/src/ui/app_view.rs`, `crates/takt/src/ui/first_run.rs`, `crates/takt/src/ui/main_view.rs`, `crates/takt/src/ui/agenda_list.rs`

**Interfaces:**
- Consumes: `build_agenda`, `config::{Paths, ConfigState, load}`, `takt_sync::{start, SyncConfig, Command, SyncEvent, SyncState}`, `Store::open`, `Store::open_read_only`.
- Produces:
  - `takt::model::{Snapshot { accounts, calendars, sections }, load_snapshot(&Path, Tz, today, from, to) -> Result<Snapshot, StoreError>, CalendarModel, ModelEvent::Reloaded, PAGE_DAYS}`; `CalendarModel::new(PathBuf, Tz, NaiveDate, &mut Context<Self>)`, `reload`, `load_later`, `load_earlier`, `jump_to_today`, `handle_sync_event(SyncEvent, cx)`, public fields `from`, `to`, `snapshot`, `statuses: HashMap<String, SyncState>`, `last_error: Option<String>`, `tz`
  - `takt::ui::MainView::new(db: PathBuf, commands: async_channel::Sender<Command>, events: async_channel::Receiver<SyncEvent>, &mut Window, &mut Context<Self>)`, `MainView::model(&self) -> &Entity<CalendarModel>`
  - `takt::ui::AppView::new(Paths, ConfigState, &mut Window, &mut Context<Self>)`
  - `takt::run()`

- [ ] **Step 1: Write the snapshot test**

Create `crates/takt/src/model.rs` with:

```rust
#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Utc};
    use serde_json::json;
    use takt_google::model::{CalendarListEntry, EventFields, Item};
    use takt_store::Store;

    use super::*;

    #[test]
    fn snapshot_reads_what_sync_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("takt.sqlite3");
        let mut store = Store::open(&db).unwrap();
        store.upsert_account("acc", "me@example.com").unwrap();
        let cal: Item<CalendarListEntry> = Item::from_value(json!({"id": "cal", "selected": true})).unwrap();
        store.apply_calendar_list_page("acc", &[cal], None).unwrap();
        let start = Utc::now() + TimeDelta::hours(1);
        let event: Item<EventFields> = Item::from_value(json!({
            "id": "e", "summary": "Soon",
            "start": {"dateTime": start.to_rfc3339()}, "end": {"dateTime": (start + TimeDelta::hours(1)).to_rfc3339()}
        }))
        .unwrap();
        store.apply_events_page("acc", "cal", &[event], None).unwrap();

        let today = Utc::now().date_naive();
        let snap = load_snapshot(&db, chrono_tz::UTC, today, today, today + TimeDelta::days(PAGE_DAYS)).unwrap();
        assert_eq!(snap.accounts.len(), 1);
        assert!(snap.sections.iter().flat_map(|s| &s.rows).any(|r| r.title == "Soon"));
    }
}
```

Add `pub mod model;` and `pub mod ui;` to `crates/takt/src/lib.rs`; create `crates/takt/src/ui/mod.rs` containing:

```rust
mod agenda_list;
mod app_view;
mod first_run;
mod main_view;

pub use app_view::AppView;
pub use main_view::MainView;
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p takt --lib model`
Expected: compile errors.

- [ ] **Step 3: Implement `model.rs` (above the tests)**

```rust
//! The UI's view of the cache. Reads run on the background executor; sync
//! events decide when to read again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, TimeDelta, Utc};
use chrono_tz::Tz;
use gpui_kit::{AppContext as _, Context, EventEmitter, Task};
use takt_store::{AccountRow, CalendarRow, Store, StoreError, local_days};
use takt_sync::{SyncEvent, SyncState};

use crate::agenda::{AgendaInput, DaySection, build_agenda};

pub const PAGE_DAYS: i64 = 30;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub accounts: Vec<AccountRow>,
    pub calendars: Vec<CalendarRow>,
    pub sections: Vec<DaySection>,
}

pub fn load_snapshot(db: &Path, tz: Tz, today: NaiveDate, from: NaiveDate, to: NaiveDate) -> Result<Snapshot, StoreError> {
    let store = Store::open_read_only(db)?;
    let accounts = store.accounts()?;
    let calendars = store.calendars(None)?;
    let mut palettes = HashMap::new();
    for a in &accounts {
        palettes.insert(a.account_id.clone(), store.palette(&a.account_id)?);
    }
    let use_24h = match accounts.first() {
        Some(a) => store.settings(&a.account_id)?.get("format24HourTime").is_none_or(|v| v == "true"),
        None => true,
    };
    let occurrences = store.occurrences(&local_days(from, to, tz), tz)?;
    let tasks = store.tasks_due(from, to)?;
    let sections = build_agenda(&AgendaInput {
        occurrences: &occurrences,
        tasks: &tasks,
        accounts: &accounts,
        calendars: &calendars,
        palettes: &palettes,
        tz,
        today,
        from,
        to,
        use_24h,
    });
    Ok(Snapshot { accounts, calendars, sections })
}

pub enum ModelEvent {
    Reloaded,
}

pub struct CalendarModel {
    db: PathBuf,
    pub tz: Tz,
    today: NaiveDate,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub snapshot: Snapshot,
    pub statuses: HashMap<String, SyncState>,
    pub last_error: Option<String>,
    loading: Option<Task<()>>,
}

impl EventEmitter<ModelEvent> for CalendarModel {}

impl CalendarModel {
    pub fn new(db: PathBuf, tz: Tz, today: NaiveDate, cx: &mut Context<Self>) -> Self {
        let mut model = Self {
            db,
            tz,
            today,
            from: today,
            to: today + TimeDelta::days(PAGE_DAYS),
            snapshot: Snapshot::default(),
            statuses: HashMap::new(),
            last_error: None,
            loading: None,
        };
        model.reload(cx);
        model
    }

    pub fn db(&self) -> &Path {
        &self.db
    }

    /// Re-reads the loaded range. A newer reload replaces (cancels) an older one.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (db, tz, today, from, to) = (self.db.clone(), self.tz, self.today, self.from, self.to);
        let job = cx.background_spawn(async move { load_snapshot(&db, tz, today, from, to) });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |model, cx| {
                model.loading = None;
                match result {
                    Ok(snapshot) => model.snapshot = snapshot,
                    Err(error) => {
                        tracing::error!(%error, "cannot read the cache");
                        model.last_error = Some(format!("Cannot read the local cache: {error}"));
                    }
                }
                cx.emit(ModelEvent::Reloaded);
                cx.notify();
            });
        }));
    }

    pub fn load_later(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_none() {
            self.to += TimeDelta::days(PAGE_DAYS);
            self.reload(cx);
        }
    }

    pub fn load_earlier(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_none() {
            self.from -= TimeDelta::days(PAGE_DAYS);
            self.reload(cx);
        }
    }

    pub fn jump_to_today(&mut self, cx: &mut Context<Self>) {
        self.today = Utc::now().with_timezone(&self.tz).date_naive();
        self.from = self.today;
        self.to = self.today + TimeDelta::days(PAGE_DAYS);
        self.reload(cx);
    }

    pub fn handle_sync_event(&mut self, event: SyncEvent, cx: &mut Context<Self>) {
        match event {
            SyncEvent::Changed { changes, .. } => {
                let range = local_days(self.from, self.to, self.tz);
                if changes.affects(range.start, range.end) {
                    self.reload(cx);
                }
            }
            SyncEvent::Status { account_id, state } => {
                self.statuses.insert(account_id, state);
                cx.notify();
            }
            SyncEvent::AuthNeeded { account_id } => {
                self.statuses.insert(account_id, SyncState::NeedsReauth);
                self.reload(cx);
            }
            SyncEvent::AccountsChanged => self.reload(cx),
            SyncEvent::SignInFailed(message) => {
                self.last_error = Some(message);
                cx.notify();
            }
        }
    }
}
```

- [ ] **Step 4: Implement the agenda list delegate**

`crates/takt/src/ui/agenda_list.rs`:

```rust
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::{ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::agenda::{AgendaRow, DaySection, RowKey};
use crate::model::CalendarModel;

pub struct AgendaDelegate {
    pub model: Entity<CalendarModel>,
    pub sections: Vec<DaySection>,
    pub selected: Option<IndexPath>,
}

impl AgendaDelegate {
    pub fn row(&self, ix: IndexPath) -> Option<&AgendaRow> {
        self.sections.get(ix.section)?.rows.get(ix.row)
    }

    pub fn index_of(&self, key: &RowKey) -> Option<IndexPath> {
        self.sections.iter().enumerate().find_map(|(s, section)| {
            section.rows.iter().position(|r| r.key == *key).map(|r| IndexPath::new(r).section(s))
        })
    }

    pub fn is_first(&self, ix: IndexPath) -> bool {
        ix.section == 0 && ix.row == 0
    }
}

impl ListDelegate for AgendaDelegate {
    type Item = ListItem;

    fn sections_count(&self, _: &App) -> usize {
        self.sections.len().max(1)
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.sections.get(section).map_or(0, |s| s.rows.len())
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        let header = self.sections.get(section)?.header.clone();
        let model = self.model.clone();
        Some(
            h_flex()
                .w_full()
                .px_3()
                .pt_3()
                .pb_1()
                .justify_between()
                .text_sm()
                .font_semibold()
                .text_color(cx.theme().muted_foreground)
                .child(header)
                .when(section == 0, |this| {
                    this.child(Button::new("load-earlier").ghost().xsmall().label("Load earlier").on_click(
                        move |_, _, cx| model.update(cx, |m, cx| m.load_earlier(cx)),
                    ))
                }),
        )
    }

    fn render_item(&mut self, ix: IndexPath, _: &mut Window, cx: &mut Context<ListState<Self>>) -> Option<ListItem> {
        let row = self.row(ix)?.clone();
        Some(
            ListItem::new(("agenda-row", ix.section * 100_000 + ix.row))
                .selected(self.selected == Some(ix))
                .child(render_row(&row, cx)),
        )
    }

    fn set_selected_index(&mut self, ix: Option<IndexPath>, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        self.selected = ix;
        cx.notify();
    }

    fn has_more(&self, _: &App) -> bool {
        true
    }

    fn load_more(&mut self, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        self.model.update(cx, |m, cx| m.load_later(cx));
    }
}

/// One line per row: gpui-kit's list needs every item the same height.
fn render_row(row: &AgendaRow, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let color = rgb(row.color);
    let title = match &row.location {
        Some(location) => format!("{}  ·  {location}", row.title),
        None => row.title.clone(),
    };
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .text_sm()
        .child(div().w(px(132.)).flex_none().text_color(theme.muted_foreground).child(row.time_label.clone()))
        .child(if row.is_task {
            div().flex_none().text_color(color).child(if row.completed { "☑" } else { "☐" })
        } else {
            div().flex_none().size(px(8.)).rounded_full().bg(color)
        })
        .child(
            div()
                .flex_1()
                .truncate()
                .when(row.declined || row.completed, |d| d.line_through().text_color(theme.muted_foreground))
                .child(title),
        )
        .when(row.warning, |this| {
            this.child(div().flex_none().text_xs().text_color(theme.danger).child("rule not understood"))
        })
        .when_some(row.account_badge.clone(), |this, badge| {
            this.child(div().flex_none().text_xs().text_color(theme.muted_foreground).child(badge))
        })
}
```

- [ ] **Step 5: Implement `MainView` (agenda only for now)**

`crates/takt/src/ui/main_view.rs`:

```rust
use std::path::PathBuf;

use async_channel::{Receiver, Sender};
use chrono::Utc;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::list::{List, ListEvent, ListState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use takt_sync::{Command, SyncEvent};

use crate::model::{CalendarModel, ModelEvent};
use crate::ui::agenda_list::AgendaDelegate;

pub struct MainView {
    model: Entity<CalendarModel>,
    list: Entity<ListState<AgendaDelegate>>,
    commands: Sender<Command>,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl MainView {
    pub fn new(
        db: PathBuf,
        commands: Sender<Command>,
        events: Receiver<SyncEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tz = crate::display_zone();
        let today = Utc::now().with_timezone(&tz).date_naive();
        let model = cx.new(|cx| CalendarModel::new(db, tz, today, cx));
        let delegate = AgendaDelegate { model: model.clone(), sections: Vec::new(), selected: None };
        let list = cx.new(|cx| ListState::new(delegate, window, cx).selectable(true));

        let subscriptions = vec![
            cx.subscribe_in(&model, window, |this, _, _: &ModelEvent, window, cx| this.refresh_list(window, cx)),
            cx.subscribe_in(&list, window, |this, _, event: &ListEvent, window, cx| this.on_list_event(event, window, cx)),
            cx.observe_window_activation(window, |this, window, _| {
                let _ = this.commands.try_send(Command::WindowFocus(window.is_window_active()));
            }),
        ];

        let bridge_model = model.clone();
        let bridge = cx.spawn(async move |_, cx| {
            while let Ok(event) = events.recv().await {
                if bridge_model.update(cx, |m, cx| m.handle_sync_event(event, cx)).is_err() {
                    break;
                }
            }
        });

        list.update(cx, |list, cx| list.focus(window, cx));
        Self { model, list, commands, _subscriptions: subscriptions, _tasks: vec![bridge] }
    }

    pub fn model(&self) -> &Entity<CalendarModel> {
        &self.model
    }

    /// Copies new sections into the list and keeps the same row selected.
    fn refresh_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sections = self.model.read(cx).snapshot.sections.clone();
        self.list.update(cx, |list, cx| {
            let selected_key = list.selected_index().and_then(|ix| list.delegate().row(ix)).map(|r| r.key.clone());
            list.delegate_mut().sections = sections;
            let ix = selected_key.and_then(|key| list.delegate().index_of(&key));
            list.set_selected_index(ix, window, cx);
            cx.notify();
        });
    }

    fn on_list_event(&mut self, _event: &ListEvent, _window: &mut Window, _cx: &mut Context<Self>) {}
}

impl Render for MainView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(h_flex().flex_1().min_h_0().child(div().flex_1().h_full().child(List::new(&self.list))))
            .text_color(cx.theme().foreground)
    }
}
```

- [ ] **Step 6: Implement the first-run screen and `AppView`**

`crates/takt/src/ui/first_run.rs`:

```rust
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::config::EXAMPLE;

/// Shown until config.toml holds a usable OAuth client.
#[derive(IntoElement)]
pub struct FirstRun {
    pub path: PathBuf,
    pub problem: Option<String>,
    pub on_retry: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl RenderOnce for FirstRun {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let folder = self.path.parent().map(PathBuf::from).unwrap_or_default();
        let on_retry = self.on_retry.clone();
        v_flex()
            .size_full()
            .p_8()
            .gap_4()
            .max_w(px(720.))
            .child(div().text_xl().font_semibold().child("Connect takt to Google"))
            .child("takt uses your own Google Cloud OAuth client. Once:")
            .child(
                v_flex()
                    .gap_1()
                    .child("1. Create a Google Cloud project and enable the Google Calendar API and Google Tasks API.")
                    .child("2. Create an OAuth client of type Desktop app.")
                    .child("3. On the consent screen, set the publishing status to In production. In Testing, Google expires sign-ins after 7 days.")
                    .child(format!("4. Save the client ID and secret to {}:", self.path.display())),
            )
            .child(div().p_3().rounded_md().bg(cx.theme().muted).font_family("monospace").child(EXAMPLE))
            .when_some(self.problem, |this, problem| {
                this.child(div().text_color(cx.theme().danger).child(format!("config.toml is not usable yet: {problem}")))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(Button::new("open-folder").label("Open config folder").on_click(move |_, _, _| {
                        let _ = std::fs::create_dir_all(&folder);
                        let _ = open::that(&folder);
                    }))
                    .child(Button::new("retry").primary().label("Check again").on_click(move |_, window, cx| on_retry(window, cx))),
            )
    }
}
```

`crates/takt/src/ui/app_view.rs`:

```rust
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::{ActiveTheme as _, TitleBar, v_flex};
use gpui_kit::*;
use takt_google::{Endpoints, KeyringTokenStore, OAuthClient, RetryPolicy};
use takt_sync::{Intervals, SyncConfig, SyncHandle};

use crate::config::{ConfigState, Paths, load};
use crate::ui::first_run::FirstRun;
use crate::ui::main_view::MainView;

enum Screen {
    FirstRun { problem: Option<String> },
    Main(Entity<MainView>),
}

pub struct AppView {
    paths: Paths,
    screen: Screen,
}

impl AppView {
    pub fn new(paths: Paths, config: ConfigState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self { paths, screen: Screen::FirstRun { problem: None } };
        view.apply(config, window, cx);
        view
    }

    fn apply(&mut self, config: ConfigState, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = match config {
            ConfigState::Missing => Screen::FirstRun { problem: None },
            ConfigState::Invalid(problem) => Screen::FirstRun { problem: Some(problem) },
            ConfigState::Ready(oauth) => match start_sync(&self.paths, oauth) {
                Ok(handle) => {
                    let db = self.paths.db_file.clone();
                    // The sync thread lives as long as the process; its handle is not joined on exit.
                    let SyncHandle { commands, events, .. } = handle;
                    Screen::Main(cx.new(|cx| MainView::new(db, commands, events, window, cx)))
                }
                Err(error) => Screen::FirstRun { problem: Some(format!("cannot start sync: {error}")) },
            },
        };
        cx.notify();
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = load(&self.paths.config_file);
        self.apply(config, window, cx);
    }
}

fn start_sync(paths: &Paths, oauth: OAuthClient) -> std::io::Result<SyncHandle> {
    takt_sync::start(SyncConfig {
        db_path: paths.db_file.clone(),
        endpoints: Endpoints::google(),
        oauth,
        tokens: Arc::new(KeyringTokenStore),
        open_browser: Arc::new(|url| {
            if let Err(error) = open::that(url.as_str()) {
                tracing::error!(%error, "cannot open the browser for sign-in");
            }
        }),
        intervals: Intervals::default(),
        retry: RetryPolicy::default(),
    })
}

impl Render for AppView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.screen {
            Screen::Main(view) => view.clone().into_any_element(),
            Screen::FirstRun { problem } => {
                let this = cx.entity().downgrade();
                FirstRun {
                    path: self.paths.config_file.clone(),
                    problem: problem.clone(),
                    on_retry: Rc::new(move |window, cx| {
                        let _ = this.update(cx, |view, cx| view.retry(window, cx));
                    }),
                }
                .into_any_element()
            }
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(TitleBar::new().child("takt"))
            .child(div().flex_1().min_h_0().child(body))
    }
}
```

`SyncHandle`'s `thread` field is private, so the destructuring above needs `..`; it compiles because the pattern ignores private fields only if they are not named. If the compiler rejects it, add `pub fn into_channels(self) -> (Sender<Command>, Receiver<SyncEvent>)` to `SyncHandle` in `crates/takt-sync/src/runtime.rs` (dropping the thread handle detaches the thread) and use that instead.

- [ ] **Step 7: Bootstrap**

Add to `crates/takt/src/lib.rs`:

```rust
pub mod config;
pub mod logging;

use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext as _, WindowOptions};

/// Starts takt: logging, config, the database, then the window.
pub fn run() {
    let paths = config::Paths::discover().expect("cannot find the user's app-data directories");
    let _log_guard = logging::init(&paths.log_dir);
    let config = config::load(&paths.config_file);
    // Create and migrate the database before any read-only connection opens it.
    if let Err(error) = takt_store::Store::open(&paths.db_file) {
        tracing::error!(%error, "cannot open the database");
        eprintln!("takt cannot open its database at {}: {error}", paths.db_file.display());
        std::process::exit(1);
    }
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        keys::bind(cx);
        let options = WindowOptions { titlebar: Some(TitleBar::title_bar_options()), ..Default::default() };
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
```

(`pub mod config;` and `pub mod logging;` were added in Task 17; keep one copy.) Create `crates/takt/src/keys.rs` now with only the bind function, and add `pub mod keys;` to `lib.rs`:

```rust
use gpui_kit::base::actions::SelectDown;
use gpui_kit::{App, KeyBinding};

pub fn bind(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("j", SelectDown, Some("List"))]);
}
```

`crates/takt/src/main.rs`:

```rust
// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    takt::run();
}
```

- [ ] **Step 8: Run tests, then launch**

Run: `cargo test -p takt --lib`
Expected: PASS (`agenda`, `detail`, `config`, `model`).

Run: `cargo run -p takt`
Expected: a window titled takt. With no `config.toml`, the first-run screen shows the setup steps and the config path. With a valid `config.toml` and no accounts, an empty agenda shows. Close the window; the process exits.

- [ ] **Step 9: Commit**

```bash
git add crates/takt crates/takt-sync
git commit -m "feat(takt): show the agenda window backed by the cache and sync thread"
```

---

### Task 19: Sidebar, status bar, detail panel, keys and account actions

**Files:**
- Modify: `crates/takt/src/keys.rs`, `crates/takt/src/ui/main_view.rs`, `crates/takt/src/ui/mod.rs`
- Create: `crates/takt/src/ui/sidebar.rs`, `crates/takt/src/ui/status_bar.rs`, `crates/takt/src/ui/detail_panel.rs`

**Interfaces:**
- Consumes: `CalendarModel` fields, `event_detail`, `Command::{AddAccount, RemoveAccount, SyncNow}`, `Store::event_raw`.
- Produces: actions `takt::keys::{JumpToToday, SyncNow, CloseDetail, PreviousRow}`, context `"Takt"`; `takt::ui::status_bar::status_text(&SyncState, DateTime<Utc>, Tz) -> String`.

- [ ] **Step 1: Write the failing status text test**

`crates/takt/src/ui/status_bar.rs`:

```rust
#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use takt_sync::SyncState;

    use super::*;

    #[test]
    fn status_texts() {
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 14, 0, 30).unwrap();
        let tz = chrono_tz::UTC;
        assert_eq!(status_text(&SyncState::Synced(Utc.with_ymd_and_hms(2026, 10, 17, 14, 0, 18).unwrap()), now, tz), "synced 12s ago");
        assert_eq!(status_text(&SyncState::Synced(Utc.with_ymd_and_hms(2026, 10, 17, 13, 55, 0).unwrap()), now, tz), "synced 5m ago");
        assert_eq!(status_text(&SyncState::Syncing, now, tz), "syncing…");
        assert_eq!(status_text(&SyncState::Offline(Utc.with_ymd_and_hms(2026, 10, 17, 13, 2, 0).unwrap()), now, tz), "offline since 13:02");
        assert_eq!(status_text(&SyncState::NeedsReauth, now, tz), "sign-in needed");
        assert_eq!(status_text(&SyncState::Error("HTTP 500".into()), now, tz), "error: HTTP 500");
    }
}
```

Add `pub mod status_bar;` (public so the test path is stable) plus `mod sidebar;` and `mod detail_panel;` to `crates/takt/src/ui/mod.rs`.

Run: `cargo test -p takt --lib status_bar` — expect a compile error.

- [ ] **Step 2: Implement the status bar (above the test)**

```rust
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
        SyncState::Offline(since) => format!("offline since {}", since.with_timezone(&tz).format("%H:%M")),
        SyncState::Error(message) => format!("error: {message}"),
        SyncState::NeedsReauth => "sign-in needed".into(),
    }
}

pub fn render(model: &CalendarModel, cx: &App) -> AnyElement {
    let now = Utc::now();
    h_flex()
        .w_full()
        .px_3()
        .py_1()
        .gap_4()
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .children(model.snapshot.accounts.iter().map(|a| {
            let state = model.statuses.get(&a.account_id).map(|s| status_text(s, now, model.tz)).unwrap_or_else(|| "waiting".into());
            div().child(format!("{} · {state}", a.email))
        }))
        .into_any_element()
}
```

Run: `cargo test -p takt --lib status_bar` — expect PASS.

- [ ] **Step 3: Implement the sidebar**

`crates/takt/src/ui/sidebar.rs`:

```rust
use async_channel::Sender;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use takt_store::AccountStatus;
use takt_sync::{Command, SyncState};

use crate::agenda::{DEFAULT_COLOR, parse_hex};
use crate::model::CalendarModel;

pub fn render(model: &CalendarModel, commands: &Sender<Command>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let add = commands.clone();
    v_flex()
        .w(px(240.))
        .h_full()
        .flex_none()
        .p_3()
        .gap_3()
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
        .when_some(model.last_error.clone(), |this, message| {
            this.child(div().text_xs().text_color(theme.danger).child(message))
        })
        .children(model.snapshot.accounts.iter().map(|account| {
            let state = model.statuses.get(&account.account_id);
            let dot = match state {
                Some(SyncState::Synced(_)) => theme.success,
                Some(SyncState::Syncing) => theme.accent_foreground,
                Some(SyncState::Error(_) | SyncState::NeedsReauth) => theme.danger,
                _ => theme.muted_foreground,
            };
            let needs_sign_in = account.status == AccountStatus::NeedsReauth || matches!(state, Some(SyncState::NeedsReauth));
            let (reauth, remove) = (commands.clone(), commands.clone());
            let (account_id, email) = (account.account_id.clone(), account.email.clone());
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().size(px(8.)).rounded_full().bg(dot))
                        .child(div().flex_1().truncate().text_sm().font_semibold().child(account.email.clone()))
                        .child(
                            Button::new(SharedString::from(format!("remove-{account_id}")))
                                .ghost()
                                .xsmall()
                                .label("Remove")
                                .on_click(move |_, window, cx| {
                                    let (remove, account_id, email) = (remove.clone(), account_id.clone(), email.clone());
                                    window.open_alert_dialog(cx, move |dialog, _, _| {
                                        let (remove, account_id) = (remove.clone(), account_id.clone());
                                        dialog
                                            .title(format!("Remove {email}?"))
                                            .description("takt deletes this account's cached data and its saved sign-in. Nothing changes in Google.")
                                            .confirm()
                                            .on_ok(move |_, _, _| {
                                                let _ = remove.try_send(Command::RemoveAccount { account_id: account_id.clone() });
                                                true
                                            })
                                    });
                                }),
                        ),
                )
                .when(needs_sign_in, |this| {
                    this.child(Button::new(SharedString::from(format!("reauth-{}", account.account_id))).small().primary().label("Sign in again").on_click(
                        move |_, _, _| {
                            let _ = reauth.try_send(Command::AddAccount);
                        },
                    ))
                })
                .children(model.snapshot.calendars.iter().filter(|c| c.account_id == account.account_id).map(|c| {
                    let color = c.background_color.as_deref().and_then(parse_hex).unwrap_or(DEFAULT_COLOR);
                    h_flex()
                        .gap_2()
                        .pl_4()
                        .items_center()
                        .text_sm()
                        .when(!c.selected || c.hidden, |row| row.text_color(theme.muted_foreground))
                        .child(div().size(px(8.)).rounded_sm().bg(rgb(color)))
                        .child(div().truncate().child(c.summary.clone()))
                }))
        }))
        .child(Button::new("add-account").ghost().label("+ Add account").on_click(move |_, _, _| {
            let _ = add.try_send(Command::AddAccount);
        }))
        .into_any_element()
}
```

If `theme.success` or `theme.sidebar` is not a field of `ThemeColor` in `gpui-component-0.7.3/src/theme/theme_color.rs`, use `theme.accent` and `theme.background`.

- [ ] **Step 4: Implement the detail panel**

`crates/takt/src/ui/detail_panel.rs`:

```rust
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::detail::EventDetail;

pub enum DetailContent {
    Event(EventDetail),
    Task { title: String, when: String, list: String, completed: bool },
}

pub fn render(content: &DetailContent, show_raw: bool, on_toggle_raw: impl Fn(&mut Window, &mut App) + 'static, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let section = |label: &'static str, lines: Vec<String>| {
        v_flex()
            .gap_1()
            .when(!lines.is_empty(), |this| {
                this.child(div().text_xs().font_semibold().text_color(theme.muted_foreground).child(label))
                    .children(lines.into_iter().map(|l| div().text_sm().child(l)))
            })
    };
    let body = match content {
        DetailContent::Task { title, when, list, completed } => v_flex()
            .gap_3()
            .child(div().text_lg().font_semibold().child(title.clone()))
            .child(div().text_sm().child(when.clone()))
            .child(section("List", vec![list.clone()]))
            .child(section("Status", vec![if *completed { "Completed".into() } else { "Not completed".into() }])),
        DetailContent::Event(d) => v_flex()
            .gap_3()
            .child(div().text_lg().font_semibold().child(d.title.clone()))
            .child(div().text_sm().child(d.when.clone()))
            .child(section("Calendar", vec![format!("{} · {}", d.calendar, d.account)]))
            .child(section("Location", d.location.clone().into_iter().collect()))
            .child(section("Repeats", d.recurrence.clone()))
            .child(section("Organizer", d.organizer.clone().into_iter().collect()))
            .child(section("Guests", d.attendees.iter().map(|(who, reply)| format!("{who} · {reply}")).collect()))
            .child(section("Join", d.conference.clone()))
            .child(section("Reminders", d.reminders.clone()))
            .child(section("Description", d.description.clone().into_iter().collect()))
            .child(
                Button::new("toggle-raw")
                    .ghost()
                    .xsmall()
                    .label(if show_raw { "Hide raw JSON" } else { "Show raw JSON" })
                    .on_click(move |_, window, cx| on_toggle_raw(window, cx)),
            )
            .when(show_raw, |this| {
                this.child(div().p_2().rounded_md().bg(theme.muted).text_xs().font_family("monospace").child(d.raw_json.clone()))
            }),
    };
    v_flex()
        .id("detail-panel")
        .w(px(380.))
        .h_full()
        .flex_none()
        .p_4()
        .border_l_1()
        .border_color(theme.border)
        .overflow_y_scroll()
        .child(h_flex().child(body))
        .into_any_element()
}
```

- [ ] **Step 5: Add actions and bindings**

Replace `crates/takt/src/keys.rs`:

```rust
use gpui_kit::base::actions::SelectDown;
use gpui_kit::{App, KeyBinding};

gpui_kit::actions!(takt, [JumpToToday, SyncNow, CloseDetail, PreviousRow]);

/// Key context set on the main view.
pub const CONTEXT: &str = "Takt";

pub fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("j", SelectDown, Some("List")),
        // `k` goes through the main view so it can load earlier days at the top.
        KeyBinding::new("k", PreviousRow, Some(CONTEXT)),
        KeyBinding::new("t", JumpToToday, Some(CONTEXT)),
        KeyBinding::new("secondary-r", SyncNow, Some(CONTEXT)),
        KeyBinding::new("escape", CloseDetail, Some(CONTEXT)),
    ]);
}
```

- [ ] **Step 6: Wire everything into `MainView`**

In `crates/takt/src/ui/main_view.rs`:

1. Add fields `detail: Option<DetailContent>`, `show_raw: bool`, and a third task in `_tasks` that re-renders the status bar every 15 seconds:

```rust
let ticker = cx.spawn(async move |this, cx| loop {
    cx.background_executor().timer(std::time::Duration::from_secs(15)).await;
    if this.update(cx, |_, cx| cx.notify()).is_err() {
        break;
    }
});
```

2. Replace `on_list_event` and add the action handlers:

```rust
    fn on_list_event(&mut self, event: &ListEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ListEvent::Confirm(ix) => self.open_detail(*ix, cx),
            ListEvent::Cancel => self.close_detail(&CloseDetail, window, cx),
            ListEvent::Select(_) => {}
        }
    }

    fn open_detail(&mut self, ix: IndexPath, cx: &mut Context<Self>) {
        let Some((row, header)) = self.list.read(cx).delegate().sections.get(ix.section).and_then(|s| {
            s.rows.get(ix.row).map(|r| (r.clone(), s.header.clone()))
        }) else {
            return;
        };
        let when = format!("{header}, {}", row.time_label);
        match row.key.clone() {
            RowKey::Task { .. } => {
                self.detail = Some(DetailContent::Task {
                    title: row.title.clone(),
                    when,
                    list: row.location.clone().unwrap_or_default(),
                    completed: row.completed,
                });
                self.show_raw = false;
                cx.notify();
            }
            RowKey::Event { account_id, calendar_id, event_id, .. } => {
                let model = self.model.read(cx);
                let db = model.db().to_path_buf();
                let calendar = model.snapshot.calendars.iter()
                    .find(|c| c.account_id == account_id && c.calendar_id == calendar_id)
                    .map(|c| c.summary.clone()).unwrap_or_default();
                let account = model.snapshot.accounts.iter()
                    .find(|a| a.account_id == account_id).map(|a| a.email.clone()).unwrap_or_default();
                let read = cx.background_spawn(async move {
                    takt_store::Store::open_read_only(&db)?.event_raw(&account_id, &calendar_id, &event_id)
                });
                cx.spawn(async move |this, cx| {
                    let raw = read.await;
                    let _ = this.update(cx, |view, cx| {
                        match raw {
                            Ok(Some(raw)) => {
                                view.detail = Some(DetailContent::Event(crate::detail::event_detail(&raw, when, calendar, account)));
                                view.show_raw = false;
                            }
                            Ok(None) => view.detail = None,
                            Err(error) => tracing::error!(%error, "cannot read event details"),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    fn close_detail(&mut self, _: &CloseDetail, _: &mut Window, cx: &mut Context<Self>) {
        self.detail = None;
        cx.notify();
    }

    fn previous_row(&mut self, _: &PreviousRow, window: &mut Window, cx: &mut Context<Self>) {
        let at_top = self.list.read(cx).selected_index().is_none_or(|ix| self.list.read(cx).delegate().is_first(ix));
        if at_top {
            self.model.update(cx, |m, cx| m.load_earlier(cx));
        } else {
            window.dispatch_action(Box::new(gpui_kit::base::actions::SelectUp), cx);
        }
    }

    fn jump_to_today(&mut self, _: &JumpToToday, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| m.jump_to_today(cx));
    }

    fn sync_now(&mut self, _: &SyncNow, _: &mut Window, _: &mut Context<Self>) {
        let _ = self.commands.try_send(Command::SyncNow);
    }
```

with these imports added: `use gpui_kit::component::IndexPath; use crate::agenda::RowKey; use crate::keys::{CONTEXT, CloseDetail, JumpToToday, PreviousRow, SyncNow}; use crate::ui::detail_panel::{self, DetailContent}; use crate::ui::{sidebar, status_bar};`. The `ListEvent` variants are `Select(IndexPath)`, `Confirm(IndexPath)` and `Cancel` in gpui-kit 0.7.3.

3. Replace `render`:

```rust
impl Render for MainView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let this = cx.entity().downgrade();
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::close_detail))
            .on_action(cx.listener(Self::previous_row))
            .on_action(cx.listener(Self::jump_to_today))
            .on_action(cx.listener(Self::sync_now))
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar::render(model, &self.commands, cx))
                    .child(div().flex_1().h_full().child(List::new(&self.list)))
                    .when_some(self.detail.as_ref(), |row, detail| {
                        row.child(detail_panel::render(
                            detail,
                            self.show_raw,
                            move |_, cx| {
                                let _ = this.update(cx, |view, cx| {
                                    view.show_raw = !view.show_raw;
                                    cx.notify();
                                });
                            },
                            cx,
                        ))
                    }),
            )
            .child(status_bar::render(model, cx))
    }
}
```

Add `use gpui_kit::prelude::FluentBuilder as _;`. The sidebar, status bar and detail panel helpers return `AnyElement`, so the elements they build do not keep `model` or `cx` borrowed; Rust 2024's `impl Trait` capture rules would otherwise tie them to those borrows.

- [ ] **Step 7: Check by hand**

Run: `cargo test -p takt --lib`, expect PASS. Then `cargo run -p takt` with a valid `config.toml` and walk through:
- **+ Add account** opens the browser; after consent the account appears in the sidebar with its calendars, and the agenda fills.
- `j` moves down, `k` moves up and at the first row loads earlier days, `Enter` opens details, **Show raw JSON** toggles the JSON, `Esc` closes details, `t` returns to today, `Ctrl+R` shows "syncing…" in the status bar.
- **Remove** asks for confirmation and then removes the account and its rows.

- [ ] **Step 8: Commit**

```bash
git add crates/takt
git commit -m "feat(takt): add sidebar, status bar, detail panel, keys and account actions"
```

---

### Task 20: Smoke test, README and final verification

**Files:**
- Create: `crates/takt/tests/smoke.rs`, `README.md`

- [ ] **Step 1: Write the smoke test**

`crates/takt/tests/smoke.rs`:

```rust
use chrono::{TimeDelta, Utc};
use serde_json::json;
use takt_google::model::{CalendarListEntry, EventFields, Item};
use takt_store::Store;

fn seed(db: &std::path::Path) {
    let mut store = Store::open(db).unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    let cal: Item<CalendarListEntry> = Item::from_value(json!({"id": "cal", "summary": "Me", "selected": true})).unwrap();
    store.apply_calendar_list_page("acc", &[cal], None).unwrap();
    let start = Utc::now() + TimeDelta::hours(1);
    let event: Item<EventFields> = Item::from_value(json!({
        "id": "e", "summary": "Smoke test",
        "start": {"dateTime": start.to_rfc3339()}, "end": {"dateTime": (start + TimeDelta::hours(1)).to_rfc3339()}
    }))
    .unwrap();
    store.apply_events_page("acc", "cal", &[event], None).unwrap();
}

#[gpui_kit::test]
fn main_view_renders_the_seeded_agenda(cx: &mut gpui_kit::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("takt.sqlite3");
    seed(&db);
    cx.update(|cx| {
        gpui_kit::init(cx);
        takt::keys::bind(cx);
    });
    let (commands, _command_rx) = async_channel::unbounded();
    let (_event_tx, events) = async_channel::unbounded();
    let window = cx.add_window(|window, cx| takt::ui::MainView::new(db.clone(), commands, events, window, cx));
    cx.run_until_parked();
    let titles: Vec<String> = window
        .update(cx, |view, _, cx| {
            view.model().read(cx).snapshot.sections.iter().flat_map(|s| s.rows.iter().map(|r| r.title.clone())).collect()
        })
        .unwrap();
    assert!(titles.contains(&"Smoke test".to_owned()), "{titles:?}");
}
```

Add `serde_json.workspace = true` and `async-channel.workspace = true` to `[dev-dependencies]` in `crates/takt/Cargo.toml` if the test cannot see them (they are normal dependencies, which integration tests can already use).

Run: `cargo test -p takt --test smoke`
Expected: PASS. If `cx.add_window` needs the window wrapped in gpui-kit's `Root` for theme access, open it with `gpui_kit::open_window`'s pattern inside `cx.update` instead and keep the assertion.

- [ ] **Step 2: Write the README**

`README.md`:

````markdown
# takt

A desktop client for Google Calendar and Google Tasks, written in Rust with [gpui-kit](https://github.com/longbridge/gpui-kit).

This is sub-project 1 of 6: a read-only agenda that merges several Google accounts and works offline from a local cache. Design: `docs/superpowers/specs/2026-10-17-takt-foundation-design.md`.

## Setting up Google access

takt uses your own OAuth client. Do this once:

1. In the [Google Cloud console](https://console.cloud.google.com/), create a project.
2. Enable the **Google Calendar API** and the **Google Tasks API**.
3. Under **APIs & Services → OAuth consent screen**, choose External and fill in the required fields. Set the publishing status to **In production**. While it is in Testing, Google expires every sign-in after 7 days. An unverified production app shows a "Google hasn't verified this app" warning once per account; choose Advanced → continue.
4. Under **Credentials**, create an **OAuth client ID** of type **Desktop app**.
5. Put its ID and secret in `config.toml`. On Windows the file is `%APPDATA%\takt\config\config.toml`; takt's first-run screen shows the exact path.

```toml
[google]
client_id = "<your id>.apps.googleusercontent.com"
client_secret = "<your secret>"
```

Refresh tokens are stored in the OS keychain (Windows Credential Manager) under the service name `takt`. The cache and logs live under `%LOCALAPPDATA%\takt\data`.

## Running

```bash
cargo run -p takt --release
```

| Key | Action |
|---|---|
| `j` / `k` | Next / previous row (`k` on the first row loads earlier days) |
| `Enter` | Open details |
| `Esc` | Close details |
| `t` | Jump to today |
| `Ctrl+R` | Sync now |

Set `TAKT_LOG=debug` for more detailed logs.

## Tests

```bash
cargo test --workspace
```

The recurrence golden tests compare takt's expansion with Google's. To capture new fixtures, sign in with takt, create series in a calendar named `takt-fixtures`, then run:

```bash
cargo run -p takt-google --example capture -- --config "<path to config.toml>" --email "<your email>" --calendar takt-fixtures --from 2026-01-01 --to 2028-01-01 --out crates/takt-recur/tests/fixtures
```

## Known limits

- The Tasks API stores only the date of a task's due time, and does not expose recurring tasks.
- Sub-project 1 is read-only: nothing is written to Google.
````

- [ ] **Step 3: Run the full verification**

```bash
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Expected: every test passes (the golden test needs Task 14's fixtures), clippy reports nothing, fmt reports nothing. Fix anything that fails before continuing.

- [ ] **Step 4: Acceptance check with your human partner**

Ask your human partner to confirm each done criterion from the spec, and record the outcome in the commit body:

1. Two accounts signed in; the agenda for the next 30 days matches the web UI for both accounts.
2. With the window focused, an event edited on the web appears in takt within about 60 seconds.
3. With the network disconnected, takt restarts and shows everything previously synced.

- [ ] **Step 5: Commit**

```bash
git add README.md crates/takt
git commit -m "docs(readme): add Google setup, running and fixture capture" -m "Adds the gpui smoke test. Acceptance: <results from Step 4>."
```
