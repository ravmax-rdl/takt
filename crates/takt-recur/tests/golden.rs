//! Local expansion must match Google's `events.instances` for every captured series.

use std::path::Path;

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use serde_json::Value;
use takt_recur::{Exception, Master, Range, When, expand};

fn when(v: &Value) -> Option<When> {
    if let Some(date) = v.get("date").and_then(Value::as_str) {
        return NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()
            .map(When::Date);
    }
    let text = v.get("dateTime")?.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| When::At(t.with_timezone(&Utc)))
}

fn master_from(v: &Value, calendar_tz: Option<&str>) -> Master {
    Master {
        id: v["id"].as_str().unwrap().to_owned(),
        start: when(&v["start"]).unwrap(),
        end: when(&v["end"]).unwrap(),
        tz: v["start"]["timeZone"]
            .as_str()
            .or(calendar_tz)
            .and_then(|n| n.parse().ok())
            .unwrap_or(chrono_tz::UTC),
        recurrence: v["recurrence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_owned())
            .collect(),
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
    DateTime::parse_from_rfc3339(v.as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn local_expansion_matches_google() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !files.is_empty(),
        "no fixtures in {}; capture them as README.md describes",
        dir.display()
    );

    let mut failures = Vec::new();
    for file in files {
        let fixture: Value = match std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
        {
            Ok(fixture) => fixture,
            Err(e) => {
                failures.push(format!("{}: cannot read fixture: {e}", file.display()));
                continue;
            }
        };
        let range = Range {
            start: instant(&fixture["range"]["start"]),
            end: instant(&fixture["range"]["end"]),
        };
        // Google reads all-day dates in the calendar's zone at the range edges; compare away from them.
        let inner = Range {
            start: range.start + TimeDelta::days(1),
            end: range.end - TimeDelta::days(1),
        };
        let keep = |start: &When| start.as_utc() >= inner.start && start.as_utc() < inner.end;

        let master = master_from(&fixture["master"], fixture["calendarTimeZone"].as_str());
        let exceptions: Vec<_> = fixture["exceptions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(exception_from)
            .collect();
        let expanded = match expand(&master, &exceptions, &range) {
            Ok(expanded) => expanded,
            Err(e) => {
                failures.push(format!("{}: expansion failed: {e:?}", file.display()));
                continue;
            }
        };
        let mut ours: Vec<(String, When, When)> = expanded
            .into_iter()
            .filter(|o| keep(&o.start))
            .map(|o| (o.instance_id, o.start, o.end))
            .collect();
        let mut google: Vec<(String, When, When)> = fixture["instances"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["status"] != "cancelled")
            .map(|i| {
                (
                    i["id"].as_str().unwrap().to_owned(),
                    when(&i["start"]).unwrap(),
                    when(&i["end"]).unwrap(),
                )
            })
            .filter(|(_, start, _)| keep(start))
            .collect();
        ours.sort_by(|a, b| a.0.cmp(&b.0));
        google.sort_by(|a, b| a.0.cmp(&b.0));
        if ours != google {
            let only_ours: Vec<_> = ours
                .iter()
                .filter(|o| !google.contains(o))
                .take(3)
                .collect();
            let only_google: Vec<_> = google
                .iter()
                .filter(|g| !ours.contains(g))
                .take(3)
                .collect();
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
