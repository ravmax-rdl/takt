//! The day and week views: day headers, an all-day strip, and hour-by-hour
//! columns that scroll.

use chrono::{Datelike as _, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::entry::format_time;
use crate::grid::{DAY_MINUTES, GridDay, TimeGrid, TimedBlock};
use crate::ui::editing::{Drag, DragMode, EventRef, Preview};
use crate::ui::grid_common::{
    BAR_HEIGHT, GridState, bar_element, chip_id, date_id, ink_on, record_area, select_on_click,
};

/// Height of the strip at a block's bottom edge that resizes it.
const RESIZE_HANDLE: f32 = 6.;

/// Height of one hour.
pub const HOUR_HEIGHT: f32 = 48.;
/// Width of one column of hour labels.
const GUTTER_WIDTH: f32 = 56.;
/// The all-day strip scrolls past this many lanes.
const STRIP_LANES: f32 = 5.;

fn y_of(minutes: u32) -> f32 {
    minutes as f32 / 60. * HOUR_HEIGHT
}

/// `now` is today's column and the minute of the day, when today is shown.
pub fn render(
    grid: &TimeGrid,
    scroll: &ScrollHandle,
    now: Option<(NaiveDate, u32)>,
    state: &GridState,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let columns = grid.days.len();
    let gutters = 1 + usize::from(grid.secondary.is_some());
    let gutter_width = px(GUTTER_WIDTH * gutters as f32);
    let week = columns > 1;

    let zone_labels = h_flex()
        .w(gutter_width)
        .flex_none()
        .items_end()
        .pb_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .when_some(grid.secondary.as_ref(), |row, zone| {
            row.child(gutter_cell().child(zone.label.clone()))
        })
        .child(gutter_cell().child(grid.zone_label.clone()));

    let header = h_flex().w_full().flex_none().child(zone_labels).children(
        grid.days
            .iter()
            .map(|day| day_header(day.date, week, state, cx)),
    );

    let strip_height = grid.lanes as f32 * BAR_HEIGHT;
    let strip = h_flex()
        .w_full()
        .flex_none()
        .border_b_1()
        .border_color(theme.border)
        .child(div().w(gutter_width).flex_none())
        .child(
            div()
                .id("all-day")
                .flex_1()
                .min_w_0()
                .max_h(px(STRIP_LANES * BAR_HEIGHT))
                .overflow_y_scroll()
                .child(div().relative().w_full().h(px(strip_height)).children(
                    grid.bars.iter().map(|bar| {
                        let row_day = grid.days.first().map_or(state.anchor, |d| d.date);
                        bar_element(
                            bar,
                            columns,
                            bar.lane as f32 * BAR_HEIGHT,
                            row_day,
                            state,
                            cx,
                        )
                    }),
                ))
                // Outside the scrolled lanes, so it is the strip as drawn.
                .child(record_area(state.day_area)),
        );

    let hours = |labels: &[String]| {
        div()
            .relative()
            .w(px(GUTTER_WIDTH))
            .h_full()
            .flex_none()
            .children(labels.iter().enumerate().skip(1).map(|(hour, label)| {
                div()
                    .absolute()
                    .top(px(hour as f32 * HOUR_HEIGHT - 8.))
                    .right(px(6.))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(label.clone())
            }))
    };

    let body = div()
        .id("time-grid")
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .child(
            h_flex()
                .w_full()
                .h(px(24. * HOUR_HEIGHT))
                .items_start()
                .when_some(grid.secondary.as_ref(), |row, zone| {
                    row.child(hours(&zone.hours))
                })
                .child(hours(&grid.hours))
                .child(
                    h_flex()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_start()
                        .children(grid.days.iter().map(|day| {
                            let minute = now.filter(|(date, _)| *date == day.date).map(|(_, m)| m);
                            day_column(day, minute, state, cx)
                        }))
                        .child(record_area(state.time_area)),
                ),
        );

    v_flex()
        .size_full()
        .child(header)
        .child(strip)
        .child(body)
        .into_any_element()
}

fn gutter_cell() -> Div {
    div()
        .w(px(GUTTER_WIDTH))
        .flex_none()
        .pr(px(6.))
        .text_right()
}

/// The weekday and day number over a column. In a week it opens that day.
fn day_header(date: NaiveDate, week: bool, state: &GridState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let today = date == state.today;
    let number = div()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_sm()
        .font_semibold()
        .border_1()
        .border_color(if date == state.anchor && week {
            theme.ring
        } else {
            transparent_black()
        })
        .when(state.drop_day() == Some(date), |d| {
            d.bg(theme.primary.opacity(0.25))
        })
        .when(today, |d| {
            d.bg(theme.primary).text_color(theme.primary_foreground)
        })
        .child(date.day().to_string());
    let this = state.this.clone();
    v_flex()
        .id(date_id("head", date))
        .flex_1()
        .min_w_0()
        .py_1()
        .items_center()
        .border_l_1()
        .border_color(theme.border)
        .when(week, |d| d.cursor_pointer())
        .on_click(move |_, window, cx| {
            let _ = this.update(cx, |view, cx| {
                if week {
                    view.open_day(date, window, cx)
                }
            });
        })
        .child(
            div()
                .text_xs()
                .text_color(if today {
                    theme.primary
                } else {
                    theme.muted_foreground
                })
                .child(date.format("%a").to_string().to_uppercase()),
        )
        .child(number)
        .test_support()
        .into_any_element()
}

/// Where a drag would put the event, drawn on the part of it that is on `date`.
fn ghost(drag: &Drag, date: NaiveDate, state: &GridState, cx: &App) -> Option<AnyElement> {
    let Preview::Timed { start, end } = drag.preview else {
        return None;
    };
    let day_start = date.and_time(NaiveTime::MIN);
    let day_end = day_start + TimeDelta::days(1);
    if !drag.moved || end <= day_start || start >= day_end {
        return None;
    }
    let minute = |t: NaiveDateTime| (t.clamp(day_start, day_end) - day_start).num_minutes() as u32;
    let top = minute(start).min(DAY_MINUTES - 15);
    let bottom = minute(end).max(top + 15);
    let theme = cx.theme();
    let (fill, ink): (Hsla, Hsla) = match &drag.event {
        Some(event) => (rgb(event.color).into(), ink_on(event.color).into()),
        None => (theme.primary, theme.primary_foreground),
    };
    let title = drag
        .event
        .as_ref()
        .map_or_else(|| "New event".to_owned(), |event| event.title.clone());
    Some(
        div()
            .absolute()
            .top(px(y_of(top)))
            .h(px(y_of(bottom - top)))
            .left_0()
            .w_full()
            .pr(px(3.))
            .child(
                v_flex()
                    .size_full()
                    .px_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme.foreground)
                    .bg(fill.opacity(0.7))
                    .text_color(ink)
                    .text_xs()
                    .overflow_hidden()
                    .child(div().font_semibold().truncate().child(title))
                    .child(div().truncate().child(format!(
                        "{}–{}",
                        format_time(start.time(), state.use_24h),
                        format_time(end.time(), state.use_24h)
                    ))),
            )
            .into_any_element(),
    )
}

/// One day's hour lines, events and, on today, the current-time line. A drag
/// on its empty part draws a new event; a double click makes an hour-long one.
fn day_column(day: &GridDay, now: Option<u32>, state: &GridState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let this = state.this.clone();
    let date = day.date;
    v_flex()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .border_l_1()
        .border_color(theme.border)
        .when(day.date == state.anchor, |d| {
            d.bg(theme.muted.opacity(0.35))
        })
        .on_mouse_down(MouseButton::Left, move |press, window, cx| {
            let _ = this.update(cx, |view, cx| {
                view.press(
                    None,
                    DragMode::Create,
                    press.position,
                    press.click_count,
                    window,
                    cx,
                )
            });
        })
        .children((0..24).map(|hour| {
            div()
                .id(SharedString::from(format!("slot-{date}-{hour:02}")))
                .h(px(HOUR_HEIGHT))
                .flex_none()
                .border_b_1()
                .border_color(theme.border)
                .test_support()
        }))
        .children(
            day.blocks
                .iter()
                .map(|block| block_element(block, day.date, state, cx)),
        )
        .children(state.drag.and_then(|drag| ghost(drag, date, state, cx)))
        .when_some(now, |column, minute| {
            column.child(
                div()
                    .absolute()
                    .top(px(y_of(minute.min(DAY_MINUTES)) - 1.))
                    .left_0()
                    .w_full()
                    .h(px(2.))
                    .bg(theme.danger),
            )
        })
        .into_any_element()
}

fn block_element(block: &TimedBlock, day: NaiveDate, state: &GridState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let chip = &block.chip;
    let selected = state.selected == Some(&chip.key);
    let color = rgb(chip.color);
    let columns = block.slot.columns.max(1) as f32;
    let minutes = block.bottom - block.top;
    let title = if chip.warning {
        format!("{} (rule not understood)", chip.title)
    } else {
        chip.title.clone()
    };
    let inner = v_flex()
        .size_full()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(if selected {
            theme.foreground
        } else {
            theme.background
        })
        .overflow_hidden()
        .text_xs();
    let inner = if chip.declined {
        inner
            .bg(theme.background)
            .border_color(color)
            .text_color(theme.muted_foreground)
            .line_through()
    } else {
        inner.bg(color).text_color(ink_on(chip.color))
    };
    let inner = if minutes < 45 {
        // Room for one line: the title, then the start.
        inner.child(
            div()
                .truncate()
                .child(format!("{title}, {}", chip.start_label)),
        )
    } else {
        inner
            .child(div().font_semibold().truncate().child(title))
            .child(div().truncate().child(chip.time_label.clone()))
            .when(minutes >= 75, |d| {
                d.when_some(chip.location.clone(), |d, location| {
                    d.child(div().truncate().child(location))
                })
                .when_some(chip.account_badge.clone(), |d, badge| {
                    d.child(div().truncate().child(badge))
                })
            })
    };
    // A series takt could not expand cannot be changed from here.
    let event = EventRef::of_chip(chip).filter(|_| !chip.warning);
    let press = |mode: DragMode| {
        let (this, event) = (state.this.clone(), event.clone());
        move |press: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            if let Some(event) = event.clone() {
                let _ = this.update(cx, |view, cx| {
                    view.press(
                        Some(event),
                        mode,
                        press.position,
                        press.click_count,
                        window,
                        cx,
                    )
                });
            }
        }
    };
    div()
        .id(chip_id("block", chip, day))
        .absolute()
        .top(px(y_of(block.top)))
        .h(px(y_of(minutes)))
        .left(relative(block.slot.column as f32 / columns))
        .w(relative(block.slot.span as f32 / columns))
        .pr(px(3.))
        .cursor_pointer()
        .when(state.is_dragged(chip), |d| d.opacity(0.4))
        .on_mouse_down(MouseButton::Left, press(DragMode::Move))
        .on_click(select_on_click(&state.this, chip))
        .child(inner)
        .child(
            div()
                .id(chip_id("resize", chip, day))
                .absolute()
                .bottom_0()
                .left_0()
                .w_full()
                .h(px(RESIZE_HANDLE))
                .cursor_row_resize()
                .on_mouse_down(MouseButton::Left, press(DragMode::Resize))
                .test_support(),
        )
        .test_support()
        .into_any_element()
}
