use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::{ActivePanel, App, AppState};
use crate::views::commands::draw_commands;
use crate::views::help::draw_help;
use crate::views::packages::draw_packages;

/// Render the entire UI for the current frame.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Three-row layout: header (1), body (fill), footer (1).
    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, header_area, app);
    draw_body(frame, body_area, app);
    draw_footer(frame, footer_area, app);

    // Help overlay on top of everything.
    if app.show_help {
        draw_help(frame, area);
    }
}

/// Render the header bar with workspace info (left) and version (right).
fn draw_header(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
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
        Span::styled(" melos-tui ", Style::default().fg(Color::Cyan).bold()),
        Span::raw("| "),
        Span::styled(workspace_info, Style::default().fg(Color::White)),
    ]);
    frame.render_widget(Paragraph::new(left), left_area);

    let right = Line::from(Span::styled(
        version_tag,
        Style::default().fg(Color::DarkGray),
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
            // Two-column split: 40% packages (left), 60% commands (right).
            let [left_area, right_area] =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
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
            let msg =
                Paragraph::new("Command is running...").style(Style::default().fg(Color::Yellow));
            frame.render_widget(msg, area);
        }
        AppState::Done => {
            let msg = Paragraph::new("Command finished. Press Esc to return.")
                .style(Style::default().fg(Color::Green));
            frame.render_widget(msg, area);
        }
    }
}

/// Render the footer bar with context-sensitive keybindings.
fn draw_footer(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let keys = match app.state {
        AppState::Idle => "q:quit  j/k:navigate  g/G:jump  f/b:page  tab:switch  enter:run  ?:help",
        AppState::Running => "esc:cancel",
        AppState::Done => "esc:back  q:quit",
    };

    let footer = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(keys, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(footer), area);
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use super::*;
    use crate::app::PackageRow;

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
        let app = App::new();
        let buf = render_app(&app, 80, 10);
        let header_line = buffer_line(&buf, 0, 80);
        assert!(header_line.contains("melos-tui"));
    }

    #[test]
    fn test_header_shows_no_workspace_when_unloaded() {
        let app = App::new();
        let buf = render_app(&app, 80, 10);
        let header_line = buffer_line(&buf, 0, 80);
        assert!(header_line.contains("no workspace loaded"));
    }

    #[test]
    fn test_header_shows_workspace_info_when_loaded() {
        let mut app = App::new();
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
        let app = App::new();
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
        let app = App::new();
        let buf = render_app(&app, 100, 10);
        let footer_line = buffer_line(&buf, 9, 100);
        assert!(footer_line.contains("q:quit"));
        assert!(footer_line.contains("j/k:navigate"));
        assert!(footer_line.contains("?:help"));
    }

    #[test]
    fn test_footer_shows_esc_in_running() {
        let mut app = App::new();
        app.state = AppState::Running;
        let buf = render_app(&app, 80, 10);
        let footer_line = buffer_line(&buf, 9, 80);
        assert!(footer_line.contains("esc:cancel"));
    }

    #[test]
    fn test_body_shows_both_panels_in_idle() {
        let mut app = App::new();
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
        let mut app = App::new();
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
}
