use ratatui::{
    Frame,
    layout::Constraint,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table, TableState},
};

use crate::app::App;

/// Draw the command/script table into the given area.
///
/// Built-in commands appear first (white), followed by user scripts (cyan).
/// Scripts with descriptions show them in a second column.
/// When `focused` is true, the border is highlighted in cyan.
pub fn draw_commands(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, focused: bool) {
    let header_cells = ["Command", "Description"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).bold()));
    let header = Row::new(header_cells).height(1);

    let rows = app.command_rows.iter().map(|cmd| {
        let name_style = if cmd.is_builtin {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Cyan)
        };

        let desc = cmd.description.as_deref().unwrap_or("");

        Row::new(vec![
            Cell::from(cmd.name.as_str()).style(name_style),
            Cell::from(desc).style(Style::default().fg(Color::DarkGray)),
        ])
    });

    let title = format!(" Commands ({}) ", app.command_count());

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let table = Table::new(rows, [Constraint::Length(24), Constraint::Fill(1)])
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(border_style),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::Indexed(237))
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    let mut table_state = TableState::default();
    if !app.command_rows.is_empty() {
        table_state.select(Some(app.selected_command));
    }

    frame.render_stateful_widget(table, area, &mut table_state);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::CommandRow;

    /// Helper: render the command table and return the buffer.
    fn render_commands(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                draw_commands(frame, frame.area(), app, true);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Extract a line from the buffer as a trimmed string.
    fn buffer_line(buf: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn test_command_table_shows_count_in_title() {
        let app = App::new();
        let buf = render_commands(&app, 80, 20);
        let title_line = buffer_line(&buf, 0, 80);
        let expected = format!("Commands ({})", app.command_count());
        assert!(
            title_line.contains(&expected),
            "Expected '{expected}' in title, got: {title_line}"
        );
    }

    #[test]
    fn test_command_table_shows_column_headers() {
        let app = App::new();
        let buf = render_commands(&app, 80, 20);
        let header_line = buffer_line(&buf, 1, 80);
        assert!(
            header_line.contains("Command"),
            "Expected 'Command' in header, got: {header_line}"
        );
        assert!(
            header_line.contains("Description"),
            "Expected 'Description' in header, got: {header_line}"
        );
    }

    #[test]
    fn test_command_table_shows_builtin_names() {
        let app = App::new();
        let buf = render_commands(&app, 80, 20);
        // First builtin is "analyze" at row y=2 (border=0, header=1)
        let row_line = buffer_line(&buf, 2, 80);
        assert!(
            row_line.contains("analyze"),
            "Expected 'analyze' in first row, got: {row_line}"
        );
    }

    #[test]
    fn test_command_table_shows_script_with_description() {
        let mut app = App::new();
        app.command_rows.push(CommandRow {
            name: "custom_script".to_string(),
            description: Some("runs custom logic".to_string()),
            is_builtin: false,
        });

        let buf = render_commands(&app, 80, 20);
        // Script is last row, after all builtins
        let row_count = app.command_rows.len();
        let script_y = 1 + row_count as u16; // header=1, then rows
        let row_line = buffer_line(&buf, script_y, 80);
        assert!(
            row_line.contains("custom_script"),
            "Expected 'custom_script' in row, got: {row_line}"
        );
        assert!(
            row_line.contains("runs custom logic"),
            "Expected description in row, got: {row_line}"
        );
    }

    #[test]
    fn test_command_table_empty_shows_zero_count() {
        let mut app = App::new();
        app.command_rows.clear();
        let buf = render_commands(&app, 80, 10);
        let title_line = buffer_line(&buf, 0, 80);
        assert!(
            title_line.contains("Commands (0)"),
            "Expected 'Commands (0)' in title, got: {title_line}"
        );
    }
}
