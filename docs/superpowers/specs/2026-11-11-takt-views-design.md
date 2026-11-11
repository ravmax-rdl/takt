# takt calendar views: design

Date: 2026-11-11
Status: implemented on `feat/views`
Scope: sub-project 2 of 6. The foundation it builds on: `2026-10-17-takt-foundation-design.md`.

## Purpose

Sub-project 1 shows a merged agenda. This one adds what is needed to stop opening the Google Calendar web UI just to look at the calendar: day, week and month grids, a mini calendar, per-calendar show/hide, overlap layout, a keymap for all of it, and time zone display.

Still read-only against Google. Nothing here calls a write endpoint.

## Decisions

| Decision | Choice |
|---|---|
| Default view | Week. `[view] default` in config.toml changes it. The view is not remembered between runs. |
| Calendar show/hide | A local choice stored in the cache (`calendars.local_visible`). It overrides Google's `selected` flag and is never sent to Google. Writing `selected` back belongs with the other writes in sub-project 3. |
| Calendars hidden in Google's settings (`hidden: true`) | Not synced (unchanged from sub-project 1), listed greyed out, cannot be shown. |
| Week start | The first account's `weekStart` setting; `[view] week_start` overrides it. Monday until the first read. |
| Time zone | The OS zone; `[view] time_zone` overrides it. `[view] secondary_time_zone` adds a second column of hour labels to the day and week views. |
| Events of 24 hours or more | Drawn as bars in the all-day strip, like all-day events. Shorter events that cross midnight are split into one block per day. |
| Dated tasks on the grids | Not yet. The agenda still lists them; the grids get them in sub-project 5 with the rest of Tasks. |
| Custom day ranges (4 days, 2 weeks), year view, search, go-to-date prompt | Out of scope. The mini calendar covers jumping to a date. |

## 1. Store and sync

Migration `002_calendar_visibility.sql` adds `calendars.local_visible INTEGER`, NULL by default.

- A calendar's events are returned by `occurrences` when `COALESCE(local_visible, selected) = 1 AND hidden = 0`. `CalendarRow.visible` carries the same value for the sidebar.
- `Store::set_calendar_visible(account, calendar, Option<bool>)` writes the choice; `None` goes back to following Google. It returns a `ChangeSet` with `everything` set, or an empty one when the calendar does not exist.
- `apply_calendar_list_page` does not list the column in its upsert, so a sync page keeps the choice.
- The sync thread stays the only writer. The UI sends `Command::SetCalendarVisible { account_id, calendar_id, visible }`; the runtime writes it and sends `SyncEvent::Changed`, and the UI re-reads as it does for any other change.

Unselected calendars were already synced in sub-project 1 (only `hidden` ones are skipped), so showing one needs no network round trip.

## 2. Pure modules (`takt`, no GPUI types)

### `view.rs`

`ViewKind` is `Day`, `Week`, `Month` or `Agenda`.

- `visible_days(view, anchor, week_start)` gives the days `from..to` a view shows: one day; the week holding the anchor; or the whole weeks that hold the anchor's month (four to six). For the agenda it is 30 days from the anchor.
- `step(view, anchor, n)` moves by days, weeks or months. A month step keeps the day of the month, or the target month's last day (31 Jan → 28 Feb).
- `title` writes the toolbar text: "Saturday 17 October 2026", "12 – 18 Oct 2026", "28 Dec 2026 – 3 Jan 2027", "October 2026".
- `zone_label(tz, instant)` writes an offset the way Google does: "GMT+5:30", "GMT-3", "GMT".

### `layout.rs`

`pack_columns(spans)` lays out one day's timed events:

1. Sort by start, longer first among equal starts.
2. Events that overlap, directly or through others, form a cluster. Each event takes the leftmost column whose last event has ended.
3. Every event in a cluster is `1 / columns` wide, then widens to the right across columns where nothing overlaps it.

`pack_lanes(bars, columns)` stacks bars that run across day columns: each bar, in input order, takes the topmost lane free on all its days.

`fit_lanes(bars, columns, cap)` decides what a month row with room for `cap` lanes shows. A day that has more bars than lanes gives up its last lane to a "+N more" line, so a bar in that lane is hidden if any of its days overflows, and is counted in every such day's N.

### `grid.rs`

`build_time_grid` (day, week) and `build_month_grid` take the store's occurrences and return plain data:

- `TimedBlock`: a timed event's part on one day, in minutes from local midnight, with its column slot. A block is drawn at least 20 minutes tall and never leaves its day.
- `Bar`: an event across day columns of one row, with its lane and whether it continues before or after the row. In the month, a timed event within one day is flagged `timed` and drawn as a line of text (dot, start time, title) instead of a filled bar.
- `TimeGrid` also carries the 24 hour labels, the zone label, the optional secondary zone's labels, and `order`: every event once, in reading order, for keyboard traversal.

Colours, account badges, titles and time formatting come from `agenda.rs` (`Styling`, `title_of`, `format_time`, `label_for`), so all four views agree.

Times are placed by wall clock. On a 23- or 25-hour day the grid still shows 24 rows.

## 3. Model

`CalendarModel` now holds the view and an **anchor**: the day the view is built around, which is also the selected day in a grid.

- `set_view`, `go_to(date)`, `step(n)` and `jump_to_today` recompute `from..to` with `visible_days`. `go_to` inside the days already shown only moves the selection and reads nothing.
- `load_snapshot` builds only the shown view's data: agenda sections, or a `TimeGrid`, or a `MonthGrid`.
- The snapshot reports the account's week start. When it differs from the one the model used and no override is configured, a week or month is read again on the right one.
- The window draws from `snapshot.view`, not `model.view()`, so switching views keeps the old view on screen until the new read lands instead of flashing an empty grid. If that read fails, the model goes back to the snapshot's view and range.
- `load_earlier`, `load_later` and scroll-triggered loading remain agenda-only.

## 4. UI

```
┌───────────────┬──────────────────────────────────────────────┐
│ November 2026 │ [Today] ‹ ›  9 – 15 Nov 2026   Day Week Month Agenda
│ M T W T F S S ├──────┬──────┬──────┬──────┬──────┬──────┬────┤
│ mini calendar │ GMT  │ MON 9│ TUE10│ WED11│ ...              │
│               │      │ ▬▬ Release freeze ▬▬▬│                │
│ ● you@gmail   ├──────┼──────┼──────┼──────┼──────────────────┤
│   ■ Personal  │ 09:00│ ▇▇▇  │      │      │                  │
│   □ Birthdays │ 10:00│ ▇▇▇▇ │      │ ──── now               │
│ + Add account │ ...  │      │      │      │                  │
├───────────────┴──────┴──────┴──────┴──────┴──────────────────┤
│ you@gmail · synced 12s ago                                   │
└──────────────────────────────────────────────────────────────┘
```

- **Toolbar** (`toolbar.rs`): Today, previous, next, the shown dates, and the view switch.
- **Mini calendar** (`mini_calendar.rs`): always six weeks. Shows today, the selected day and, as a band, the days the main view shows. A click selects that day and keeps the view. Its arrows page the mini month only; it follows the main view whenever the selected day moves.
- **Sidebar**: each calendar is a row with a colour square, filled when shown. A click sends `SetCalendarVisible`.
- **Time grid** (`time_grid.rs`): day headers, the all-day strip (scrolls past five lanes), and a 24-hour scroll area at 48 px per hour. It opens scrolled to an hour before now. Today's column has a line at the current time, redrawn by the existing 15-second tick. In a week, a day header opens that day.
- **Month grid** (`month_grid.rs`): one row per week. The number of lanes per row follows the window height. A click on a cell selects the day; a click on the day number or on "+N more" opens it in the day view.
- **Events**: a click selects the event and opens the detail panel. Declined events are outlined and struck through. A series takt could not expand says so in its title.
- **Detail panel**: gains a "Time zone" line for a timed event whose own zone reads a different clock time than the display zone, e.g. "09:00–10:30 in America/New_York (GMT-4)".

gpui-kit's test helpers find an element only if it is registered with `.test_support()`, so every clickable grid element calls it after its ID is set.

### Keyboard

| Key | Action |
|---|---|
| `d` / `1`, `w` / `2`, `m` / `3`, `a` / `4` | Day, week, month, agenda |
| `j`, `n`, `PageDown` | Next day, week or month (`j` in the agenda: next row) |
| `k`, `p`, `PageUp` | Previous (`k` in the agenda: previous row) |
| `t` | Today |
| `←` / `h`, `→` / `l` | Selected day back or forward |
| `↑`, `↓` | Month: a week back or forward. Day and week: previous or next event on the selected day |
| `Tab`, `Shift+Tab` | Next or previous event in the view, wrapping |
| `Enter` | Open the selected event; with none selected, open the selected day |
| `Esc` | Close the details; again, clear the selection |
| `Ctrl+R` (`Cmd+R`) | Sync now |

Grid keys are bound in the `Takt > Grid` context, so they need the grid to hold focus. Every toolbar, sidebar and mini calendar handler gives focus back to the grid (or the agenda list) after it runs.

## 5. Configuration

```toml
[view]
default = "week"                          # day, week, month or agenda
time_zone = "Europe/Berlin"               # default: the OS zone
secondary_time_zone = "America/New_York"  # second column of hour labels
week_start = "monday"                     # default: the Google account's setting
```

All four are optional. A value takt cannot read is logged and skipped; it never blocks startup.

## 6. Tests

| Where | Tests |
|---|---|
| `takt-store` | Local visibility overrides `selected`, not `hidden`; survives a sync page; `None` restores Google's value |
| `takt-sync` | `SetCalendarVisible` is stored, announced with `Changed`, and kept across the next round |
| `view.rs` | Ranges per view and week start, four- to six-week months, stepping, titles, zone labels |
| `layout.rs` | Column packing (chains, widening, independent clusters), lanes, overflow |
| `grid.rs` | Minutes in the display zone, midnight splits, minimum height, strip classification, clipping, reading order, 12-hour and secondary-zone labels, month rows |
| `model.rs` | A grid reads again only when the anchor leaves it; view switches keep the anchor; week start from the account and from config; a failed read after a switch |
| `main_view.rs` | Headless window: click a block and a bar, every view key, stepping keys, Tab and Enter, calendar toggle, toolbar and mini calendar, week header, "+N more", a hidden calendar's events leaving the grid |

`cargo run -p takt --example demo -- month` opens the main view over made-up events with no account and no sync, for looking at the views by hand.

## Done criteria for sub-project 2

1. `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` pass.
2. For a week and a month with overlapping, multi-day and all-day events, takt's day, week and month views show the same events at the same times as the web UI, in both accounts.
3. Hiding a calendar in the sidebar removes its events from every view and survives a restart and a sync.

Criterion 1 is checked. Criteria 2 and 3 need the owner's accounts and have not been checked against live data; the demo and the headless tests cover them with made-up events.

## Out of scope for sub-project 2

Any write to Google, including `calendarList.patch` for `selected`; creating or dragging events (sub-project 3); tasks on the grids (5); a go-to-date prompt; search; remembering the last view; print.
