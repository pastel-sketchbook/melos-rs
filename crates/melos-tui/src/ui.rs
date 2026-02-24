use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{ActivePanel, App, AppState};
use crate::views::commands::draw_commands;
use crate::views::execution::draw_running;
use crate::views::health::draw_health;
use crate::views::help::draw_help;
use crate::views::options::draw_options;
use crate::views::packages::draw_packages;
use crate::views::results::draw_results;

/// Render the entire UI for the current frame.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Five-row layout: header (1), top spacer (1), body (fill), bottom spacer (1), footer (1).
    let [
        header_area,
        _top_spacer,
        body_area,
        _bottom_spacer,
        footer_area,
    ] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, header_area, app);
    draw_body(frame, body_area, app);
    draw_footer(frame, footer_area, app);

    // Help overlay on top of everything.
    if app.show_help {
        draw_help(frame, area, app);
    }

    // Options overlay on top of everything (but below help).
    if app.show_options {
        draw_options(frame, area, app);
    }
}

/// Render the header bar with workspace info (left) and version (right).
fn draw_header(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let theme = &app.theme;
    let workspace_info = match (&app.workspace_name, &app.config_source_label) {
        (Some(name), Some(source)) => {
            let pkg_count = app.package_count();
            format!("{name} ({source}, {pkg_count} packages)")
        }
        _ => "no workspace loaded".to_string(),
    };

    let version_tag = format!("v{} ", env!("CARGO_PKG_VERSION"));
    let version_width = version_tag.len() as u16;

    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(version_width)]).areas(area);

    let left = Line::from(vec![
        Span::styled(" melos-tui ", Style::default().fg(theme.accent).bold()),
        Span::raw("| "),
        Span::styled(workspace_info, Style::default().fg(theme.text)),
    ]);
    frame.render_widget(Paragraph::new(left), left_area);

    let right = Line::from(Span::styled(
        version_tag,
        Style::default().fg(theme.text_muted),
    ));
    frame.render_widget(
        Paragraph::new(right).alignment(Alignment::Right),
        right_area,
    );
}

/// Render the main body area with two-column layout in Idle state.
fn draw_body(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    match app.state {
        AppState::Idle => {
            // Two-column split: 47/53.
            let [left_area, right_area] =
                Layout::horizontal([Constraint::Percentage(47), Constraint::Percentage(53)])
                    .areas(area);

            draw_packages(
                frame,
                left_area,
                app,
                app.active_panel == ActivePanel::Packages,
            );
            draw_commands(
                frame,
                right_area,
                app,
                app.active_panel == ActivePanel::Commands,
            );
        }
        AppState::Running => {
            draw_running(frame, area, app);
        }
        AppState::Done => {
            if app.health_report.is_some() {
                draw_health(frame, area, app);
            } else {
                draw_results(frame, area, app);
            }
        }
    }
}

/// Render the footer bar with context-sensitive keybindings or filter input.
fn draw_footer(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let theme = &app.theme;

    // When the filter input bar is active, show the filter prompt instead of keybindings.
    if app.filter_active {
        let footer = Line::from(vec![
            Span::styled(" /", Style::default().fg(theme.header).bold()),
            Span::raw(&app.filter_text),
            Span::styled("_", Style::default().fg(theme.text_muted)),
        ]);
        frame.render_widget(Paragraph::new(footer), area);
        return;
    }

    // When a filter is applied (but input is closed), show the filter indicator.
    if app.has_filter() && app.state == AppState::Idle {
        let footer = Line::from(vec![
            Span::styled(
                format!(" filter: {} ", app.filter_text),
                Style::default().fg(theme.header),
            ),
            Span::styled(
                "| /:edit  esc:clear  j/k:navigate  tab:switch  enter:run  ?:help",
                Style::default().fg(theme.text_muted),
            ),
        ]);
        frame.render_widget(Paragraph::new(footer), area);
        return;
    }

    let keys = match app.state {
        AppState::Idle => {
            "q:quit  j/k:navigate  g/G:jump  f/b:page  tab:switch  /:filter  t:theme  enter:run  ?:help"
        }
        AppState::Running => "esc:cancel",
        AppState::Done if app.health_report.is_some() => {
            "esc/enter/q:back  tab:switch tabs  j/k:scroll  g/G:jump  f/b:page  ctrl+c:quit"
        }
        AppState::Done => "esc/enter/q:back  j/k:scroll  g/G:jump  f/b:page  ctrl+c:quit",
    };

    let footer = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(keys, Style::default().fg(theme.text_muted)),
    ]);
    frame.render_widget(Paragraph::new(footer), area);

    // In Idle state, show the current theme name at the right edge.
    if app.state == AppState::Idle {
        let theme_label = format!(" {} ", app.theme_name());
        let label_width = theme_label.len() as u16;
        if area.width > label_width {
            let right_area = ratatui::layout::Rect::new(
                area.x + area.width - label_width,
                area.y,
                label_width,
                1,
            );
            let right = Paragraph::new(Line::from(Span::styled(
                theme_label,
                Style::default().fg(theme.text_muted),
            )))
            .alignment(Alignment::Right);
            frame.render_widget(right, right_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::PackageRow;
    use crate::theme::Theme;

    /// Helper: render app state into a test backend and return the buffer.
    fn render_app(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// Extract a line from the buffer as a string.
    fn buffer_line(buf: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect::<String>()
    }

    #[test]
    fn test_header_contains_melos_tui() {
        let app = App::new(Theme::default());
        let buf = render_app(&app, 80, 10);
        let header_line = buffer_line(&buf, 0, 80);
        assert!(header_line.contains("melos-tui"));
    }

    #[test]
    fn test_header_shows_no_workspace_when_unloaded() {
        let app = App::new(Theme::default());
        let buf = render_app(&app, 80, 10);
        let header_line = buffer_line(&buf, 0, 80);
        assert!(header_line.contains("no workspace loaded"));
    }

    #[test]
    fn test_header_shows_workspace_info_when_loaded() {
        let mut app = App::new(Theme::default());
        app.workspace_name = Some("my_workspace".to_string());
        app.config_source_label = Some("melos.yaml".to_string());
        app.package_rows = vec![
            PackageRow {
                name: "a".to_string(),
                version: "1.0.0".to_string(),
                sdk: "Dart",
                path: "packages/a".to_string(),
                is_private: false,
            },
            PackageRow {
                name: "b".to_string(),
                version: "2.0.0".to_string(),
                sdk: "Flutter",
                path: "packages/b".to_string(),
                is_private: false,
            },
        ];

        let buf = render_app(&app, 100, 10);
        let header_line = buffer_line(&buf, 0, 100);
        assert!(
            header_line.contains("my_workspace"),
            "Expected workspace name, got: {header_line}"
        );
        assert!(
            header_line.contains("melos.yaml"),
            "Expected config source, got: {header_line}"
        );
        assert!(
            header_line.contains("2 packages"),
            "Expected package count, got: {header_line}"
        );
    }

    #[test]
    fn test_header_shows_version_at_right() {
        let app = App::new(Theme::default());
        let buf = render_app(&app, 80, 10);
        let header_line = buffer_line(&buf, 0, 80);
        let expected = format!("v{}", env!("CARGO_PKG_VERSION"));
        assert!(
            header_line.contains(&expected),
            "Expected '{expected}' in header, got: {header_line}"
        );
        // Verify it's right-aligned: version should appear near the end of the line.
        let trimmed = header_line.trim_end();
        assert!(
            trimmed.ends_with(&expected),
            "Expected version at right edge, got: {trimmed}"
        );
    }

    #[test]
    fn test_footer_shows_navigation_keys_in_idle() {
        let app = App::new(Theme::default());
        let buf = render_app(&app, 100, 10);
        let footer_line = buffer_line(&buf, 9, 100);
        assert!(footer_line.contains("q:quit"));
        assert!(footer_line.contains("j/k:navigate"));
        assert!(footer_line.contains("?:help"));
    }

    #[test]
    fn test_footer_shows_esc_in_running() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        let buf = render_app(&app, 80, 10);
        let footer_line = buffer_line(&buf, 9, 80);
        assert!(footer_line.contains("esc:cancel"));
    }

    #[test]
    fn test_body_shows_both_panels_in_idle() {
        let mut app = App::new(Theme::default());
        app.package_rows = vec![PackageRow {
            name: "test_pkg".to_string(),
            version: "1.0.0".to_string(),
            sdk: "Dart",
            path: "packages/test_pkg".to_string(),
            is_private: false,
        }];

        let buf = render_app(&app, 120, 20);
        let all_text: String = (0..20)
            .map(|y| buffer_line(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_text.contains("Packages"),
            "Expected 'Packages' panel in body"
        );
        assert!(
            all_text.contains("Commands"),
            "Expected 'Commands' panel in body"
        );
    }

    #[test]
    fn test_help_overlay_renders_when_show_help_true() {
        let mut app = App::new(Theme::default());
        app.show_help = true;

        let buf = render_app(&app, 100, 40);
        let all_text: String = (0..40)
            .map(|y| buffer_line(&buf, y, 100))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_text.contains("Help"),
            "Expected 'Help' overlay title when show_help is true"
        );
        assert!(
            all_text.contains("Navigation"),
            "Expected 'Navigation' section in help overlay"
        );
    }

    #[test]
    fn test_footer_shows_filter_hint_in_idle() {
        let app = App::new(Theme::default());
        let buf = render_app(&app, 100, 10);
        let footer_line = buffer_line(&buf, 9, 100);
        assert!(
            footer_line.contains("/:filter"),
            "Expected '/:filter' in Idle footer, got: {footer_line}"
        );
    }

    #[test]
    fn test_footer_shows_filter_input_bar_when_active() {
        let mut app = App::new(Theme::default());
        app.filter_active = true;
        app.filter_text = "abc".to_string();
        let buf = render_app(&app, 100, 10);
        let footer_line = buffer_line(&buf, 9, 100);
        assert!(
            footer_line.contains("/abc"),
            "Expected '/abc' filter prompt, got: {footer_line}"
        );
    }

    #[test]
    fn test_footer_shows_filter_indicator_when_applied() {
        let mut app = App::new(Theme::default());
        app.filter_text = "test".to_string();
        app.filtered_indices = vec![0]; // Simulate non-empty filter result.
        let buf = render_app(&app, 100, 10);
        let footer_line = buffer_line(&buf, 9, 100);
        assert!(
            footer_line.contains("filter: test"),
            "Expected 'filter: test' indicator, got: {footer_line}"
        );
        assert!(
            footer_line.contains("/:edit"),
            "Expected '/:edit' hint, got: {footer_line}"
        );
        assert!(
            footer_line.contains("esc:clear"),
            "Expected 'esc:clear' hint, got: {footer_line}"
        );
    }
}
