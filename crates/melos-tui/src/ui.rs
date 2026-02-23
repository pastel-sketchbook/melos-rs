use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};

use crate::app::{App, AppState};

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
fn draw_header(frame: &mut Frame, area: ratatui::layout::Rect, _app: &App) {
    let header = Line::from(vec![
        Span::styled(" melos-tui ", Style::default().fg(Color::Cyan).bold()),
        Span::raw("| "),
        Span::styled("no workspace loaded", Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(header), area);
}

/// Render the main body area.
fn draw_body(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let message = match app.state {
        AppState::Idle => "Press Tab to switch panels. Press Enter to run a command.",
        AppState::Running => "Command is running...",
        AppState::Done => "Command finished. Press Esc to return.",
    };

    let body = Paragraph::new(message)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Workspace ")
                .padding(Padding::new(1, 1, 1, 1)),
        )
        .style(Style::default().fg(Color::White));

    frame.render_widget(body, area);
}

/// Render the footer bar with context-sensitive keybindings.
fn draw_footer(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let keys = match app.state {
        AppState::Idle => "q:quit  tab:switch  enter:run  /:filter  ?:help",
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

    /// Helper: render app state into a test backend and return the buffer.
    fn render_app(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn test_header_contains_melos_tui() {
        let app = App::new();
        let buf = render_app(&app, 60, 10);
        let header_line: String = (0..60)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(header_line.contains("melos-tui"));
    }

    #[test]
    fn test_footer_shows_quit_key_in_idle() {
        let app = App::new();
        let buf = render_app(&app, 60, 10);
        let footer_line: String = (0..60)
            .map(|x| buf.cell((x, 9)).unwrap().symbol().to_string())
            .collect();
        assert!(footer_line.contains("q:quit"));
    }

    #[test]
    fn test_footer_shows_esc_in_running() {
        let mut app = App::new();
        app.state = AppState::Running;
        let buf = render_app(&app, 60, 10);
        let footer_line: String = (0..60)
            .map(|x| buf.cell((x, 9)).unwrap().symbol().to_string())
            .collect();
        assert!(footer_line.contains("esc:cancel"));
    }

    #[test]
    fn test_body_shows_workspace_border() {
        let app = App::new();
        let buf = render_app(&app, 60, 10);
        // The body block title should contain "Workspace"
        let top_border: String = (0..60)
            .map(|x| buf.cell((x, 1)).unwrap().symbol().to_string())
            .collect();
        assert!(top_border.contains("Workspace"));
    }
}
