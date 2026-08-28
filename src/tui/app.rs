//! module app - Central application state and keyboard-driven transitions.
//! This is the "controller" of the TUI: it owns state and mutates it in
//! response to events, but never renders anything itself.

use crate::{
    tui::{event::AppEvent, runner},
    workflow::Workflow,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
    pub fn workflow_title(&self) -> &'static str {
        self.selected_workflow
            .map(|workflow| workflow.title())
            .unwrap_or("")
    }

    /// Returns true when the selected workflow uses bitrate settings.
    pub fn workflow_uses_bitrate(&self) -> bool {
        self.selected_workflow
            .is_some_and(|workflow| workflow.uses_bitrate())
    }

    /// Returns true when the selected workflow uses cover-size settings.
    pub fn workflow_uses_cover_size(&self) -> bool {
        self.selected_workflow
            .is_some_and(|workflow| workflow.uses_cover_size())
    }

    /// Returns the option rows formatted for the UI.
    pub fn option_rows(&self) -> Vec<String> {
        vec![
            format!(
                "Force overwrite      : {}",
                if self.options.force { "ON" } else { "OFF" }
            ),
            self.bitrate_row_text(),
            self.cover_size_row_text(),
            format!(
                "Output directory     : {}",
                self.options.output_dir.display()
            ),
            format!(
                "Parallelism          : {}",
                if self.parallel {
                    "Parallel"
                } else {
                    "Sequential"
                }
            ),
            "Start".to_string(),
        ]
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
            .map(|workflow| workflow.input_extension())
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
            KeyCode::Enter => self.select_current_workflow(),
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
                self.select_all_files();
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
                self.options_index = (self.options_index + 1).min(OptionsRow::count() - 1);
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
                let path = entry.path();
                if self.should_show_entry(&path) {
                    entries.push(DirEntry {
                        name: entry.file_name().to_string_lossy().into_owned(),
                        path,
                        is_dir: entry
                            .file_type()
                            .map(|file_type| file_type.is_dir())
                            .unwrap_or(false),
                    });
                }
            }
        }

        entries.sort_by(|left, right| match (left.is_dir, right.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => left.name.cmp(&right.name),
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
        if !self.selected_files.is_empty() {
            self.options_index = 0;
            self.screen = Screen::Options;
        }
    }

    /// Adjusts the numeric option under the cursor by `delta`'s sign.
    fn adjust_current(&mut self, delta: i32) {
        match self.current_options_row() {
            OptionsRow::AudioBitrate if self.workflow_uses_bitrate() => {
                self.options.bitrate = adjust_u32(
                    self.options.bitrate,
                    delta,
                    BITRATE_STEP,
                    BITRATE_MIN,
                    BITRATE_MAX,
                );
            }
            OptionsRow::CoverSize if self.workflow_uses_cover_size() => {
                self.options.cover_size = adjust_u32(
                    self.options.cover_size,
                    delta,
                    COVER_SIZE_STEP,
                    COVER_SIZE_MIN,
                    COVER_SIZE_MAX,
                );
            }
            OptionsRow::ExecutionMode => {
                self.parallel = !self.parallel;
            }
            _ => {}
        }
    }

    /// Activates the options row under the cursor.
    fn activate_current(&mut self, tx: &Sender<AppEvent>) {
        match self.current_options_row() {
            OptionsRow::ForceOverwrite => self.options.force = !self.options.force,
            OptionsRow::ExecutionMode => self.parallel = !self.parallel,
            OptionsRow::Start => self.start_run(tx),
            OptionsRow::AudioBitrate | OptionsRow::CoverSize | OptionsRow::OutputDirectory => {}
        }
    }

    /// User chose to remove (or keep) residual files after a cancel.
    pub fn handle_cleanup_choice(&mut self, remove: bool) {
        if remove {
            for path in &self.residual_files {
                let _ = fs::remove_file(path);
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
    }

    /// Builds the file queue and spawns the background Worker.
    fn start_run(&mut self, tx: &Sender<AppEvent>) {
        let Some(workflow) = self.selected_workflow else {
            return;
        };

        let files = self.selected_files_sorted();
        let cancel = CancellationToken::new();
        self.prepare_run(&files, cancel.clone());

        runner::spawn_Worker(
            tx.clone(),
            self.ffmpeg_path.clone(),
            workflow,
            files,
            self.options.clone(),
            self.parallel,
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
            self.push_log("⊙ Cancellation requested...".into());
        }

        true
    }

    /// Selects the workflow under the home-screen cursor.
    fn select_current_workflow(&mut self) {
        let workflow = self.workflows[self.home_index];
        let current_dir = self.cwd.clone();

        self.selected_workflow = Some(workflow);
        self.selected_files.clear();
        self.log.clear();
        self.finished = false;
        self.load_directory(&current_dir);
        self.screen = Screen::FilePicker;
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

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case(self.input_ext()))
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

    /// Formats the bitrate row based on the selected workflow.
    fn bitrate_row_text(&self) -> String {
        if self.workflow_uses_bitrate() {
            format!("Audio bitrate        : {} kbps", self.options.bitrate)
        } else {
            "Audio bitrate        : N/A for this workflow".to_string()
        }
    }

    /// Formats the cover-size row based on the selected workflow.
    fn cover_size_row_text(&self) -> String {
        if self.workflow_uses_cover_size() {
            format!("Cover size           : {} px", self.options.cover_size)
        } else {
            "Cover size           : N/A for this workflow".to_string()
        }
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
