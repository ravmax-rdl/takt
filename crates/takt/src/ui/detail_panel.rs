use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::detail::EventDetail;

pub enum DetailContent {
    Event(Box<EventDetail>),
    Task {
        title: String,
        when: String,
        list: String,
        completed: bool,
    },
}

pub fn render(
    content: &DetailContent,
    show_raw: bool,
    on_toggle_raw: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let section = |label: &'static str, lines: Vec<String>| {
        v_flex().gap_1().when(!lines.is_empty(), |this| {
            this.child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
            .children(lines.into_iter().map(|l| div().text_sm().child(l)))
        })
    };
    let body = match content {
        DetailContent::Task {
            title,
            when,
            list,
            completed,
        } => v_flex()
            .gap_3()
            .child(div().text_lg().font_semibold().child(title.clone()))
            .child(div().text_sm().child(when.clone()))
            .child(section("List", vec![list.clone()]))
            .child(section(
                "Status",
                vec![if *completed {
                    "Completed".into()
                } else {
                    "Not completed".into()
                }],
            )),
        DetailContent::Event(d) => v_flex()
            .gap_3()
            .child(div().text_lg().font_semibold().child(d.title.clone()))
            .child(div().text_sm().child(d.when.clone()))
            .child(section(
                "Calendar",
                vec![format!("{} · {}", d.calendar, d.account)],
            ))
            .child(section(
                "Location",
                d.location.clone().into_iter().collect(),
            ))
            .child(section("Repeats", d.recurrence.clone()))
            .child(section(
                "Organizer",
                d.organizer.clone().into_iter().collect(),
            ))
            .child(section(
                "Guests",
                d.attendees
                    .iter()
                    .map(|(who, reply)| format!("{who} · {reply}"))
                    .collect(),
            ))
            .child(section("Join", d.conference.clone()))
            .child(section("Reminders", d.reminders.clone()))
            .child(section(
                "Description",
                d.description.clone().into_iter().collect(),
            ))
            .child(
                Button::new("toggle-raw")
                    .ghost()
                    .xsmall()
                    .label(if show_raw {
                        "Hide raw JSON"
                    } else {
                        "Show raw JSON"
                    })
                    .on_click(move |_, window, cx| on_toggle_raw(window, cx)),
            )
            .when(show_raw, |this| {
                this.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(theme.muted)
                        .text_xs()
                        .font_family("monospace")
                        .child(d.raw_json.clone()),
                )
            }),
    };
    v_flex()
        .id("detail-panel")
        .w(px(380.))
        .h_full()
        .flex_none()
        .p_4()
        .border_l_1()
        .border_color(theme.border)
        .overflow_y_scroll()
        .child(h_flex().w_full().child(body.flex_1().min_w_0()))
        .into_any_element()
}
