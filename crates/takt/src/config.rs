//! Where takt keeps its files, and the OAuth client it reads from config.toml.

use std::path::{Path, PathBuf};

use chrono::Weekday;
use chrono_tz::Tz;
use serde::Deserialize;
use takt_google::OAuthClient;

use crate::view::{ViewKind, parse_weekday};

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
            let (id, secret) = (
                file.google.client_id.trim(),
                file.google.client_secret.trim(),
            );
            if id.is_empty() || secret.is_empty() {
                ConfigState::Invalid("client_id and client_secret must both be set".into())
            } else if id.contains("<your") || secret.contains("<your") {
                ConfigState::Invalid(
                    "replace the placeholder client_id/client_secret in config.toml".into(),
                )
            } else {
                ConfigState::Ready(OAuthClient {
                    client_id: id.to_owned(),
                    client_secret: secret.to_owned(),
                })
            }
        }
        Err(e) => ConfigState::Invalid(e.to_string()),
    }
}

/// The optional `[view]` section of config.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewSettings {
    /// The view takt opens in.
    pub default_view: ViewKind,
    /// Shown instead of the OS time zone.
    pub time_zone: Option<Tz>,
    /// A second column of hour labels in the day and week views.
    pub secondary_time_zone: Option<Tz>,
    /// Overrides the Google account's "week starts on" setting.
    pub week_start: Option<Weekday>,
}

#[derive(Deserialize, Default)]
struct ViewFile {
    #[serde(default)]
    view: ViewSection,
}

#[derive(Deserialize, Default)]
struct ViewSection {
    default: Option<String>,
    time_zone: Option<String>,
    secondary_time_zone: Option<String>,
    week_start: Option<String>,
}

/// Reads `[view]`. A missing file or section gives the defaults; a value
/// takt cannot read is logged and skipped, so a typo never blocks startup.
pub fn load_view_settings(path: &Path) -> ViewSettings {
    let Ok(text) = std::fs::read_to_string(path) else {
        return ViewSettings::default();
    };
    match toml::from_str::<ViewFile>(&text) {
        Ok(file) => view_settings(file.view),
        Err(error) => {
            tracing::warn!(%error, "cannot read [view] from config.toml");
            ViewSettings::default()
        }
    }
}

fn view_settings(section: ViewSection) -> ViewSettings {
    fn read<T>(key: &str, value: Option<String>, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
        let value = value?;
        let parsed = parse(&value);
        if parsed.is_none() {
            tracing::warn!(key, value, "ignoring an unreadable [view] setting");
        }
        parsed
    }
    let zone = |name: &str| name.trim().parse::<Tz>().ok();
    ViewSettings {
        default_view: read("default", section.default, ViewKind::parse).unwrap_or_default(),
        time_zone: read("time_zone", section.time_zone, zone),
        secondary_time_zone: read("secondary_time_zone", section.secondary_time_zone, zone),
        week_start: read("week_start", section.week_start, parse_weekday),
    }
}

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
        let (_dir, path) = write(
            "[google]\nclient_id = \" id.apps.googleusercontent.com \"\nclient_secret = \"s3cret\"\n",
        );
        assert_eq!(
            load(&path),
            ConfigState::Ready(OAuthClient {
                client_id: "id.apps.googleusercontent.com".into(),
                client_secret: "s3cret".into()
            })
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
    fn example_placeholders_are_invalid() {
        let (_dir, path) = write(EXAMPLE);
        assert_eq!(
            load(&path),
            ConfigState::Invalid(
                "replace the placeholder client_id/client_secret in config.toml".into()
            )
        );
    }

    #[test]
    fn missing_google_section_is_invalid() {
        let (_dir, path) = write("[other]\nkey = 1\n");
        assert!(matches!(load(&path), ConfigState::Invalid(_)));
    }

    #[test]
    fn directory_at_config_path_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(load(&path), ConfigState::Invalid(_)));
    }

    #[test]
    fn view_settings_default_without_a_section() {
        let (_dir, path) = write(
            "[google]
client_id = \"a\"
client_secret = \"b\"
",
        );
        assert_eq!(load_view_settings(&path), ViewSettings::default());
        assert_eq!(ViewSettings::default().default_view, ViewKind::Week);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_view_settings(&dir.path().join("config.toml")),
            ViewSettings::default()
        );
    }

    #[test]
    fn view_settings_are_read() {
        let (_dir, path) = write(
            "[view]
default = \"month\"
time_zone = \"Europe/Berlin\"
             secondary_time_zone = \"America/New_York\"
week_start = \"sunday\"
",
        );
        assert_eq!(
            load_view_settings(&path),
            ViewSettings {
                default_view: ViewKind::Month,
                time_zone: Some(Tz::Europe__Berlin),
                secondary_time_zone: Some(Tz::America__New_York),
                week_start: Some(Weekday::Sun),
            }
        );
    }

    #[test]
    fn unreadable_view_values_are_skipped() {
        let (_dir, path) = write(
            "[view]
default = \"year\"
time_zone = \"Mars/Olympus\"
week_start = \"monday\"
",
        );
        assert_eq!(
            load_view_settings(&path),
            ViewSettings {
                week_start: Some(Weekday::Mon),
                ..ViewSettings::default()
            }
        );
    }
}
