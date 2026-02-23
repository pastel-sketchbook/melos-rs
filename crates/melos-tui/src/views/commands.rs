use ratatui::{
    Frame,
    layout::Constraint,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table, TableState},
};

use crate::app::App;

/// Build a section header row that spans both columns.
fn section_header(label: &str) -> Row<'_> {
    Row::new(vec![
        Cell::from(label).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(""),
    ])
}

/// Draw the command/script table into the given area.
///
/// Commands are grouped under "Built-in" and "Scripts" section headers.
/// Built-in commands appear in white, user scripts in cyan.
/// Section headers are visual-only rows; the selection highlight always
/// lands on a real command row.
/// When `focused` is true, the border is highlighted in cyan.
pub fn draw_commands(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, focused: bool) {
    let header_cells = ["Command", "Description"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).bold()));
    let header = Row::new(header_cells).height(1);

    let has_builtins = app.command_rows.iter().any(|c| c.is_builtin);
    let first_script_idx = app.command_rows.iter().position(|c| !c.is_builtin);

    let mut visual_rows: Vec<Row<'_>> = Vec::new();
    let mut selected_visual: usize = 0;

    if has_builtins {
        visual_rows.push(section_header("-- Built-in --"));
    }

    for (i, cmd) in app.command_rows.iter().enumerate() {
        if Some(i) == first_script_idx {
            visual_rows.push(section_header("-- Scripts --"));
        }

        if i == app.selected_command {
            selected_visual = visual_rows.len();
        }

        let name_style = if cmd.is_builtin {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Cyan)
        };

        let desc = cmd.description.as_deref().unwrap_or("");

        visual_rows.push(Row::new(vec![
            Cell::from(cmd.name.as_str()).style(name_style),
            Cell::from(desc).style(Style::default().fg(Color::DarkGray)),
        ]));
    }

    let title = format!(" Commands ({}) ", app.command_count());

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let table = Table::new(visual_rows, [Constraint::Length(24), Constraint::Fill(1)])
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
        table_state.select(Some(selected_visual));
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
        // Section header "-- Built-in --" at y=2, first builtin "analyze" at y=3
        let header_row = buffer_line(&buf, 2, 80);
        assert!(
            header_row.contains("Built-in"),
            "Expected 'Built-in' section header, got: {header_row}"
        );
        let row_line = buffer_line(&buf, 3, 80);
        assert!(
            row_line.contains("analyze"),
            "Expected 'analyze' in first command row, got: {row_line}"
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

        let buf = render_commands(&app, 80, 22);
        // After builtins: "-- Scripts --" header, then script row.
        // y=0: border, y=1: column header, y=2: "-- Built-in --",
        // y=3..14: 12 builtins, y=15: "-- Scripts --", y=16: script
        let scripts_header = buffer_line(&buf, 15, 80);
        assert!(
            scripts_header.contains("Scripts"),
            "Expected 'Scripts' section header, got: {scripts_header}"
        );
        let script_row = buffer_line(&buf, 16, 80);
        assert!(
            script_row.contains("custom_script"),
            "Expected 'custom_script' in row, got: {script_row}"
        );
        assert!(
            script_row.contains("runs custom logic"),
            "Expected description in row, got: {script_row}"
        );
    }

    #[test]
    fn test_command_table_scripts_only_no_builtin_header() {
        let mut app = App::new();
        app.command_rows = vec![CommandRow {
            name: "my_script".to_string(),
            description: None,
            is_builtin: false,
        }];

        let buf = render_commands(&app, 80, 10);
        // y=2 should be "-- Scripts --" (no built-in header)
        let section = buffer_line(&buf, 2, 80);
        assert!(
            section.contains("Scripts"),
            "Expected 'Scripts' section header, got: {section}"
        );
        let row = buffer_line(&buf, 3, 80);
        assert!(
            row.contains("my_script"),
            "Expected 'my_script' in row, got: {row}"
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
