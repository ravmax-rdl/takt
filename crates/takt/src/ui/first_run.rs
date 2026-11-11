use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::config::EXAMPLE;

pub type RetryHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Shown until config.toml holds a usable OAuth client.
#[derive(IntoElement)]
pub struct FirstRun {
    pub path: PathBuf,
    pub problem: Option<String>,
    pub on_retry: RetryHandler,
}

impl RenderOnce for FirstRun {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let folder = self.path.parent().map(PathBuf::from).unwrap_or_default();
        let on_retry = self.on_retry.clone();
        v_flex()
            .size_full()
            .p_8()
            .gap_4()
            .max_w(px(720.))
            .child(div().text_xl().font_semibold().child("Connect takt to Google"))
            .child("takt uses your own Google Cloud OAuth client. Once:")
            .child(
                v_flex()
                    .gap_1()
                    .child("1. Create a Google Cloud project and enable the Google Calendar API and Google Tasks API.")
                    .child("2. Create an OAuth client of type Desktop app.")
                    .child("3. On the consent screen, set the publishing status to In production. In Testing, Google expires sign-ins after 7 days.")
                    .child(format!("4. Save the client ID and secret to {}:", self.path.display())),
            )
            .child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .font_family("monospace")
                    .child(EXAMPLE),
            )
            .when_some(self.problem, |this, problem| {
                this.child(
                    div()
                        .text_color(cx.theme().danger)
                        .child(format!("config.toml is not usable yet: {problem}")),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("open-folder")
                            .label("Open config folder")
                            .on_click(move |_, _, _| {
                                let _ = std::fs::create_dir_all(&folder);
                                let _ = open::that(&folder);
                            }),
                    )
                    .child(
                        Button::new("retry")
                            .primary()
                            .label("Check again")
                            .on_click(move |_, window, cx| on_retry(window, cx)),
                    ),
            )
    }
}
