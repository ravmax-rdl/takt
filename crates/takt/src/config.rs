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
}
