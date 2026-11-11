//! Turns store rows into what the day, week and month views draw. Pure: no
//! GPUI types.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, TimeZone as _, Timelike as _, Utc};
use chrono_tz::Tz;
use takt_store::{Occurrence, display_instant};

use crate::agenda::{
    RowKey, Styling, day_span, event_key, format_time, label_for, short_date, title_of,
};
use crate::layout::{BarSpan, Slot, Span, pack_columns, pack_lanes};
use crate::view::zone_label;

pub const DAY_MINUTES: u32 = 24 * 60;
/// A block is drawn at least this tall, so a short event keeps a readable line.
pub const MIN_BLOCK_MINUTES: u32 = 20;

pub struct GridInput<'a> {
    pub occurrences: &'a [Occurrence],
    pub styling: &'a Styling<'a>,
    pub tz: Tz,
    pub today: NaiveDate,
    pub from: NaiveDate,
    /// Exclusive.
    pub to: NaiveDate,
    pub use_24h: bool,
}

/// What every drawn event carries, whatever its shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub key: RowKey,
    pub title: String,
    /// The time as shown on this piece, e.g. "09:00–09:30" or "Until 01:00".
    pub time_label: String,
    /// The start time alone, for tight spaces. Empty for all-day events.
    pub start_label: String,
    /// The whole event's time, for the detail panel.
    pub when: String,
    pub location: Option<String>,
    pub color: u32,
    pub account_badge: Option<String>,
    pub declined: bool,
    pub warning: bool,
}

/// A timed event's part on one day of the time grid.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedBlock {
    pub chip: Chip,
    /// Minutes from local midnight, drawn extent: at least
    /// `MIN_BLOCK_MINUTES` long and inside the day.
    pub top: u32,
    pub bottom: u32,
    pub slot: Slot,
}

/// An event drawn across the day columns `first..first + len` of a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub chip: Chip,
    pub first: usize,
    pub len: usize,
    pub lane: usize,
    /// The event began before this row's first day.
    pub starts_before: bool,
    /// The event goes on past this row's last day.
    pub ends_after: bool,
    /// A timed event within one day: the month view draws it as a line of
    /// text, not a filled bar.
    pub timed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GridDay {
    pub date: NaiveDate,
    pub blocks: Vec<TimedBlock>,
}

/// A second zone's hour labels beside the main ones.
#[derive(Debug, Clone, PartialEq)]
pub struct SecondaryZone {
    pub label: String,
    /// One per hour of the first shown day; empty where that local hour does
    /// not exist.
    pub hours: Vec<String>,
}

/// The day and week views: an all-day strip over hour-by-hour columns.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimeGrid {
    pub days: Vec<GridDay>,
    pub bars: Vec<Bar>,
    /// Lanes the all-day strip needs.
    pub lanes: usize,
    pub zone_label: String,
    /// 24 labels, midnight first.
    pub hours: Vec<String>,
    pub secondary: Option<SecondaryZone>,
    /// Every event once, in reading order, with the day it first shows on.
    pub order: Vec<(NaiveDate, RowKey)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonthWeek {
    pub days: Vec<NaiveDate>,
    pub bars: Vec<Bar>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonthGrid {
    pub weeks: Vec<MonthWeek>,
    pub order: Vec<(NaiveDate, RowKey)>,
}

/// An occurrence with the shown days it touches.
struct Placed<'a> {
    o: &'a Occurrence,
    /// First and last local day, unclipped.
    first: NaiveDate,
    last: NaiveDate,
    start: DateTime<Tz>,
    end: DateTime<Tz>,
    /// `start` again, for sorting.
    instant: DateTime<Utc>,
    /// Drawn in the all-day strip of a time grid: an all-day event, or a
    /// timed one of 24 hours or more.
    strip: bool,
}

fn place<'a>(input: &GridInput<'a>) -> Vec<Placed<'a>> {
    let last_shown = input.to - TimeDelta::days(1);
    input
        .occurrences
        .iter()
        .filter_map(|o| {
            let (first, last) = day_span(&o.start, &o.end, input.tz);
            if first > last_shown || last < input.from {
                return None;
            }
            let instant = display_instant(&o.start, input.tz);
            let start = instant.with_timezone(&input.tz);
            let end = display_instant(&o.end, input.tz).with_timezone(&input.tz);
            let strip = o.start.is_all_day() || end - start >= TimeDelta::hours(24);
            Some(Placed {
                o,
                first,
                last,
                start,
                end,
                instant,
                strip,
            })
        })
        .collect()
}

/// "Tue 20 Oct, 09:00–10:00", "Tue 20 Oct 22:00 – Wed 21 Oct 01:00",
/// "Tue 20 Oct, all day" or "Tue 20 Oct – Thu 22 Oct, all day".
fn when_text(p: &Placed, input: &GridInput) -> String {
    let date = |d: NaiveDate| short_date(d, input.today);
    let time = |t: DateTime<Tz>| format_time(t, input.use_24h);
    match (p.o.start.is_all_day(), p.first == p.last) {
        (true, true) => format!("{}, all day", date(p.first)),
        (true, false) => format!("{} – {}, all day", date(p.first), date(p.last)),
        (false, true) => format!("{}, {}–{}", date(p.first), time(p.start), time(p.end)),
        (false, false) => format!(
            "{} {} – {} {}",
            date(p.first),
            time(p.start),
            date(p.end.date_naive()),
            time(p.end)
        ),
    }
}

/// The chip for the piece of `p` shown on `date`.
fn chip(p: &Placed, date: NaiveDate, input: &GridInput) -> Chip {
    let (_, time_label) = label_for(p.o, date, p.first, p.last, input.tz, input.use_24h);
    Chip {
        key: event_key(p.o),
        title: title_of(p.o),
        time_label,
        start_label: if p.o.start.is_all_day() {
            String::new()
        } else {
            format_time(p.start, input.use_24h)
        },
        when: when_text(p, input),
        location: p.o.location.clone(),
        color: input.styling.color(p.o),
        account_badge: input.styling.badge(&p.o.account_id),
        declined: p.o.self_response.as_deref() == Some("declined"),
        warning: p.o.recurrence_error,
    }
}

fn minutes(t: NaiveTime) -> u32 {
    t.hour() * 60 + t.minute()
}

/// The bars of one row of days `row_from..row_from + columns`, with lanes.
/// Bars and whole-day pieces go above single-day timed events, longer ones
/// first.
fn row_bars(
    placed: &[&Placed],
    row_from: NaiveDate,
    columns: usize,
    input: &GridInput,
) -> Vec<Bar> {
    let row_last = row_from + TimeDelta::days(columns as i64 - 1);
    let mut bars: Vec<(Bar, DateTime<Utc>)> = placed
        .iter()
        .filter(|p| p.first <= row_last && p.last >= row_from)
        .map(|p| {
            let from = p.first.max(row_from);
            let to = p.last.min(row_last);
            let bar = Bar {
                chip: chip(p, from, input),
                first: (from - row_from).num_days() as usize,
                len: (to - from).num_days() as usize + 1,
                lane: 0,
                starts_before: p.first < row_from,
                ends_after: p.last > row_last,
                timed: !p.o.start.is_all_day() && p.first == p.last,
            };
            (bar, p.instant)
        })
        .collect();
    bars.sort_by(|(a, a_start), (b, b_start)| {
        (
            a.first,
            a.timed,
            std::cmp::Reverse(a.len),
            a_start,
            &a.chip.title,
        )
            .cmp(&(
                b.first,
                b.timed,
                std::cmp::Reverse(b.len),
                b_start,
                &b.chip.title,
            ))
    });
    let spans: Vec<BarSpan> = bars
        .iter()
        .map(|(b, _)| BarSpan {
            first: b.first,
            len: b.len,
        })
        .collect();
    let lanes = pack_lanes(&spans, columns);
    bars.into_iter()
        .zip(lanes)
        .map(|((mut bar, _), lane)| {
            bar.lane = lane;
            bar
        })
        .collect()
}

/// Every event once, by the first shown day it touches, strip events before
/// timed ones, then by start and title.
fn reading_order(placed: &[Placed], from: NaiveDate) -> Vec<(NaiveDate, RowKey)> {
    let mut order: Vec<_> = placed
        .iter()
        .map(|p| {
            (
                p.first.max(from),
                !p.strip,
                p.instant,
                title_of(p.o),
                event_key(p.o),
            )
        })
        .collect();
    order.sort_by(|a, b| (a.0, a.1, a.2, &a.3).cmp(&(b.0, b.1, b.2, &b.3)));
    order.into_iter().map(|o| (o.0, o.4)).collect()
}

fn hour_label(hour: u32, use_24h: bool) -> String {
    if use_24h {
        format!("{hour:02}:00")
    } else {
        let (h, half) = match hour {
            0 => (12, "AM"),
            1..=11 => (hour, "AM"),
            12 => (12, "PM"),
            _ => (hour - 12, "PM"),
        };
        format!("{h} {half}")
    }
}

/// Noon on the first shown day: the instant zone labels are worked out for.
fn noon(input: &GridInput) -> DateTime<Utc> {
    input
        .tz
        .from_local_datetime(&input.from.and_hms_opt(12, 0, 0).unwrap_or_default())
        .earliest()
        .map_or_else(Utc::now, |t| t.with_timezone(&Utc))
}

fn secondary_zone(input: &GridInput, zone: Tz) -> SecondaryZone {
    SecondaryZone {
        label: zone_label(zone, noon(input)),
        hours: (0..24)
            .map(|hour| {
                input
                    .from
                    .and_hms_opt(hour, 0, 0)
                    .and_then(|local| input.tz.from_local_datetime(&local).earliest())
                    .map(|t| {
                        let there = t.with_timezone(&zone);
                        if there.minute() == 0 {
                            hour_label(there.hour(), input.use_24h)
                        } else {
                            format_time(there, input.use_24h)
                        }
                    })
                    .unwrap_or_default()
            })
            .collect(),
    }
}

pub fn build_time_grid(input: &GridInput, secondary: Option<Tz>) -> TimeGrid {
    let placed = place(input);
    let columns = (input.to - input.from).num_days().max(0) as usize;
    let strip: Vec<&Placed> = placed.iter().filter(|p| p.strip).collect();
    let bars = row_bars(&strip, input.from, columns, input);
    let lanes = bars.iter().map(|b| b.lane + 1).max().unwrap_or(0);

    let days = input
        .from
        .iter_days()
        .take(columns)
        .map(|date| {
            let pieces: Vec<(&Placed, u32, u32)> = placed
                .iter()
                .filter(|p| !p.strip && p.first <= date && p.last >= date)
                .map(|p| {
                    let start = if p.start.date_naive() == date {
                        minutes(p.start.time())
                    } else {
                        0
                    };
                    let end = if p.end.date_naive() == date {
                        minutes(p.end.time())
                    } else {
                        DAY_MINUTES
                    };
                    // Tall enough to read, without leaving the day.
                    let top = start.min(DAY_MINUTES - MIN_BLOCK_MINUTES);
                    let bottom = end.max(top + MIN_BLOCK_MINUTES).min(DAY_MINUTES);
                    (p, top, bottom)
                })
                .collect();
            let spans: Vec<Span> = pieces
                .iter()
                .map(|(_, top, bottom)| Span {
                    start: *top,
                    end: *bottom,
                })
                .collect();
            let blocks = pieces
                .into_iter()
                .zip(pack_columns(&spans))
                .map(|((p, top, bottom), slot)| TimedBlock {
                    chip: chip(p, date, input),
                    top,
                    bottom,
                    slot,
                })
                .collect();
            GridDay { date, blocks }
        })
        .collect();

    TimeGrid {
        days,
        bars,
        lanes,
        zone_label: zone_label(input.tz, noon(input)),
        hours: (0..24).map(|h| hour_label(h, input.use_24h)).collect(),
        secondary: secondary
            .filter(|zone| *zone != input.tz)
            .map(|zone| secondary_zone(input, zone)),
        order: reading_order(&placed, input.from),
    }
}

pub fn build_month_grid(input: &GridInput) -> MonthGrid {
    let placed = place(input);
    let all: Vec<&Placed> = placed.iter().collect();
    let weeks = input
        .from
        .iter_days()
        .step_by(7)
        .take_while(|week| *week < input.to)
        .map(|week| MonthWeek {
            days: week.iter_days().take(7).collect(),
            bars: row_bars(&all, week, 7, input),
        })
        .collect();
    MonthGrid {
        weeks,
        order: reading_order(&placed, input.from),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{DateTime, NaiveDate, Utc};
    use takt_store::{AccountRow, AccountStatus, CalendarRow, ColorPalette, When};

    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn at(s: &str) -> When {
        When::At(DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
    }

    fn occ(id: &str, start: When, end: When) -> Occurrence {
        Occurrence {
            account_id: "a".into(),
            calendar_id: "cal".into(),
            event_id: id.into(),
            instance_id: id.into(),
            start,
            end,
            summary: Some(id.into()),
            location: None,
            color_id: None,
            self_response: None,
            recurrence_error: false,
        }
    }

    struct Fixture {
        occurrences: Vec<Occurrence>,
        accounts: Vec<AccountRow>,
        calendars: Vec<CalendarRow>,
        palettes: HashMap<String, ColorPalette>,
        tz: Tz,
        from: NaiveDate,
        to: NaiveDate,
        use_24h: bool,
    }

    impl Fixture {
        /// The Monday week of 19 October 2026, in UTC.
        fn week(occurrences: Vec<Occurrence>) -> Self {
            Self {
                occurrences,
                accounts: vec![AccountRow {
                    account_id: "a".into(),
                    email: "a@example.com".into(),
                    status: AccountStatus::Ok,
                }],
                calendars: vec![CalendarRow {
                    account_id: "a".into(),
                    calendar_id: "cal".into(),
                    summary: "Cal".into(),
                    time_zone: None,
                    background_color: Some("#9fe1e7".into()),
                    foreground_color: None,
                    access_role: Some("owner".into()),
                    selected: true,
                    hidden: false,
                    primary: true,
                    visible: true,
                }],
                palettes: HashMap::new(),
                tz: Tz::UTC,
                from: d(19),
                to: d(26),
                use_24h: true,
            }
        }

        fn with<T>(&self, f: impl FnOnce(&GridInput) -> T) -> T {
            let styling = Styling::new(&self.accounts, &self.calendars, &self.palettes);
            f(&GridInput {
                occurrences: &self.occurrences,
                styling: &styling,
                tz: self.tz,
                today: d(17),
                from: self.from,
                to: self.to,
                use_24h: self.use_24h,
            })
        }

        fn time_grid(&self) -> TimeGrid {
            self.with(|input| build_time_grid(input, None))
        }

        fn month_grid(&self) -> MonthGrid {
            self.with(build_month_grid)
        }
    }

    fn block_titles(day: &GridDay) -> Vec<&str> {
        day.blocks.iter().map(|b| b.chip.title.as_str()).collect()
    }

    #[test]
    fn timed_events_land_on_their_day_at_their_minutes() {
        let grid = Fixture::week(vec![occ(
            "standup",
            at("2026-10-20T09:00:00Z"),
            at("2026-10-20T09:30:00Z"),
        )])
        .time_grid();
        assert_eq!(grid.days.len(), 7);
        assert_eq!(grid.days[0].date, d(19));
        assert!(grid.days[0].blocks.is_empty());
        let block = &grid.days[1].blocks[0];
        assert_eq!((block.top, block.bottom), (540, 570));
        assert_eq!(block.chip.time_label, "09:00–09:30");
        assert_eq!(block.chip.start_label, "09:00");
        assert_eq!(block.chip.when, "Tue 20 Oct, 09:00–09:30");
        assert_eq!(block.chip.color, 0x9fe1e7);
        assert!(grid.bars.is_empty());
        assert_eq!(grid.lanes, 0);
        assert_eq!(grid.hours[0], "00:00");
        assert_eq!(grid.hours[13], "13:00");
        assert_eq!(grid.zone_label, "GMT");
    }

    #[test]
    fn times_follow_the_display_zone() {
        let mut f = Fixture::week(vec![occ(
            "call",
            at("2026-10-20T23:30:00Z"),
            at("2026-10-21T00:30:00Z"),
        )]);
        f.tz = Tz::Asia__Kolkata;
        let grid = f.time_grid();
        // 05:00–06:00 on the 21st in India.
        assert!(grid.days[1].blocks.is_empty());
        let block = &grid.days[2].blocks[0];
        assert_eq!((block.top, block.bottom), (300, 360));
        assert_eq!(grid.zone_label, "GMT+5:30");
    }

    #[test]
    fn an_overnight_event_is_split_at_midnight() {
        let grid = Fixture::week(vec![occ(
            "night",
            at("2026-10-20T22:00:00Z"),
            at("2026-10-21T01:00:00Z"),
        )])
        .time_grid();
        let first = &grid.days[1].blocks[0];
        let second = &grid.days[2].blocks[0];
        assert_eq!((first.top, first.bottom), (1320, 1440));
        assert_eq!((second.top, second.bottom), (0, 60));
        assert_eq!(first.chip.time_label, "From 22:00");
        assert_eq!(second.chip.time_label, "Until 01:00");
        assert_eq!(first.chip.when, "Tue 20 Oct 22:00 – Wed 21 Oct 01:00");
        // One event, listed once.
        assert_eq!(grid.order.len(), 1);
        assert_eq!(grid.order[0].0, d(20));
    }

    #[test]
    fn an_event_ending_at_midnight_fills_to_the_end_of_its_day() {
        let grid = Fixture::week(vec![occ(
            "late",
            at("2026-10-20T20:00:00Z"),
            at("2026-10-21T00:00:00Z"),
        )])
        .time_grid();
        let block = &grid.days[1].blocks[0];
        assert_eq!((block.top, block.bottom), (1200, 1440));
        assert!(grid.days[2].blocks.is_empty());
    }

    #[test]
    fn short_and_late_events_keep_a_readable_height() {
        let grid = Fixture::week(vec![
            occ(
                "blip",
                at("2026-10-20T09:00:00Z"),
                at("2026-10-20T09:00:00Z"),
            ),
            occ(
                "last",
                at("2026-10-20T23:55:00Z"),
                at("2026-10-20T23:59:00Z"),
            ),
        ])
        .time_grid();
        let blocks = &grid.days[1].blocks;
        assert_eq!((blocks[0].top, blocks[0].bottom), (540, 560));
        assert_eq!((blocks[1].top, blocks[1].bottom), (1420, 1440));
    }

    #[test]
    fn overlapping_events_share_the_day_column() {
        let grid = Fixture::week(vec![
            occ("a", at("2026-10-20T09:00:00Z"), at("2026-10-20T10:00:00Z")),
            occ("b", at("2026-10-20T09:30:00Z"), at("2026-10-20T10:30:00Z")),
            occ("c", at("2026-10-20T11:00:00Z"), at("2026-10-20T12:00:00Z")),
        ])
        .time_grid();
        let slots: Vec<_> = grid.days[1]
            .blocks
            .iter()
            .map(|b| (b.chip.title.as_str(), b.slot.column, b.slot.columns))
            .collect();
        assert_eq!(slots, [("a", 0, 2), ("b", 1, 2), ("c", 0, 1)]);
    }

    #[test]
    fn all_day_and_day_long_events_go_to_the_strip() {
        let grid = Fixture::week(vec![
            occ("holiday", When::Date(d(20)), When::Date(d(21))),
            occ("trip", When::Date(d(21)), When::Date(d(24))),
            occ(
                "conference",
                at("2026-10-20T10:00:00Z"),
                at("2026-10-22T15:00:00Z"),
            ),
        ])
        .time_grid();
        assert!(grid.days.iter().all(|day| day.blocks.is_empty()));
        let bars: Vec<_> = grid
            .bars
            .iter()
            .map(|b| (b.chip.title.as_str(), b.first, b.len, b.lane))
            .collect();
        // Tuesday: the longer conference sits above the one-day holiday; the
        // trip starts Wednesday under the conference, where the holiday ended.
        assert_eq!(
            bars,
            [
                ("conference", 1, 3, 0),
                ("holiday", 1, 1, 1),
                ("trip", 2, 3, 1)
            ]
        );
        assert_eq!(grid.lanes, 2);
        assert_eq!(grid.bars[1].chip.when, "Tue 20 Oct, all day");
        assert_eq!(grid.bars[2].chip.when, "Wed 21 Oct – Fri 23 Oct, all day");
        assert_eq!(grid.bars[1].chip.start_label, "");
    }

    #[test]
    fn bars_are_clipped_to_the_shown_days_and_say_so() {
        let grid = Fixture::week(vec![occ(
            "sabbatical",
            When::Date(d(10)),
            When::Date(d(30)),
        )])
        .time_grid();
        let bar = &grid.bars[0];
        assert_eq!((bar.first, bar.len), (0, 7));
        assert!(bar.starts_before && bar.ends_after);
        assert_eq!(grid.order[0].0, d(19));
    }

    #[test]
    fn day_view_is_a_one_column_grid() {
        let mut f = Fixture::week(vec![
            occ("in", at("2026-10-20T09:00:00Z"), at("2026-10-20T10:00:00Z")),
            occ(
                "out",
                at("2026-10-21T09:00:00Z"),
                at("2026-10-21T10:00:00Z"),
            ),
            occ("trip", When::Date(d(19)), When::Date(d(23))),
        ]);
        f.from = d(20);
        f.to = d(21);
        let grid = f.time_grid();
        assert_eq!(grid.days.len(), 1);
        assert_eq!(block_titles(&grid.days[0]), ["in"]);
        assert_eq!((grid.bars[0].first, grid.bars[0].len), (0, 1));
        assert_eq!(grid.order.len(), 2);
    }

    #[test]
    fn reading_order_is_by_day_then_strip_then_time() {
        let grid = Fixture::week(vec![
            occ(
                "late",
                at("2026-10-20T15:00:00Z"),
                at("2026-10-20T16:00:00Z"),
            ),
            occ(
                "early",
                at("2026-10-20T09:00:00Z"),
                at("2026-10-20T10:00:00Z"),
            ),
            occ("holiday", When::Date(d(20)), When::Date(d(21))),
            occ(
                "monday",
                at("2026-10-19T18:00:00Z"),
                at("2026-10-19T19:00:00Z"),
            ),
        ])
        .time_grid();
        let ids: Vec<_> = grid
            .order
            .iter()
            .map(|(_, key)| match key {
                RowKey::Event { event_id, .. } => event_id.as_str(),
                RowKey::Task { .. } => "task",
            })
            .collect();
        assert_eq!(ids, ["monday", "holiday", "early", "late"]);
    }

    #[test]
    fn twelve_hour_labels() {
        let mut f = Fixture::week(vec![occ(
            "lunch",
            at("2026-10-20T12:00:00Z"),
            at("2026-10-20T13:15:00Z"),
        )]);
        f.use_24h = false;
        let grid = f.time_grid();
        assert_eq!(grid.hours[0], "12 AM");
        assert_eq!(grid.hours[9], "9 AM");
        assert_eq!(grid.hours[12], "12 PM");
        assert_eq!(grid.hours[23], "11 PM");
        assert_eq!(grid.days[1].blocks[0].chip.time_label, "12:00 PM–1:15 PM");
    }

    #[test]
    fn a_second_zone_gets_its_own_hour_labels() {
        let mut f = Fixture::week(vec![]);
        f.tz = Tz::Europe__Berlin;
        let grid = f.with(|input| build_time_grid(input, Some(Tz::Asia__Kolkata)));
        let zone = grid.secondary.unwrap();
        // Berlin is on GMT+2 that week; India is 3 h 30 min ahead of it.
        assert_eq!(grid.zone_label, "GMT+2");
        assert_eq!(zone.label, "GMT+5:30");
        assert_eq!(zone.hours.len(), 24);
        assert_eq!(zone.hours[0], "03:30");
        assert_eq!(zone.hours[9], "12:30");

        let same = f.with(|input| build_time_grid(input, Some(Tz::Europe__Berlin)));
        assert!(same.secondary.is_none(), "the same zone is not shown twice");

        let whole = f.with(|input| build_time_grid(input, Some(Tz::America__New_York)));
        assert_eq!(whole.secondary.unwrap().hours[9], "03:00");
    }

    #[test]
    fn month_rows_hold_bars_and_timed_lines() {
        let mut f = Fixture::week(vec![
            occ("trip", When::Date(d(23)), When::Date(d(28))),
            occ(
                "standup",
                at("2026-10-23T09:00:00Z"),
                at("2026-10-23T09:30:00Z"),
            ),
            occ(
                "night",
                at("2026-10-23T22:00:00Z"),
                at("2026-10-24T01:00:00Z"),
            ),
        ]);
        // October 2026 as Monday weeks.
        f.from = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        f.to = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let grid = f.month_grid();
        assert_eq!(grid.weeks.len(), 5);
        assert_eq!(grid.weeks[0].days[0], f.from);
        assert_eq!(grid.weeks[3].days[0], d(19));

        // Week of the 19th: Friday the 23rd is column 4.
        let row: Vec<_> = grid.weeks[3]
            .bars
            .iter()
            .map(|b| (b.chip.title.as_str(), b.first, b.len, b.lane, b.timed))
            .collect();
        assert_eq!(
            row,
            [
                ("trip", 4, 3, 0, false),
                ("night", 4, 2, 1, false),
                ("standup", 4, 1, 2, true),
            ]
        );
        assert!(grid.weeks[3].bars[0].ends_after);
        assert!(!grid.weeks[3].bars[0].starts_before);

        // The trip carries on into the next row, Monday and Tuesday.
        let next = &grid.weeks[4].bars[0];
        assert_eq!(
            (next.chip.title.as_str(), next.first, next.len),
            ("trip", 0, 2)
        );
        assert!(next.starts_before && !next.ends_after);
        assert_eq!(grid.order.len(), 3);
    }
}
