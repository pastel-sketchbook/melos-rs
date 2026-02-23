use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Wrap},
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
fn build_color_map(output_log: &[(String, String, bool)]) -> HashMap<&str, Color> {
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
fn render_output_lines<'a>(
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

/// Draw the Running state: command title + progress bar + live output.
pub fn draw_running(frame: &mut Frame, area: Rect, app: &App) {
    let cmd_name = app.running_command.as_deref().unwrap_or("command");

    // Split: title (1) + progress bar (1) + spacer (1) + output log (fill).
    let [title_area, gauge_area, _spacer, output_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);

    // Command title.
    let title = Line::from(vec![
        Span::styled("Running ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            cmd_name,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(title), title_area);

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

    // Live output log: always auto-scroll to the bottom.
    let visible_height = output_area.height as usize;
    let auto_scroll = app.output_log.len().saturating_sub(visible_height);
    let color_map = build_color_map(&app.output_log);
    let lines = render_output_lines(&app.output_log, &color_map, auto_scroll, visible_height);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, output_area);
}

/// Draw the Done state: summary header + scrollable output log.
pub fn draw_done(frame: &mut Frame, area: Rect, app: &App) {
    let passed = app.finished_packages.iter().filter(|(_, s, _)| *s).count();
    let failed = app.finished_packages.iter().filter(|(_, s, _)| !*s).count();

    // Per-package result lines.
    let mut summary_lines: Vec<Line<'_>> = Vec::new();

    let cmd_name = app.running_command.as_deref().unwrap_or("command");
    let header_color = if failed > 0 { Color::Red } else { Color::Green };
    summary_lines.push(Line::from(Span::styled(
        format!("{cmd_name}: {passed} passed, {failed} failed"),
        Style::default()
            .fg(header_color)
            .add_modifier(Modifier::BOLD),
    )));

    if let Some(ref err) = app.command_error {
        summary_lines.push(Line::from(Span::styled(
            format!("Error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    // Blank separator.
    summary_lines.push(Line::from(""));

    let summary_height = summary_lines.len() as u16;

    // Layout: summary (fixed) + output log (fill).
    let [summary_area, output_area] =
        Layout::vertical([Constraint::Length(summary_height), Constraint::Min(0)]).areas(area);

    let summary = Paragraph::new(summary_lines);
    frame.render_widget(summary, summary_area);

    // Scrollable output log.
    let visible_height = output_area.height as usize;
    let color_map = build_color_map(&app.output_log);
    let lines = render_output_lines(
        &app.output_log,
        &color_map,
        app.output_scroll,
        visible_height,
    );

    // Show scroll position indicator in the border title if scrolling is possible.
    let total_lines = app.output_log.len();
    let title = if total_lines > visible_height {
        let end_line = (app.output_scroll + visible_height).min(total_lines);
        format!(
            " Output ({}-{} of {}) ",
            app.output_scroll + 1,
            end_line,
            total_lines
        )
    } else {
        format!(" Output ({total_lines} lines) ")
    };

    let block = Block::default().borders(Borders::TOP).title(title);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
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
    fn test_done_shows_summary() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.finished_packages = vec![
            ("pkg_a".to_string(), true, Duration::from_millis(100)),
            ("pkg_b".to_string(), false, Duration::from_millis(200)),
        ];

        let buf = render_frame(draw_done, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("1 passed, 1 failed"),
            "Expected summary, got:\n{text}"
        );
    }

    #[test]
    fn test_done_shows_output_log() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.finished_packages = vec![("pkg_a".to_string(), true, Duration::from_millis(100))];
        app.output_log = vec![("pkg_a".to_string(), "All checks passed".to_string(), false)];

        let buf = render_frame(draw_done, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("[pkg_a]"),
            "Expected pkg_a prefix, got:\n{text}"
        );
        assert!(
            text.contains("All checks passed"),
            "Expected output text, got:\n{text}"
        );
    }

    #[test]
    fn test_done_shows_error_message() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.command_error = Some("connection failed".to_string());

        let buf = render_frame(draw_done, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("Error: connection failed"),
            "Expected error message, got:\n{text}"
        );
    }

    #[test]
    fn test_done_scroll_indicator_when_content_overflows() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.finished_packages = vec![("pkg_a".to_string(), true, Duration::from_millis(50))];
        // Create more output lines than visible height.
        for i in 0..30 {
            app.output_log
                .push(("pkg_a".to_string(), format!("line {i}"), false));
        }

        let buf = render_frame(draw_done, &app, 80, 10);
        let text = buffer_text(&buf, 80, 10);
        // Should show "Output (1-N of 30)" scroll indicator.
        assert!(
            text.contains("of 30"),
            "Expected scroll indicator with total 30, got:\n{text}"
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
}
