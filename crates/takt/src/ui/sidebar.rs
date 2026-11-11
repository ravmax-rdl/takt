use async_channel::Sender;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use takt_store::AccountStatus;
use takt_sync::{Command, SyncState};

use crate::agenda::{DEFAULT_COLOR, parse_hex};
use crate::model::CalendarModel;

/// `mini` is the mini calendar, drawn above the accounts.
pub fn render(
    model: &CalendarModel,
    commands: &Sender<Command>,
    mini: AnyElement,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let add = commands.clone();
    v_flex()
        .id("sidebar")
        .w(px(240.))
        .h_full()
        .flex_none()
        .p_3()
        .gap_3()
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
        .overflow_y_scroll()
        .child(mini)
        .when_some(model.last_error.clone(), |this, message| {
            this.child(div().text_xs().text_color(theme.danger).child(message))
        })
        .children(model.snapshot.accounts.iter().map(|account| {
            let state = model.statuses.get(&account.account_id);
            let dot = match state {
                Some(SyncState::Synced(_)) => theme.success,
                Some(SyncState::Syncing) => theme.info,
                Some(SyncState::Error(_) | SyncState::NeedsReauth) => theme.danger,
                _ => theme.muted_foreground,
            };
            let needs_sign_in = account.status == AccountStatus::NeedsReauth
                || matches!(state, Some(SyncState::NeedsReauth));
            let (reauth, remove) = (commands.clone(), commands.clone());
            let (account_id, email) = (account.account_id.clone(), account.email.clone());
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().flex_none().size(px(8.)).rounded_full().bg(dot))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_semibold()
                                .child(account.email.clone()),
                        )
                        .child(
                            Button::new(SharedString::from(format!("remove-{account_id}")))
                                .ghost()
                                .xsmall()
                                .label("Remove")
                                .on_click(move |_, window, cx| {
                                    let (remove, account_id, email) =
                                        (remove.clone(), account_id.clone(), email.clone());
                                    window.open_alert_dialog(cx, move |dialog, _, _| {
                                        let (remove, account_id) =
                                            (remove.clone(), account_id.clone());
                                        dialog
                                            .title(format!("Remove {email}?"))
                                            .description(
                                                "takt deletes this account's cached data and its \
                                                 saved sign-in. Nothing changes in Google.",
                                            )
                                            .confirm()
                                            .on_ok(move |_, _, _| {
                                                let _ = remove.try_send(Command::RemoveAccount {
                                                    account_id: account_id.clone(),
                                                });
                                                true
                                            })
                                    });
                                }),
                        ),
                )
                .when(needs_sign_in, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("reauth-{}", account.account_id)))
                            .small()
                            .primary()
                            .label("Sign in again")
                            .on_click(move |_, _, _| {
                                let _ = reauth.try_send(Command::AddAccount);
                            }),
                    )
                })
                .children(
                    model
                        .snapshot
                        .calendars
                        .iter()
                        .filter(|c| c.account_id == account.account_id)
                        .map(|c| {
                            let color = c
                                .background_color
                                .as_deref()
                                .and_then(parse_hex)
                                .unwrap_or(DEFAULT_COLOR);
                            // Filled when shown. A click shows or hides the
                            // calendar in takt; a calendar hidden in Google's
                            // own settings is not synced and cannot be shown.
                            let toggle = commands.clone();
                            let (account_id, calendar_id) =
                                (c.account_id.clone(), c.calendar_id.clone());
                            let (visible, hidden) = (c.visible, c.hidden);
                            h_flex()
                                .id(SharedString::from(format!(
                                    "calendar-{}-{}",
                                    c.account_id, c.calendar_id
                                )))
                                .gap_2()
                                .pl_4()
                                .py(px(1.))
                                .items_center()
                                .rounded_sm()
                                .text_sm()
                                .when(!visible, |row| row.text_color(theme.muted_foreground))
                                .when(!hidden, |row| {
                                    row.cursor_pointer().hover(|s| s.bg(theme.list_hover))
                                })
                                .on_click(move |_, _, _| {
                                    if !hidden {
                                        let _ = toggle.try_send(Command::SetCalendarVisible {
                                            account_id: account_id.clone(),
                                            calendar_id: calendar_id.clone(),
                                            visible: !visible,
                                        });
                                    }
                                })
                                .child(
                                    div()
                                        .flex_none()
                                        .size(px(12.))
                                        .rounded_sm()
                                        .border_2()
                                        .border_color(rgb(color))
                                        .when(visible, |d| d.bg(rgb(color))),
                                )
                                .child(div().min_w_0().truncate().child(c.summary.clone()))
                                .test_support()
                        }),
                )
        }))
        .child(
            Button::new("add-account")
                .ghost()
                .label("+ Add account")
                .on_click(move |_, _, _| {
                    let _ = add.try_send(Command::AddAccount);
                }),
        )
        .into_any_element()
}
