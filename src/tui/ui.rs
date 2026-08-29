//! module ui - Pure rendering layer. Reads `App` state and draws widgets.
//! No mutation happens here; that keeps drawing predictable and testable.

use crate::tui::app::{App, Screen, StatusState};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListState, Paragraph},
    Frame,
};

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Accent color used for highlights and selections.
const ACCENT: Color = Color::Cyan;
/// Dim color for secondary text.
const DIM: Color = Color::DarkGray;

// ----------------------------------------- Public API ----------------------------------------- //

/// Draws the entire frame based on the current app state.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(f, app, chunks[0]);
    match app.screen {
        Screen::Home => render_home(f, app, chunks[1]),
        Screen::FilePicker => render_picker(f, app, chunks[1]),
        Screen::Options => render_options(f, app, chunks[1]),
        Screen::Running => render_running(f, app, chunks[1]),
    }
    render_footer(f, app, chunks[2]);
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Renders the boxed header with app title and a breadcrumb.
fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let crumb = match app.screen {
        Screen::Home => "workflows".to_string(),
        Screen::FilePicker => format!("{} · {}", app.command_title(), app.cwd.display()),
        Screen::Options => format!("options · {}", app.command_title()),
        Screen::Running => format!("running · {}", app.command_title()),
    };
    let title = Line::from(vec![
        Span::styled(
            " toolkitrs ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("· terminal media workflows ", Style::default().fg(DIM)),
        Span::styled(format!("· {crumb}"), Style::default().fg(Color::Magenta)),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
        .title(title);
    f.render_widget(block, area);
}

/// Renders the workflow selection list.
fn render_home(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<Line> = app
        .commands
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let selected = i == app.home_index;
            let marker = if selected { "▶" } else { " " };
            let style = if selected {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(vec![
                Span::styled(format!(" {marker} "), style),
                Span::styled(format!("{:<12}", c.title()), style),
                Span::styled(c.description(), Style::default().fg(DIM)),
            ])
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DIM))
            .title(" Choose a workflow "),
    );

    // Use ListState to enable scrolling
    let mut state = ListState::default();
    state.select(Some(app.home_index));
    f.render_stateful_widget(list, area, &mut state);
}

/// Renders the directory/file browser with selection marks.
fn render_picker(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<Line> = app
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let cursor = i == app.picker_index;
            let is_selected = !e.is_dir && app.selected_files.contains(&e.path);
            let icon = if e.is_dir {
                "📁"
            } else if is_selected {
                "✓"
            } else {
                "•"
            };
            let style = if cursor {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(Color::Green)
            } else if e.is_dir {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            Line::from(vec![
                Span::styled(format!(" {} ", if cursor { "▶" } else { " " }), style),
                Span::styled(format!("{icon} "), style),
                Span::styled(&e.name, style),
            ])
        })
        .collect();

    let header = format!(
        " {} selected · {}",
        app.selected_files.len(),
        app.cwd.display()
    );
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DIM))
            .title(header),
    );

    // Use ListState to enable scrolling
    let mut state = ListState::default();
    state.select(Some(app.picker_index));
    f.render_stateful_widget(list, area, &mut state);
}

/// Renders the options list with current values.
fn render_options(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<Line> = app
        .option_rows()
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let cursor = i == app.options_index;
            let is_na_row = text.contains("N/A for this workflow");
            let style = if cursor {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else if is_na_row {
                Style::default().fg(DIM)
            } else {
                Style::default()
            };
            let prefix = if cursor { "▶" } else { " " };
            Line::from(vec![Span::styled(format!(" {prefix} {text}"), style)])
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DIM))
            .title(format!(" Options · {} ", app.command_title())),
    );

    // Use ListState to enable scrolling
    let mut state = ListState::default();
    state.select(Some(app.options_index));
    f.render_stateful_widget(list, area, &mut state);

    // Render inline text editor popup if active
    if let Some((label, buffer, placeholder)) = &app.editing_text {
        // Show buffer if typing, otherwise show placeholder in dim color
        let (display_text, is_placeholder) = if buffer.is_empty() {
            (placeholder.as_str(), true)
        } else {
            (buffer.as_str(), false)
        };

        let text = format!(
            "Editing: {}\n\n> {}█\n\n[Enter] save   [Esc] cancel",
            label, display_text
        );

        let popup = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(" Text Input "),
            )
            .style(Style::default().fg(if is_placeholder {
                Color::DarkGray // Dim placeholder
            } else {
                Color::White // Bright when typing
            }));

        let popup_area = centered_rect(60, 30, area);
        f.render_widget(Clear, popup_area);
        f.render_widget(popup, popup_area);
    }
}

/// Renders the running screen: per-file statuses + live log + progress gauge.
fn render_running(f: &mut Frame, app: &App, area: Rect) {
    // Reserve a bounded region for the file list, rest goes to the log.
    let status_height = (app.file_statuses.len() as u16 + 2).min(10);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(status_height),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);

    render_status_list(f, app, chunks[0]);
    render_log(f, app, chunks[1]);
    render_progress(f, app, chunks[2]);
    if app.show_cleanup_prompt {
        render_cleanup_popup(f, app, area);
    }
}

/// Renders each file with a state icon (pending/running/done/failed).
fn render_status_list(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<Line> = app
        .file_statuses
        .iter()
        .map(|s| {
            let (icon, color) = match s.state {
                StatusState::Pending => ("·", DIM),
                StatusState::Running => (app.spinner(), ACCENT),
                StatusState::Done => ("✔", Color::Green),
                StatusState::Failed => ("✘", Color::Red),
            };
            Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(color)),
                Span::styled(s.path.display().to_string(), Style::default()),
            ])
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DIM))
            .title(" Files "),
    );

    // Use ListState to auto-scroll to the bottom as files finish
    let mut state = ListState::default();
    if !app.file_statuses.is_empty() {
        state.select(Some(app.file_statuses.len() - 1));
    }
    f.render_stateful_widget(list, area, &mut state);
}

/// Renders the scrolling ffmpeg log, pinned to the newest lines.
fn render_log(f: &mut Frame, app: &App, area: Rect) {
    let visible = area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(visible);
    let text = app.log[start..].join("\n");
    let log = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DIM))
            .title(" ffmpeg output "),
    );
    f.render_widget(log, area);
}

/// Renders an overall progress gauge across all files.
fn render_progress(f: &mut Frame, app: &App, area: Rect) {
    let total = app.file_statuses.len();
    let done = app
        .file_statuses
        .iter()
        .filter(|s| matches!(s.state, StatusState::Done | StatusState::Failed))
        .count();
    let ratio = if total == 0 {
        0.0
    } else {
        done as f64 / total as f64
    };

    let label = if app.running {
        format!("{} processing {done}/{total}", app.spinner())
    } else if app.finished {
        format!("done · {} ok · {} failed", app.succeeded, app.failed)
    } else {
        format!("{done}/{total}")
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(DIM))
                .title(" Progress "),
        )
        .gauge_style(Style::default().fg(ACCENT))
        .ratio(ratio)
        .label(Span::styled(
            label,
            Style::default().add_modifier(Modifier::BOLD),
        ));
    f.render_widget(gauge, area);
}

/// Renders the context-sensitive keybinding hints bar.
fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.screen {
        Screen::Home => "↑↓ move · ⏎ select · q quit",
        Screen::FilePicker => {
            "↑↓ move · ⏎ enter dir/confirm · space toggle · ⌃a all · ⌫ up · b back"
        }
        Screen::Options => "↑↓ move · ←→ adjust · ⏎ toggle/start · b back · q quit",
        Screen::Running => {
            if app.show_cleanup_prompt {
                "y remove · n keep"
            } else if app.running {
                "c cancel batch · (Ctrl+C quits)"
            } else {
                "⏎/esc home · q quit"
            }
        }
    };
    let footer = Paragraph::new(Span::styled(hints, Style::default().fg(DIM)));
    f.render_widget(footer, area);
}

/// Renders a centered Yes/No popup for residual file cleanup.
fn render_cleanup_popup(f: &mut Frame, app: &App, area: Rect) {
    let n = app.residual_files.len();
    let text = format!(
        "Batch cancelled. {n} partial files remain.\n\nRemove them?\n\n  [y] Yes, remove   [n] No, keep"
    );
    let popup = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow))
                .title(" Cleanup "),
        )
        .style(Style::default().fg(Color::White));

    // Center the popup in a ~50% wide, ~40% tall rect.
    let popup_area = centered_rect(60, 40, area);
    f.render_widget(popup, popup_area);
}

/// Helper for centering a rect inside `r`.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
