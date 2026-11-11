//! The sync thread: one tokio runtime, a supervisor that owns account loops,
//! and the polling schedule.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use takt_google::{
    Auth, Endpoints, GoogleClient, GoogleError, OAuthClient, RetryPolicy, SignedIn, TokenStore, Url,
};
use takt_store::{AccountStatus, Store};
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinError, JoinHandle};

use crate::{AccountEngine, Command, SyncEvent, SyncState};

const OFFLINE_FIRST_RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct Intervals {
    pub focused: Duration,
    pub unfocused: Duration,
    pub meta: Duration,
    pub offline_max: Duration,
    pub restart_max: Duration,
}

impl Default for Intervals {
    fn default() -> Self {
        Self {
            focused: Duration::from_secs(60),
            unfocused: Duration::from_secs(300),
            meta: Duration::from_secs(3600),
            offline_max: Duration::from_secs(300),
            restart_max: Duration::from_secs(300),
        }
    }
}

pub struct SyncConfig {
    pub db_path: PathBuf,
    pub endpoints: Endpoints,
    pub oauth: OAuthClient,
    pub tokens: Arc<dyn TokenStore>,
    pub open_browser: Arc<dyn Fn(&Url) + Send + Sync>,
    pub intervals: Intervals,
    pub retry: RetryPolicy,
}

pub struct SyncHandle {
    pub commands: async_channel::Sender<Command>,
    pub events: async_channel::Receiver<SyncEvent>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SyncHandle {
    /// Stops every account loop and waits for the sync thread to exit.
    pub fn shutdown(mut self) {
        let _ = self.commands.try_send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start(config: SyncConfig) -> std::io::Result<SyncHandle> {
    let (command_tx, command_rx) = async_channel::unbounded();
    let (event_tx, event_rx) = async_channel::unbounded();
    let thread = std::thread::Builder::new()
        .name("takt-sync".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("cannot build the sync runtime");
            runtime.block_on(run(config, command_rx, event_tx));
            // Account loops are aborted by then; do not wait long for blocking work.
            runtime.shutdown_timeout(Duration::from_secs(2));
        })?;
    Ok(SyncHandle {
        commands: command_tx,
        events: event_rx,
        thread: Some(thread),
    })
}

/// State every account loop reads.
struct Shared {
    store: Arc<Mutex<Store>>,
    http: reqwest::Client,
    endpoints: Endpoints,
    oauth: OAuthClient,
    tokens: Arc<dyn TokenStore>,
    open_browser: Arc<dyn Fn(&Url) + Send + Sync>,
    intervals: Intervals,
    retry: RetryPolicy,
    events: async_channel::Sender<SyncEvent>,
    focused: watch::Receiver<bool>,
    wake: watch::Receiver<u64>,
}

impl Shared {
    /// The store, even if a thread panicked while holding it: each write is
    /// one transaction, so a panic cannot leave it half-applied.
    fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, event: SyncEvent) {
        let _ = self.events.try_send(event);
    }

    fn status(&self, account_id: &str, state: SyncState) {
        self.send(SyncEvent::Status {
            account_id: account_id.to_owned(),
            state,
        });
    }

    fn poll_interval(&self) -> Duration {
        if *self.focused.borrow() {
            self.intervals.focused
        } else {
            self.intervals.unfocused
        }
    }

    fn mark_needs_reauth(&self, account_id: &str) {
        if let Err(error) = self
            .store()
            .set_account_status(account_id, AccountStatus::NeedsReauth)
        {
            tracing::error!(%error, "cannot record re-auth status");
        }
        self.status(account_id, SyncState::NeedsReauth);
        self.send(SyncEvent::AuthNeeded {
            account_id: account_id.to_owned(),
        });
    }
}

async fn run(
    config: SyncConfig,
    commands: async_channel::Receiver<Command>,
    events: async_channel::Sender<SyncEvent>,
) {
    let store = match Store::open(&config.db_path) {
        Ok(store) => store,
        Err(error) => {
            tracing::error!(%error, "sync cannot open the database");
            let _ = events.try_send(SyncEvent::SignInFailed(format!(
                "cannot open the database: {error}"
            )));
            return;
        }
    };
    let (focus_tx, focused) = watch::channel(true);
    let (wake_tx, wake) = watch::channel(0u64);
    let shared = Arc::new(Shared {
        store: Arc::new(Mutex::new(store)),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client"),
        endpoints: config.endpoints,
        oauth: config.oauth,
        tokens: config.tokens,
        open_browser: config.open_browser,
        intervals: config.intervals,
        retry: config.retry,
        events,
        focused,
        wake,
    });
    let (sign_in_tx, mut sign_ins) = mpsc::unbounded_channel::<Result<SignedIn, GoogleError>>();
    let mut loops: HashMap<String, AbortOnDrop<()>> = HashMap::new();

    let accounts = shared.store().accounts().unwrap_or_default();
    for account in accounts {
        match account.status {
            AccountStatus::Ok => {
                loops.insert(
                    account.account_id.clone(),
                    spawn_account(&shared, account.account_id, account.email, None),
                );
            }
            AccountStatus::NeedsReauth => {
                shared.status(&account.account_id, SyncState::NeedsReauth);
                shared.send(SyncEvent::AuthNeeded {
                    account_id: account.account_id,
                });
            }
        }
    }

    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Ok(Command::Shutdown) | Err(_) => break,
                Ok(Command::AddAccount) => {
                    let shared = shared.clone();
                    let sign_in_tx = sign_in_tx.clone();
                    tokio::spawn(async move {
                        let open = shared.open_browser.clone();
                        let result = takt_google::sign_in(&shared.http, &shared.endpoints, &shared.oauth, |url| open(url)).await;
                        let _ = sign_in_tx.send(result);
                    });
                }
                Ok(Command::RemoveAccount { account_id }) => {
                    loops.remove(&account_id);
                    remove_account(&shared, &account_id);
                }
                Ok(Command::SyncNow) => {
                    wake_tx.send_modify(|n| *n += 1);
                }
                Ok(Command::WindowFocus(focused)) => {
                    focus_tx.send_replace(focused);
                    if focused {
                        wake_tx.send_modify(|n| *n += 1);
                    }
                }
            },
            Some(result) = sign_ins.recv() => match result {
                Ok(signed) => {
                    if let Some(handle) = finish_sign_in(&shared, signed) {
                        loops.insert(handle.0, handle.1);
                    }
                }
                Err(error) => shared.send(SyncEvent::SignInFailed(error.to_string())),
            },
        }
    }
}

fn finish_sign_in(shared: &Arc<Shared>, signed: SignedIn) -> Option<(String, AbortOnDrop<()>)> {
    if let Err(error) = shared.tokens.set(&signed.email, &signed.refresh_token) {
        shared.send(SyncEvent::SignInFailed(format!(
            "cannot save the sign-in to the keychain: {error}"
        )));
        return None;
    }
    if let Err(error) = shared.store().upsert_account(&signed.sub, &signed.email) {
        shared.send(SyncEvent::SignInFailed(format!(
            "cannot save the account: {error}"
        )));
        return None;
    }
    shared.send(SyncEvent::AccountsChanged);
    let seed = Some((signed.access_token, Duration::from_secs(signed.expires_in)));
    let handle = spawn_account(shared, signed.sub.clone(), signed.email, seed);
    Some((signed.sub, handle))
}

fn remove_account(shared: &Arc<Shared>, account_id: &str) {
    let mut store = shared.store();
    let email = store
        .accounts()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a.account_id == account_id)
        .map(|a| a.email);
    if let Some(email) = email
        && let Err(error) = shared.tokens.delete(&email)
    {
        tracing::warn!(%error, "cannot delete the keychain entry");
    }
    if let Err(error) = store.remove_account(account_id) {
        tracing::error!(%error, "cannot remove the account");
    }
    drop(store);
    shared.send(SyncEvent::AccountsChanged);
}

fn spawn_account(
    shared: &Arc<Shared>,
    account_id: String,
    email: String,
    seed: Option<(String, Duration)>,
) -> AbortOnDrop<()> {
    let shared = shared.clone();
    AbortOnDrop(tokio::spawn(async move {
        let restart_max = shared.intervals.restart_max;
        let on_panic = {
            let shared = shared.clone();
            let account_id = account_id.clone();
            move || {
                shared.status(
                    &account_id,
                    SyncState::Error("sync crashed; restarting".into()),
                )
            }
        };
        supervise(
            move || {
                account_loop(
                    shared.clone(),
                    account_id.clone(),
                    email.clone(),
                    seed.clone(),
                )
            },
            on_panic,
            restart_max,
        )
        .await;
    }))
}

async fn account_loop(
    shared: Arc<Shared>,
    account_id: String,
    email: String,
    seed: Option<(String, Duration)>,
) {
    // A loop starts with the wake-ups sent before it marked as seen, so it
    // runs one round now, not an extra one straight after.
    let mut wake = shared.wake.clone();
    wake.borrow_and_update();
    let mut offline_delay = OFFLINE_FIRST_RETRY;
    let refresh_token = loop {
        match shared.tokens.get(&email) {
            Ok(Some(token)) => break token,
            Ok(None) => return shared.mark_needs_reauth(&account_id),
            Err(error) => {
                // A locked or busy keychain may answer later; retry like offline.
                tracing::warn!(account = %account_id, %error, "cannot read the refresh token");
                shared.status(&account_id, SyncState::Error(error.to_string()));
                let wait = offline_delay;
                offline_delay = (offline_delay * 2).min(shared.intervals.offline_max);
                if !sleep_or_wake(wait, &mut wake).await {
                    return;
                }
            }
        }
    };
    offline_delay = OFFLINE_FIRST_RETRY;
    let mut auth = Auth::new(
        shared.http.clone(),
        shared.endpoints.token.clone(),
        shared.oauth.clone(),
        refresh_token,
    );
    if let Some((token, ttl)) = seed {
        auth = auth.with_access_token(token, ttl);
    }
    let client = GoogleClient::new(shared.http.clone(), shared.endpoints.clone(), auth)
        .with_retry(shared.retry.clone());
    let engine = AccountEngine::new(
        account_id.clone(),
        client,
        shared.store.clone(),
        shared.events.clone(),
    );
    let mut offline_since: Option<DateTime<Utc>> = None;
    loop {
        let with_meta = engine.meta_due(shared.intervals.meta);
        shared.status(&account_id, SyncState::Syncing);
        let wait = match engine.round(with_meta).await {
            Ok(()) => {
                offline_since = None;
                offline_delay = OFFLINE_FIRST_RETRY;
                shared.status(&account_id, SyncState::Synced(Utc::now()));
                shared.poll_interval()
            }
            Err(error) if error.needs_reauth() => return shared.mark_needs_reauth(&account_id),
            Err(error) if error.is_offline() => {
                let since = *offline_since.get_or_insert_with(Utc::now);
                shared.status(&account_id, SyncState::Offline(since));
                let wait = offline_delay;
                offline_delay = (offline_delay * 2).min(shared.intervals.offline_max);
                wait
            }
            Err(error) => {
                offline_since = None;
                offline_delay = OFFLINE_FIRST_RETRY;
                tracing::warn!(account = %account_id, %error, "sync round failed");
                shared.status(&account_id, SyncState::Error(error.to_string()));
                shared.poll_interval()
            }
        };
        if !sleep_or_wake(wait, &mut wake).await {
            return;
        }
    }
}

/// Waits for `wait` or a wake-up. False when the runtime is shutting down.
async fn sleep_or_wake(wait: Duration, wake: &mut watch::Receiver<u64>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(wait) => true,
        changed = wake.changed() => changed.is_ok(),
    }
}

/// A task handle that aborts the task when dropped.
pub(crate) struct AbortOnDrop<T>(pub(crate) JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs `make()` as a task and starts it again, with backoff, whenever it
/// panics. A run that lasted longer than `max_backoff` starts the backoff
/// again from the beginning. Returns when a run finishes normally.
pub(crate) async fn supervise<F, Fut>(mut make: F, on_panic: impl Fn(), max_backoff: Duration)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let first_delay = Duration::from_secs(1).min(max_backoff);
    let mut delay = first_delay;
    loop {
        let started = tokio::time::Instant::now();
        let mut task = AbortOnDrop(tokio::spawn(make()));
        let outcome: Result<(), JoinError> = (&mut task.0).await;
        match outcome {
            Ok(()) => return,
            Err(error) if error.is_panic() => {
                on_panic();
                if started.elapsed() > max_backoff {
                    delay = first_delay;
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_backoff);
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn panicking_task_is_restarted() {
        let runs = Arc::new(AtomicUsize::new(0));
        let panics = Arc::new(AtomicUsize::new(0));
        let make = {
            let runs = runs.clone();
            move || {
                let runs = runs.clone();
                async move {
                    if runs.fetch_add(1, SeqCst) == 0 {
                        panic!("first run fails");
                    }
                }
            }
        };
        let on_panic = {
            let panics = panics.clone();
            move || {
                panics.fetch_add(1, SeqCst);
            }
        };
        supervise(make, on_panic, Duration::from_millis(1)).await;
        assert_eq!(runs.load(SeqCst), 2);
        assert_eq!(panics.load(SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn long_run_resets_the_restart_delay() {
        let starts = Arc::new(Mutex::new(Vec::new()));
        let make = {
            let starts = starts.clone();
            move || {
                let starts = starts.clone();
                async move {
                    let run = {
                        let mut starts = starts.lock().unwrap();
                        starts.push(tokio::time::Instant::now());
                        starts.len()
                    };
                    match run {
                        1 => panic!("fails at once"),
                        2 => {
                            tokio::time::sleep(Duration::from_secs(10)).await;
                            panic!("fails after a long run");
                        }
                        _ => {}
                    }
                }
            }
        };
        supervise(make, || {}, Duration::from_secs(4)).await;
        let starts = starts.lock().unwrap();
        assert_eq!(starts.len(), 3);
        assert_eq!(starts[1] - starts[0], Duration::from_secs(1));
        // Without the reset this wait would be the doubled 2 s.
        assert_eq!(starts[2] - starts[1], Duration::from_secs(11));
    }

    #[tokio::test]
    async fn dropping_the_handle_aborts_the_task() {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let handle = AbortOnDrop(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            flag.store(true, SeqCst);
        }));
        drop(handle);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!finished.load(SeqCst));
    }
}
