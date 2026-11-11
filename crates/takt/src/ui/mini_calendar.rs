//! The small month in the sidebar: shows where the main view is and jumps
//! to a clicked day.

use chrono::{Datelike as _, Months, NaiveDate, TimeDelta, Weekday};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::grid_common::date_id;
use crate::ui::main_view::MainView;
use crate::view::week_start_of;

pub struct MiniCalendar {
    /// The first day of the month on show.
    pub month: NaiveDate,
    pub week_start: Weekday,
    pub today: NaiveDate,
    /// The main view's selected day.
    pub anchor: NaiveDate,
    /// The days the main view shows, `from..to`.
    pub shown: (NaiveDate, NaiveDate),
}

/// The six weeks a mini calendar draws for `month`, so its height never changes.
pub fn mini_days(month: NaiveDate, week_start: Weekday) -> Vec<NaiveDate> {
    week_start_of(month, week_start)
        .iter_days()
        .take(42)
        .collect()
}

/// The first day of the month `count` months from `month`.
pub fn shift_month(month: NaiveDate, count: i32) -> NaiveDate {
    let months = Months::new(count.unsigned_abs());
    let moved = if count >= 0 {
        month.checked_add_months(months)
    } else {
        month.checked_sub_months(months)
    };
    moved.unwrap_or(month).with_day(1).unwrap_or(month)
}

pub fn render(mini: &MiniCalendar, this: &WeakEntity<MainView>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let days = mini_days(mini.month, mini.week_start);
    let step = |id: &'static str, label: &'static str, count: i32| {
        let this = this.clone();
        Button::new(id)
            .ghost()
            .xsmall()
            .label(label)
            .on_click(move |_, _, cx| {
                let _ = this.update(cx, |view, cx| view.shift_mini_month(count, cx));
            })
    };
    v_flex()
        .w_full()
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .pl_1()
                        .text_sm()
                        .font_semibold()
                        .child(mini.month.format("%B %Y").to_string()),
                )
                .child(step("mini-prev", "‹", -1))
                .child(step("mini-next", "›", 1)),
        )
        .child(h_flex().w_full().children(days.iter().take(7).map(|date| {
            div()
                .flex_1()
                .text_xs()
                .text_center()
                .text_color(theme.muted_foreground)
                .child(
                    date.format("%a")
                        .to_string()
                        .chars()
                        .take(1)
                        .collect::<String>(),
                )
        })))
        .children(days.chunks(7).map(|week| {
            h_flex().w_full().children(week.iter().map(|date| {
                let date = *date;
                let this = this.clone();
                let in_month = date.month() == mini.month.month();
                let shown = date >= mini.shown.0 && date < mini.shown.1;
                let last_shown = mini.shown.1 - TimeDelta::days(1);
                div()
                    .id(date_id("mini", date))
                    .flex_1()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_xs()
                    .when(!in_month, |d| d.text_color(theme.muted_foreground))
                    // The main view's days read as one band.
                    .when(shown, |d| d.bg(theme.muted))
                    .when(shown && date == mini.shown.0, |d| d.rounded_l_md())
                    .when(shown && date == last_shown, |d| d.rounded_r_md())
                    .hover(|s| s.bg(theme.list_hover))
                    .on_click(move |_, window, cx| {
                        let _ = this.update(cx, |view, cx| view.go_to(date, window, cx));
                    })
                    .child(
                        div()
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .border_1()
                            .border_color(if date == mini.anchor {
                                theme.ring
                            } else {
                                transparent_black()
                            })
                            .when(date == mini.today, |d| {
                                d.bg(theme.primary)
                                    .text_color(theme.primary_foreground)
                                    .font_semibold()
                            })
                            .child(date.day().to_string()),
                    )
                    .test_support()
            }))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro, which shadows `#[test]`.
    use chrono::{NaiveDate, Weekday};

    use super::{mini_days, shift_month};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn six_weeks_from_the_week_holding_the_first() {
        let days = mini_days(d(2026, 10, 1), Weekday::Mon);
        assert_eq!(days.len(), 42);
        assert_eq!(days[0], d(2026, 9, 28));
        assert_eq!(days[41], d(2026, 11, 8));
        assert_eq!(mini_days(d(2026, 10, 1), Weekday::Sun)[0], d(2026, 9, 27));
    }

    #[test]
    fn months_shift_from_their_first_day() {
        assert_eq!(shift_month(d(2026, 12, 1), 1), d(2027, 1, 1));
        assert_eq!(shift_month(d(2026, 1, 1), -1), d(2025, 12, 1));
        assert_eq!(shift_month(d(2026, 3, 31), -1), d(2026, 2, 1));
    }
}
