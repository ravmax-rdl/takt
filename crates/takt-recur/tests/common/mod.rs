#![allow(dead_code)]

use chrono::{DateTime, NaiveDate, Utc};
use takt_recur::{Range, When};

pub fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

pub fn at(s: &str) -> When {
    When::At(utc(s))
}

pub fn day(s: &str) -> When {
    When::Date(NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap())
}

pub fn range(start: &str, end: &str) -> Range {
    Range {
        start: utc(start),
        end: utc(end),
    }
}
