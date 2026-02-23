use ratatui::{
    layout::Constraint,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Row, Table, TableState},
    Frame,
};

use crate::app::App;

/// Draw the package table into the given area.
///
/// Uses `TableState` to track selection highlighting and scroll offset.
/// When `focused` is true, the border is highlighted in cyan.
pub fn draw_packages(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, focused: bool) {
    let header_cells = ["Name", "Version", "SDK", "Path"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).bold()));
    let header = Row::new(header_cells).height(1);

    let max_name_len = app
        .package_rows
        .iter()
        .map(|pkg| pkg.name.len())
        .max()
        .unwrap_or(0);

    let rows = app.package_rows.iter().map(|pkg| {
        let name_display = if pkg.is_private {
            format!("{:<width$}(private)", pkg.name, width = max_name_len + 1)
        } else {
            pkg.name.clone()
        };

        let sdk_color = if pkg.sdk == "Flutter" {
            Color::Cyan
        } else {
            Color::Green
        };

        Row::new(vec![
            Cell::from(name_display),
            Cell::from(Span::raw(&pkg.version)),
            Cell::from(Span::styled(pkg.sdk, Style::default().fg(sdk_color))),
            Cell::from(Span::styled(
                &pkg.path,
                Style::default().fg(Color::DarkGray),
            )),
        ])
    });

    let title = format!(" Packages ({}) ", app.package_count());

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let table = Table::new(
        rows,
        [
            Constraint::Fill(2),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Fill(2),
        ],
    )
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
    if !app.package_rows.is_empty() {
        table_state.select(Some(app.selected_package));
    }

    frame.render_stateful_widget(table, area, &mut table_state);
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use super::*;
    use crate::app::PackageRow;

    /// Helper: render the package table and return the buffer.
    fn render_packages(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                draw_packages(frame, frame.area(), app, true);
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

    fn make_app_with_rows(rows: Vec<PackageRow>) -> App {
        let mut app = App::new();
        app.package_rows = rows;
        app
    }

    #[test]
    fn test_table_shows_package_count_in_title() {
        let app = make_app_with_rows(vec![
            PackageRow {
                name: "app".to_string(),
                version: "1.0.0".to_string(),
                sdk: "Flutter",
                path: "packages/app".to_string(),
                is_private: false,
            },
            PackageRow {
                name: "core".to_string(),
                version: "0.5.0".to_string(),
                sdk: "Dart",
                path: "packages/core".to_string(),
                is_private: false,
            },
        ]);

        let buf = render_packages(&app, 80, 10);
        let title_line = buffer_line(&buf, 0, 80);
        assert!(
            title_line.contains("Packages (2)"),
            "Expected 'Packages (2)' in title, got: {title_line}"
        );
    }

    #[test]
    fn test_table_shows_column_headers() {
        let app = make_app_with_rows(vec![PackageRow {
            name: "test_pkg".to_string(),
            version: "1.0.0".to_string(),
            sdk: "Dart",
            path: "packages/test_pkg".to_string(),
            is_private: false,
        }]);

        let buf = render_packages(&app, 80, 10);
        let header_line = buffer_line(&buf, 1, 80);
        assert!(
            header_line.contains("Name"),
            "Expected 'Name' in header, got: {header_line}"
        );
        assert!(
            header_line.contains("Version"),
            "Expected 'Version' in header, got: {header_line}"
        );
        assert!(
            header_line.contains("SDK"),
            "Expected 'SDK' in header, got: {header_line}"
        );
        assert!(
            header_line.contains("Path"),
            "Expected 'Path' in header, got: {header_line}"
        );
    }

    #[test]
    fn test_table_shows_private_suffix() {
        let app = make_app_with_rows(vec![PackageRow {
            name: "internal".to_string(),
            version: "0.1.0".to_string(),
            sdk: "Dart",
            path: "packages/internal".to_string(),
            is_private: true,
        }]);

        let buf = render_packages(&app, 80, 10);
        // Row content starts at y=2 (border at 0, header at 1)
        let row_line = buffer_line(&buf, 2, 80);
        assert!(
            row_line.contains("(private)"),
            "Expected '(private)' suffix, got: {row_line}"
        );
    }

    #[test]
    fn test_private_suffix_aligned_to_longest_name() {
        let app = make_app_with_rows(vec![
            PackageRow {
                name: "app".to_string(),
                version: "1.0.0".to_string(),
                sdk: "Dart",
                path: "packages/app".to_string(),
                is_private: true,
            },
            PackageRow {
                name: "long_package_name".to_string(),
                version: "2.0.0".to_string(),
                sdk: "Dart",
                path: "packages/long".to_string(),
                is_private: true,
            },
        ]);

        let buf = render_packages(&app, 100, 10);
        let row1 = buffer_line(&buf, 2, 100); // "app" row
        let row2 = buffer_line(&buf, 3, 100); // "long_package_name" row

        // Both "(private)" suffixes should start at the same column
        let col1 = row1
            .find("(private)")
            .expect("row1 should contain (private)");
        let col2 = row2
            .find("(private)")
            .expect("row2 should contain (private)");
        assert_eq!(
            col1, col2,
            "Expected (private) aligned at same column, got col {col1} vs {col2}\nrow1: {row1}\nrow2: {row2}"
        );
    }

    #[test]
    fn test_empty_table_shows_zero_count() {
        let app = App::new();
        let buf = render_packages(&app, 80, 10);
        let title_line = buffer_line(&buf, 0, 80);
        assert!(
            title_line.contains("Packages (0)"),
            "Expected 'Packages (0)' in title, got: {title_line}"
        );
    }

    #[test]
    fn test_table_shows_package_data() {
        let app = make_app_with_rows(vec![PackageRow {
            name: "my_app".to_string(),
            version: "2.0.0".to_string(),
            sdk: "Flutter",
            path: "packages/my_app".to_string(),
            is_private: false,
        }]);

        let buf = render_packages(&app, 80, 10);
        let row_line = buffer_line(&buf, 2, 80);
        assert!(
            row_line.contains("my_app"),
            "Expected 'my_app' in row, got: {row_line}"
        );
        assert!(
            row_line.contains("2.0.0"),
            "Expected '2.0.0' in row, got: {row_line}"
        );
        assert!(
            row_line.contains("Flutter"),
            "Expected 'Flutter' in row, got: {row_line}"
        );
    }
}
