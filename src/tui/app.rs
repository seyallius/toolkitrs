//! module app - Central application state and keyboard-driven transitions.
//! This is the "controller" of the TUI: it owns state and mutates it in
//! response to events, but never renders anything itself.

use crate::tui::{event::AppEvent, runner, workflow::Workflow};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
};
use tokio_util::sync::CancellationToken;

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Number of selectable rows on the options screen.
const OPTIONS_ROWS: usize = 6;
/// Spinner animation frames (braille dots).
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ------------------------------------------ Types & Impls ------------------------------------- //

/// Which screen is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    FilePicker,
    Options,
    Running,
}

/// A single entry shown in the file picker.
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// Absolute path of the entry.
    pub path: PathBuf,
    /// Display name (file or directory name).
    pub name: String,
    /// Whether this entry is a directory (navigable).
    pub is_dir: bool,
}

/// Per-file processing state shown on the running screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusState {
    Pending,
    Running,
    Done,
    Failed,
}

/// File + its current status, in processing order.
#[derive(Debug, Clone)]
pub struct FileStatus {
    /// The input file this status describes.
    pub path: PathBuf,
    /// Current processing state.
    pub state: StatusState,
}

/// Runtime options the user can tweak before launching a batch.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Directory where outputs are written (ignored by vidwrap).
    pub output_dir: PathBuf,
    /// Overwrite existing outputs.
    pub force: bool,
    /// Audio bitrate in kbps (audio workflows only).
    pub bitrate: u32,
    /// Cover art square size in pixels (cover workflows only).
    pub cover_size: u32,
}
impl Default for RunOptions {
    /// Sensible defaults matching the CLI's defaults.
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("out"),
            force: false,
            bitrate: 320,
            cover_size: 600,
        }
    }
}

/// The whole TUI state machine.
#[derive(Debug)]
pub struct App {
    /// Active screen.
    pub screen: Screen,
    /// All workflows, in display order.
    pub workflows: Vec<Workflow>,
    /// Cursor index on the home screen.
    pub home_index: usize,
    /// The workflow chosen on the home screen.
    pub selected_workflow: Option<Workflow>,
    /// Directory currently browsed by the picker.
    pub cwd: PathBuf,
    /// Entries in `cwd` (dirs first, then matching files).
    pub entries: Vec<DirEntry>,
    /// Cursor index in the picker.
    pub picker_index: usize,
    /// Set of files the user has toggled on.
    pub selected_files: HashSet<PathBuf>,
    /// User-tunable run options.
    pub options: RunOptions,
    /// Cursor index on the options screen.
    pub options_index: usize,
    /// Scrolling ffmpeg log lines.
    pub log: Vec<String>,
    /// Per-file statuses for the running screen.
    pub file_statuses: Vec<FileStatus>,
    /// Whether a batch is currently executing.
    pub running: bool,
    /// Whether a batch has completed.
    pub finished: bool,
    /// Successful file count after a run.
    pub succeeded: usize,
    /// Failed file count after a run.
    pub failed: usize,
    /// Set to true to exit the event loop.
    pub should_quit: bool,
    /// Spinner animation counter.
    pub spinner_frame: usize,
    /// Resolved ffmpeg binary path.
    pub ffmpeg_path: PathBuf,
    /// Whether to run in parallel (true) or sequentially (false).
    pub parallel: bool,
    /// Cancellation token for the current batch (None when idle).
    pub cancel_token: Option<tokio_util::sync::CancellationToken>,
    /// Residual files left over after a canceled batch (for cleanup prompt).
    pub residual_files: Vec<PathBuf>,
    /// Pending cleanup decision after cancellation.
    pub show_cleanup_prompt: bool,
}
impl App {
    /// Creates a fresh app state starting on the home screen.
    pub fn new(ffmpeg_path: Option<PathBuf>) -> Self {
        Self {
            screen: Screen::Home,
            workflows: Workflow::all(),
            home_index: 0,
            selected_workflow: None,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            entries: Vec::new(),
            picker_index: 0,
            selected_files: HashSet::new(),
            options: RunOptions::default(),
            options_index: 0,
            log: Vec::new(),
            file_statuses: Vec::new(),
            running: false,
            finished: false,
            succeeded: 0,
            failed: 0,
            should_quit: false,
            spinner_frame: 0,
            ffmpeg_path: ffmpeg_path.unwrap_or_else(|| PathBuf::from("ffmpeg")),
            parallel: true,
            cancel_token: None,
            residual_files: Vec::new(),
            show_cleanup_prompt: false,
        }
    }

    /// Advances the spinner animation by one frame.
    pub fn tick(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
    }

    /// Returns the current spinner glyph.
    pub fn spinner(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_frame]
    }

    /// Appends a line to the log pane.
    pub fn push_log(&mut self, line: String) {
        self.log.push(line);
    }

    /// Marks the file at `index` as running.
    pub fn file_started(&mut self, index: usize) {
        if let Some(status) = self.file_statuses.get_mut(index) {
            status.state = StatusState::Running;
        }
    }

    /// Marks the file at `index` done/failed based on `ok`.
    pub fn file_done(&mut self, index: usize, ok: bool) {
        if let Some(status) = self.file_statuses.get_mut(index) {
            status.state = if ok {
                StatusState::Done
            } else {
                StatusState::Failed
            };
        }
    }

    /// Finalizes the run with aggregate counts.
    pub fn all_done(&mut self, succeeded: usize, failed: usize) {
        self.running = false;
        self.finished = true;
        self.succeeded = succeeded;
        self.failed = failed;
        self.push_log(format!("✔ Done: {succeeded} succeeded, {failed} failed"));
    }

    /// Routes a key event to the active screen's handler.
    pub fn handle_key(&mut self, key: KeyEvent, tx: &Sender<AppEvent>) {
        // Global quit: Ctrl+C always exits.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        // 'c' cancels the running batch.
        if self.screen == Screen::Running && self.running && key.code == KeyCode::Char('c') {
            if let Some(token) = &self.cancel_token {
                token.cancel();
                self.push_log("⊙ Cancellation requested...".into());
            }
            return;
        }
        match self.screen {
            Screen::Home => self.handle_home(key),
            Screen::FilePicker => self.handle_picker(key),
            Screen::Options => self.handle_options(key, tx),
            Screen::Running => self.handle_running(key),
        }
    }
}
impl App {
    /// Returns the picker's file-extension filter for the chosen workflow.
    fn input_ext(&self) -> &'static str {
        self.selected_workflow
            .map(|w| w.input_extension())
            .unwrap_or("")
    }

    /// Home screen: navigate the workflow list, Enter to pick one.
    fn handle_home(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.home_index = self.home_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.home_index + 1 < self.workflows.len() {
                    self.home_index += 1;
                }
            }
            KeyCode::Enter => {
                let wf = self.workflows[self.home_index];
                self.selected_workflow = Some(wf);
                self.selected_files.clear();
                self.log.clear();
                self.finished = false;
                self.load_directory(&self.cwd.clone());
                self.screen = Screen::FilePicker;
            }
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
    }

    /// File picker: browse directories, toggle files, Enter to confirm.
    fn handle_picker(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.picker_index = self.picker_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.picker_index + 1 < self.entries.len() {
                    self.picker_index += 1;
                }
            }
            KeyCode::Char(' ') => {
                self.toggle_current();
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.select_all_files();
            }
            KeyCode::Backspace | KeyCode::Char('h') => self.go_parent(),
            KeyCode::Enter | KeyCode::Char('l') => {
                // Enter on a directory navigates into it; otherwise confirm.
                let is_dir = self
                    .entries
                    .get(self.picker_index)
                    .map(|e| e.is_dir)
                    .unwrap_or(false);
                if is_dir {
                    let target = self.entries[self.picker_index].path.clone();
                    self.load_directory(&target);
                } else {
                    self.confirm_selection();
                }
            }
            KeyCode::Char('b') | KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Char('q') => self.should_quit = true,
            _ => {}
        }
    }

    /// Options screen: tweak settings, Enter on Start to launch.
    fn handle_options(&mut self, key: KeyEvent, tx: &Sender<AppEvent>) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.options_index = self.options_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.options_index + 1 < OPTIONS_ROWS {
                    self.options_index += 1;
                }
            }
            KeyCode::Left => self.adjust_current(-1),
            KeyCode::Right => self.adjust_current(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_current(tx),
            KeyCode::Char('b') | KeyCode::Esc => self.screen = Screen::FilePicker,
            KeyCode::Char('q') => self.should_quit = true,
            _ => {}
        }
    }

    /// Running screen: mostly read-only; Enter/Esc after finishing goes home.
    fn handle_running(&mut self, key: KeyEvent) {
        if self.show_cleanup_prompt {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => self.handle_cleanup_choice(true),
                KeyCode::Char('n') => self.handle_cleanup_choice(false),
                _ => {}
            }
            return;
        }
        if self.running {
            return; // ignore input mid-run for v1
        }
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.screen = Screen::Home;
                self.finished = false;
            }
            KeyCode::Char('q') => self.should_quit = true,
            _ => {}
        }
    }

    /// Loads and sorts the picker entries for `path`.
    fn load_directory(&mut self, path: &Path) {
        let mut entries = Vec::new();
        if let Ok(read_dir) = fs::read_dir(path) {
            for entry in read_dir.flatten() {
                let p = entry.path();
                let is_dir = p.is_dir();
                let matches_ext = !is_dir
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case(self.input_ext()))
                        .unwrap_or(false);
                if is_dir || matches_ext {
                    entries.push(DirEntry {
                        name: entry.file_name().to_string_lossy().into_owned(),
                        path: p,
                        is_dir,
                    });
                }
            }
        }
        // Directories first, then alphabetical.
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
        self.cwd = path.to_path_buf();
        self.entries = entries;
        self.picker_index = 0;
    }

    /// Moves the picker up to the parent directory.
    fn go_parent(&mut self) {
        if let Some(parent) = self.cwd.parent().map(|p| p.to_path_buf()) {
            self.load_directory(&parent);
        }
    }

    /// Toggles selection on the file under the cursor.
    fn toggle_current(&mut self) {
        if let Some(entry) = self.entries.get(self.picker_index) {
            if !entry.is_dir {
                let path = entry.path.clone();
                if !self.selected_files.remove(&path) {
                    self.selected_files.insert(path);
                }
            }
        }
    }

    /// Selects every visible file in the current directory.
    fn select_all_files(&mut self) {
        for entry in &self.entries {
            if !entry.is_dir {
                self.selected_files.insert(entry.path.clone());
            }
        }
    }

    /// Confirms the picker selection and moves to the options screen.
    fn confirm_selection(&mut self) {
        // If nothing is selected but the cursor sits on a file, grab it.
        if self.selected_files.is_empty() {
            if let Some(entry) = self.entries.get(self.picker_index) {
                if !entry.is_dir {
                    self.selected_files.insert(entry.path.clone());
                }
            }
        }
        if !self.selected_files.is_empty() {
            self.options_index = 0;
            self.screen = Screen::Options;
        }
    }

    /// Adjusts the numeric option under the cursor by `delta`'s sign.
    fn adjust_current(&mut self, delta: i32) {
        match self.options_index {
            1 => {
                // bitrate row (±64 kbps, clamped to a sane range)
                let next = self.options.bitrate as i32 + delta * 64;
                self.options.bitrate = next.clamp(64, 640) as u32;
            }
            2 => {
                // cover size row (±100 px)
                let next = self.options.cover_size as i32 + delta * 100;
                self.options.cover_size = next.clamp(100, 2000) as u32;
            }
            4 => {
                // parallelism toggle row (left/right flips it)
                self.parallel = !self.parallel;
            }
            _ => {}
        }
    }

    /// Activates the options row under the cursor.
    fn activate_current(&mut self, tx: &Sender<AppEvent>) {
        match self.options_index {
            0 => self.options.force = !self.options.force,
            4 => self.parallel = !self.parallel,
            5 => self.start_run(tx), // "Start" row moved to index 5
            _ => {}
        }
    }

    /// User chose to remove (or keep) residual files after a cancel.
    pub fn handle_cleanup_choice(&mut self, remove: bool) {
        if remove {
            for p in &self.residual_files {
                let _ = fs::remove_file(p);
            }
            self.push_log(format!(
                "✔ Removed {} residual files.",
                self.residual_files.len()
            ));
        } else {
            self.push_log(format!(
                "⊗ Kept {} residual files.",
                self.residual_files.len()
            ));
        }
        self.residual_files.clear();
        self.show_cleanup_prompt = false;
        // Stay on Running screen showing the final state; user presses Enter to go home.
    }

    /// Builds the file queue and spawns the background worker.
    fn start_run(&mut self, tx: &Sender<AppEvent>) {
        let Some(workflow) = self.selected_workflow else {
            return;
        };
        let mut files: Vec<PathBuf> = self.selected_files.iter().cloned().collect();
        files.sort(); // deterministic order

        let cancel = CancellationToken::new();
        self.cancel_token = Some(cancel.clone());

        self.file_statuses = files
            .iter()
            .map(|p| FileStatus {
                path: p.clone(),
                state: StatusState::Pending,
            })
            .collect();
        self.log.clear();
        self.running = true;
        self.finished = false;
        self.residual_files.clear();
        self.show_cleanup_prompt = false;
        self.screen = Screen::Running;

        runner::spawn_worker(
            tx.clone(),
            self.ffmpeg_path.clone(),
            workflow,
            files,
            self.options.clone(),
            self.parallel,
            cancel,
        );
    }
}
