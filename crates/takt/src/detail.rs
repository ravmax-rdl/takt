//! The read-only detail panel's content, built from an event's raw JSON.

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct EventDetail {
    pub title: String,
    pub when: String,
    pub calendar: String,
    pub account: String,
    pub location: Option<String>,
    pub organizer: Option<String>,
    /// (who, response) pairs.
    pub attendees: Vec<(String, String)>,
    pub conference: Vec<String>,
    pub reminders: Vec<String>,
    pub recurrence: Vec<String>,
    pub description: Option<String>,
    pub raw_json: String,
}

pub fn event_detail(raw: &Value, when: String, calendar: String, account: String) -> EventDetail {
    let text = |v: &Value| {
        v.as_str()
            .map(str::to_owned)
            .filter(|s| !s.trim().is_empty())
    };
    let person = |p: &Value| text(&p["displayName"]).or_else(|| text(&p["email"]));
    let attendees = raw["attendees"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some((
                        person(a)?,
                        response_label(a["responseStatus"].as_str().unwrap_or("needsAction"))
                            .to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut conference: Vec<String> = raw["conferenceData"]["entryPoints"]
        .as_array()
        .map(|points| points.iter().filter_map(|p| text(&p["uri"])).collect())
        .unwrap_or_default();
    if let Some(link) = text(&raw["hangoutLink"])
        && !conference.contains(&link)
    {
        conference.insert(0, link);
    }
    EventDetail {
        title: text(&raw["summary"]).unwrap_or_else(|| "(No title)".into()),
        when,
        calendar,
        account,
        location: text(&raw["location"]),
        organizer: person(&raw["organizer"]),
        attendees,
        conference,
        reminders: reminders(&raw["reminders"]),
        recurrence: raw["recurrence"]
            .as_array()
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(describe_recurrence_line)
                    .collect()
            })
            .unwrap_or_default(),
        description: text(&raw["description"])
            .map(|d| strip_html(&d))
            .filter(|d| !d.is_empty()),
        raw_json: serde_json::to_string_pretty(raw).unwrap_or_default(),
    }
}

fn response_label(status: &str) -> &'static str {
    match status {
        "accepted" => "Going",
        "declined" => "Declined",
        "tentative" => "Maybe",
        _ => "Awaiting reply",
    }
}

fn reminders(value: &Value) -> Vec<String> {
    if value["useDefault"].as_bool() == Some(true) {
        return vec!["Calendar's default reminders".into()];
    }
    value["overrides"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|r| {
                    let method = if r["method"] == "email" {
                        "Email"
                    } else {
                        "Notification"
                    };
                    Some(format!(
                        "{method} {} before",
                        duration(r["minutes"].as_u64()?)
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn duration(minutes: u64) -> String {
    let plural = |n: u64, unit: &str| {
        if n == 1 {
            format!("1 {unit}")
        } else {
            format!("{n} {unit}s")
        }
    };
    if minutes > 0 && minutes.is_multiple_of(10_080) {
        plural(minutes / 10_080, "week")
    } else if minutes > 0 && minutes.is_multiple_of(1_440) {
        plural(minutes / 1_440, "day")
    } else if minutes > 0 && minutes.is_multiple_of(60) {
        plural(minutes / 60, "hour")
    } else {
        plural(minutes, "minute")
    }
}

/// Plain-language reading of one recurrence line. Lines it cannot read are returned as they are.
pub fn describe_recurrence_line(line: &str) -> String {
    let Some((head, value)) = line.split_once(':') else {
        return line.to_owned();
    };
    let name = head
        .split(';')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    match name.as_str() {
        "EXDATE" => {
            let n = value.split(',').count();
            if n == 1 {
                "Except 1 date".into()
            } else {
                format!("Except {n} dates")
            }
        }
        "RDATE" => {
            let n = value.split(',').count();
            if n == 1 {
                "Also 1 extra date".into()
            } else {
                format!("Also {n} extra dates")
            }
        }
        "RRULE" => describe_rule(value).unwrap_or_else(|| line.to_owned()),
        _ => line.to_owned(),
    }
}

fn describe_rule(rule: &str) -> Option<String> {
    let parts: std::collections::HashMap<String, String> = rule
        .split(';')
        .filter_map(|p| {
            p.split_once('=')
                .map(|(k, v)| (k.to_ascii_uppercase(), v.to_owned()))
        })
        .collect();
    let interval: u32 = parts
        .get("INTERVAL")
        .and_then(|i| i.parse().ok())
        .unwrap_or(1);
    let (single, unit) = match parts.get("FREQ")?.as_str() {
        "DAILY" => ("Daily", "days"),
        "WEEKLY" => ("Weekly", "weeks"),
        "MONTHLY" => ("Monthly", "months"),
        "YEARLY" => ("Annually", "years"),
        _ => return None,
    };
    let mut text = if interval > 1 {
        format!("Every {interval} {unit}")
    } else {
        single.to_owned()
    };
    if let Some(days) = parts.get("BYDAY") {
        let names: Vec<String> = days.split(',').filter_map(describe_byday).collect();
        if let Some(first) = names
            .first()
            .filter(|_| names.len() == 1 && first_is_ordinal(days))
        {
            text.push_str(&format!(" on the {first}"));
        } else {
            text.push_str(&format!(" on {}", names.join(", ")));
        }
    } else if let Some(day) = parts.get("BYMONTHDAY") {
        text.push_str(&format!(" on day {day}"));
    }
    if let Some(count) = parts.get("COUNT") {
        text.push_str(&format!(", {count} times"));
    }
    if let Some(until) = parts.get("UNTIL") {
        let date = NaiveDateTime::parse_from_str(until.trim_end_matches('Z'), "%Y%m%dT%H%M%S")
            .map(|t| t.date())
            .or_else(|_| NaiveDate::parse_from_str(until, "%Y%m%d"))
            .ok()?;
        text.push_str(&format!(", until {}", date.format("%-d %b %Y")));
    }
    Some(text)
}

fn first_is_ordinal(days: &str) -> bool {
    days.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+')
}

fn describe_byday(token: &str) -> Option<String> {
    let token = token.trim();
    let split = token.len().checked_sub(2)?;
    let (ordinal, day) = token.split_at(split);
    let (short, long) = match day {
        "MO" => ("Mon", "Monday"),
        "TU" => ("Tue", "Tuesday"),
        "WE" => ("Wed", "Wednesday"),
        "TH" => ("Thu", "Thursday"),
        "FR" => ("Fri", "Friday"),
        "SA" => ("Sat", "Saturday"),
        "SU" => ("Sun", "Sunday"),
        _ => return None,
    };
    if ordinal.is_empty() {
        return Some(short.to_owned());
    }
    let which = match ordinal.trim_start_matches('+') {
        "1" => "first",
        "2" => "second",
        "3" => "third",
        "4" => "fourth",
        "5" => "fifth",
        "-1" => "last",
        "-2" => "second to last",
        _ => return Some(format!("{ordinal}{short}")),
    };
    Some(format!("{which} {long}"))
}

/// Google stores descriptions as light HTML. Keep line breaks, drop tags,
/// decode the common entities.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = rest[open + 1..open + close].trim().to_ascii_lowercase();
        if tag.starts_with("br") || tag == "/p" || tag == "/div" || tag == "/li" {
            out.push('\n');
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn describes_common_rules() {
        assert_eq!(describe_recurrence_line("RRULE:FREQ=DAILY"), "Daily");
        assert_eq!(
            describe_recurrence_line("RRULE:FREQ=WEEKLY;BYDAY=MO,WE"),
            "Weekly on Mon, Wed"
        );
        assert_eq!(
            describe_recurrence_line("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=FR"),
            "Every 2 weeks on Fri"
        );
        assert_eq!(
            describe_recurrence_line("RRULE:FREQ=MONTHLY;BYDAY=3TU"),
            "Monthly on the third Tuesday"
        );
        assert_eq!(
            describe_recurrence_line("RRULE:FREQ=MONTHLY;BYDAY=-1FR"),
            "Monthly on the last Friday"
        );
        assert_eq!(
            describe_recurrence_line("RRULE:FREQ=MONTHLY;BYMONTHDAY=31"),
            "Monthly on day 31"
        );
        assert_eq!(
            describe_recurrence_line("RRULE:FREQ=YEARLY;COUNT=5"),
            "Annually, 5 times"
        );
        assert_eq!(
            describe_recurrence_line("RRULE:FREQ=WEEKLY;UNTIL=20261231T235959Z"),
            "Weekly, until 31 Dec 2026"
        );
        assert_eq!(
            describe_recurrence_line("EXDATE;TZID=Europe/London:20261012T090000,20261019T090000"),
            "Except 2 dates"
        );
        assert_eq!(
            describe_recurrence_line("RRULE:FREQ=MINUTELY"),
            "RRULE:FREQ=MINUTELY"
        );
    }

    #[test]
    fn strips_html_descriptions() {
        assert_eq!(
            strip_html("Agenda:<br>1. Plans &amp; goals<br/><b>Bring</b> laptop"),
            "Agenda:\n1. Plans & goals\nBring laptop"
        );
    }

    #[test]
    fn builds_detail_from_raw_json() {
        let raw = json!({
            "id": "e1", "summary": "Planning", "location": "Room 4", "description": "Bring <i>notes</i>",
            "organizer": {"email": "boss@example.com", "displayName": "Boss"},
            "attendees": [
                {"email": "a@example.com", "responseStatus": "accepted"},
                {"email": "me@example.com", "self": true, "responseStatus": "tentative"}
            ],
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "conferenceData": {"entryPoints": [{"entryPointType": "video", "uri": "https://meet.google.com/abc-defg-hij"}, {"entryPointType": "phone", "uri": "tel:+1-555-0100"}]},
            "reminders": {"useDefault": false, "overrides": [{"method": "popup", "minutes": 10}, {"method": "email", "minutes": 1440}]},
            "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=TU"],
            "x-future": true
        });
        let d = event_detail(
            &raw,
            "Tue 20 Oct, 09:00–10:00".into(),
            "Work".into(),
            "me@example.com".into(),
        );
        assert_eq!(d.title, "Planning");
        assert_eq!(d.organizer.as_deref(), Some("Boss"));
        assert_eq!(
            d.attendees,
            [
                ("a@example.com".to_owned(), "Going".to_owned()),
                ("me@example.com".to_owned(), "Maybe".to_owned())
            ]
        );
        assert_eq!(
            d.conference,
            ["https://meet.google.com/abc-defg-hij", "tel:+1-555-0100"]
        );
        assert_eq!(
            d.reminders,
            ["Notification 10 minutes before", "Email 1 day before"]
        );
        assert_eq!(d.recurrence, ["Weekly on Tue"]);
        assert_eq!(d.description.as_deref(), Some("Bring notes"));
        assert!(d.raw_json.contains("x-future"));
    }

    #[test]
    fn default_reminders_and_missing_fields() {
        let d = event_detail(
            &json!({"id": "e", "reminders": {"useDefault": true}}),
            String::new(),
            String::new(),
            String::new(),
        );
        assert_eq!(d.title, "(No title)");
        assert_eq!(d.reminders, ["Calendar's default reminders"]);
        assert!(d.attendees.is_empty() && d.conference.is_empty() && d.description.is_none());
    }
}
