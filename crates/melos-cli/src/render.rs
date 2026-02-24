use std::collections::HashMap;

use anyhow::Result;
use colored::{Color, Colorize};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use melos_core::events::Event;

/// Colors assigned to packages for distinguishing concurrent output.
const PKG_COLORS: &[Color] = &[
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Red,
    Color::BrightCyan,
    Color::BrightGreen,
    Color::BrightYellow,
    Color::BrightBlue,
];

/// Create a styled progress bar for package processing.
///
/// Uses a consistent style across all commands:
/// `{spinner} [{bar}] {pos}/{len} {msg}`
pub fn create_progress_bar(total: u64, message: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("=> "),
    );
    pb.set_message(message.to_string());
    pb
}

/// Spawn a renderer task with a progress bar.
///
/// Returns an event sender and a join handle. Drop the sender when done
/// to signal the render loop to finish, then await the handle.
pub fn spawn_renderer(
    total: usize,
    message: &str,
) -> (mpsc::UnboundedSender<Event>, JoinHandle<Result<()>>) {
    let pb = create_progress_bar(total as u64, message);
    let (tx, rx) = mpsc::unbounded_channel();
    let handle = tokio::spawn(async move { render_loop(rx, Some(pb)).await });
    (tx, handle)
}

/// Spawn a renderer task without a progress bar.
///
/// Useful for commands that want colored output but no progress indicator.
pub fn spawn_plain_renderer() -> (mpsc::UnboundedSender<Event>, JoinHandle<Result<()>>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let handle = tokio::spawn(async move { render_loop(rx, None).await });
    (tx, handle)
}

/// Get the color for a package name, assigning a new one if not seen before.
fn pkg_color(color_map: &mut HashMap<String, Color>, color_idx: &mut usize, name: &str) -> Color {
    *color_map.entry(name.to_string()).or_insert_with(|| {
        let c = PKG_COLORS[*color_idx % PKG_COLORS.len()];
        *color_idx += 1;
        c
    })
}

/// Width (in characters) of the separator line drawn around package output.
const SEPARATOR_WIDTH: usize = 60;

/// Build a separator line: `─── pkg_name ─────────────────`
fn separator_line(name: &str, color: Color) -> String {
    let label = format!(" {} ", name);
    let prefix_dashes = 3;
    let suffix_dashes = SEPARATOR_WIDTH.saturating_sub(prefix_dashes + label.len());
    format!(
        "{}{}{}",
        "─".repeat(prefix_dashes).color(color),
        label.color(color).bold(),
        "─".repeat(suffix_dashes).color(color),
    )
}

/// Build a plain closing separator line: `──────────────────`
fn closing_separator(color: Color) -> String {
    format!("{}", "─".repeat(SEPARATOR_WIDTH).color(color))
}

/// Internal render loop that processes events and produces terminal output.
async fn render_loop(
    mut rx: mpsc::UnboundedReceiver<Event>,
    pb: Option<ProgressBar>,
) -> Result<()> {
    let mut color_map: HashMap<String, Color> = HashMap::new();
    let mut color_idx = 0usize;

    while let Some(event) = rx.recv().await {
        match event {
            Event::PackageStarted { ref name } => {
                let color = pkg_color(&mut color_map, &mut color_idx, name);
                println!("{}", separator_line(name, color));
            }
            Event::PackageOutput {
                ref name,
                ref line,
                is_stderr,
            } => {
                let color = pkg_color(&mut color_map, &mut color_idx, name);
                let prefix = format!("[{}]", name).color(color).bold();
                if is_stderr {
                    eprintln!("{} {}", prefix, line);
                } else {
                    println!("{} {}", prefix, line);
                }
            }
            Event::PackageFinished {
                ref name,
                success,
                duration,
            } => {
                let color = pkg_color(&mut color_map, &mut color_idx, name);
                let prefix = format!("[{}]", name).color(color).bold();
                let elapsed = format!("({:.1}s)", duration.as_secs_f64());
                if success {
                    println!("{} {} {}", prefix, "SUCCESS".green(), elapsed.dimmed());
                } else {
                    eprintln!("{} {} {}", prefix, "FAILED".red(), elapsed.dimmed());
                }
                println!("{}", closing_separator(color));
                if let Some(ref pb) = pb {
                    pb.inc(1);
                }
            }
            Event::Progress { ref message, .. } => {
                if let Some(ref pb) = pb {
                    pb.set_message(message.clone());
                }
            }
            Event::Warning(ref msg) => {
                eprintln!("{} {}", "WARNING:".yellow().bold(), msg);
            }
            Event::Info(ref msg) => {
                println!("{}", msg);
            }
            Event::CommandStarted { .. } | Event::CommandFinished { .. } => {
                // Reserved for future use by TUI/JSON frontends
            }
        }
    }

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    Ok(())
}
