use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::{ActiveTheme as _, TitleBar, v_flex};
use gpui_kit::*;
use takt_google::{Endpoints, KeyringTokenStore, OAuthClient, RetryPolicy};
use takt_sync::{Intervals, SyncConfig, SyncHandle};

use crate::config::{ConfigState, Paths, load};
use crate::ui::first_run::FirstRun;
use crate::ui::main_view::MainView;

enum Screen {
    FirstRun { problem: Option<String> },
    Main(Entity<MainView>),
}

pub struct AppView {
    paths: Paths,
    screen: Screen,
}

impl AppView {
    pub fn new(
        paths: Paths,
        config: ConfigState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            paths,
            screen: Screen::FirstRun { problem: None },
        };
        view.apply(config, window, cx);
        view
    }

    fn apply(&mut self, config: ConfigState, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = match config {
            ConfigState::Missing => Screen::FirstRun { problem: None },
            ConfigState::Invalid(problem) => Screen::FirstRun {
                problem: Some(problem),
            },
            ConfigState::Ready(oauth) => match start_sync(&self.paths, oauth) {
                Ok(handle) => {
                    let db = self.paths.db_file.clone();
                    // The sync thread lives as long as the process; its handle is not joined on exit.
                    let SyncHandle {
                        commands, events, ..
                    } = handle;
                    Screen::Main(cx.new(|cx| MainView::new(db, commands, events, window, cx)))
                }
                Err(error) => Screen::FirstRun {
                    problem: Some(format!("cannot start sync: {error}")),
                },
            },
        };
        cx.notify();
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = load(&self.paths.config_file);
        self.apply(config, window, cx);
    }
}

fn start_sync(paths: &Paths, oauth: OAuthClient) -> std::io::Result<SyncHandle> {
    takt_sync::start(SyncConfig {
        db_path: paths.db_file.clone(),
        endpoints: Endpoints::google(),
        oauth,
        tokens: Arc::new(KeyringTokenStore),
        open_browser: Arc::new(|url| {
            if let Err(error) = open::that(url.as_str()) {
                tracing::error!(%error, "cannot open the browser for sign-in");
            }
        }),
        intervals: Intervals::default(),
        retry: RetryPolicy::default(),
    })
}

impl Render for AppView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.screen {
            Screen::Main(view) => view.clone().into_any_element(),
            Screen::FirstRun { problem } => {
                let this = cx.entity().downgrade();
                FirstRun {
                    path: self.paths.config_file.clone(),
                    problem: problem.clone(),
                    on_retry: Rc::new(move |window, cx| {
                        let _ = this.update(cx, |view, cx| view.retry(window, cx));
                    }),
                }
                .into_any_element()
            }
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(TitleBar::new().child("takt"))
            .child(div().flex_1().min_h_0().child(body))
    }
}
