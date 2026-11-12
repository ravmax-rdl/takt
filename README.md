# takt

A desktop client for Google Calendar and Google Tasks, written in Rust with [gpui-kit](https://github.com/longbridge/gpui-kit).

Sub-projects 1 to 3 of 6 are done: day, week, month and agenda views that merge several Google accounts and read from a local cache, and creating, editing, deleting and dragging single events. Designs, under `docs/superpowers/specs/`: `2026-10-17-takt-foundation-design.md` (accounts, cache, sync), `2026-11-11-takt-views-design.md` (views) and `2026-11-12-takt-editing-design.md` (editing).

## Building

You need a recent stable Rust toolchain (the crates use edition 2024; it builds with rustc 1.97).

On Linux, GPUI also needs some system development packages. On Debian/Ubuntu these were needed to build here:

```bash
sudo apt install libxkbcommon-dev libxkbcommon-x11-dev
```

Depending on the distribution you may also need the libxcb, Wayland, fontconfig and Vulkan loader development packages (for example `libxcb1-dev`, `libwayland-dev`, `libfontconfig-dev`, `libvulkan-dev`).

macOS and Windows need nothing beyond the Rust toolchain, as far as we know.

## Setting up Google access

takt uses your own OAuth client. Do this once:

1. In the [Google Cloud console](https://console.cloud.google.com/), create a project.
2. Enable the **Google Calendar API** and the **Google Tasks API**.
3. Under **APIs & Services → OAuth consent screen**, choose External and fill in the required fields. Set the publishing status to **In production**. While it is in Testing, Google expires every sign-in after 7 days. An unverified production app shows a "Google hasn't verified this app" warning once per account; choose Advanced → continue.
4. Under **Credentials**, create an **OAuth client ID** of type **Desktop app**.
5. Put its ID and secret in `config.toml` (paths below). takt's first-run screen shows the exact path.

```toml
[google]
client_id = "<your id>.apps.googleusercontent.com"
client_secret = "<your secret>"
```

### Where takt keeps its files

| | Config file | Cache (`takt.sqlite3`) and `logs/` |
|---|---|---|
| Windows | `%APPDATA%\takt\config\config.toml` | `%LOCALAPPDATA%\takt\data` |
| macOS | `~/Library/Application Support/takt/config.toml` | `~/Library/Application Support/takt` |
| Linux | `~/.config/takt/config.toml` (or `$XDG_CONFIG_HOME/takt`) | `~/.local/share/takt` (or `$XDG_DATA_HOME/takt`) |

Refresh tokens are stored in the OS keychain under the service name `takt`, one entry per account email: the macOS Keychain, the Windows Credential Manager, or the Secret Service on Linux (GNOME Keyring, KWallet). Access tokens stay in memory.

## Running

```bash
cargo run -p takt --release
```

takt opens in the week view. The toolbar switches between day, week, month and agenda; the mini calendar in the sidebar jumps to a date; clicking a calendar in the sidebar shows or hides it in takt only (nothing is sent to Google).

| Key | Action |
|---|---|
| `d` / `1`, `w` / `2`, `m` / `3`, `a` / `4` | Day, week, month, agenda |
| `j`, `n`, `PageDown` | Next day, week or month. In the agenda, `j` is the next row |
| `k`, `p`, `PageUp` | Previous day, week or month. In the agenda, `k` is the previous row, and on the first row it loads earlier days |
| `t` | Jump to today |
| `←` / `h`, `→` / `l` | Move the selected day |
| `↑` / `↓` | Month: move the selected day by a week. Day and week: previous / next event on the selected day |
| `Tab` / `Shift+Tab` | Next / previous event in the view |
| `Enter` | Open the selected event's details; with no event selected, open the selected day |
| `Esc` | Close details; pressed again, clear the selection |
| `c` | New event |
| `e` | Edit the selected event |
| `Delete` / `Backspace` | Delete the selected event, after asking |
| `Ctrl+R` (`Cmd+R` on macOS) | Sync now |

Set `TAKT_LOG=debug` for more detailed logs.

### Changing events

`c` opens the editor on a new event; `e` or a double click opens it on an existing one. In the editor, `Enter` in the title or `Ctrl+Enter` anywhere saves and `Esc` cancels. Dates and times are typed: `20 Oct`, `fri`, `tomorrow`, `+3`; `9`, `930`, `9:30pm`, `noon`.

With the mouse, in the day and week views: drag an event to move it, drag its bottom edge to change its end, drag on empty space to draw a new event, double-click empty space for an hour-long one. In the month and in the all-day strip, drag an event to another day.

The detail panel has Edit and Delete, and Yes / Maybe / No for an invitation.

What to know:

- Changes go straight to Google and need a connection. Offline, a save fails with a message and the editor keeps what you typed.
- takt sends only the fields you changed. If the event changed in Google in those same fields while you were editing, the save is refused, the editor says what changed, and saving again puts your version over it.
- For a repeating event, a change applies to the one occurrence you picked. Changing the whole series or its rule comes with sub-project 4.
- Google emails an event's guests only when asked. The editor has a tick box for it; deleting or dragging an event you organize that has guests asks first.

### View settings

Optional, in the same `config.toml`:

```toml
[view]
default = "week"                          # day, week, month or agenda
time_zone = "Europe/Berlin"               # default: the OS time zone
secondary_time_zone = "America/New_York"  # a second column of hour labels in day and week
week_start = "monday"                     # default: your Google Calendar setting
```

### Trying it without an account

```bash
cargo run -p takt --example demo -- month
```

This opens the main window over made-up events in a temporary cache, with no sign-in and no sync. The argument is the view to open in; add `new` to open with the editor on a new event. Changes are applied to the temporary cache, so the editor, drags and deletes can be tried. The demo cannot change one occurrence of a repeating event.

## Tests

```bash
cargo test --workspace
```

The recurrence golden test (`crates/takt-recur/tests/golden.rs`) compares takt's expansion with Google's `events.instances` output for the series captured in `crates/takt-recur/tests/fixtures`.

To capture them again or add a series, sign in with takt, create the series in a calendar named `takt-fixtures`, then run:

```bash
cargo run -p takt-google --example capture -- --config "<path to config.toml>" --email "<your email>" --calendar takt-fixtures --from 2026-01-01 --to 2028-01-01 --out crates/takt-recur/tests/fixtures
```

The capture keeps only an allowlist of event fields (`id`, `status`, `summary`, `start`, `end`, `recurrence`, `recurringEventId`, `originalStartTime`, `iCalUID`, `eventType`, `kind`) and drops every other field, so attendees, descriptions and locations never land in the repository. As a safety net, any value left under an `email` key (such as `creator.email` or `organizer.email`, if they were ever allowlisted) is replaced with a placeholder; email addresses elsewhere, for example inside a kept `summary`, are not. Still, review the files before committing them.

## Known limits

- The Tasks API stores only the date of a task's due time, and does not expose recurring tasks.
- Writes have been tested against a fake Google server only, not yet against the real API.
- Hiding a calendar is a local choice and does not change the calendar's setting in Google.
- Whole repeating series and their rules cannot be changed yet, only single occurrences. Tasks cannot be changed.
- Focus time, out of office and working location events need a Google Workspace account and its primary calendar; elsewhere Google refuses them.
- A description that contains HTML is edited as HTML.
- Tasks with a due date appear in the agenda only, not yet on the day, week and month grids.
- Logs rotate daily and only the last 14 days are kept.
