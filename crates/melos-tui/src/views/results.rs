use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::views::execution::{build_color_map, render_output_lines};

/// Draw the Done state results panel: summary header, per-package result list,
/// and scrollable output log. Shows red border when an error is present.
pub fn draw_results(frame: &mut Frame, area: Rect, app: &App) {
    let passed = app.finished_packages.iter().filter(|(_, s, _)| *s).count();
    let failed = app.finished_packages.iter().filter(|(_, s, _)| !*s).count();

    let mut summary_lines: Vec<Line<'_>> = Vec::new();

    let cmd_name = app.running_command.as_deref().unwrap_or("command");
    let header_color = if failed > 0 || app.command_error.is_some() {
        Color::Red
    } else {
        Color::Green
    };

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

    // Per-package result list: check/X + name + duration.
    if !app.finished_packages.is_empty() {
        summary_lines.push(Line::from(""));
        for (name, success, duration) in &app.finished_packages {
            let icon = if *success { "+" } else { "x" };
            let icon_color = if *success { Color::Green } else { Color::Red };
            let dur_ms = duration.as_millis();
            let dur_str = if dur_ms >= 1000 {
                format!("{:.1}s", duration.as_secs_f64())
            } else {
                format!("{dur_ms}ms")
            };
            summary_lines.push(Line::from(vec![
                Span::styled(
                    format!(" {icon} "),
                    Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(name.as_str()),
                Span::styled(
                    format!("  ({dur_str})"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    // Blank separator before output log.
    summary_lines.push(Line::from(""));

    let summary_height = summary_lines.len() as u16;

    // Layout: summary (fixed) + output log (fill).
    let [summary_area, output_area] =
        Layout::vertical([Constraint::Length(summary_height), Constraint::Min(0)]).areas(area);

    let summary = Paragraph::new(summary_lines);
    frame.render_widget(summary, summary_area);

    // Scrollable output log.
    let visible_height = output_area.height.saturating_sub(1) as usize;
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

    let border_color = if app.command_error.is_some() {
        Color::Red
    } else {
        Color::Reset
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(border_color))
        .title(title);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, output_area);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui::{backend::TestBackend, Terminal};

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

    /// Extract all lines from the buffer as a single string.
    fn buffer_text(buf: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_results_shows_summary() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.running_command = Some("analyze".to_string());
        app.finished_packages = vec![
            ("pkg_a".to_string(), true, Duration::from_millis(100)),
            ("pkg_b".to_string(), false, Duration::from_millis(200)),
        ];

        let buf = render_frame(draw_results, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("1 passed, 1 failed"),
            "Expected summary, got:\n{text}"
        );
        assert!(
            text.contains("analyze"),
            "Expected command name, got:\n{text}"
        );
    }

    #[test]
    fn test_results_shows_per_package_list() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.running_command = Some("test".to_string());
        app.finished_packages = vec![
            ("pkg_a".to_string(), true, Duration::from_millis(120)),
            ("pkg_b".to_string(), false, Duration::from_secs(2)),
        ];

        let buf = render_frame(draw_results, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("pkg_a") && text.contains("120ms"),
            "Expected pkg_a with duration, got:\n{text}"
        );
        assert!(
            text.contains("pkg_b") && text.contains("2.0s"),
            "Expected pkg_b with duration, got:\n{text}"
        );
    }

    #[test]
    fn test_results_shows_output_log() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.running_command = Some("analyze".to_string());
        app.finished_packages = vec![("pkg_a".to_string(), true, Duration::from_millis(100))];
        app.output_log = vec![("pkg_a".to_string(), "All checks passed".to_string(), false)];

        let buf = render_frame(draw_results, &app, 80, 20);
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
    fn test_results_shows_error_message() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.command_error = Some("connection failed".to_string());

        let buf = render_frame(draw_results, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("Error: connection failed"),
            "Expected error message, got:\n{text}"
        );
    }

    #[test]
    fn test_results_scroll_indicator_when_content_overflows() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.running_command = Some("test".to_string());
        app.finished_packages = vec![("pkg_a".to_string(), true, Duration::from_millis(50))];
        for i in 0..30 {
            app.output_log
                .push(("pkg_a".to_string(), format!("line {i}"), false));
        }

        let buf = render_frame(draw_results, &app, 80, 10);
        let text = buffer_text(&buf, 80, 10);
        assert!(
            text.contains("of 30"),
            "Expected scroll indicator with total 30, got:\n{text}"
        );
    }

    #[test]
    fn test_results_shows_command_name_from_running_command() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.running_command = Some("format".to_string());
        app.finished_packages = vec![("pkg_a".to_string(), true, Duration::from_millis(50))];

        let buf = render_frame(draw_results, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("format"),
            "Expected command name 'format', got:\n{text}"
        );
    }

    #[test]
    fn test_results_empty_packages_shows_zero_counts() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.running_command = Some("bootstrap".to_string());

        let buf = render_frame(draw_results, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("0 passed, 0 failed"),
            "Expected zero counts, got:\n{text}"
        );
    }
}
