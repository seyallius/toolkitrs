//! module tui - Interactive terminal UI for the toolkitrs built on ratatui.
//!
//! Responsibilities are split deliberately (Single Responsibility):
//!   * `app`    → state machine + input handling
//!   * `ui`     → pure rendering (reads state, draws widgets)
//!   * `event`  → keyboard + async events over a channel
//!   * `runner` → background workflow execution
//!
//! Shared workflow metadata lives in `crate::workflow`, outside the TUI,
//! so the CLI and TUI use the same domain model.
//!
//! This mirrors a classic loop: read events → update state → draw.

mod app;
mod event;
mod runner;
mod ui;

use crate::tui::{app::App, event::AppEvent};
use anyhow::Result;
use crossterm::{self, cursor, event as crossterm_event, terminal};
use ratatui::{self, backend};
use std::{io, path::PathBuf, sync::mpsc, time::Duration};

// --------------------------------- Types, Constants & Variables ------------------------------- //

const RECV_TIMEOUT: Duration = Duration::from_millis(250);

// ------------------------------------------ Types & Impls ------------------------------------- //

/// RAII guard that always restores the terminal, even on early `?` returns.
///
/// This is the Rust analogue of Go's `defer` or Java's `finally` —
/// no matter how we exit the scope, the user's shell is left usable.
struct TerminalGuard;
impl Drop for TerminalGuard {
    /// Restores raw mode and leaves the alternate screen.
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            io::stdout(),
            cursor::Show,
            terminal::LeaveAlternateScreen,
            crossterm_event::DisableMouseCapture
        );
    }
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Starts the TUI event loop and blocks until the user quits.
///
/// # Arguments
/// * `ffmpeg_path` - Optional explicit ffmpeg binary location.
///
/// # Errors
/// Returns an error if the terminal cannot be initialized.
pub fn run(ffmpeg_path: Option<PathBuf>) -> Result<()> {
    // ---- Enter the alternate screen + raw mode -----------------
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        terminal::EnterAlternateScreen,
        crossterm_event::EnableMouseCapture,
        cursor::Hide,
    )?;
    let backend = backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;
    let _guard = TerminalGuard; // guarantees teardown on every exit path

    // ---- Channels + app state ----------------------------------
    let (tx, rx) = mpsc::channel::<AppEvent>();
    let _input_handle = event::spawn_input_thread(tx.clone());
    let mut app = App::new(ffmpeg_path);

    // ---- Main loop ---------------------------------------------
    let result = event_loop(&mut terminal, &mut app, tx, rx);

    // Explicitly show the cursor again before the guard drops.
    terminal.show_cursor()?;
    result
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Drives the render/update cycle until the app requests a quit.
///
/// `recv_timeout` doubles as our tick source: when nothing arrives for
/// [RECV_TIMEOUT] we advance the spinner animation frame.
fn event_loop(
    terminal: &mut ratatui::Terminal<backend::CrosstermBackend<io::Stdout>>,
    app: &mut App,
    tx: mpsc::Sender<AppEvent>,
    rx: mpsc::Receiver<AppEvent>,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        match rx.recv_timeout(RECV_TIMEOUT) {
            Ok(AppEvent::Key(key)) => {
                app.handle_key(key, &tx);
                if app.should_quit {
                    break;
                }
            }
            Ok(AppEvent::Log(line)) => app.push_log(line),
            Ok(AppEvent::FileStarted(i)) => app.file_started(i),
            Ok(AppEvent::FileDone(i, ok)) => app.file_done(i, ok),
            Ok(AppEvent::AllDone { succeeded, failed }) => app.all_done(succeeded, failed),
            Ok(AppEvent::Tick) | Err(mpsc::RecvTimeoutError::Timeout) => app.tick(),
            Ok(AppEvent::CancelAll) => {
                if let Some(token) = &app.cancel_token {
                    token.cancel();
                }
            }
            Ok(AppEvent::CancelledWithResidual(files)) => app.cancelled_with_residual(files),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}
