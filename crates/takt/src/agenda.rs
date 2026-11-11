//! Turns store rows into the agenda's day sections. Pure: no GPUI types.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeDelta, Utc};
use chrono_tz::Tz;
use takt_store::{
    AccountRow, CalendarRow, ColorPalette, Occurrence, TaskItem, When, day_start_utc,
    display_instant,
};

pub const DEFAULT_COLOR: u32 = 0x039be5;
pub const TASK_COLOR: u32 = 0x4285f4;

pub struct AgendaInput<'a> {
    pub occurrences: &'a [Occurrence],
    pub tasks: &'a [TaskItem],
    pub accounts: &'a [AccountRow],
    pub calendars: &'a [CalendarRow],
    pub palettes: &'a HashMap<String, ColorPalette>,
    pub tz: Tz,
    pub today: NaiveDate,
    pub from: NaiveDate,
    /// Exclusive.
    pub to: NaiveDate,
    pub use_24h: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKey {
    Event {
        account_id: String,
        calendar_id: String,
        event_id: String,
        instance_id: String,
    },
    Task {
        account_id: String,
        list_id: String,
        task_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgendaRow {
    pub key: RowKey,
    pub time_label: String,
    pub title: String,
    /// The event's location, or the task list's title for a task.
    pub location: Option<String>,
    pub color: u32,
    pub account_badge: Option<String>,
    pub declined: bool,
    pub is_task: bool,
    pub completed: bool,
    pub warning: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DaySection {
    pub date: NaiveDate,
    pub header: String,
    pub rows: Vec<AgendaRow>,
}

/// Rows sort by group (all-day events, tasks, timed), then start, then title.
type SortKey = (u8, DateTime<Utc>, String);

pub fn build_agenda(input: &AgendaInput) -> Vec<DaySection> {
    let several_accounts = input.accounts.len() > 1;
    let emails: HashMap<&str, &str> = input
        .accounts
        .iter()
        .map(|a| (a.account_id.as_str(), a.email.as_str()))
        .collect();
    let calendar_colors: HashMap<(&str, &str), u32> = input
        .calendars
        .iter()
        .filter_map(|c| {
            Some((
                (c.account_id.as_str(), c.calendar_id.as_str()),
                parse_hex(c.background_color.as_deref()?)?,
            ))
        })
        .collect();
    let badge = |account_id: &str| {
        several_accounts.then(|| {
            emails
                .get(account_id)
                .copied()
                .unwrap_or(account_id)
                .to_owned()
        })
    };
    let mut days: BTreeMap<NaiveDate, Vec<(SortKey, AgendaRow)>> = BTreeMap::new();

    for o in input.occurrences {
        let color = o
            .color_id
            .as_deref()
            .and_then(|id| input.palettes.get(&o.account_id)?.event.get(id))
            .and_then(|(background, _)| parse_hex(background))
            .or_else(|| {
                calendar_colors
                    .get(&(o.account_id.as_str(), o.calendar_id.as_str()))
                    .copied()
            })
            .unwrap_or(DEFAULT_COLOR);
        let (first, last) = day_span(&o.start, &o.end, input.tz);
        let shown_from = first.max(input.from);
        let shown_last = input.to.pred_opt().map_or(last, |end| last.min(end));
        for date in shown_from.iter_days().take_while(|d| *d <= shown_last) {
            let (group, label) = label_for(o, date, first, last, input.tz, input.use_24h);
            let row = AgendaRow {
                key: RowKey::Event {
                    account_id: o.account_id.clone(),
                    calendar_id: o.calendar_id.clone(),
                    event_id: o.event_id.clone(),
                    instance_id: o.instance_id.clone(),
                },
                time_label: label,
                title: o
                    .summary
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "(No title)".into()),
                location: o.location.clone(),
                color,
                account_badge: badge(&o.account_id),
                declined: o.self_response.as_deref() == Some("declined"),
                is_task: false,
                completed: false,
                warning: o.recurrence_error,
            };
            let key = (
                group,
                display_instant(&o.start, input.tz),
                row.title.clone(),
            );
            days.entry(date).or_default().push((key, row));
        }
    }

    for t in input
        .tasks
        .iter()
        .filter(|t| t.due_date >= input.from && t.due_date < input.to)
    {
        let row = AgendaRow {
            key: RowKey::Task {
                account_id: t.account_id.clone(),
                list_id: t.list_id.clone(),
                task_id: t.task_id.clone(),
            },
            time_label: "All day".into(),
            title: if t.title.trim().is_empty() {
                "(No title)".into()
            } else {
                t.title.clone()
            },
            location: Some(t.list_title.clone()),
            color: TASK_COLOR,
            account_badge: badge(&t.account_id),
            declined: false,
            is_task: true,
            completed: t.completed,
            warning: false,
        };
        let key = (
            1,
            t.due_date.and_time(NaiveTime::MIN).and_utc(),
            row.title.clone(),
        );
        days.entry(t.due_date).or_default().push((key, row));
    }

    days.into_iter()
        .map(|(date, mut rows)| {
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            DaySection {
                date,
                header: header(date, input.today),
                rows: rows.into_iter().map(|(_, r)| r).collect(),
            }
        })
        .collect()
}

/// The first and last local dates an occurrence touches (inclusive). An end
/// at exactly the start of a local day does not count as touching that day;
/// on a day whose midnight does not exist, its start is the first instant
/// that does.
fn day_span(start: &When, end: &When, tz: Tz) -> (NaiveDate, NaiveDate) {
    match (start, end) {
        (When::Date(s), When::Date(e)) => (*s, if e > s { *e - TimeDelta::days(1) } else { *s }),
        _ => {
            let s = display_instant(start, tz).with_timezone(&tz);
            let e = display_instant(end, tz).with_timezone(&tz);
            let mut last = e.date_naive();
            if e > s && e.with_timezone(&Utc) == day_start_utc(last, tz) {
                last -= TimeDelta::days(1);
            }
            (s.date_naive(), last.max(s.date_naive()))
        }
    }
}

fn label_for(
    o: &Occurrence,
    date: NaiveDate,
    first: NaiveDate,
    last: NaiveDate,
    tz: Tz,
    use_24h: bool,
) -> (u8, String) {
    if o.start.is_all_day() {
        return (0, "All day".into());
    }
    let start = display_instant(&o.start, tz).with_timezone(&tz);
    let end = display_instant(&o.end, tz).with_timezone(&tz);
    let time = |t: DateTime<Tz>| {
        if use_24h {
            t.format("%H:%M").to_string()
        } else {
            t.format("%-I:%M %p").to_string()
        }
    };
    match (date == first, date == last) {
        (true, true) => (2, format!("{}–{}", time(start), time(end))),
        (true, false) => (2, format!("From {}", time(start))),
        (false, true) => (2, format!("Until {}", time(end))),
        (false, false) => (0, "All day".into()),
    }
}

fn header(date: NaiveDate, today: NaiveDate) -> String {
    let mut text = date.format("%a %-d %b").to_string();
    if date.year() != today.year() {
        text.push_str(&format!(" {}", date.year()));
    }
    if date == today {
        format!("Today · {text}")
    } else if date == today + TimeDelta::days(1) {
        format!("Tomorrow · {text}")
    } else {
        text
    }
}

pub fn parse_hex(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{DateTime, NaiveDate, Utc};
    use chrono_tz::Tz;
    use takt_store::{
        AccountRow, AccountStatus, CalendarRow, ColorPalette, Occurrence, TaskItem, When,
    };

    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn at(s: &str) -> When {
        When::At(DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
    }

    fn account(id: &str) -> AccountRow {
        AccountRow {
            account_id: id.into(),
            email: format!("{id}@example.com"),
            status: AccountStatus::Ok,
        }
    }

    fn calendar(account: &str, color: &str) -> CalendarRow {
        CalendarRow {
            account_id: account.into(),
            calendar_id: "cal".into(),
            summary: "Cal".into(),
            time_zone: None,
            background_color: Some(color.into()),
            foreground_color: None,
            access_role: Some("owner".into()),
            selected: true,
            hidden: false,
            primary: true,
        }
    }

    fn occ(account: &str, id: &str, title: Option<&str>, start: When, end: When) -> Occurrence {
        Occurrence {
            account_id: account.into(),
            calendar_id: "cal".into(),
            event_id: id.into(),
            instance_id: id.into(),
            start,
            end,
            summary: title.map(str::to_owned),
            location: None,
            color_id: None,
            self_response: None,
            recurrence_error: false,
        }
    }

    struct Fixture {
        occurrences: Vec<Occurrence>,
        tasks: Vec<TaskItem>,
        accounts: Vec<AccountRow>,
        calendars: Vec<CalendarRow>,
        palettes: HashMap<String, ColorPalette>,
        use_24h: bool,
        tz: Tz,
        from: NaiveDate,
        to: NaiveDate,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                occurrences: vec![],
                tasks: vec![],
                accounts: vec![account("a")],
                calendars: vec![calendar("a", "#9fe1e7")],
                palettes: HashMap::new(),
                use_24h: true,
                tz: Tz::UTC,
                from: d(17),
                to: d(31),
            }
        }

        fn build(&self) -> Vec<DaySection> {
            build_agenda(&AgendaInput {
                occurrences: &self.occurrences,
                tasks: &self.tasks,
                accounts: &self.accounts,
                calendars: &self.calendars,
                palettes: &self.palettes,
                tz: self.tz,
                today: d(17),
                from: self.from,
                to: self.to,
                use_24h: self.use_24h,
            })
        }
    }

    fn titles(section: &DaySection) -> Vec<&str> {
        section.rows.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn orders_all_day_events_then_tasks_then_timed() {
        let mut f = Fixture::new();
        f.occurrences = vec![
            occ(
                "a",
                "late",
                Some("Late"),
                at("2026-10-20T15:00:00Z"),
                at("2026-10-20T16:00:00Z"),
            ),
            occ(
                "a",
                "early",
                Some("Early"),
                at("2026-10-20T09:00:00Z"),
                at("2026-10-20T10:00:00Z"),
            ),
            occ(
                "a",
                "ad",
                Some("Holiday"),
                When::Date(d(20)),
                When::Date(d(21)),
            ),
        ];
        f.tasks = vec![TaskItem {
            account_id: "a".into(),
            list_id: "L".into(),
            list_title: "My Tasks".into(),
            task_id: "t".into(),
            title: "Pay rent".into(),
            due_date: d(20),
            completed: false,
        }];
        let sections = f.build();
        assert_eq!(sections.len(), 1);
        assert_eq!(
            titles(&sections[0]),
            ["Holiday", "Pay rent", "Early", "Late"]
        );
        assert_eq!(sections[0].rows[2].time_label, "09:00–10:00");
        assert_eq!(sections[0].rows[1].location.as_deref(), Some("My Tasks"));
        assert!(sections[0].rows[1].is_task);
    }

    #[test]
    fn overnight_event_appears_on_both_days() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ(
            "a",
            "n",
            Some("Night"),
            at("2026-10-20T22:00:00Z"),
            at("2026-10-21T01:00:00Z"),
        )];
        let sections = f.build();
        assert_eq!(
            sections.iter().map(|s| s.date).collect::<Vec<_>>(),
            [d(20), d(21)]
        );
        assert_eq!(sections[0].rows[0].time_label, "From 22:00");
        assert_eq!(sections[1].rows[0].time_label, "Until 01:00");
    }

    #[test]
    fn event_ending_at_midnight_stays_on_one_day() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ(
            "a",
            "e",
            Some("Evening"),
            at("2026-10-20T20:00:00Z"),
            at("2026-10-21T00:00:00Z"),
        )];
        assert_eq!(f.build().len(), 1);
    }

    #[test]
    fn event_ending_at_a_skipped_midnight_stays_on_one_day() {
        // America/Santiago skips 2026-09-06 00:00; that day starts at 01:00
        // (-03). 22:00 (-04) on the 5th to 01:00 on the 6th ends exactly at
        // the start of the 6th.
        let mut f = Fixture::new();
        f.tz = "America/Santiago".parse().unwrap();
        f.from = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        f.to = NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
        f.occurrences = vec![occ(
            "a",
            "s",
            Some("Late"),
            at("2026-09-06T02:00:00Z"),
            at("2026-09-06T04:00:00Z"),
        )];
        let sections = f.build();
        assert_eq!(
            sections.iter().map(|s| s.date).collect::<Vec<_>>(),
            [NaiveDate::from_ymd_opt(2026, 9, 5).unwrap()]
        );
        assert_eq!(sections[0].rows[0].time_label, "22:00–01:00");
    }

    #[test]
    fn three_day_timed_event_is_all_day_in_the_middle() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ(
            "a",
            "c",
            Some("Conference"),
            at("2026-10-20T10:00:00Z"),
            at("2026-10-22T15:00:00Z"),
        )];
        let sections = f.build();
        assert_eq!(
            sections.iter().map(|s| s.date).collect::<Vec<_>>(),
            [d(20), d(21), d(22)]
        );
        let labels: Vec<_> = sections
            .iter()
            .map(|s| s.rows[0].time_label.as_str())
            .collect();
        assert_eq!(labels, ["From 10:00", "All day", "Until 15:00"]);
    }

    #[test]
    fn event_started_before_the_window_shows_only_its_days_inside() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ(
            "a",
            "o",
            Some("Ongoing"),
            at("2026-10-10T20:00:00Z"),
            at("2026-10-18T02:00:00Z"),
        )];
        let sections = f.build();
        assert_eq!(
            sections.iter().map(|s| s.date).collect::<Vec<_>>(),
            [d(17), d(18)]
        );
        assert_eq!(sections[0].rows[0].time_label, "All day");
        assert_eq!(sections[1].rows[0].time_label, "Until 02:00");
    }

    #[test]
    fn multi_day_all_day_event_repeats_each_day() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ(
            "a",
            "trip",
            Some("Trip"),
            When::Date(d(20)),
            When::Date(d(23)),
        )];
        let sections = f.build();
        assert_eq!(
            sections.iter().map(|s| s.date).collect::<Vec<_>>(),
            [d(20), d(21), d(22)]
        );
        assert!(sections.iter().all(|s| s.rows[0].time_label == "All day"));
    }

    #[test]
    fn event_colour_beats_calendar_colour() {
        let mut f = Fixture::new();
        let mut palette = ColorPalette::default();
        palette
            .event
            .insert("11".into(), ("#dc2127".into(), "#1d1d1d".into()));
        f.palettes.insert("a".into(), palette);
        let mut coloured = occ(
            "a",
            "c",
            Some("Coloured"),
            at("2026-10-20T09:00:00Z"),
            at("2026-10-20T10:00:00Z"),
        );
        coloured.color_id = Some("11".into());
        f.occurrences = vec![
            coloured,
            occ(
                "a",
                "p",
                Some("Plain"),
                at("2026-10-20T11:00:00Z"),
                at("2026-10-20T12:00:00Z"),
            ),
        ];
        let rows = &f.build()[0].rows;
        assert_eq!(rows[0].color, 0xdc2127);
        assert_eq!(rows[1].color, 0x9fe1e7);
    }

    #[test]
    fn badges_appear_only_with_several_accounts_and_duplicates_stay_separate() {
        let mut f = Fixture::new();
        f.occurrences = vec![occ(
            "a",
            "x",
            Some("Shared"),
            at("2026-10-20T09:00:00Z"),
            at("2026-10-20T10:00:00Z"),
        )];
        assert_eq!(f.build()[0].rows[0].account_badge, None);
        f.accounts.push(account("b"));
        f.calendars.push(calendar("b", "#ffad46"));
        f.occurrences.push(occ(
            "b",
            "x",
            Some("Shared"),
            at("2026-10-20T09:00:00Z"),
            at("2026-10-20T10:00:00Z"),
        ));
        let rows = &f.build()[0].rows;
        assert_eq!(rows.len(), 2);
        let badges: Vec<_> = rows
            .iter()
            .map(|r| r.account_badge.clone().unwrap())
            .collect();
        assert!(
            badges.contains(&"a@example.com".to_owned())
                && badges.contains(&"b@example.com".to_owned())
        );
    }

    #[test]
    fn declined_and_untitled_events() {
        let mut f = Fixture::new();
        let mut declined = occ(
            "a",
            "d",
            None,
            at("2026-10-20T09:00:00Z"),
            at("2026-10-20T10:00:00Z"),
        );
        declined.self_response = Some("declined".into());
        f.occurrences = vec![declined];
        let row = &f.build()[0].rows[0];
        assert!(row.declined);
        assert_eq!(row.title, "(No title)");
    }

    #[test]
    fn twelve_hour_labels() {
        let mut f = Fixture::new();
        f.use_24h = false;
        f.occurrences = vec![occ(
            "a",
            "m",
            Some("M"),
            at("2026-10-20T09:00:00Z"),
            at("2026-10-20T13:30:00Z"),
        )];
        assert_eq!(f.build()[0].rows[0].time_label, "9:00 AM–1:30 PM");
    }

    #[test]
    fn headers_name_today_and_tomorrow_and_skip_empty_days() {
        let mut f = Fixture::new();
        f.occurrences = vec![
            occ(
                "a",
                "1",
                Some("A"),
                at("2026-10-17T09:00:00Z"),
                at("2026-10-17T10:00:00Z"),
            ),
            occ(
                "a",
                "2",
                Some("B"),
                at("2026-10-18T09:00:00Z"),
                at("2026-10-18T10:00:00Z"),
            ),
            occ(
                "a",
                "3",
                Some("C"),
                at("2026-10-22T09:00:00Z"),
                at("2026-10-22T10:00:00Z"),
            ),
        ];
        let headers: Vec<_> = f.build().into_iter().map(|s| s.header).collect();
        assert_eq!(
            headers,
            ["Today · Sat 17 Oct", "Tomorrow · Sun 18 Oct", "Thu 22 Oct"]
        );
    }

    #[test]
    fn hex_colours_parse() {
        assert_eq!(parse_hex("#a4bdfc"), Some(0xa4bdfc));
        assert_eq!(parse_hex("a4bdfc"), Some(0xa4bdfc));
        assert_eq!(parse_hex("#zzz"), None);
    }
}
