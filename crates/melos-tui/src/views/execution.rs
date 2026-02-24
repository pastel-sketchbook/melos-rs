use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Padding, Paragraph, Wrap},
};

use crate::app::App;

/// Per-package color palette matching the CLI renderer (10 rotating colors).
const PKG_COLORS: &[Color] = &[
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Red,
    Color::LightCyan,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
];

/// Assign a color to each unique package name based on first-seen order.
pub(crate) fn build_color_map(output_log: &[(String, String, bool)]) -> HashMap<&str, Color> {
    let mut map = HashMap::new();
    let mut idx = 0usize;
    for (name, _, _) in output_log {
        map.entry(name.as_str()).or_insert_with(|| {
            let c = PKG_COLORS[idx % PKG_COLORS.len()];
            idx += 1;
            c
        });
    }
    map
}

/// Render output log lines as styled ratatui Lines.
///
/// Each line is formatted as `[pkg_name] output_text` with the prefix
/// colored per-package, matching the CLI style.
pub(crate) fn render_output_lines<'a>(
    output_log: &'a [(String, String, bool)],
    color_map: &HashMap<&str, Color>,
    scroll: usize,
    visible_height: usize,
) -> Vec<Line<'a>> {
    let end = output_log.len().min(scroll + visible_height);
    let start = scroll.min(end);

    output_log[start..end]
        .iter()
        .map(|(name, line, is_stderr)| {
            let color = color_map
                .get(name.as_str())
                .copied()
                .unwrap_or(Color::White);
            let prefix = Span::styled(
                format!("[{name}] "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            );
            let text_style = if *is_stderr {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            Line::from(vec![prefix, Span::styled(line.as_str(), text_style)])
        })
        .collect()
}

/// Format a duration as `MM:SS` or `H:MM:SS` for display.
fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    let mins = secs / 60;
    let hours = mins / 60;
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, mins % 60, secs % 60)
    } else {
        format!("{}:{:02}", mins, secs % 60)
    }
}

/// Draw the Running state: command title + progress bar + live output.
/// Wrapped in a bordered block matching the results view style.
pub fn draw_running(frame: &mut Frame, area: Rect, app: &App) {
    let cmd_name = app.running_command.as_deref().unwrap_or("command");

    // Outer border matching the results view panels.
    let elapsed_str = match app.elapsed() {
        Some(d) => format!("  {}", format_elapsed(d)),
        None => String::new(),
    };
    let outer_title = format!(" Running: {cmd_name}{elapsed_str} ");
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(outer_title)
        .padding(Padding::horizontal(2));
    let inner_area = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    // Split: progress bar (1) + spacer (1) + output log (fill).
    let [gauge_area, _spacer, output_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner_area);

    // Progress gauge.
    let (completed, total) = match &app.progress {
        Some((done, total, _)) => (*done, *total),
        None => (0, 0),
    };
    let ratio = if total > 0 {
        completed as f64 / total as f64
    } else {
        0.0
    };
    let running_names = if app.running_packages.is_empty() {
        String::new()
    } else {
        format!("  ({})", app.running_packages.join(", "))
    };
    let label = format!("{completed}/{total}{running_names}");
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::NONE))
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(ratio)
        .label(label);
    frame.render_widget(gauge, gauge_area);

    // Live output log: use auto_scroll or manual scroll offset.
    let visible_height = output_area.height as usize;
    let scroll = if app.auto_scroll {
        app.output_log.len().saturating_sub(visible_height)
    } else {
        app.output_scroll
    };
    let color_map = build_color_map(&app.output_log);
    let lines = render_output_lines(&app.output_log, &color_map, scroll, visible_height);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, output_area);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{App, AppState};

    /// Helper: render a frame into a test buffer.
    fn render_frame(
        draw_fn: impl FnOnce(&mut Frame, Rect, &App),
        app: &App,
        width: u16,
        height: u16,
    ) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_fn(frame, area, app);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Extract a line from the buffer as a string.
    fn buffer_line(buf: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect::<String>()
    }

    /// Extract all lines from the buffer as a single string.
    fn buffer_text(buf: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
        (0..height)
            .map(|y| buffer_line(buf, y, width))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_running_shows_progress_gauge() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.running_command = Some("analyze".to_string());
        app.progress = Some((3, 10, "analyze".to_string()));
        app.running_packages = vec!["pkg_a".to_string()];

        let buf = render_frame(draw_running, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("Running") && text.contains("analyze"),
            "Expected title with command name, got:\n{text}"
        );
        assert!(
            text.contains("3/10"),
            "Expected progress text, got:\n{text}"
        );
        assert!(
            text.contains("pkg_a"),
            "Expected running package name, got:\n{text}"
        );
    }

    #[test]
    fn test_running_shows_output_lines() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.running_command = Some("analyze".to_string());
        app.output_log = vec![
            ("pkg_a".to_string(), "Analyzing...".to_string(), false),
            ("pkg_b".to_string(), "No issues".to_string(), false),
        ];

        let buf = render_frame(draw_running, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("[pkg_a]"),
            "Expected pkg_a prefix, got:\n{text}"
        );
        assert!(
            text.contains("Analyzing..."),
            "Expected output text, got:\n{text}"
        );
        assert!(
            text.contains("[pkg_b]"),
            "Expected pkg_b prefix, got:\n{text}"
        );
    }

    #[test]
    fn test_color_map_assigns_unique_colors() {
        let log = vec![
            ("pkg_a".to_string(), "line1".to_string(), false),
            ("pkg_b".to_string(), "line2".to_string(), false),
            ("pkg_a".to_string(), "line3".to_string(), false),
        ];
        let map = build_color_map(&log);
        assert_eq!(map.len(), 2);
        assert_ne!(map["pkg_a"], map["pkg_b"]);
    }

    #[test]
    fn test_color_map_wraps_after_palette_exhausted() {
        let mut log = Vec::new();
        for i in 0..15 {
            log.push((format!("pkg_{i}"), "line".to_string(), false));
        }
        let map = build_color_map(&log);
        assert_eq!(map.len(), 15);
        // Colors wrap after PKG_COLORS.len() (10), so pkg_0 and pkg_10 share a color.
        assert_eq!(map["pkg_0"], map["pkg_10"]);
    }

    // --- format_elapsed tests ---

    #[test]
    fn test_format_elapsed_seconds() {
        assert_eq!(format_elapsed(Duration::from_secs(5)), "0:05");
    }

    #[test]
    fn test_format_elapsed_minutes() {
        assert_eq!(format_elapsed(Duration::from_secs(125)), "2:05");
    }

    #[test]
    fn test_format_elapsed_hours() {
        assert_eq!(format_elapsed(Duration::from_secs(3661)), "1:01:01");
    }

    #[test]
    fn test_format_elapsed_zero() {
        assert_eq!(format_elapsed(Duration::from_secs(0)), "0:00");
    }

    #[test]
    fn test_format_elapsed_exact_minute() {
        assert_eq!(format_elapsed(Duration::from_secs(60)), "1:00");
    }

    // --- Running state renders elapsed time ---

    #[test]
    fn test_running_shows_elapsed_time() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.running_command = Some("analyze".to_string());
        // Set command_start to a known instant in the past.
        app.command_start = Some(std::time::Instant::now() - Duration::from_secs(65));
        let buf = render_frame(draw_running, &app, 60, 10);
        let content = buffer_text(&buf, 60, 10);
        // Should contain "1:05" (65 seconds = 1 min 5 sec).
        assert!(
            content.contains("1:0"),
            "expected elapsed time in output, got: {content}"
        );
    }

    // --- Running state uses auto_scroll vs manual scroll ---

    #[test]
    fn test_running_auto_scroll_shows_tail() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.running_command = Some("test".to_string());
        app.auto_scroll = true;
        // Add 50 lines.
        for i in 0..50 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        // With auto-scroll, the visible area (height ~6 for output) shows the tail.
        let buf = render_frame(draw_running, &app, 40, 10);
        let content = buffer_text(&buf, 40, 10);
        assert!(
            content.contains("line 49"),
            "expected last line visible with auto-scroll, got: {content}"
        );
    }

    #[test]
    fn test_running_manual_scroll_shows_offset() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.running_command = Some("test".to_string());
        app.auto_scroll = false;
        app.output_scroll = 0;
        // Add 50 lines.
        for i in 0..50 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        // With manual scroll at 0, the first lines should be visible.
        let buf = render_frame(draw_running, &app, 40, 10);
        let content = buffer_text(&buf, 40, 10);
        assert!(
            content.contains("line 0"),
            "expected first line visible with manual scroll, got: {content}"
        );
    }
}
