//! module event - Event types and the background keyboard listener.
//! Keyboard polling runs on its own thread so the UI never blocks on input.

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use std::{
    path::PathBuf,
    sync::mpsc::Sender,
    thread::{self, JoinHandle},
    time::Duration,
};

/// Everything the event loop can react to.
///
/// Terminal events (keys/ticks) and process events (logs/progress)
/// share one channel, which keeps the main loop a single `match`.
#[derive(Debug)]
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// Periodic tick for animating the spinner.
    Tick,
    /// A line of ffmpeg output to append to the log pane.
    Log(String),
    /// File at index `usize` started processing.
    FileStarted(usize),
    /// File at index `usize` finished (`bool` = success).
    FileDone(usize, bool),
    /// Whole batch finished with counts.
    AllDone { succeeded: usize, failed: usize },
    /// User requested cancellation of the whole batch.
    CancelAll,
    /// Batch was cancelled; reports residual files for cleanup decision.
    CancelledWithResidual(Vec<PathBuf>),
}

/// Spawns a thread that forwards pressed keys and periodic ticks to `tx`.
///
/// We filter on [KeyEventKind::Press] because some platforms deliver
/// Repeat/Release events too, which would double-fire actions.
pub fn spawn_input_thread(tx: Sender<AppEvent>) -> JoinHandle<()> {
    thread::spawn(move || loop {
        const POLL_TIMEOUT: Duration = Duration::from_millis(200);

        let polled = event::poll(POLL_TIMEOUT).unwrap_or(false);
        if polled {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press && tx.send(AppEvent::Key(key)).is_err() {
                    break;
                }
            }
        } else if tx.send(AppEvent::Tick).is_err() {
            break;
        }
    })
}
