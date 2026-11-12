use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::detail::EventDetail;
use crate::ui::main_view::MainView;

pub enum DetailContent {
    Event(Box<EventDetail>),
    Task {
        title: String,
        when: String,
        list: String,
        completed: bool,
    },
}

/// What can be done with the shown event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailActions {
    /// Its calendar can be written to.
    pub writable: bool,
    /// The signed-in account's reply, when it is a guest.
    pub response: Option<String>,
}

/// Edit and Delete, and for an invitation the reply buttons.
fn actions(actions: &DetailActions, this: &WeakEntity<MainView>, cx: &App) -> Div {
    let theme = cx.theme();
    let (edit, delete) = (this.clone(), this.clone());
    v_flex()
        .gap_2()
        .when(actions.writable, |column| {
            column.child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("detail-edit")
                            .outline()
                            .xsmall()
                            .label("Edit")
                            .tooltip("Edit (E)")
                            .on_click(move |_, window, cx| {
                                let _ = edit.update(cx, |view, cx| {
                                    if let Some(event) = view.shown_event(cx) {
                                        view.edit_event(event, window, cx);
                                    }
                                });
                            }),
                    )
                    .child(
                        Button::new("detail-delete")
                            .ghost()
                            .xsmall()
                            .label("Delete")
                            .tooltip("Delete (Del)")
                            .on_click(move |_, window, cx| {
                                let _ = delete.update(cx, |view, cx| {
                                    if let Some(event) = view.shown_event(cx) {
                                        view.delete_event(event, window, cx);
                                    }
                                });
                            }),
                    ),
            )
        })
        .when_some(actions.response.clone(), |column, current| {
            column.child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(theme.muted_foreground)
                            .child("Going?"),
                    )
                    .children(
                        [
                            ("accepted", "detail-reply-accepted", "Yes"),
                            ("tentative", "detail-reply-tentative", "Maybe"),
                            ("declined", "detail-reply-declined", "No"),
                        ]
                        .map(|(response, id, label)| {
                            let this = this.clone();
                            Button::new(id)
                                .ghost()
                                .xsmall()
                                .label(label)
                                .selected(current == response)
                                .on_click(move |_, window, cx| {
                                    let _ = this.update(cx, |view, cx| {
                                        if let Some(event) = view.shown_event(cx) {
                                            view.reply(event, response, window, cx);
                                        }
                                    });
                                })
                        }),
                    ),
            )
        })
}

pub fn render(
    content: &DetailContent,
    show_raw: bool,
    event_actions: &DetailActions,
    this: &WeakEntity<MainView>,
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
    let toggle = this.clone();
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
            .child(actions(event_actions, this, cx))
            .child(section(
                "Time zone",
                d.zone_note.clone().into_iter().collect(),
            ))
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
                    .on_click(move |_, _, cx| {
                        let _ = toggle.update(cx, |view, cx| view.toggle_raw(cx));
                    }),
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
