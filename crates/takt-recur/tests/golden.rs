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

fn str_at<'a>(v: &'a Value, what: &str) -> Result<&'a str, String> {
    v.as_str().ok_or_else(|| format!("{what} is not a string"))
}

fn when_at(v: &Value, what: &str) -> Result<When, String> {
    when(v).ok_or_else(|| format!("{what} is not a date or dateTime"))
}

fn array_at<'a>(v: &'a Value, what: &str) -> Result<&'a Vec<Value>, String> {
    v.as_array()
        .ok_or_else(|| format!("{what} is not an array"))
}

fn master_from(v: &Value, calendar_tz: Option<&str>) -> Result<Master, String> {
    Ok(Master {
        id: str_at(&v["id"], "master.id")?.to_owned(),
        start: when_at(&v["start"], "master.start")?,
        end: when_at(&v["end"], "master.end")?,
        tz: v["start"]["timeZone"]
            .as_str()
            .or(calendar_tz)
            .and_then(|n| n.parse().ok())
            .unwrap_or(chrono_tz::UTC),
        recurrence: array_at(&v["recurrence"], "master.recurrence")?
            .iter()
            .map(|l| str_at(l, "master.recurrence line").map(str::to_owned))
            .collect::<Result<_, _>>()?,
    })
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

fn instant(v: &Value, what: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(str_at(v, what)?)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| format!("{what}: {e}"))
}

/// Compares one fixture; `Err` describes a malformed fixture or a mismatch.
fn check(fixture: &Value) -> Result<(), String> {
    let range = Range {
        start: instant(&fixture["range"]["start"], "range.start")?,
        end: instant(&fixture["range"]["end"], "range.end")?,
    };
    // Google reads all-day dates in the calendar's zone at the range edges; compare away from them.
    let inner = Range {
        start: range.start + TimeDelta::days(1),
        end: range.end - TimeDelta::days(1),
    };
    let keep = |start: &When| start.as_utc() >= inner.start && start.as_utc() < inner.end;

    let master = master_from(&fixture["master"], fixture["calendarTimeZone"].as_str())?;
    let exceptions: Vec<_> = array_at(&fixture["exceptions"], "exceptions")?
        .iter()
        .filter_map(exception_from)
        .collect();
    let expanded =
        expand(&master, &exceptions, &range).map_err(|e| format!("expansion failed: {e:?}"))?;
    let mut ours: Vec<(String, When, When)> = expanded
        .into_iter()
        .filter(|o| keep(&o.start))
        .map(|o| (o.instance_id, o.start, o.end))
        .collect();
    let mut google: Vec<(String, When, When)> = Vec::new();
    for i in array_at(&fixture["instances"], "instances")? {
        if i["status"] == "cancelled" {
            continue;
        }
        let instance = (
            str_at(&i["id"], "instance id")?.to_owned(),
            when_at(&i["start"], "instance start")?,
            when_at(&i["end"], "instance end")?,
        );
        if keep(&instance.1) {
            google.push(instance);
        }
    }
    ours.sort_by(|a, b| a.0.cmp(&b.0));
    google.sort_by(|a, b| a.0.cmp(&b.0));
    if ours == google {
        return Ok(());
    }
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
    Err(format!(
        "({}): {} ours vs {} Google's\n  only ours: {only_ours:?}\n  only Google's: {only_google:?}",
        fixture["name"],
        ours.len(),
        google.len()
    ))
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
        if let Err(e) = check(&fixture) {
            failures.push(format!("{}: {e}", file.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
