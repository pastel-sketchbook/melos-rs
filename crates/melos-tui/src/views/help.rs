use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::App;
use crate::theme::Theme;

/// Navigation keybindings displayed in the left column.
const NAV_KEYS: &[(&str, &str)] = &[
    ("j / Down", "Move down"),
    ("k / Up", "Move up"),
    ("g / Home", "Jump to first"),
    ("G / End", "Jump to last"),
    ("f / PgDn", "Page down"),
    ("b / PgUp", "Page up"),
    ("Ctrl+d", "Half page down"),
    ("Ctrl+u", "Half page up"),
    ("h / l", "Focus left / right panel"),
    ("Tab", "Toggle panel"),
    ("Enter", "Run selected command"),
    ("/", "Filter packages"),
    ("t", "Cycle theme"),
    ("Esc", "Back / quit"),
    ("q", "Quit"),
    ("?", "Toggle this help"),
];

/// Melos commands displayed in the right column.
const MELOS_COMMANDS: &[(&str, &str)] = &[
    ("analyze", "Run dart analyze"),
    ("bootstrap", "Install dependencies"),
    ("build", "Build packages (Android/iOS)"),
    ("clean", "Clean build artifacts"),
    ("exec", "Run command in each package"),
    ("format", "Run dart format"),
    ("health", "Workspace health checks"),
    ("list", "List workspace packages"),
    ("publish", "Publish to pub.dev"),
    ("run", "Run named scripts"),
    ("test", "Run dart/flutter tests"),
    ("version", "Bump package versions"),
];

/// Draw the help overlay as a centered popup.
///
/// The overlay clears the background area, draws a bordered box, and renders
/// two columns: navigation keys (left) and melos commands (right).
pub fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let popup = centered_rect(70, 80, area);

    // Clear the area behind the popup.
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .title_style(Style::default().fg(theme.accent).bold())
        .border_style(Style::default().fg(theme.accent));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Split inner into two columns.
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(inner);

    // Left column: navigation keys
    let nav_lines = build_key_lines("Navigation", NAV_KEYS, theme);
    frame.render_widget(Paragraph::new(nav_lines), left_area);

    // Right column: melos commands
    let cmd_lines = build_key_lines("Commands", MELOS_COMMANDS, theme);
    frame.render_widget(Paragraph::new(cmd_lines), right_area);
}

/// Build styled lines for a key/description table with a section header.
fn build_key_lines<'a>(
    header: &'a str,
    entries: &[(&'a str, &'a str)],
    theme: &Theme,
) -> Vec<Line<'a>> {
    let mut lines = Vec::with_capacity(entries.len() + 2);

    lines.push(Line::from(Span::styled(
        header,
        Style::default()
            .fg(theme.header)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for (key, desc) in entries {
        lines.push(Line::from(vec![
            Span::styled(format!(" {key:<16}"), Style::default().fg(theme.success)),
            Span::styled(*desc, Style::default().fg(theme.text)),
        ]));
    }

    lines
}

/// Calculate a centered rectangle within `area` using percentage width/height.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let [_, v_center, _] = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .areas(area);

    let [_, h_center, _] = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .areas(v_center);

    h_center
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    use super::*;

    /// Helper: render the help overlay and return the buffer.
    fn render_help(width: u16, height: u16) -> ratatui::buffer::Buffer {
        let app = App::new(Theme::default());
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                draw_help(frame, frame.area(), &app);
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

    #[test]
    fn test_centered_rect_dimensions() {
        let area = Rect::new(0, 0, 100, 50);
        let popup = centered_rect(70, 80, area);
        // Should be roughly 70% of 100 = 70 wide, 80% of 50 = 40 tall.
        assert!(
            popup.width >= 60 && popup.width <= 80,
            "width: {}",
            popup.width
        );
        assert!(
            popup.height >= 30 && popup.height <= 45,
            "height: {}",
            popup.height
        );
    }

    #[test]
    fn test_help_overlay_shows_title() {
        let buf = render_help(100, 40);
        let all_text: String = (0..40)
            .map(|y| buffer_line(&buf, y, 100))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all_text.contains("Help"), "Expected 'Help' in overlay");
    }

    #[test]
    fn test_help_overlay_shows_navigation_header() {
        let buf = render_help(100, 40);
        let all_text: String = (0..40)
            .map(|y| buffer_line(&buf, y, 100))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_text.contains("Navigation"),
            "Expected 'Navigation' section header"
        );
    }

    #[test]
    fn test_help_overlay_shows_commands_header() {
        let buf = render_help(100, 40);
        let all_text: String = (0..40)
            .map(|y| buffer_line(&buf, y, 100))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_text.contains("Commands"),
            "Expected 'Commands' section header"
        );
    }

    #[test]
    fn test_help_overlay_shows_nav_keys() {
        let buf = render_help(100, 40);
        let all_text: String = (0..40)
            .map(|y| buffer_line(&buf, y, 100))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all_text.contains("j / Down"), "Expected 'j / Down' key");
        assert!(all_text.contains("Move down"), "Expected 'Move down' desc");
    }

    #[test]
    fn test_help_overlay_shows_melos_commands() {
        let buf = render_help(100, 40);
        let all_text: String = (0..40)
            .map(|y| buffer_line(&buf, y, 100))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all_text.contains("analyze"), "Expected 'analyze' command");
        assert!(
            all_text.contains("bootstrap"),
            "Expected 'bootstrap' command"
        );
        assert!(all_text.contains("version"), "Expected 'version' command");
    }
}
