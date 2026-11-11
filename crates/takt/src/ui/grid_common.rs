//! Pieces the time grid and the month grid share.

use std::cell::Cell;
use std::rc::Rc;

use chrono::NaiveDate;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::agenda::RowKey;
use crate::grid::{Bar, Chip};
use crate::ui::editing::{Drag, DragMode, EventRef, Preview};
use crate::ui::main_view::MainView;

/// Height of one bar lane, gap included.
pub const BAR_HEIGHT: f32 = 22.;

/// What a grid needs from the main view to draw and to call back.
pub struct GridState<'a> {
    pub this: WeakEntity<MainView>,
    pub selected: Option<&'a RowKey>,
    pub today: NaiveDate,
    /// The selected day.
    pub anchor: NaiveDate,
    pub use_24h: bool,
    /// The drag in progress or being saved, if any.
    pub drag: Option<&'a Drag>,
    /// Where the grids record the area a drag is measured against.
    pub time_area: &'a Rc<Cell<Option<Bounds<Pixels>>>>,
    pub day_area: &'a Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl GridState<'_> {
    /// Whether `chip` is the event being dragged to somewhere else.
    pub fn is_dragged(&self, chip: &Chip) -> bool {
        self.drag.is_some_and(|drag| {
            drag.moved && drag.event.as_ref().is_some_and(|e| e.key() == chip.key)
        })
    }

    /// The day a day-to-day drag is over.
    pub fn drop_day(&self) -> Option<NaiveDate> {
        match self.drag {
            Some(Drag {
                preview: Preview::Day(day),
                moved: true,
                ..
            }) => Some(*day),
            _ => None,
        }
    }
}

/// An invisible element that records where its parent was drawn.
pub fn record_area(area: &Rc<Cell<Option<Bounds<Pixels>>>>) -> impl IntoElement {
    let area = area.clone();
    canvas(move |bounds, _, _| area.set(Some(bounds)), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// A mouse-down handler for the chip's event: a press starts a drag, a
/// double click opens the editor.
pub fn drag_on_press(
    this: &WeakEntity<MainView>,
    chip: &Chip,
    mode: DragMode,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    let this = this.clone();
    // A series takt could not expand cannot be changed from here.
    let event = EventRef::of_chip(chip).filter(|_| !chip.warning);
    move |press, window, cx| {
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
}

/// Black or white, whichever reads better on `color`.
pub fn ink_on(color: u32) -> Rgba {
    let channel = |shift: u32| f64::from((color >> shift) & 0xff) / 255.;
    let luminance = 0.299 * channel(16) + 0.587 * channel(8) + 0.114 * channel(0);
    if luminance > 0.6 {
        rgb(0x1f1f1f)
    } else {
        rgb(0xffffff)
    }
}

/// A stable ID for one drawn piece of an event: `<prefix>-<instance>-<day>`,
/// with the account and calendar in front, since the same event can sit in
/// several of them.
pub fn chip_id(prefix: &str, chip: &Chip, day: NaiveDate) -> ElementId {
    let name = match &chip.key {
        RowKey::Event {
            account_id,
            calendar_id,
            instance_id,
            ..
        } => format!("{prefix}-{account_id}-{calendar_id}-{instance_id}-{day}"),
        RowKey::Task {
            account_id,
            list_id,
            task_id,
        } => format!("{prefix}-{account_id}-{list_id}-{task_id}-{day}"),
    };
    ElementId::from(SharedString::from(name))
}

pub fn date_id(prefix: &str, date: NaiveDate) -> ElementId {
    ElementId::from(SharedString::from(format!("{prefix}-{date}")))
}

/// A click handler that selects the chip's event and opens its details.
pub fn select_on_click(
    this: &WeakEntity<MainView>,
    chip: &Chip,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let this = this.clone();
    let (key, when) = (chip.key.clone(), chip.when.clone());
    move |_, window, cx| {
        let _ = this.update(cx, |view, cx| {
            view.select_event(key.clone(), when.clone(), window, cx)
        });
    }
}

/// A bar in a row of `columns` day columns, `top` below the row's top edge.
/// `row_day` is the row's first day, which makes the ID unique per row.
pub fn bar_element(
    bar: &Bar,
    columns: usize,
    top: f32,
    row_day: NaiveDate,
    state: &GridState,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let chip = &bar.chip;
    let selected = state.selected == Some(&chip.key);
    let color = rgb(chip.color);
    let columns = columns.max(1) as f32;
    let title = if chip.warning {
        format!("{} (rule not understood)", chip.title)
    } else {
        chip.title.clone()
    };
    let inner = h_flex()
        .size_full()
        .gap_1()
        .px_1()
        .items_center()
        .rounded_sm()
        .border_1()
        .border_color(if selected {
            theme.foreground
        } else {
            transparent_black()
        })
        .text_xs()
        .overflow_hidden();
    let inner = if bar.timed {
        // One line of text: a dot in the event's colour, its start, its title.
        inner
            .hover(|s| s.bg(theme.list_hover))
            .child(div().flex_none().size(px(6.)).rounded_full().bg(color))
            .child(
                div()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(chip.start_label.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(chip.declined, |d| {
                        d.line_through().text_color(theme.muted_foreground)
                    })
                    .child(title),
            )
    } else if chip.declined {
        inner
            .border_color(color)
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .line_through()
                    .child(title),
            )
    } else {
        inner
            .bg(color)
            .text_color(ink_on(chip.color))
            .when(bar.starts_before, |d| d.rounded_l_none())
            .when(bar.ends_after, |d| d.rounded_r_none())
            .child(div().flex_1().min_w_0().truncate().child(title))
    };
    div()
        .id(chip_id("bar", chip, row_day))
        .absolute()
        .top(px(top))
        .left(relative(bar.first as f32 / columns))
        .w(relative(bar.len as f32 / columns))
        .h(px(BAR_HEIGHT))
        .px(px(2.))
        .pb(px(2.))
        .occlude()
        .cursor_pointer()
        .when(state.is_dragged(chip), |d| d.opacity(0.4))
        .on_mouse_down(
            MouseButton::Left,
            drag_on_press(&state.this, chip, DragMode::Days),
        )
        .on_click(select_on_click(&state.this, chip))
        .child(inner)
        .test_support()
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro, which shadows `#[test]`.
    use gpui_kit::rgb;

    use super::ink_on;

    #[test]
    fn ink_contrasts_with_the_background() {
        assert_eq!(ink_on(0xffffff), rgb(0x1f1f1f));
        assert_eq!(ink_on(0x9fe1e7), rgb(0x1f1f1f));
        assert_eq!(ink_on(0x000000), rgb(0xffffff));
        assert_eq!(ink_on(0x039be5), rgb(0xffffff));
        assert_eq!(ink_on(0x7986cb), rgb(0xffffff));
    }
}
