//! module spinner - A non-blocking spinner for stderr output.

use std::{
    io::{self, IsTerminal, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Delay between spinner frame updates in milliseconds.
const SPINNER_DELAY_MS: u64 = 200;

// ------------------------------------------ Types & Impls ------------------------------------- //

/// Supported spinner frame sequences.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum SpinnerStyle {
    Dots,
    Arrow,
    Bounce,
    Pulse,
    Bar,
    Spin,
    Circle,
    Dots2,
    Dots3,
    Dots4,
    Dots5,
    Dots6,
    Dots7,
    Dots8,
    Dots9,
    Dots10,
    Dots11,
    Dots12,
    Dots13,
    Line,
    Line2,
    Pipe,
    SimpleDots,
    SimpleDots2,
    Star,
    Star2,
    Flip,
    Hamburger,
    GrowVertical,
    GrowHorizontal,
    Balloon,
    Balloon2,
    Noise,
    Bounce2,
    BoxBounce,
    BoxBounce2,
    Triangle,
    Arc,
    CircleQuarters,
    CircleHalves,
    SquareCorners,
    SquareQuarters,
    SquareSpin,
    Globe,
    Moon,
    Pinwheel,
    Weather,
    Christmas,
    Grenade,
    Point,
    Layer,
    BetaWave,
    FingerDance,
    FistBump,
    SoccerHeader,
    Mindblown,
    Speaker,
    OrangePulse,
    BluePulse,
    OrangeBluePulse,
    TimeTravel,
    Earth,
    Clock,
}
impl SpinnerStyle {
    /// Returns the frame string for the given index.
    pub fn frame(self, index: usize) -> &'static str {
        let frames: &[&str] = match self {
            Self::Dots => &["⠋", "⠙", "⠹", "⠸"],
            Self::Arrow => &["←", "↖", "↑", "↗", "→", "↘", "↓", "↙"],
            Self::Bounce => &["⠁", "⠂", "⠄", "⠂"],
            Self::Pulse => &["◐", "◓", "◑", "◒"],
            Self::Bar => &["▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"],
            Self::Spin => &["-", "\\", "|", "/"],
            Self::Circle => &["◴", "◷", "◶", "◵"],

            // Simple dots variants
            Self::Dots2 => &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"],
            Self::Dots3 => &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            Self::Dots4 => &[
                "⠄", "⠆", "⠇", "⠋", "⠙", "⠸", "⠰", "⠠", "⠰", "⠸", "⠙", "⠋", "⠇", "⠆",
            ],
            Self::Dots5 => &["⠋", "⠙", "⠚", "⠞", "⠖", "⠦", "⠴", "⠲", "⠳", "⠓"],
            Self::Dots6 => &[
                "⠁", "⠉", "⠙", "⠚", "⠒", "⠂", "⠒", "⠲", "⠴", "⠤", "⠄", "⠄", "⠤", "⠴", "⠲", "⠒",
            ],
            Self::Dots7 => &[
                "⠈", "⠉", "⠋", "⠓", "⠒", "⠐", "⠐", "⠒", "⠖", "⠦", "⠤", "⠠", "⠠", "⠤", "⠦", "⠖",
            ],
            Self::Dots8 => &[
                "⠁", "⠁", "⠉", "⠙", "⠚", "⠒", "⠂", "⠂", "⠒", "⠲", "⠴", "⠤", "⠄", "⠄", "⠤", "⠠",
                "⠠", "⠤", "⠦", "⠖", "⠒", "⠐", "⠐", "⠒", "⠓", "⠋", "⠉", "⠈",
            ],
            Self::Dots9 => &["⢹", "⢺", "⢼", "⣸", "⣇", "⡧", "⡗", "⡏"],
            Self::Dots10 => &["⢄", "⢂", "⢁", "⡁", "⡈", "⡐", "⡠"],
            Self::Dots11 => &["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"],
            Self::Dots12 => &["⢀", "⢠", "⢰", "⢸", "⣸", "⣄", "⣆", "⣇", "⡇", "⡏", "⡗", "⡧"],
            Self::Dots13 => &["⢈", "⢉", "⢋", "⢓", "⢗", "⢧", "⢧", "⢦", "⢖", "⢒", "⢑", "⢑"],

            // Lines and pipes
            Self::Line => &["-", "=", "≡", "="],
            Self::Line2 => &["╴", "╸", "╶", "╺"],
            Self::Pipe => &["┤", "┘", "┴", "└", "├", "┌", "┬", "┐"],

            // Simple dots
            Self::SimpleDots => &[".  ", ".. ", "...", "   "],
            Self::SimpleDots2 => &[".  ", ".. ", "...", " ..", "  .", "   "],

            // Stars
            Self::Star => &["✶", "✸", "✹", "✺", "✹", "✷"],
            Self::Star2 => &["+", "x", "*"],

            // Other shapes
            Self::Flip => &["_", "_", "_", "-", "`", "`", "'", "´", "-", "_", "_", "_"],
            Self::Hamburger => &["☱", "☲", "☴"],
            Self::GrowVertical => &["▁", "▃", "▄", "▅", "▆", "▇", "▆", "▅", "▄", "▃"],
            Self::GrowHorizontal => &["▏", "▎", "▍", "▌", "▋", "▊", "▉", "▊", "▋", "▌", "▍", "▎"],
            Self::Balloon => &[" ", ".", "o", "O", "@", "*", " "],
            Self::Balloon2 => &[".", "o", "O", "°", "O", "o", "."],
            Self::Noise => &["▓", "▒", "░"],
            Self::Bounce2 => &[
                "⠁", "⠂", "⠃", "⠄", "⠅", "⠆", "⠇", "⠈", "⠉", "⠊", "⠋", "⠌", "⠍", "⠎", "⠏",
            ],
            Self::BoxBounce => &["▖", "▘", "▝", "▗"],
            Self::BoxBounce2 => &["▌", "▀", "▐", "▄"],
            Self::Triangle => &["◢", "◣", "◤", "◥"],
            Self::Arc => &["◜", "◝", "◞", "◟"],
            Self::CircleQuarters => &["◴", "◷", "◶", "◵"],
            Self::CircleHalves => &["◐", "◓", "◑", "◒"],
            Self::SquareCorners => &["◰", "◳", "◲", "◱"],
            Self::SquareQuarters => &["◖", "◗", "◖", "◗"],
            Self::SquareSpin => &["▤", "▥", "▦", "▧", "▨", "▩", "▨", "▧", "▦", "▥"],
            Self::Globe => &["🌍", "🌎", "🌏"],
            Self::Moon => &["🌑", "🌒", "🌓", "🌔", "🌕", "🌖", "🌗", "🌘"],
            Self::Pinwheel => &["🌀", "🌪"],

            // Fun/novelty
            Self::Weather => &["☀️", "☁️", "🌤️", "⛅", "🌥️"],
            Self::Christmas => &["🎄", "🎅", "⭐", "❄️", "🎁"],
            Self::Grenade => &["•", "•", "•", "•", "•", "💥"],
            Self::Point => &["👉", "👉", "👉", "👉", "👈"],
            Self::Layer => &["-", "=", "≡"],
            Self::BetaWave => &["ρ", "β", "β", "β"],
            Self::FingerDance => &["👉", "👆", "🖕", "👇", "👈", "👉"],
            Self::FistBump => &["👊", "🤛", "🤜", "👊"],
            Self::SoccerHeader => &["⚽", "⏹️", "⚽", "⏹️"],
            Self::Mindblown => &["😐", "😮", "😲", "🤯"],
            Self::Speaker => &["🔇", "🔈", "🔉", "🔊", "🔉", "🔈"],
            Self::OrangePulse => &["🟧", "🟨", "🟧"],
            Self::BluePulse => &["🟦", "🟩", "🟦"],
            Self::OrangeBluePulse => &["🟧", "🟦", "🟧"],
            Self::TimeTravel => &[
                "🕐", "🕑", "🕒", "🕓", "🕔", "🕕", "🕖", "🕗", "🕘", "🕙", "🕚", "🕛",
            ],
            Self::Earth => &["🌏", "🌍", "🌎"],
            Self::Clock => &[
                "🕐", "🕑", "🕒", "🕓", "🕔", "🕕", "🕖", "🕗", "🕘", "🕙", "🕚", "🕛",
            ],
        };
        frames[index % frames.len()]
    }
}

/// A lightweight non-blocking stderr spinner. It is disabled on non-terminals.
pub struct Spinner {
    stop_signal: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    enabled: bool,
}
impl Spinner {
    /// Starts a new spinner with the given style and message.
    ///
    /// # Arguments
    /// * `style` - The spinner frame style.
    /// * `message` - The message to display next to the spinner.
    /// * `force` - If true, enable spinner even on non-terminals.
    ///
    /// # Returns
    /// A `Spinner` instance that must be stopped to clean up the thread.
    pub fn start(style: SpinnerStyle, message: String, force: bool) -> Self {
        let enabled = force || io::stderr().is_terminal();
        let stop_signal = Arc::new(AtomicBool::new(false));
        let handle = if enabled {
            let signal = Arc::clone(&stop_signal);
            Some(thread::spawn(move || {
                let mut index = 0;
                while !signal.load(Ordering::Relaxed) {
                    eprint!("\r{} {message}", style.frame(index));
                    let _ = io::stderr().flush();
                    index += 1;
                    thread::sleep(Duration::from_millis(SPINNER_DELAY_MS));
                }
                eprint!("\r{}\r", " ".repeat(message.len() + 4));
                let _ = io::stderr().flush();
            }))
        } else {
            None
        };
        Self {
            stop_signal,
            handle,
            enabled,
        }
    }

    /// Returns whether the spinner is enabled (i.e., outputting frames).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Stops the spinner and waits for the thread to finish.
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Signals the render thread to stop and joins it.
    ///
    /// Shared by [`Spinner::stop`] and the `Drop` impl so both paths tear
    /// down exactly the same way.
    fn shutdown(&mut self) {
        self.stop_signal.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl Drop for Spinner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cycles_frames() {
        assert_eq!(SpinnerStyle::Spin.frame(0), "-");
        assert_eq!(SpinnerStyle::Spin.frame(4), "-");
    }
}
