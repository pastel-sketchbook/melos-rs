use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, AppState};
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
}

/// Render the header bar with workspace info.
fn draw_header(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let workspace_info = match (&app.workspace_name, &app.config_source_label) {
        (Some(name), Some(source)) => {
            let pkg_count = app.package_count();
            format!("{name} ({source}, {pkg_count} packages)")
        }
        _ => "no workspace loaded".to_string(),
    };

    let header = Line::from(vec![
        Span::styled(" melos-tui ", Style::default().fg(Color::Cyan).bold()),
        Span::raw("| "),
        Span::styled(workspace_info, Style::default().fg(Color::White)),
    ]);
    frame.render_widget(Paragraph::new(header), area);
}

/// Render the main body area.
fn draw_body(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    match app.state {
        AppState::Idle => {
            draw_packages(frame, area, app);
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
        AppState::Idle => {
            "q:quit  up/down:navigate  home/end:jump  pgup/pgdn:page  tab:switch  enter:run"
        }
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
    use ratatui::{Terminal, backend::TestBackend};

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
    fn test_footer_shows_navigation_keys_in_idle() {
        let app = App::new();
        let buf = render_app(&app, 100, 10);
        let footer_line = buffer_line(&buf, 9, 100);
        assert!(footer_line.contains("q:quit"));
        assert!(footer_line.contains("up/down:navigate"));
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
    fn test_body_shows_package_table_in_idle() {
        let mut app = App::new();
        app.package_rows = vec![PackageRow {
            name: "test_pkg".to_string(),
            version: "1.0.0".to_string(),
            sdk: "Dart",
            path: "packages/test_pkg".to_string(),
            is_private: false,
        }];

        let buf = render_app(&app, 80, 12);
        // The body area should contain the Packages table border/title
        let body_top = buffer_line(&buf, 1, 80);
        assert!(
            body_top.contains("Packages"),
            "Expected 'Packages' in body, got: {body_top}"
        );
    }
}
