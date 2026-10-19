//! The read-only detail panel's content, built from an event's raw JSON.

use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
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
    let zone = raw["start"]["timeZone"].as_str();
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
                    .map(|line| describe_recurrence_line_in(line, zone))
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
    let list: Vec<String> = value["overrides"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|r| {
                    let method = if r["method"] == "email" {
                        "Email"
                    } else {
                        "Notification"
                    };
                    Some(match r["minutes"].as_u64()? {
                        0 if method == "Email" => "Email at start time".into(),
                        0 => "At start time".into(),
                        m => format!("{method} {} before", duration(m)),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if list.is_empty() && value["useDefault"].as_bool() == Some(false) {
        vec!["None".into()]
    } else {
        list
    }
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
    describe_recurrence_line_in(line, None)
}

/// Like [`describe_recurrence_line`], showing a UTC `UNTIL` as a date in `zone` (an IANA name).
fn describe_recurrence_line_in(line: &str, zone: Option<&str>) -> String {
    let Some((head, value)) = line.split_once(':') else {
        return line.to_owned();
    };
    let name = head
        .split(';')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let n = value.split(',').count();
    if value.split(',').any(|d| d.trim().is_empty()) {
        return line.to_owned();
    }
    match name.as_str() {
        "EXDATE" if n == 1 => "Except 1 date".into(),
        "EXDATE" => format!("Except {n} dates"),
        "RDATE" if n == 1 => "Also 1 extra date".into(),
        "RDATE" => format!("Also {n} extra dates"),
        "RRULE" => describe_rule(value, zone).unwrap_or_else(|| line.to_owned()),
        _ => line.to_owned(),
    }
}

const RULE_KEYS: [&str; 7] = [
    "FREQ",
    "INTERVAL",
    "BYDAY",
    "BYMONTHDAY",
    "COUNT",
    "UNTIL",
    "WKST",
];

fn describe_rule(rule: &str, zone: Option<&str>) -> Option<String> {
    let mut parts: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for piece in rule.split(';').filter(|p| !p.is_empty()) {
        let (key, value) = piece.split_once('=')?;
        let key = key.to_ascii_uppercase();
        if !RULE_KEYS.contains(&key.as_str()) {
            return None;
        }
        parts.insert(key, value.to_owned());
    }
    if parts.contains_key("BYDAY") && parts.contains_key("BYMONTHDAY") {
        return None;
    }
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
        if names.is_empty() {
            return None;
        }
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
        let count: u32 = count.parse().ok()?;
        text.push_str(&format!(", {count} times"));
    }
    if let Some(until) = parts.get("UNTIL") {
        let date = until_date(until, zone)?;
        text.push_str(&format!(", until {}", date.format("%-d %b %Y")));
    }
    Some(text)
}

/// The calendar date an `UNTIL` value falls on. A UTC value is read in the event's zone.
fn until_date(until: &str, zone: Option<&str>) -> Option<NaiveDate> {
    if let Some(utc) = until.strip_suffix('Z') {
        let t = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        let zone = zone.and_then(|z| z.parse::<chrono_tz::Tz>().ok());
        return Some(match zone {
            Some(tz) => Utc.from_utc_datetime(&t).with_timezone(&tz).date_naive(),
            None => t.date(),
        });
    }
    NaiveDateTime::parse_from_str(until, "%Y%m%dT%H%M%S")
        .map(|t| t.date())
        .or_else(|_| NaiveDate::parse_from_str(until, "%Y%m%d"))
        .ok()
}

fn first_is_ordinal(days: &str) -> bool {
    days.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+')
}

fn describe_byday(token: &str) -> Option<String> {
    let token = token.trim();
    let split = token.len().checked_sub(2)?;
    let ordinal = token.get(..split)?;
    let day = token.get(split..)?;
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
/// show link targets, decode the common entities.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    // Link target and where its text starts in `out`.
    let mut link: Option<(String, usize)> = None;
    while let Some(open) = rest.find('<') {
        out.push_str(&decode_entities(&rest[..open]));
        let after = &rest[open + 1..];
        let starts_tag = after
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!');
        let close = after.find('>').filter(|_| starts_tag);
        let Some(close) = close.filter(|&c| !after[..c].contains('@')) else {
            out.push('<');
            rest = after;
            continue;
        };
        let tag = &after[..close];
        rest = &after[close + 1..];
        let lower = tag.trim().to_ascii_lowercase();
        let name = lower
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default();
        match (lower.starts_with('/'), name) {
            (false, "br") | (true, "p" | "div" | "li") => out.push('\n'),
            (false, "script" | "style") if !lower.ends_with('/') => {
                let end = format!("</{name}");
                rest = match rest.to_ascii_lowercase().find(&end) {
                    Some(at) => rest[at..].find('>').map_or("", |g| &rest[at + g + 1..]),
                    None => "",
                };
            }
            (false, "a") => {
                link = attribute(tag, "href").map(|href| (decode_entities(&href), out.len()));
            }
            (true, "a") => {
                if let Some((href, start)) = link.take()
                    && !href.is_empty()
                    && out[start..].trim() != href
                {
                    out.push_str(&format!(" ({href})"));
                }
            }
            _ => {}
        }
    }
    out.push_str(&decode_entities(rest));
    out.trim().to_owned()
}

/// The value of an HTML attribute inside one tag's text.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        from = start + name.len();
        let boundary = lower[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        let Some(value) = lower[from..]
            .trim_start()
            .strip_prefix('=')
            .filter(|_| boundary)
        else {
            continue;
        };
        // Offset of the value within `tag`; ASCII lowercasing keeps byte offsets.
        let value = tag[tag.len() - value.len()..].trim_start();
        return Some(match value.chars().next()? {
            q @ ('"' | '\'') => value[1..].split(q).next()?.to_owned(),
            _ => value
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()?
                .to_owned(),
        });
    }
    None
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..]
            .find(';')
            .filter(|&end| end <= 10)
            .and_then(|end| Some((decode_entity(&rest[1..1 + end])?, end + 2)));
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "nbsp" => ' ',
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "rsquo" => '\u{2019}',
        "lsquo" => '\u{2018}',
        "rdquo" => '\u{201d}',
        "ldquo" => '\u{201c}',
        "mdash" => '\u{2014}',
        "ndash" => '\u{2013}',
        "hellip" => '\u{2026}',
        _ => return None,
    })
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
    fn odd_rules_fall_back_to_the_raw_line() {
        for line in [
            "RRULE:FREQ=WEEKLY;BYDAY=\u{e9}x",
            "RRULE:FREQ=WEEKLY;BYDAY=",
            "RRULE:",
            "EXDATE:",
            "RRULE:FREQ=WEEKLY;UNTIL=garbage",
            "RRULE:FREQ=WEEKLY;BYDAY=XX",
            "RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=4TH",
            "RRULE:FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU,WE,TH,FR",
            "RRULE:FREQ=WEEKLY;COUNT=many",
            "RRULE:FREQ=MONTHLY;BYDAY=MO;BYMONTHDAY=1",
        ] {
            assert_eq!(describe_recurrence_line(line), line);
        }
    }

    #[test]
    fn until_is_shown_in_the_event_zone() {
        let raw = json!({
            "start": {"dateTime": "2026-10-12T09:00:00-07:00", "timeZone": "America/Los_Angeles"},
            "recurrence": ["RRULE:FREQ=WEEKLY;UNTIL=20261020T065959Z"]
        });
        let d = event_detail(&raw, String::new(), String::new(), String::new());
        assert_eq!(d.recurrence, ["Weekly, until 19 Oct 2026"]);
        assert_eq!(
            describe_recurrence_line("RRULE:FREQ=WEEKLY;UNTIL=20261020T065959Z"),
            "Weekly, until 20 Oct 2026"
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
    fn strip_html_keeps_text_that_is_not_markup() {
        assert_eq!(
            strip_html("John <john@example.com> wrote"),
            "John <john@example.com> wrote"
        );
        assert_eq!(strip_html("x < 5 and y > 3"), "x < 5 and y > 3");
        assert_eq!(strip_html("a <"), "a <");
    }

    #[test]
    fn strip_html_skips_scripts_and_shows_links() {
        assert_eq!(
            strip_html("Hi<script>var a = '<b>';</script><style>p{}</style> there"),
            "Hi there"
        );
        assert_eq!(
            strip_html(r#"See <a href="https://x.test/a?b=1&amp;c=2">the doc</a>."#),
            "See the doc (https://x.test/a?b=1&c=2)."
        );
        assert_eq!(
            strip_html(r#"<a href="https://x.test">https://x.test</a>"#),
            "https://x.test"
        );
    }

    #[test]
    fn strip_html_decodes_entities() {
        assert_eq!(
            strip_html("&#65;&#x42; &rsquo;&mdash;&hellip; &amp;lt; &bogus; &#1114112;"),
            "AB \u{2019}\u{2014}\u{2026} &lt; &bogus; &#1114112;"
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

    #[test]
    fn reminder_edge_cases() {
        let reminders = |r: Value| {
            event_detail(
                &json!({"reminders": r}),
                String::new(),
                String::new(),
                String::new(),
            )
            .reminders
        };
        assert_eq!(reminders(json!({"useDefault": false})), ["None"]);
        assert_eq!(
            reminders(json!({"useDefault": false, "overrides": []})),
            ["None"]
        );
        assert_eq!(
            reminders(
                json!({"useDefault": false, "overrides": [{"method": "popup", "minutes": 0}]})
            ),
            ["At start time"]
        );
    }
}
