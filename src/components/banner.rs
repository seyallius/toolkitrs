//! module banner - Renders a boxed heading for command output.

use console::Style;
use std::io::{self, IsTerminal};

// ----------------------------------------- Public API ----------------------------------------- //

/// Renders a boxed heading suitable for any command. with an optional subtitle.
///
/// # Arguments
/// * `title` - Main title text.
/// * `subtitle` - Optional subtitle text.
/// * `color` - Whether to apply ANSI color styling.
///
/// # Returns
/// A string containing the boxed banner (or a simple separator if piped).
pub fn render(title: &str, subtitle: Option<&str>, color: bool) -> String {
    let content = match subtitle {
        Some(value) => format!("{title} — {value}"),
        None => title.to_owned(),
    };

    // If piped to a file/CI, don't use multi-line unicode boxes
    if !io::stdout().is_terminal() {
        return format!("== {content} ==");
    }

    render_box(&content, color)
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Renders the unicode-box banner form used on terminals.
///
/// Split out from [`render`] so the box layout itself is testable without
/// depending on whether the test runner owns a TTY.
fn render_box(content: &str, color: bool) -> String {
    let border = "═".repeat(content.chars().count() + 2);
    let title = if color {
        Style::new().cyan().bold().apply_to(content).to_string()
    } else {
        content.to_owned()
    };
    format!("╔{border}╗\n║ {title} ║\n╚{border}╝")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_box() {
        assert!(render_box("Hello", false).contains("║ Hello ║"));
    }
}
