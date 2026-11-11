# takt

A desktop client for Google Calendar and Google Tasks, written in Rust with [gpui-kit](https://github.com/longbridge/gpui-kit).

Sub-projects 1 and 2 of 6 are done: read-only day, week, month and agenda views that merge several Google accounts and work offline from a local cache. Designs: `docs/superpowers/specs/2026-10-17-takt-foundation-design.md` (accounts, cache, sync) and `docs/superpowers/specs/2026-11-11-takt-views-design.md` (views).

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
| `Ctrl+R` (`Cmd+R` on macOS) | Sync now |

Set `TAKT_LOG=debug` for more detailed logs.

### View settings

Optional, in the same `config.toml`:

```toml
[view]
default = "week"                          # day, week, month or agenda
time_zone = "Europe/Berlin"               # default: the OS time zone
secondary_time_zone = "America/New_York"  # a second column of hour labels in day and week
week_start = "monday"                     # default: your Google Calendar setting
```

### Trying the views without an account

```bash
cargo run -p takt --example demo -- month
```

This opens the main window over made-up events in a temporary cache, with no sign-in and no sync. The argument is the view to open in.

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
- takt is read-only so far: nothing is written to Google. Hiding a calendar is a local choice and does not change the calendar's setting in Google.
- Tasks with a due date appear in the agenda only, not yet on the day, week and month grids.
- Logs rotate daily and only the last 14 days are kept.
