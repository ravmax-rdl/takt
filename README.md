# takt

A desktop client for Google Calendar and Google Tasks, written in Rust with [gpui-kit](https://github.com/longbridge/gpui-kit).

This is sub-project 1 of 6: a read-only agenda that merges several Google accounts and works offline from a local cache. Design: `docs/superpowers/specs/2026-10-17-takt-foundation-design.md`.

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

| Key | Action |
|---|---|
| `j` / `k` | Next / previous row (`k` on the first row loads earlier days) |
| `Enter` | Open details |
| `Esc` | Close details |
| `t` | Jump to today |
| `Ctrl+R` (`Cmd+R` on macOS) | Sync now |

Set `TAKT_LOG=debug` for more detailed logs.

## Tests

```bash
cargo test --workspace --no-fail-fast
```

The recurrence golden test (`crates/takt-recur/tests/golden.rs`) compares takt's expansion with Google's, and fails with a "no fixtures in …" message until fixtures have been captured. Until then plain `cargo test --workspace` stops at it; `--no-fail-fast` runs everything else as well.

To capture fixtures, sign in with takt, create series in a calendar named `takt-fixtures`, then run:

```bash
cargo run -p takt-google --example capture -- --config "<path to config.toml>" --email "<your email>" --calendar takt-fixtures --from 2026-01-01 --to 2028-01-01 --out crates/takt-recur/tests/fixtures
```

The capture keeps only an allowlist of event fields (`id`, `status`, `summary`, `start`, `end`, `recurrence`, `recurringEventId`, `originalStartTime`, `iCalUID`, `eventType`, `kind`) and replaces any email address left with a placeholder, so attendees, descriptions and locations never land in the repository. Still, review the files before committing them.

## Known limits

- The Tasks API stores only the date of a task's due time, and does not expose recurring tasks.
- Sub-project 1 is read-only: nothing is written to Google.
- Logs rotate daily and only the last 14 days are kept.
