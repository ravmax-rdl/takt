use std::cell::RefCell;
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
    FirstRun {
        problem: Option<String>,
    },
    Main(Entity<MainView>),
    /// takt cannot run at all (e.g. its database will not open).
    Fatal(String),
}

pub struct AppView {
    paths: Paths,
    screen: Screen,
    /// Taken and shut down when the app quits.
    sync: Rc<RefCell<Option<SyncHandle>>>,
}

impl AppView {
    /// `startup_error` is a problem found before the window opened; it is
    /// shown instead of the app.
    pub fn new(
        paths: Paths,
        config: ConfigState,
        startup_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sync: Rc<RefCell<Option<SyncHandle>>> = Rc::default();
        // App-level, not tied to this view: the window (and this view) may be
        // gone by the time the quit handlers run.
        let slot = sync.clone();
        App::on_app_quit(cx, move |cx| {
            let shutdown = slot
                .borrow_mut()
                .take()
                .map(|handle| cx.background_spawn(async move { handle.shutdown() }));
            // GPUI waits for this only briefly (SHUTDOWN_TIMEOUT) before exiting.
            async move {
                if let Some(shutdown) = shutdown {
                    shutdown.await;
                }
            }
        })
        .detach();
        let mut view = Self {
            paths,
            screen: Screen::FirstRun { problem: None },
            sync,
        };
        match startup_error {
            Some(message) => view.screen = Screen::Fatal(message),
            None => view.apply(config, window, cx),
        }
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
                    let (commands, events) = (handle.commands.clone(), handle.events.clone());
                    if let Some(old) = self.sync.borrow_mut().replace(handle) {
                        cx.background_spawn(async move { old.shutdown() }).detach();
                    }
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

fn render_fatal(message: &str, cx: &App) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .p_6()
        .text_color(cx.theme().foreground)
        .child(div().text_lg().child("takt cannot start"))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(message.to_owned()),
        )
        .into_any_element()
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
            Screen::Fatal(message) => render_fatal(message, cx),
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
