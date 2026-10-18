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
    args.get(name)
        .cloned()
        .unwrap_or_else(|| panic!("missing --{name}"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args: HashMap<String, String> = raw
        .chunks(2)
        .map(|pair| {
            (
                pair[0].trim_start_matches("--").to_owned(),
                pair.get(1).cloned().unwrap_or_default(),
            )
        })
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

    let refresh = KeyringTokenStore
        .get(&email)?
        .ok_or("no refresh token for that email; sign in with takt first")?;
    let endpoints = Endpoints::google();
    let http = reqwest::Client::new();
    let oauth = OAuthClient {
        client_id: config.google.client_id,
        client_secret: config.google.client_secret,
    };
    let client = GoogleClient::new(
        http.clone(),
        endpoints.clone(),
        Auth::new(http, endpoints.token, oauth, refresh),
    );

    let mut calendar_id = None;
    let mut calendar_tz = None;
    let mut page_token: Option<String> = None;
    loop {
        let page = client
            .calendar_list_page(None, page_token.as_deref())
            .await?;
        if let Some(c) = page
            .items
            .iter()
            .find(|c| c.fields.display_name() == calendar_name)
        {
            calendar_id = Some(c.fields.id.clone());
            calendar_tz = c.fields.time_zone.clone();
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
        let page = client
            .events_page(&calendar_id, None, page_token.as_deref())
            .await?;
        events.extend(page.items);
        match page.next_page_token {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }

    let mut scrubber = Scrubber::default();
    for master in events
        .iter()
        .filter(|e| !e.fields.recurrence.is_empty() && !e.fields.is_cancelled())
    {
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
        let name = master
            .fields
            .summary
            .clone()
            .unwrap_or_else(|| master.fields.id.clone());
        let mut fixture = json!({
            "name": name,
            "range": {"start": from.to_rfc3339(), "end": to.to_rfc3339()},
            "calendarTimeZone": calendar_tz,
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
        let id_part = master
            .fields
            .id
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        let file = out.join(format!("{}-{}.json", slug.trim_matches('-'), id_part));
        std::fs::write(&file, serde_json::to_string_pretty(&fixture)?)?;
        println!("wrote {}", file.display());
    }
    Ok(())
}

/// Keys that survive scrubbing on masters, exceptions and instances.
const ALLOWED: &[&str] = &[
    "id",
    "status",
    "summary",
    "start",
    "end",
    "recurrence",
    "recurringEventId",
    "originalStartTime",
    "iCalUID",
    "eventType",
    "kind",
];

/// Keeps only allowlisted keys on every event; as a safety net, replaces any
/// email address left anywhere with a stable placeholder.
#[derive(Default)]
struct Scrubber {
    emails: HashMap<String, String>,
}

impl Scrubber {
    fn scrub(&mut self, fixture: &mut Value) {
        let Some(map) = fixture.as_object_mut() else {
            return;
        };
        for key in ["master", "exceptions", "instances"] {
            match map.get_mut(key) {
                Some(Value::Array(events)) => events.iter_mut().for_each(|e| self.event(e)),
                Some(event) => self.event(event),
                None => {}
            }
        }
    }

    fn event(&mut self, event: &mut Value) {
        if let Value::Object(map) = event {
            map.retain(|key, _| ALLOWED.contains(&key.as_str()));
        }
        self.emails(event);
    }

    fn emails(&mut self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, v) in map.iter_mut() {
                    match v {
                        Value::String(email) if key == "email" => {
                            let n = self.emails.len() + 1;
                            let placeholder = self
                                .emails
                                .entry(email.clone())
                                .or_insert_with(|| format!("person{n}@example.com"));
                            *v = Value::String(placeholder.clone());
                        }
                        _ => self.emails(v),
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|v| self.emails(v)),
            _ => {}
        }
    }
}
