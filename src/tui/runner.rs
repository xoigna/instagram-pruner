use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::client::{self, InstagramClient};
use crate::config::Config;
use crate::events::PrunerEvent;
use crate::ratelimit::RateLimiter;

use super::action::Effect;
use super::state::AppState;
use super::views;

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

const TICK: Duration = Duration::from_millis(120);
const EVENT_BUFFER: usize = 512;

pub struct TerminalGuard {
    active: bool,
    previous: Arc<Mutex<Option<PanicHook>>>,
}

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("enable raw mode")?;
        let mut out = stdout();
        if let Err(error) = execute!(out, EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error).context("enter alternate screen");
        }

        let previous: Arc<Mutex<Option<PanicHook>>> =
            Arc::new(Mutex::new(Some(std::panic::take_hook())));
        let hook_slot = previous.clone();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture);
            if let Some(prev) = hook_slot.lock().ok().and_then(|mut slot| slot.take()) {
                prev(info);
            }
        }));

        Ok(Self {
            active: true,
            previous,
        })
    }

    pub fn restore(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        disable_raw_mode().ok();
        execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture).ok();
        if let Some(prev) = self.previous.lock().ok().and_then(|mut slot| slot.take()) {
            std::panic::set_hook(prev);
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

pub async fn run(cfg: Config) -> Result<()> {
    let mut guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend).context("create terminal")?;
    terminal.clear().ok();

    let (tx, mut rx) = mpsc::channel::<PrunerEvent>(EVENT_BUFFER);
    let run_cancel = Arc::new(AtomicBool::new(false));
    let probe_cancel = Arc::new(AtomicBool::new(false));
    let limiter = Arc::new(RateLimiter::new(
        Duration::from_millis(2000),
        run_cancel.clone(),
    ));

    let mut state = AppState::new(cfg);

    let mut probe: Option<tokio::task::JoinHandle<()>> = None;
    let mut run_task: Option<tokio::task::JoinHandle<()>> = None;

    if !state.form.sessionid.trim().is_empty() {
        state.connection = super::state::Connection::Connecting;
        state.targets.loading = true;
        let sessionid = state.form.sessionid.clone();
        let session_path = state.config.session_path.clone();
        let base_url = state.config.base_url.clone();
        let probe_tx = tx.clone();
        let cancel = probe_cancel.clone();
        probe = Some(tokio::spawn(async move {
            probe_account(sessionid, session_path, base_url, probe_tx, cancel).await;
        }));
    }

    let mut input = EventStream::new();
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let outcome = loop {
        while let Ok(ev) = rx.try_recv() {
            state.apply_event(ev);
        }
        reap_finished(&mut state, &mut run_task).await;

        let frame = terminal.draw(|f| views::render(f, &state));
        if let Err(e) = frame {
            break Err(anyhow::anyhow!("render failed: {e}"));
        }

        tokio::select! {
            biased;
            maybe_event = input.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        if let Some(action) = super::keys::map(state.input_mode(), key) {
                            if action == super::action::Action::Quit {
                                break Ok(());
                            }
                            let effects = state.apply_action(action);
                            if effects.iter().any(|effect| matches!(effect, Effect::Quit)) {
                                break Ok(());
                            }
                            if let Err(e) = apply_effects(
                                effects,
                                &mut state,
                                &tx,
                                &run_cancel,
                                &probe_cancel,
                                &limiter,
                                &mut probe,
                                &mut run_task,
                            )
                            .await
                            {
                                break Err(e);
                            }
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => {}
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break Err(anyhow::anyhow!("input error: {e}")),
                    None => break Ok(()),
                }
            }
            Some(ev) = rx.recv() => {
                state.apply_event(ev);
                while let Ok(ev) = rx.try_recv() {
                    state.apply_event(ev);
                }
            }
            _ = ticker.tick() => {
                if let Some(toast) = &state.toast {
                    if toast.is_expired(std::time::Instant::now()) {
                        state.toast = None;
                    }
                }
            }
        }
    };

    run_cancel.store(true, Ordering::Relaxed);
    probe_cancel.store(true, Ordering::Relaxed);
    if let Some(task) = run_task {
        let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
    }
    if let Some(task) = probe {
        task.abort();
    }
    terminal.show_cursor().ok();
    guard.restore();
    outcome
}

async fn reap_finished(state: &mut AppState, run_task: &mut Option<tokio::task::JoinHandle<()>>) {
    let finished = run_task.as_ref().is_some_and(|t| t.is_finished());
    if !finished {
        return;
    }
    if let Some(task) = run_task.take() {
        let _ = task.await;
        if state.run.is_running() {
            state.run.status = crate::tui::state::RunStatus::Completed;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_effects(
    effects: Vec<Effect>,
    state: &mut AppState,
    tx: &mpsc::Sender<PrunerEvent>,
    run_cancel: &Arc<AtomicBool>,
    probe_cancel: &Arc<AtomicBool>,
    limiter: &Arc<RateLimiter>,
    probe: &mut Option<tokio::task::JoinHandle<()>>,
    run_task: &mut Option<tokio::task::JoinHandle<()>>,
) -> Result<()> {
    for effect in effects {
        match effect {
            Effect::Quit => return Ok(()),
            Effect::Probe | Effect::LoadTargets => {
                if let Some(task) = probe.take() {
                    task.abort();
                }
                let sessionid = state.form.sessionid.trim().to_string();
                if sessionid.is_empty() {
                    state.apply_event(PrunerEvent::ProbeFailed {
                        message: "no sessionid configured".into(),
                    });
                    continue;
                }
                let session_path = state.config.session_path.clone();
                let base_url = state.config.base_url.clone();
                let tx = tx.clone();
                probe_cancel.store(false, Ordering::Relaxed);
                let cancel = probe_cancel.clone();
                *probe = Some(tokio::spawn(async move {
                    probe_account(sessionid, session_path, base_url, tx, cancel).await;
                }));
            }
            Effect::StartRun(config) => {
                if run_task.as_ref().is_some_and(|task| !task.is_finished()) {
                    state.set_toast(
                        "a prune run is already active",
                        crate::events::LogLevel::Warning,
                    );
                    continue;
                }
                run_task.take();
                limiter.reset();
                run_cancel.store(false, Ordering::Relaxed);
                limiter.set_user_throttle(Duration::from_millis(config.throttle_ms));
                let sessionid = state.form.sessionid.trim().to_string();
                let base_url = state.config.base_url.clone();
                let session_path = state.config.session_path.clone();
                let tx = tx.clone();
                let cancel = run_cancel.clone();
                let limiter = limiter.clone();
                *run_task = Some(tokio::spawn(async move {
                    let client = match InstagramClient::new(
                        &sessionid,
                        Some(&base_url),
                        Some(&session_path),
                        limiter.clone(),
                    )
                    .await
                    {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = tx
                                .send(PrunerEvent::Error {
                                    message: format!("connect failed: {e}"),
                                })
                                .await;
                            return;
                        }
                    };
                    start_pruner(client, config, limiter, tx, cancel).await;
                }));
            }
            Effect::CancelRun => run_cancel.store(true, Ordering::Relaxed),
            Effect::Yank(text) => {
                if let Err(e) = crate::clipboard::copy(&text) {
                    state.set_toast(format!("clipboard: {e}"), crate::events::LogLevel::Error);
                } else {
                    state.set_toast(
                        "copied record to clipboard",
                        crate::events::LogLevel::Success,
                    );
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn probe_account(
    sessionid: String,
    session_path: String,
    base_url: String,
    tx: mpsc::Sender<PrunerEvent>,
    cancel: Arc<AtomicBool>,
) {
    let limiter = Arc::new(RateLimiter::new(Duration::from_millis(350), cancel.clone()));
    let client =
        match InstagramClient::new(&sessionid, Some(&base_url), Some(&session_path), limiter).await
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(PrunerEvent::ProbeFailed {
                        message: format!("client login failed: {e}"),
                    })
                    .await;
                return;
            }
        };

    let user_id = client.user_id();
    let username = client.username();
    let mut full_name = String::new();
    if let Ok(info) = client.fetch_user_info(&user_id).await {
        full_name = info.full_name;
    }

    let _ = tx
        .send(PrunerEvent::Identity {
            user_id,
            username,
            full_name,
        })
        .await;

    match client::list_all_threads(&client, 500).await {
        Ok(threads) => {
            let _ = tx
                .send(PrunerEvent::DMsLoaded {
                    channels: threads,
                    gen: 0,
                    truncated: false,
                })
                .await;
        }
        Err(e) => {
            let _ = tx
                .send(PrunerEvent::DMsLoadFailed {
                    message: e.to_string(),
                    gen: 0,
                })
                .await;
        }
    }
}

async fn start_pruner(
    client: InstagramClient,
    config: crate::pruner::PrunerConfig,
    rate_limiter: Arc<RateLimiter>,
    tx: mpsc::Sender<PrunerEvent>,
    cancel: Arc<AtomicBool>,
) {
    let stats = crate::pruner::run(client, config, rate_limiter, tx.clone(), cancel).await;
    let _ = tx.send(PrunerEvent::Done { stats }).await;
}
