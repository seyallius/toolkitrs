//! module app - Central application state and keyboard-driven transitions.
//! This is the "controller" of the TUI: it owns state and mutates it in
//! response to events, but never renders anything itself.

use crate::tui::command::CommandOption;
use crate::{
    github,
    tui::{command, event::AppEvent},
    workflow::Workflow,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use std::sync::Arc;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
};
use tokio_util::sync::CancellationToken;
// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Spinner animation frames (braille dots).
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Bitrate step used by the options screen.
const BITRATE_STEP: i32 = 64;
/// Minimum bitrate allowed by the options screen.
const BITRATE_MIN: i32 = 64;
/// Maximum bitrate allowed by the options screen.
const BITRATE_MAX: i32 = 640;
/// Cover size step used by the options screen.
const COVER_SIZE_STEP: i32 = 100;
/// Minimum cover size allowed by the options screen.
const COVER_SIZE_MIN: i32 = 100;
/// Maximum cover size allowed by the options screen.
const COVER_SIZE_MAX: i32 = 2000;

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

/// Rows shown on the options screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionsRow {
    ForceOverwrite,
    AudioBitrate,
    CoverSize,
    OutputDirectory,
    ExecutionMode,
    Start,
}
impl OptionsRow {
    /// Returns every row in display order.
    fn all() -> [Self; 6] {
        [
            Self::ForceOverwrite,
            Self::AudioBitrate,
            Self::CoverSize,
            Self::OutputDirectory,
            Self::ExecutionMode,
            Self::Start,
        ]
    }

    /// Returns the row for the given cursor index.
    fn from_index(index: usize) -> Self {
        Self::all().get(index).copied().unwrap_or(Self::Start)
    }

    /// Returns the number of selectable rows.
    fn count() -> usize {
        Self::all().len()
    }
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
    pub parallel: bool,
    /// For text inputs.
    pub custom: HashMap<String, String>,
}
impl Default for RunOptions {
    /// Sensible defaults matching the CLI's defaults.
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("out"),
            force: false,
            bitrate: 320,
            cover_size: 600,
            parallel: true,
            custom: HashMap::new(),
        }
    }
}

/// The whole TUI state machine.
#[derive(Debug)]
pub struct App {
    /// Active screen.
    pub screen: Screen,
    /// All available commands, in display order.
    pub commands: Vec<Arc<dyn command::TuiCommand>>,
    /// Cursor index on the home screen.
    pub home_index: usize,
    /// The command chosen on the home screen.
    pub selected_command: Option<usize>,
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
    pub cancel_token: Option<CancellationToken>,
    /// Residual files left over after a canceled batch (for cleanup prompt).
    pub residual_files: Vec<PathBuf>,
    /// Pending cleanup decision after cancellation.
    pub show_cleanup_prompt: bool,
    /// Inline text editor state: (label, buffer, placeholder)
    pub editing_text: Option<(String, String, String)>,
}
impl App {
    /// Creates a fresh app state starting on the home screen.
    pub fn new(ffmpeg_path: Option<PathBuf>) -> Self {
        let commands: Vec<Arc<dyn command::TuiCommand>> = vec![
            Arc::new(Workflow::Ts2Mp4),
            Arc::new(Workflow::Mkv2Mp3),
            Arc::new(Workflow::Mp32Mp4),
            Arc::new(Workflow::Vidwrap),
            Arc::new(github::tui::GhContribCommand),
        ];

        Self {
            screen: Screen::Home,
            commands,
            home_index: 0,
            selected_command: None,
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
            editing_text: None,
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
        self.cancel_token = None;
        self.push_log(format!("✔ Done: {succeeded} succeeded, {failed} failed"));
    }

    /// Records a cancelled run that left residual files behind.
    pub fn cancelled_with_residual(&mut self, files: Vec<PathBuf>) {
        self.running = false;
        self.finished = true;
        self.residual_files = files;
        self.show_cleanup_prompt = true;
        self.cancel_token = None;
    }

    /// Returns the selected workflow title for headings.
    pub fn command_title(&self) -> &'static str {
        self.selected_command
            .map(|i| self.commands[i].title())
            .unwrap_or("")
    }

    /// Returns the option rows formatted for the UI.
    pub fn option_rows(&self) -> Vec<String> {
        let mut rows = Vec::new();
        let Some(cmd_idx) = self.selected_command else {
            return vec!["Start".to_string()];
        };
        let cmd = &self.commands[cmd_idx];

        for opt in cmd.options() {
            match opt {
                CommandOption::Toggle { label, .. } => {
                    let val = match label {
                        "Force overwrite" => self.options.force,
                        _ => self
                            .options
                            .custom
                            .get(label)
                            .map(|v| v == "true")
                            .unwrap_or(false),
                    };
                    rows.push(format!("{:<20}: {}", label, if val { "ON" } else { "OFF" }));
                }
                CommandOption::Numeric { label, unit, .. } => {
                    let val = match label {
                        "Audio bitrate" => self.options.bitrate.to_string(),
                        "Cover size" => self.options.cover_size.to_string(),
                        _ => "N/A".to_string(),
                    };
                    rows.push(format!("{:<20}: {} {}", label, val, unit));
                }
                CommandOption::Text { label, default, placeholder: _ } => {
                    let val = self
                        .options
                        .custom
                        .get(label)
                        .map(|s| s.as_str())
                        .unwrap_or(default);
                    let display = if val.is_empty() { "<empty>" } else { val };
                    rows.push(format!("{:<20}: {}", label, display));
                }
            }
        }
        if cmd.file_picker_mode() != command::FilePickerMode::NotNeeded {
            rows.push(format!(
                "{:<20}: {}",
                "Output directory",
                self.options.output_dir.display()
            ));
            rows.push(format!(
                "{:<20}: {}",
                "Parallelism",
                if self.options.parallel {
                    "Parallel"
                } else {
                    "Sequential"
                }
            ));
        }
        rows.push("Start".to_string());
        rows
    }

    /// Routes a key event to the active screen's handler.
    pub fn handle_key(&mut self, key: KeyEvent, tx: &Sender<AppEvent>) {
        if self.is_global_quit(key) {
            self.should_quit = true;
            return;
        }

        if self.try_cancel_running_batch(key) {
            return;
        }

        // Inline text editor intercepts keys
        if let Some((label, buffer, _placeholder)) = &mut self.editing_text {
            match key.code {
                KeyCode::Enter => {
                    let l = label.clone();
                    let mut b = buffer.clone();

                    // Parse "today" and "yesterday" keywords
                    if l.contains("Since") || l.contains("Until") {
                        b = parse_date_keyword(&b);
                    }

                    self.options.custom.insert(l, b);
                    self.editing_text = None;
                }
                KeyCode::Esc => {
                    self.editing_text = None;
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                }
                KeyCode::Backspace => {
                    buffer.pop();
                }
                _ => {}
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
        self.selected_command
            .and_then(|i| match self.commands[i].file_picker_mode() {
                command::FilePickerMode::Required { extension, .. } => Some(extension),
                _ => None,
            })
            .unwrap_or("")
    }

    /// Home screen: navigate the workflow list, Enter to pick one.
    fn handle_home(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.home_index = self.home_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.home_index + 1 < self.commands.len() {
                    self.home_index += 1;
                }
            }
            KeyCode::Enter => self.select_current_command(),
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
            KeyCode::Char(' ') => self.toggle_current(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.select_all_files()
            }
            KeyCode::Backspace | KeyCode::Char('h') => self.go_parent(),
            KeyCode::Enter | KeyCode::Char('l') => self.enter_current_or_confirm(),
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
                self.options_index =
                    (self.options_index + 1).min(self.option_rows().len().saturating_sub(1));
            }
            KeyCode::Left => self.adjust_current(-1),
            KeyCode::Right => self.adjust_current(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_current(tx),
            KeyCode::Char('b') | KeyCode::Esc => {
                if self
                    .selected_command
                    .map(|i| self.commands[i].file_picker_mode())
                    == Some(command::FilePickerMode::NotNeeded)
                {
                    self.screen = Screen::Home;
                } else {
                    self.screen = Screen::FilePicker;
                }
            }
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
            return;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Esc => self.reset_to_home(),
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
                if self.should_show_entry(&p) {
                    entries.push(DirEntry {
                        name: entry.file_name().to_string_lossy().into_owned(),
                        path: p,
                        is_dir: entry.file_type().map(|t| t.is_dir()).unwrap_or(false),
                    });
                }
            }
        }
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
        if let Some(parent) = self.cwd.parent().map(Path::to_path_buf) {
            self.load_directory(&parent);
        }
    }

    /// Toggles selection on the file under the cursor.
    fn toggle_current(&mut self) {
        let Some(entry) = self.entries.get(self.picker_index) else {
            return;
        };
        if entry.is_dir {
            return;
        }
        let path = entry.path.clone();
        if !self.selected_files.remove(&path) {
            self.selected_files.insert(path);
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
        self.select_current_file_if_needed();
        if !self.selected_files.is_empty()
            || self
                .selected_command
                .map(|i| self.commands[i].file_picker_mode())
                == Some(command::FilePickerMode::NotNeeded)
        {
            self.options_index = 0;
            self.screen = Screen::Options;
        }
    }

    /// Adjusts the numeric option under the cursor by `delta`'s sign.
    fn adjust_current(&mut self, delta: i32) {
        let rows = self.option_rows();
        let Some(row_text) = rows.get(self.options_index) else {
            return;
        };
        if row_text.starts_with("Audio bitrate") {
            self.options.bitrate = adjust_u32(
                self.options.bitrate,
                delta,
                BITRATE_STEP,
                BITRATE_MIN,
                BITRATE_MAX,
            );
        } else if row_text.starts_with("Cover size") {
            self.options.cover_size = adjust_u32(
                self.options.cover_size,
                delta,
                COVER_SIZE_STEP,
                COVER_SIZE_MIN,
                COVER_SIZE_MAX,
            );
        } else if row_text.starts_with("Parallelism") {
            self.options.parallel = !self.options.parallel;
        }
    }

    /// Activates the options row under the cursor.
    fn activate_current(&mut self, tx: &Sender<AppEvent>) {
        let rows = self.option_rows();
        let Some(row_text) = rows.get(self.options_index) else {
            return;
        };

        if row_text.starts_with("Force overwrite") || row_text.starts_with("Skip README") {
            let label = row_text.split(':').next().unwrap().trim();
            let is_on = row_text.contains("ON");
            self.options
                .custom
                .insert(label.to_string(), (!is_on).to_string());
            if label == "Force overwrite" {
                self.options.force = !is_on;
            }
        } else if row_text == "Start" {
            self.start_run(tx);
        } else if row_text.starts_with("Output directory") {
            //note: Future - could be a directory picker
        } else if row_text.starts_with("Parallelism") {
            self.options.parallel = !self.options.parallel;
        } else {
            // It's a Text option! Open inline editor
            let label = row_text.split(':').next().unwrap().trim();
            let current = self.options.custom.get(label).cloned().unwrap_or_default();

            // ✅ Find the placeholder from the command's options
            let placeholder = self
                .selected_command
                .and_then(|i| {
                    self.commands[i].options().into_iter()
                    .find(|opt| matches!(opt, CommandOption::Text { label: l, .. } if *l == label))
                    .and_then(|opt| match opt {
                        CommandOption::Text { placeholder, .. } => Some(placeholder),
                        _ => None,
                    })
                })
                .unwrap_or("");

            self.editing_text = Some((label.to_string(), current, placeholder.to_string()));
        }
    }

    /// User chose to remove (or keep) residual files after a cancel.
    pub fn handle_cleanup_choice(&mut self, remove: bool) {
        if remove {
            for path in &self.residual_files {
                let _ = fs::remove_file(path);
            }
            self.push_log(format!(
                "🗑️ Removed {} residual files.",
                self.residual_files.len()
            ));
        } else {
            self.push_log(format!(
                "📁 Kept {} residual files.",
                self.residual_files.len()
            ));
        }

        self.residual_files.clear();
        self.show_cleanup_prompt = false;
    }

    /// Builds the file queue and spawns the background Worker.
    fn start_run(&mut self, tx: &Sender<AppEvent>) {
        let Some(cmd_idx) = self.selected_command else {
            return;
        };

        // Clone the Arc pointer to move into the thread safely
        let command = self.commands[cmd_idx].clone();
        let files = self.selected_files_sorted();
        let cancel = CancellationToken::new();
        self.prepare_run(&files, cancel.clone());

        crate::tui::runner::spawn_worker(
            tx.clone(),
            self.ffmpeg_path.clone(),
            command,
            files,
            self.options.clone(),
            cancel,
        );
    }

    /// Returns true when the key means “quit now” regardless of screen.
    fn is_global_quit(&self, key: KeyEvent) -> bool {
        key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
    }

    /// Cancels the active batch when the user presses `c` on the running screen.
    fn try_cancel_running_batch(&mut self, key: KeyEvent) -> bool {
        if self.screen != Screen::Running || !self.running || key.code != KeyCode::Char('c') {
            return false;
        }
        if let Some(token) = &self.cancel_token {
            token.cancel();
            self.push_log("⚠️ Cancellation requested...".into());
        }
        true
    }

    /// Selects the workflow under the home-screen cursor.
    fn select_current_command(&mut self) {
        let cmd_idx = self.home_index;
        let current_dir = self.cwd.clone();
        self.selected_command = Some(cmd_idx);
        self.selected_files.clear();
        self.log.clear();
        self.finished = false;

        if self.commands[cmd_idx].file_picker_mode() == command::FilePickerMode::NotNeeded {
            self.screen = Screen::Options;
        } else {
            self.load_directory(&current_dir);
            self.screen = Screen::FilePicker;
        }
    }

    /// Enters the current directory entry or confirms file selection.
    fn enter_current_or_confirm(&mut self) {
        let Some(entry) = self.entries.get(self.picker_index).cloned() else {
            return;
        };
        if entry.is_dir {
            self.load_directory(&entry.path);
        } else {
            self.confirm_selection();
        }
    }

    /// Returns true when a path should be shown in the file picker.
    fn should_show_entry(&self, path: &Path) -> bool {
        if path.is_dir() {
            return true;
        }
        let ext = self.input_ext();
        if ext.is_empty() {
            return false;
        }
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case(ext))
            .unwrap_or(false)
    }

    /// Selects the current file if the user confirmed with an empty selection.
    fn select_current_file_if_needed(&mut self) {
        if !self.selected_files.is_empty() {
            return;
        }
        let Some(entry) = self.entries.get(self.picker_index) else {
            return;
        };
        if !entry.is_dir {
            self.selected_files.insert(entry.path.clone());
        }
    }

    /// Returns the currently selected options row.
    fn current_options_row(&self) -> OptionsRow {
        OptionsRow::from_index(self.options_index)
    }

    /// Returns selected files sorted for deterministic processing.
    fn selected_files_sorted(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self.selected_files.iter().cloned().collect();
        files.sort();
        files
    }

    /// Prepares the running screen state before the Worker thread starts.
    fn prepare_run(&mut self, files: &[PathBuf], cancel: CancellationToken) {
        self.cancel_token = Some(cancel);
        self.file_statuses = files
            .iter()
            .map(|path| FileStatus {
                path: path.clone(),
                state: StatusState::Pending,
            })
            .collect();
        self.log.clear();
        self.running = true;
        self.finished = false;
        self.succeeded = 0;
        self.failed = 0;
        self.residual_files.clear();
        self.show_cleanup_prompt = false;
        self.screen = Screen::Running;
    }

    /// Returns to the home screen after a finished run.
    fn reset_to_home(&mut self) {
        self.screen = Screen::Home;
        self.finished = false;
        self.show_cleanup_prompt = false;
        self.residual_files.clear();
    }
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Applies a stepped delta to a `u32` field while clamping to a range.
fn adjust_u32(current: u32, delta: i32, step: i32, min: i32, max: i32) -> u32 {
    let next = current as i32 + delta * step;
    next.clamp(min, max) as u32
}

/// Parses special date keywords ("today", "yesterday") into YYYY-MM-DD format.
fn parse_date_keyword(input: &str) -> String {
    match input.trim().to_lowercase().as_str() {
        "today" => chrono::Local::now().format("%Y-%m-%d").to_string(),
        "yesterday" => (chrono::Local::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string(),
        other => other.to_string(),
    }
}
