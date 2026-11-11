//! Above the calendar: today, previous and next, the shown dates, and the
//! view switch.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, h_flex,
};
use gpui_kit::*;

use crate::ui::main_view::MainView;
use crate::view::ViewKind;

pub const TOOLBAR_HEIGHT: f32 = 44.;

fn view_button_id(view: ViewKind) -> &'static str {
    match view {
        ViewKind::Day => "view-day",
        ViewKind::Week => "view-week",
        ViewKind::Month => "view-month",
        ViewKind::Agenda => "view-agenda",
    }
}

fn view_tooltip(view: ViewKind) -> &'static str {
    match view {
        ViewKind::Day => "Day (D or 1)",
        ViewKind::Week => "Week (W or 2)",
        ViewKind::Month => "Month (M or 3)",
        ViewKind::Agenda => "Agenda (A or 4)",
    }
}

pub fn render(title: String, view: ViewKind, this: &WeakEntity<MainView>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let step = |id: &'static str, label: &'static str, tooltip: &'static str, count: i64| {
        let this = this.clone();
        Button::new(id)
            .ghost()
            .small()
            .label(label)
            .tooltip(tooltip)
            .on_click(move |_, window, cx| {
                let _ = this.update(cx, |view, cx| view.step(count, window, cx));
            })
    };
    let today = {
        let this = this.clone();
        Button::new("today")
            .outline()
            .small()
            .label("Today")
            .tooltip("Today (T)")
            .on_click(move |_, window, cx| {
                let _ = this.update(cx, |view, cx| view.go_today(window, cx));
            })
    };
    h_flex()
        .w_full()
        .h(px(TOOLBAR_HEIGHT))
        .flex_none()
        .px_3()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .child(today)
        .child(step("prev", "‹", "Previous (K or P)", -1))
        .child(step("next", "›", "Next (J or N)", 1))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_lg()
                .font_semibold()
                .child(title),
        )
        .child(
            h_flex()
                .gap_1()
                .flex_none()
                .children(ViewKind::ALL.into_iter().map(|kind| {
                    let this = this.clone();
                    Button::new(view_button_id(kind))
                        .ghost()
                        .small()
                        .label(kind.label())
                        .tooltip(view_tooltip(kind))
                        .selected(kind == view)
                        .on_click(move |_, window, cx| {
                            let _ = this.update(cx, |view, cx| view.show_view(kind, window, cx));
                        })
                })),
        )
        .into_any_element()
}
