# takt foundation: design

Date: 2026-10-17
Status: approved in conversation, pending written-spec review
Scope: sub-project 1 of 6

## Purpose

takt is a personal desktop client for Google Calendar and Google Tasks, written in Rust with [gpui-kit](https://github.com/longbridge/gpui-kit) (GPUI docs: <https://gpui.rs/>). It is meant to replace the Google Calendar web UI as the owner's daily driver. Speed and keyboard control come first; onboarding polish does not matter yet.

"Full compatibility" means anything edited in takt shows up correctly in Google's own apps and the reverse. takt displays every field Google returns, never drops a field it does not understand, and writes only the fields the user changed.

## Decisions

| Decision | Choice |
|---|---|
| Who it is for | The owner, as a daily driver |
| Offline behaviour | Read from a local cache; writes go straight to the API and fail visibly when offline. The store and write path must allow an offline outbox to be added later without rewriting sync. |
| Accounts | Several Google accounts at once, merged into one view |
| Duplicates across accounts | Not merged. An event present in two accounts is two rows, shown twice with account badges and edited separately through its own account. |
| Views (whole project) | Day, week, month, agenda |
| Tasks (whole project) | Dockable side panel, plus dated tasks drawn on the calendar |
| Editable (whole project) | Title, time, description, location, calendar, colour; full recurrence; attendees and RSVP; reminders; Google Meet; Focus time, Out of office, Working location |
| Recurrence storage | Store series and exceptions as Google returns them and expand locally (approach A) |

### Sub-projects

| # | Sub-project | Done when |
|---|---|---|
| 1 | **Foundation** (this spec): workspace, multi-account OAuth, read-only API clients, SQLite store, sync engine, read-only recurrence expansion, agenda view | Two accounts show one merged agenda; web edits appear within about 60 s; takt reopens offline with cached data |
| 2 | Calendar views: day/week/month grids, mini-calendar, calendar visibility and colours, overlap layout, full keymap, time zone display | The web UI is no longer needed for looking at the calendar |
| 3 | Single-event editing: create, edit, delete, drag move/resize, full editor, etag conflict handling | Every editable field round-trips through the web |
| 4 | Recurrence editing: rule editor limited to what the web can express; this / this and following / all | Series edits match what the web produces |
| 5 | Tasks: panel, subtasks, reorder, complete, dated tasks on the grid | Google Tasks is fully usable in takt |
| 6 | Reminders: OS notifications, system tray | Replaces the browser tab's notifications |
| later | Offline outbox | Edits made offline queue and sync |

Order: 1 → 2 → 3 → 4. Sub-project 5 can start after 2; sub-project 6 after 3.

Sub-project 1 is entirely read-only against Google. It writes nothing to the API.

## 1. Crates and threads

```
takt/
  Cargo.toml        workspace
  crates/
    takt-google/    OAuth + REST clients for Calendar v3 and Tasks v1. No storage, no UI.
    takt-recur/     Pure function: (master, exceptions, range) -> occurrences. No I/O.
    takt-store/     SQLite schema, migrations, upserts, range queries. Calls takt-recur.
    takt-sync/      Per-account sync engine. Uses takt-google and takt-store.
    takt/           The GPUI binary. Reads takt-store, sends commands to takt-sync.
```

Dependencies point downward only. Only `takt` depends on gpui-kit, so the other four crates build and test without GPUI.

Threads:

- **UI thread**: GPUI's main thread. No network I/O and no blocking disk I/O.
- **Sync thread**: one dedicated tokio runtime (needed by `reqwest`), running one task per account. That task is the only writer for its account's rows.
- **Reads**: UI queries run on GPUI's background executor, each with its own read-only SQLite connection. The database runs in WAL mode so readers and the writer do not block each other.

Channels between UI and sync (`async-channel`, so either executor can await them):

- UI → sync: `AddAccount`, `RemoveAccount { account }`, `SyncNow`, `WindowFocus(bool)`. Sub-project 3 adds write commands.
- sync → UI: `Changed { account, calendars, range }`, `Status { account, state }`, `AuthNeeded { account }`.

Sync never sends event data to the UI; the UI re-queries the store. The store is the only source of truth for the UI, which is what lets a later outbox write locally first without UI changes.

Main dependencies: `gpui-kit` 0.7, `reqwest` 0.13 (rustls), `oauth2` 5, `keyring` 4, `rusqlite` 0.40 (bundled), `rusqlite_migration`, `rrule` 0.14, `chrono`, `chrono-tz`, `iana-time-zone`, `serde`, `serde_json`, `thiserror`, `tracing`, `directories`, `async-channel`, `tokio`. Test-only: `wiremock`, `proptest`.

## 2. Auth and the Google client (`takt-google`)

### One-time Google Cloud setup (manual, documented in the README)

1. Create a Google Cloud project.
2. Enable the Google Calendar API and the Google Tasks API.
3. Create an OAuth client of type **Desktop app**.
4. Set the consent screen's publishing status to **In production**. In "Testing" status Google expires refresh tokens after 7 days for these scopes. An unverified production app shows a one-time warning per account and has a 100-user cap, which does not matter for personal use.
5. Put the client ID and secret in `config.toml` in the platform config directory from `directories` (under `%APPDATA%` on Windows). The file is never committed.

### Sign-in flow (per account)

1. Bind a loopback listener on `127.0.0.1` at a random port.
2. Open the system browser to Google's consent page with PKCE (S256) and that redirect URI.
3. Receive the redirect, check `state`, exchange the code for tokens.
4. Read `sub` and `email` from the ID token. `sub` is the account's stable ID.

Scopes, all requested on first sign-in so later sub-projects never need a second consent: `https://www.googleapis.com/auth/calendar`, `https://www.googleapis.com/auth/tasks`, `openid`, `email`.

### Tokens

- Refresh token: OS keychain via `keyring`, service `takt`, user `<account-email>`.
- Access token: memory only. Refreshed 60 s before expiry, or once after a 401.
- `invalid_grant` on refresh: the account becomes `NeedsReauth`, `AuthNeeded` is sent, and that account's sync loop stops. Cached data stays visible until the user re-authenticates or removes the account.

### REST client

Hand-written with `reqwest`. Endpoints used in sub-project 1:

| Endpoint | Notes |
|---|---|
| `calendarList.list` | Supports `syncToken` |
| `events.list` | Supports `syncToken`; called with `showDeleted=true`, `maxResults=2500`, `singleEvents` unset |
| `colors.get` | Event and calendar colour palettes |
| `settings.list` | `timezone`, `weekStart`, `format24HourTime` |
| `tasklists.list` | Full list |
| `tasks.list` | `updatedMin`, `showCompleted`, `showDeleted`, `showHidden` |

Each item is returned as a small typed struct with only the fields takt indexes, alongside the complete `serde_json::Value` as received. The generated `google-calendar3` crate is not used: its structs drop unknown fields and it brings its own HTTP and OAuth stack.

Retries inside the client:

- 429, 5xx, or 403 with reason `rateLimitExceeded` or `userRateLimitExceeded`: exponential backoff with jitter, honouring `Retry-After`, at most 5 attempts.
- 401: refresh the access token once and retry.
- Anything else: returned as a typed `GoogleError`.

## 3. Store and recurrence (`takt-store`, `takt-recur`)

### Rule

Every row stores Google's original JSON. Extracted columns exist only for indexing and querying and can be rebuilt from the JSON by a migration.

### Tables

| Table | Primary key | Extracted columns besides `raw_json` |
|---|---|---|
| `accounts` | `account_id` (Google `sub`) | `email`, `status`, `calendar_list_sync_token`, `added_at` |
| `account_settings` | `account_id`, `key` | `value` |
| `colors` | `account_id`, `kind`, `color_id` | `background`, `foreground` |
| `calendars` | `account_id`, `calendar_id` | `summary`, `time_zone`, `background_color`, `foreground_color`, `access_role`, `selected`, `hidden`, `primary`, `events_sync_token` |
| `events` | `account_id`, `calendar_id`, `event_id` | `kind` (`single` / `master` / `exception`), `status`, `ical_uid`, `recurring_event_id`, `original_start`, `start_utc`, `end_utc`, `all_day`, `series_end_utc` (masters; NULL when the series never ends), `etag`, `updated` |
| `task_lists` | `account_id`, `list_id` | `title`, `updated`, `updated_min_checkpoint` |
| `tasks` | `account_id`, `list_id`, `task_id` | `title`, `status`, `due_date`, `parent`, `position`, `completed`, `deleted`, `hidden`, `updated` |

Indexes: `events (account_id, calendar_id, kind, start_utc, end_utc)`; `events (kind, start_utc, series_end_utc)` for masters; `events (account_id, calendar_id, recurring_event_id)` for exceptions; `tasks (due_date)`.

Migrations: `rusqlite_migration`, starting from an empty database. WAL mode is set on open.

### Applying a sync page

One transaction per page:

- Upsert every returned item by primary key.
- Cancelled single event: delete the row.
- Cancelled master: delete the master and all its exceptions.
- Cancelled exception: keep the row; it suppresses that occurrence.
- A calendar's sync token is written only in the transaction of the final page.

An interrupted sync restarts from the previous token and re-applies the same items. Because every write is an upsert, the result is the same.

### All-day events

All-day events use floating dates. `start_utc` and `end_utc` store the date at UTC midnight. Range queries widen the index lookup by one day on each side; Rust then filters exactly in the display time zone.

### Range queries

`occurrences(range, calendars) -> Vec<Occurrence>` returns the union of:

1. Single events overlapping `range`.
2. Masters whose `[start_utc, series_end_utc)` overlaps `range`, expanded by `takt-recur`.
3. Exceptions whose own new times overlap `range`, even when their original occurrence is outside it.

Each `Occurrence` carries `account_id` and `calendar_id`. Rows from different accounts are never merged, even when they share an `iCalUID`.

`tasks_due(range) -> Vec<TaskItem>` returns non-deleted tasks with `due_date` in range.

### `takt-recur`

`expand(master, exceptions, range) -> Vec<Occurrence>`, a pure function:

1. Build `DTSTART;TZID=<master.start.timeZone>:<local start>` (or `DTSTART;VALUE=DATE:<date>` for all-day) and prepend it to the master's `recurrence` lines.
2. Parse with `rrule` and iterate from `range.start - duration` to `range.end`.
3. Assign Google's instance ID to each occurrence: `<masterId>_<YYYYMMDDTHHMMSSZ>` (UTC) for timed events, `<masterId>_<YYYYMMDD>` for all-day.
4. Match exceptions by original start. Cancelled: drop the occurrence. Modified: emit the exception's fields instead.
5. Stop after 10,000 occurrences per series per call.

Cases the rules leave open, such as an exception whose original start the current rule no longer generates, follow Google's observed behaviour as recorded in the golden fixtures (section 6).

If `rrule` cannot parse a master's recurrence, the master is returned as a single occurrence flagged `recurrence_error`, and the UI shows a warning badge.

## 4. Sync engine (`takt-sync`)

One tokio task per account.

### Schedule

Calendar push notifications need a public HTTPS webhook, which a desktop app cannot receive, so takt polls:

- every 60 s while the window has focus
- every 5 min while unfocused or minimized
- immediately on focus gain and on `SyncNow` (Ctrl+R)

### Round

1. Settings and colours: on startup, then hourly.
2. Calendar list: incremental with the account's calendar-list token. Calendars returned with `deleted: true` are removed with their events.
3. Events: for each calendar where `hidden` is false, incremental `events.list` with that calendar's token, following `nextPageToken` until a page returns `nextSyncToken`. At most 4 calendars in parallel per account. A calendar with no token gets a full sync with no `timeMin`.
4. Task lists: full list each round.
5. Tasks: per list, `tasks.list` with `updatedMin` = checkpoint − 60 s and `showCompleted`, `showDeleted`, `showHidden` all true. The checkpoint becomes the largest `updated` value seen. Tasks returned with `deleted: true` are removed.

First sync of a new account runs in this order: calendar list, colours, settings, the primary calendar's events, then the remaining calendars and tasks.

### Change notification

After each committed page, send `Changed { account, calendars, range }`, where `range` covers the times touched by the changed rows. For a master the range runs from its first occurrence to `series_end_utc` (or unbounded).

### Recovery

| Condition | Action |
|---|---|
| `410 Gone` on `events.list` | Delete that calendar's events, clear its token, full re-sync of the calendar |
| `410 Gone` on `calendarList.list` | Clear the account's calendar-list token and all calendar tokens; full re-sync of the account |
| `404` or `403` (not rate limit) on a calendar's events | Remove the calendar and its events |
| Connection failure | Status `Offline(since)`; retry with backoff capped at 5 min; `SyncNow` retries immediately |
| `invalid_grant` | Status `NeedsReauth`; stop the loop |
| Task panic | Restart the task with backoff; status `Error(msg)` |

Status values: `Synced(at)`, `Syncing`, `Offline(since)`, `Error(msg)`, `NeedsReauth`.

### Known Tasks API limits

- `due` keeps only the date. A time of day set in Google's UI cannot be read or written through the API.
- Recurring tasks are not exposed.

takt shows dated tasks as all-day items. Sub-project 5 starts by confirming both limits against captured responses.

## 5. UI (`takt`)

### Window

gpui-kit application with its own title bar, following the OS light or dark theme. Times display in the OS time zone (`iana-time-zone`).

```
┌──────────────┬─────────────────────────────────────────┐
│ Accounts     │ Today · Fri 17 Oct                      │
│ ● you@gmail  │  09:00–09:30  ● Standup        [work]   │
│   ▸ Personal │  12:00–13:00  ● Lunch w/ Sam   [me]     │
│   ▸ Birthdays│  All day      ☐ Pay rent (task)         │
│ ● you@work   │ Sat 18 Oct                              │
│   ▸ Work     │  ...                                    │
│ + Add account│                                         │
├──────────────┴─────────────────────────────────────────┤
│ you@gmail ✓ synced 12s ago · you@work ⟳ syncing        │
└────────────────────────────────────────────────────────┘
```

### Sidebar

- Accounts with a status dot, and under each its calendars with their colours.
- Calendars follow Google's `selected` flag and cannot be toggled in sub-project 1.
- **Add account** starts the sign-in flow.
- **Remove account** deletes the account's rows and its keychain entry, after a confirmation.

### Agenda

- gpui-kit virtual list with day headers, opening on today, loading 30 days at a time when scrolling in either direction.
- Row: colour (event `colorId` from the palette, else the calendar colour), time range or "All day", title, location.
- Account badge on each row when more than one account is signed in. An event present in two accounts appears as two rows.
- Declined events are dimmed and struck through.
- Tasks with a due date appear as read-only task rows on that day.

### Detail panel

Opens on Enter or click. Read-only. Shows attendees with responses, conference links, reminders, the recurrence rule in words, the description, and a toggle to show the raw JSON as received from Google.

### Status bar and first run

- Status bar: each account's sync status.
- First run: when `config.toml` has no OAuth client, show the setup steps from section 2 and the config file path.

### Keyboard

`j` / `k` move the selection, `Enter` opens details, `Esc` closes them, `t` jumps to today, `Ctrl+R` syncs now.

### State flow

A `CalendarModel` entity holds the loaded range and query results. On a `Changed` event whose range overlaps the loaded range, it re-queries on the background executor and calls `cx.notify()`. Grouping, sorting and formatting are plain functions outside GPUI types.

## 6. Errors, logging and testing

### Errors

- One `thiserror` enum per library crate: `GoogleError`, `StoreError`, `SyncError`, `RecurError`.
- The app maps failures to account statuses. Background failures never open a modal.

### Logging

`tracing` to a rolling file in the platform data directory from `directories` (under `%LOCALAPPDATA%` on Windows). Tokens are never logged. Event titles and descriptions are logged only at `trace` level.

### Tests

| Crate | Tests |
|---|---|
| `takt-recur` | Golden tests: local expansion equals Google's `events.instances` (IDs, starts, ends) for every captured series. Unit tests per rule shape. Property tests: output sorted and in range, unique IDs, no panics on random rules. |
| `takt-google` | Captured responses deserialize and the retained JSON equals the input, unknown fields included. `wiremock` tests: pagination, 401 → refresh → retry, 429 with `Retry-After`, 410 surfaced as a typed error. |
| `takt-store` | In-memory SQLite: upserts; cancelled single, master and exception handling; applying the same page twice gives the same state; range queries including all-day events viewed from UTC−10 and UTC+13; migration from empty. |
| `takt-sync` | Scripted scenarios against a `wiremock` fake Google: first sync, incremental, 410 recovery, interruption mid-pages then resume, offline then recovery, `invalid_grant`. |
| `takt` | Unit tests for agenda grouping and formatting. One gpui test-support smoke test rendering the window from a seeded store. |

### Fixture capture

`cargo run -p takt-google --example capture -- --calendar takt-fixtures` reads a dedicated `takt-fixtures` calendar in the owner's account. For each recurring series it saves the master, its exceptions, and Google's `events.instances` output for a fixed range as JSON under `crates/takt-recur/tests/fixtures/`, with attendee emails replaced. Fixtures are committed. Tests never call the live API.

Series to create in that calendar through the web UI:

- weekly series crossing a daylight-saving change
- monthly on the 3rd Tuesday; monthly on the 31st
- ending on a date; ending after N occurrences
- all-day series
- one occurrence moved, one cancelled
- a "this and following" split
- a series whose time zone differs from its calendar's

## Done criteria for sub-project 1

1. `cargo test --workspace`, `cargo clippy --workspace -- -D warnings` and `cargo fmt --check` pass.
2. With two accounts signed in, the agenda for the next 30 days matches the web UI for both accounts.
3. With the window focused, an event edited on the web appears in takt within about 60 s.
4. With the network disconnected, takt restarts and shows all previously synced data.

## Out of scope for sub-project 1

Any write to Google; day, week and month views; calendar visibility toggles; OS notifications; the offline outbox; CI (added once the repo has a remote).
