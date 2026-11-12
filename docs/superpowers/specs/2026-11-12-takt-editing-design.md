# takt single-event editing: design

Date: 2026-11-12
Status: implemented on `feat/editing`
Scope: sub-project 3 of 6. It builds on `2026-10-17-takt-foundation-design.md` and `2026-11-11-takt-views-design.md`.

## Purpose

Sub-projects 1 and 2 only read. This one writes: create, edit and delete events, reply to invitations, and move or resize events by dragging, with every editable field of the foundation spec except the recurrence rule. It is the first code in takt that calls a Google write endpoint.

The rule from the foundation spec holds for every write: takt sends only the fields the user changed and keeps the event's whole JSON, so fields it does not show survive.

## Decisions

| Decision | Choice |
|---|---|
| Where writes run | On the sync thread, as before the only writer of the cache. The UI sends `Command::Write`; sync calls Google, stores Google's answer, announces the change and answers with `SyncEvent::WriteDone`. The UI never changes the cache itself, which is what leaves room for an offline outbox later. |
| Offline | A write without a connection fails with a message and changes nothing. The editor stays open with what was typed. |
| Which endpoint | `events.patch` with the changed top-level fields. A cleared field is sent as `null`. |
| Etag conflicts | Updates carry `If-Match` with the etag the edit started from. On a mismatch takt reads Google's version and stores it; the patch is sent again only if none of the fields it writes changed in Google. Otherwise the write fails as a conflict (§3). |
| Deletes | Not guarded by an etag. An event changed elsewhere is still deleted; one already deleted counts as done. |
| Repeating events | An occurrence is edited, moved and deleted on its own ("this event"). "This and following", "all events" and the rule editor are sub-project 4. A series takt could not expand cannot be changed here. |
| Emails to guests | Google emails guests only when asked (`sendUpdates=all`). The editor has a tick box, on by default when the event has other guests. Deleting or dragging an event the account organizes and that has other guests asks first, with "and email guests" and "without email" as the answers. A reply to an invitation never sends email. |
| Changing the calendar | `events.move`, within one account. Not offered for an occurrence. Moving an event to another account would be a copy and a delete, and is not offered. |
| Event type | Chosen when creating: event, focus time, out of office, working location. Google does not allow changing it afterwards, so on an existing event only the type's properties are editable. |
| Description | Edited as Google stores it. A description with HTML is shown and edited as HTML; the editor says so. |
| Dates and times | Typed, not picked: `20 Oct`, `fri`, `+3`, `tomorrow`; `9`, `930`, `9:30pm`, `noon`. |
| Calendar show/hide | Stays a local choice. The views spec put writing Google's `selected` flag "with the other writes in sub-project 3"; it is left out on purpose, since hiding a calendar in takt should not hide it on the owner's phone. |
| Visibility, busy/free, guest permissions | Not editable. Shown in the raw JSON and preserved. |

## 1. Google client (`takt-google`)

`GoogleClient::send_json(method, url, body, if_match)` is the request loop `get_json` already had, with a method, an optional JSON body and an optional `If-Match`. The retry rules are the same for writes as for reads: 429, 5xx and rate-limit 403s are tried again, a 401 refreshes the token once. Two statuses get their own errors: 412 is `PreconditionFailed`, 409 is `Conflict`.

A write Google applied but answered with a 5xx is sent again. Each endpoint is used so that the repeat is harmless:

| Endpoint | Repeat |
|---|---|
| `events.insert` | The body carries an ID takt chose (`new_event_id`, 26 base32hex characters). A second insert fails with 409, and sync then reads the event the first one made. |
| `events.patch` | With `If-Match`, the repeat fails with 412 and goes through the conflict check, which sees Google already has the values. |
| `events.delete` | The repeat fails with 410 or 404, which count as deleted. |
| `events.move` | The repeat fails with 404 and the write is reported as failed although it happened. The next sync round shows the event in its new calendar. |

Every event write is sent with `conferenceDataVersion=1`, so a patch can add or remove a Meet link and other writes keep the one there is, and with `supportsAttachments=true`. `events.get` reads one event for the conflict check.

## 2. Store (`takt-store`)

No migration.

- `apply_event(account, calendar, event)` stores one event as Google returned it from a write. It is `apply_events_page` with one item and no sync token, so the next sync round returns the same event again and stores the same row.
- `mark_event_deleted(account, calendar, event_id, instance)` records a delete the way the next sync would: a single event's row goes, a series goes with its exceptions, and an occurrence stays as a cancelled exception that hides it. For an occurrence the series generates, which has no row, the caller passes the series ID and original start, and a minimal cancelled exception is written.

## 3. Sync (`takt-sync`)

```rust
Command::Write(Box<WriteRequest>)
WriteRequest { id, account_id, calendar_id, op, notify_guests }
WriteOp::Create { body }
WriteOp::Update { event_id, etag, patch, base, move_to }
WriteOp::Delete { event_id, instance }
SyncEvent::WriteDone { id, result: Result<WriteOutcome, WriteError> }
WriteError: Conflict | Gone | Offline | NotSignedIn | Rejected(message)
```

`id` is chosen by the UI and echoed in the answer. On success `Changed` is sent before `WriteDone`, so the UI's read for the change is already running when it hears the write is done.

Each running account loop registers its engine; the supervisor hands a write to that engine on its own task, so a slow write holds up neither sync nor other writes. An account whose loop is not running (signed out, keychain not read yet) answers `NotSignedIn`.

### An update

1. `events.patch` with `If-Match: <etag>`. An occurrence the series generates has no row and so no etag; its patch goes to the instance ID without `If-Match`.
2. On 412: `events.get`, and store what it returns. Then:
   - the event is cancelled: `Gone`;
   - Google's copy already has every value in the patch (an earlier attempt landed, or someone made the same change): success;
   - every top-level field in the patch is the same in Google's copy as in `base`, the values the edit started from: send the patch again with the new etag;
   - otherwise `Conflict`. Nothing was written, and the cache holds Google's version.
3. With `move_to`, `events.move` afterwards. The row leaves the old calendar and is stored in the new one.

A 404 or 410 on an update removes the event from the cache and answers `Gone`.

The comparison in step 2 is by top-level field. An edit of the description and a guest's reply arriving meanwhile do not conflict; two changes to the time do.

### Sync and writes on one calendar

A page of events fetched before a write could be stored after the write's result and put the old version back on screen until the next round. Each calendar has a lock, held while one page of its events is fetched and stored and while a write to it runs. A write that also moves the event takes both calendars' locks in a fixed order.

## 4. The draft (`takt/src/draft.rs`, no GPUI types)

`EventDraft` holds what the editor edits: title, location, description, time, colour, guests, reminders, the Meet link, and the event type with its properties.

- `EventDraft::from_raw(json, display_zone)` reads an event.
- `draft.patch(original, display_zone, request_id)` reads `original` again and emits Google's JSON for each field whose value differs. An untouched draft gives `{}`.
- `draft.insert_body(request_id)` is the body of a new event, without empty fields.
- `base_of(original, patch)` gives the patched fields as they were, for the conflict check.

Details:

- **Time.** A timed event is kept as wall-clock times in the event's own zone (`start.timeZone`, else the display zone). Only the side that moved is sent: a resize sends `end` alone. An end kept in another zone than the start (a flight) is written back in that zone unless the zone field was edited. Switching between all-day and timed sends both sides with the other form's members set to `null`. A local time that does not exist (clocks going forward) becomes an hour later; one that happens twice is the first.
- **Guests.** Each guest is sent as the JSON Google had for them with the edits on top, so `comment`, `additionalGuests` and the like survive. A reply changes only the account's own `responseStatus`. Adding the first guest to an event on a primary calendar also adds the account itself as accepted, as the web does.
- **Reminders.** Either `{"useDefault": true, "overrides": null}` or `useDefault: false` with the full list.
- **Meet.** Adding sends a `createRequest` for `hangoutsMeet`; removing sends `conferenceData: null`.
- **Types.** Focus time and out of office are sent with `transparency: opaque`; working location with `transparency: transparent` and `visibility: public`, which Google requires.
- **An occurrence.** `occurrence_raw(series, instance_id, start, end)` builds the JSON an occurrence would have as an event of its own: the series with that occurrence's times and no rule. The draft is compared with it and the patch goes to the instance ID, which is how Google creates the exception.

`takt/src/entry.rs` reads and writes the typed dates, times and reminder amounts.

## 5. UI

```
┌───────────┬─────────────────────────────┬────────────────────┐
│ sidebar   │ toolbar                     │ [Save] Cancel   Delete
│           ├─────────────────────────────┤ Edit event         │
│           │ grid or agenda              │ ┌ Lunch ─────────┐ │
│           │   ▇▇ dragged event          │ ☐ All day          │
│           │   ░░ where it would land    │ From 12 Nov  12:00 │
│           │                             │ To   12 Nov  13:00 │
│           │                             │ Calendar, Colour…  │
├───────────┴─────────────────────────────┴────────────────────┤
│ status bar                                                   │
└──────────────────────────────────────────────────────────────┘
```

### The editor (`ui/editor.rs`)

A panel in the detail panel's place. Top to bottom: Save, Cancel and Delete; a note when the event changed in Google or cannot be saved; title; all day, start, end and zone; calendar; colour; location; video call; description; guests with the account's reply and the email tick box; reminders; and for a new event, the type.

- The editor is drawn beside the main view's `Takt` key context, not inside it. Otherwise typing `d` in the title would switch to the day view.
- `Enter` in the title saves. `Ctrl+Enter` (`Cmd+Enter`) or `Ctrl+S` saves from anywhere; it is caught before the focused input sees it, so a description gets no stray newline. `Esc` cancels, asking first if anything was typed.
- Changing the start moves the end along, keeping the length.
- Saving an existing event with nothing changed sends nothing and closes.
- While a write runs the Save button reads "Saving…". Success closes the editor and selects the event. A refusal keeps it open with Google's message. A conflict keeps what was typed, names the parts that changed in Google ("Changed in Google while you were editing: title, time"), and makes the next save against the version now in the cache, so saving again puts the typed values over Google's. Cancel keeps Google's.
- One editor at a time. `c`, `e` or a double click while an editor holds typed changes shows "Save or cancel the event you are editing first".

### Without the editor (`ui/editing.rs`)

- **Detail panel.** Edit and Delete buttons on a writable calendar, and Yes / Maybe / No when the account is a guest. An open panel is read again whenever the view reloads, so it shows a change as soon as it lands.
- **Delete.** Always asks. For an occurrence the question says only that occurrence goes.
- **Read-only calendars.** Editing and deleting say "This calendar is read-only"; drags do not start.

### Dragging

| Where | Gesture | Result |
|---|---|---|
| Time grid, on an event | Drag | Moves it, to another day too. 15-minute steps |
| Time grid, an event's bottom edge | Drag | Changes its end |
| Time grid, empty space | Drag | Opens the editor on a new event over the drawn time |
| Time grid, empty space | Double click | New event, an hour from that half hour |
| Month and the all-day strip, on an event | Drag to another day | Moves it by whole days, keeping the time of day |
| Month, a day | Double click | New all-day event |
| Any event | Double click | Opens the editor |

A drag shows where the event would land and dims the original. After the drop the preview stays until the write is answered and the view has been read again, so the event does not jump back to its old place in between. A failed move removes the preview and says why.

Mouse-move and mouse-up are heard window-wide while a drag runs, since the pointer crosses other events on the way. Each grid records where its day columns were drawn; the pointer's position against that area gives the day and the minute.

### Keyboard

| Key | Action |
|---|---|
| `c` | New event: all day on the selected day in the month, else an hour from the next half hour (today) or from 09:00 |
| `e` | Edit the selected event, or the one whose details are open |
| `Delete`, `Backspace` | Delete it, after asking |
| `Enter` (in the title) | Save |
| `Ctrl+Enter`, `Ctrl+S` (`Cmd` on macOS) | Save |
| `Esc` | Cancel the edit |

## 6. Tests

| Where | Tests |
|---|---|
| `takt-google` (11) | Each endpoint's path, query, body and `If-Match`; 412 and 409 as typed errors; a repeat after 503; a 204 delete |
| `takt-store` (8) | A written event keeps the sync token; a move reports the old and the new time; deletes of a single event, a series, a generated occurrence (timed and all-day) and a moved occurrence |
| `takt-sync` (18) | Against a fake Google: create with a chosen ID, the repeated insert, patch with etag, the three outcomes of a stale etag, an event deleted elsewhere, calendar change with and without a patch, deletes, guest emails, Google's refusal text, offline; the command answered through the runtime |
| `takt` draft and entry (29) | One test per field's patch, the zone cases, the special types, occurrences, typed dates and times |
| `takt` window (31) | A headless window: `c`, typing, `Enter`; `e` and the exact patch sent; escape and the discard question; conflict, refusal and deleted-elsewhere answers; occurrences; delete and reply with and without guests; read-only calendars; move, resize, draw, double click, and the month's day move |

The `takt` crate goes from 110 to 170 tests.

`cargo run -p takt --example demo -- week new` opens the editor over made-up events. The demo answers writes by changing its temporary cache, so the editor, drags and deletes can be tried without an account. It cannot change one occurrence of a repeating event.

## Done criteria for sub-project 3

1. `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` pass.
2. Every editable field, changed in takt, reads the same in the Google Calendar web UI, and the reverse.
3. An event edited in takt and on the web at the same time ends with both changes when they touch different fields, and with a conflict notice in takt when they touch the same one.

Criterion 1 is checked. Criteria 2 and 3 need the owner's accounts: no write has been sent to the real Google API yet. The request shapes follow Google's API reference and are tested against a fake server.

## Known limits

- Focus time, out of office and working location need a Google Workspace account and the primary calendar. On other accounts Google refuses them; takt shows its message.
- After a 5xx whose write did land, the conflict check can report a conflict although the change was made, when Google stores a value in another form than takt sent it (a time with another offset). The cache and the view are right either way.
- As a guest, changes other than the reply, colour and reminders stay in the account's own copy and are overwritten when the organizer next changes the event. The editor says so.
- An event's start cannot be dragged on its own (only moved or the end changed), there is no keyboard move, and no undo.
- After saving from the agenda the new event is not selected in the list.

## Out of scope for sub-project 3

The recurrence rule and "this and following" / "all events" (4); tasks (5); notifications (6); the offline outbox; writing a calendar's `selected` flag; moving an event between accounts; attachments; visibility, busy/free and guest permissions.
