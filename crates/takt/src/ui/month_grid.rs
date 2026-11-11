//! The month view: one row per week, events as bars and lines of text.

use chrono::{Datelike as _, NaiveDate};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::grid::{MonthGrid, MonthWeek};
use crate::layout::{BarSpan, fit_lanes};
use crate::ui::grid_common::{BAR_HEIGHT, GridState, bar_element, date_id};

/// Height of the weekday names above the grid.
const WEEKDAYS_HEIGHT: f32 = 24.;
/// Height of the day number at the top of a cell.
const DAY_LABEL_HEIGHT: f32 = 26.;

/// How many bar lanes fit in each week row of a month drawn `height` tall.
pub fn lanes_that_fit(height: f32, weeks: usize) -> usize {
    let row = (height - WEEKDAYS_HEIGHT) / weeks.max(1) as f32;
    ((row - DAY_LABEL_HEIGHT) / BAR_HEIGHT).floor().max(0.) as usize
}

/// `height` is the room the whole view has, used to decide how many events
/// a cell shows before "+N more".
pub fn render(month: &MonthGrid, height: f32, state: &GridState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let cap = lanes_that_fit(height, month.weeks.len());
    let weekdays = h_flex()
        .w_full()
        .h(px(WEEKDAYS_HEIGHT))
        .flex_none()
        .children(
            month
                .weeks
                .first()
                .into_iter()
                .flat_map(|week| &week.days)
                .map(|date| {
                    div()
                        .flex_1()
                        .text_xs()
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .child(date.format("%a").to_string().to_uppercase())
                }),
        );
    v_flex()
        .size_full()
        .child(weekdays)
        .children(
            month
                .weeks
                .iter()
                .map(|week| week_row(week, cap, state, cx)),
        )
        .into_any_element()
}

fn week_row(week: &MonthWeek, cap: usize, state: &GridState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let row_day = week.days.first().copied().unwrap_or(state.anchor);
    let columns = week.days.len().max(1);
    let spans: Vec<(BarSpan, usize)> = week
        .bars
        .iter()
        .map(|bar| {
            (
                BarSpan {
                    first: bar.first,
                    len: bar.len,
                },
                bar.lane,
            )
        })
        .collect();
    let (shown, more) = fit_lanes(&spans, columns, cap);
    let more_top = DAY_LABEL_HEIGHT + cap.saturating_sub(1) as f32 * BAR_HEIGHT;

    div()
        .relative()
        .flex_1()
        .min_h_0()
        .w_full()
        .border_t_1()
        .border_color(theme.border)
        .overflow_hidden()
        .child(
            h_flex()
                .size_full()
                .children(week.days.iter().map(|date| day_cell(*date, state, cx))),
        )
        .children(
            week.bars
                .iter()
                .zip(shown)
                .filter(|(_, shown)| *shown)
                .map(|(bar, _)| {
                    let top = DAY_LABEL_HEIGHT + bar.lane as f32 * BAR_HEIGHT;
                    bar_element(bar, columns, top, row_day, state, cx)
                }),
        )
        .children(
            week.days
                .iter()
                .zip(more)
                .enumerate()
                .filter(|(_, (_, more))| *more > 0)
                .map(|(column, (date, more))| {
                    let this = state.this.clone();
                    let date = *date;
                    div()
                        .id(date_id("more", date))
                        .absolute()
                        .top(px(more_top))
                        .left(relative(column as f32 / columns as f32))
                        .w(relative(1. / columns as f32))
                        .h(px(BAR_HEIGHT))
                        .px_2()
                        .occlude()
                        .cursor_pointer()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.muted_foreground)
                        .hover(|s| s.text_color(theme.foreground))
                        .on_click(move |_, window, cx| {
                            let _ = this.update(cx, |view, cx| view.open_day(date, window, cx));
                        })
                        .child(format!("+{more} more"))
                        .test_support()
                }),
        )
        .into_any_element()
}

/// A day's cell: click to select the day, click its number to open it.
fn day_cell(date: NaiveDate, state: &GridState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let today = date == state.today;
    let in_month = date.month() == state.anchor.month() && date.year() == state.anchor.year();
    let (select, open) = (state.this.clone(), state.this.clone());
    let label = if date.day() == 1 {
        date.format("%-d %b").to_string()
    } else {
        date.day().to_string()
    };
    div()
        .id(date_id("day", date))
        .flex_1()
        .min_w_0()
        .h_full()
        .border_l_1()
        .border_color(theme.border)
        .when(date == state.anchor, |d| d.bg(theme.muted.opacity(0.5)))
        .on_click(move |_, window, cx| {
            let _ = select.update(cx, |view, cx| view.go_to(date, window, cx));
        })
        .child(
            h_flex()
                .h(px(DAY_LABEL_HEIGHT))
                .w_full()
                .justify_center()
                .child(
                    div()
                        .id(date_id("num", date))
                        .h(px(22.))
                        .min_w(px(22.))
                        .px_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .occlude()
                        .cursor_pointer()
                        .text_xs()
                        .when(!in_month, |d| d.text_color(theme.muted_foreground))
                        .when(today, |d| {
                            d.bg(theme.primary)
                                .text_color(theme.primary_foreground)
                                .font_semibold()
                        })
                        .hover(|s| s.bg(theme.list_hover))
                        .on_click(move |_, window, cx| {
                            let _ = open.update(cx, |view, cx| view.open_day(date, window, cx));
                        })
                        .child(label)
                        .test_support(),
                ),
        )
        .test_support()
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro, which shadows `#[test]`.
    use super::lanes_that_fit;

    #[test]
    fn taller_months_show_more_lanes() {
        // 24 for the weekday names, then per row 26 for the day number and 22 a lane.
        assert_eq!(lanes_that_fit(24. + 5. * (26. + 3. * 22.), 5), 3);
        assert_eq!(lanes_that_fit(24. + 6. * (26. + 3. * 22.) - 6., 6), 2);
        assert_eq!(lanes_that_fit(100., 6), 0);
        assert_eq!(lanes_that_fit(600., 0), 25);
    }
}
