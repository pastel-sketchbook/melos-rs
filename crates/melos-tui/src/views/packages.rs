use ratatui::{
    Frame,
    layout::Constraint,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::app::App;

/// Draw the package table into the given area.
///
/// Uses `TableState` to track selection highlighting and scroll offset.
/// When `focused` is true, the border is highlighted in the accent color.
/// Respects the active package filter: only matching packages are shown.
pub fn draw_packages(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, focused: bool) {
    let theme = &app.theme;
    let border_style = if focused {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.text_muted)
    };

    // No workspace loaded: show actionable error message.
    if app.workspace_name.is_none() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Packages ")
            .border_style(Style::default().fg(theme.error));
        let message = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No workspace found",
                Style::default().fg(theme.error).bold(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Run `melos-rs init` to create a workspace,",
                Style::default().fg(theme.text_muted),
            )),
            Line::from(Span::styled(
                "  or run from a directory with melos.yaml.",
                Style::default().fg(theme.text_muted),
            )),
        ])
        .block(block);
        frame.render_widget(message, area);
        return;
    }

    let visible = app.visible_packages();

    // No packages match the current filter.
    if visible.is_empty() && app.has_filter() {
        let title = format!(" Packages (0/{}) ", app.package_count());
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style);
        let message = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  No packages match \"{}\"", app.filter_text),
                Style::default().fg(theme.text_muted),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press Esc to clear filter",
                Style::default().fg(theme.text_muted),
            )),
        ])
        .block(block);
        frame.render_widget(message, area);
        return;
    }

    // No packages at all (workspace loaded but empty).
    if visible.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Packages (0) ")
            .border_style(border_style);
        let message = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No packages found",
                Style::default().fg(theme.text_muted),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Check `packages` globs in melos.yaml",
                Style::default().fg(theme.text_muted),
            )),
        ])
        .block(block);
        frame.render_widget(message, area);
        return;
    }

    let header_cells = ["Name", "Version", "SDK", "Path"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(theme.header).bold()));
    let header = Row::new(header_cells).height(1);

    let max_name_len = visible.iter().map(|pkg| pkg.name.len()).max().unwrap_or(0);

    let rows = visible.iter().map(|pkg| {
        let name_display = if pkg.is_private {
            format!("{:<width$}(private)", pkg.name, width = max_name_len + 1)
        } else {
            pkg.name.clone()
        };

        let sdk_color = if pkg.sdk == "Flutter" {
            theme.accent
        } else {
            theme.success
        };

        Row::new(vec![
            Cell::from(name_display),
            Cell::from(Span::raw(&pkg.version)),
            Cell::from(Span::styled(pkg.sdk, Style::default().fg(sdk_color))),
            Cell::from(Span::styled(
                &pkg.path,
                Style::default().fg(theme.text_muted),
            )),
        ])
    });

    let title = if app.has_filter() {
        format!(
            " Packages ({}/{}) ",
            app.visible_package_count(),
            app.package_count()
        )
    } else {
        format!(" Packages ({}) ", app.package_count())
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
            .bg(theme.highlight_bg)
            .fg(theme.highlight_fg)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    let mut table_state = TableState::default();
    if !visible.is_empty() {
        table_state.select(Some(app.selected_package));
    }

    frame.render_stateful_widget(table, area, &mut table_state);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::PackageRow;
    use crate::theme::Theme;

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
        let mut app = App::new(Theme::default());
        app.workspace_name = Some("test".to_string());
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
    fn test_no_workspace_shows_error_message() {
        let app = App::new(Theme::default());
        let buf = render_packages(&app, 80, 10);
        let all_text: String = (0..10)
            .map(|y| buffer_line(&buf, y, 80))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_text.contains("No workspace found"),
            "Expected 'No workspace found' message, got: {all_text}"
        );
        assert!(
            all_text.contains("melos-rs init"),
            "Expected init suggestion, got: {all_text}"
        );
    }

    #[test]
    fn test_empty_workspace_shows_no_packages() {
        let mut app = App::new(Theme::default());
        app.workspace_name = Some("test".to_string());
        let buf = render_packages(&app, 80, 10);
        let all_text: String = (0..10)
            .map(|y| buffer_line(&buf, y, 80))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_text.contains("No packages found"),
            "Expected 'No packages found' message, got: {all_text}"
        );
        assert!(
            all_text.contains("Packages (0)"),
            "Expected 'Packages (0)' in title, got: {all_text}"
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

    #[test]
    fn test_filtered_title_shows_visible_of_total() {
        let mut app = make_app_with_rows(vec![
            PackageRow {
                name: "alpha".to_string(),
                version: "1.0.0".to_string(),
                sdk: "Dart",
                path: "packages/alpha".to_string(),
                is_private: false,
            },
            PackageRow {
                name: "beta".to_string(),
                version: "1.0.0".to_string(),
                sdk: "Dart",
                path: "packages/beta".to_string(),
                is_private: false,
            },
            PackageRow {
                name: "gamma".to_string(),
                version: "1.0.0".to_string(),
                sdk: "Dart",
                path: "packages/gamma".to_string(),
                is_private: false,
            },
        ]);
        // Simulate an applied filter matching "alpha" and "gamma" (contain 'a').
        app.filter_text = "a".to_string();
        app.filtered_indices = vec![0, 2];

        let buf = render_packages(&app, 80, 10);
        let title_line = buffer_line(&buf, 0, 80);
        assert!(
            title_line.contains("Packages (2/3)"),
            "Expected 'Packages (2/3)' in title, got: {title_line}"
        );
    }

    #[test]
    fn test_empty_filter_result_shows_message() {
        let mut app = make_app_with_rows(vec![PackageRow {
            name: "alpha".to_string(),
            version: "1.0.0".to_string(),
            sdk: "Dart",
            path: "packages/alpha".to_string(),
            is_private: false,
        }]);
        // Simulate a filter with no matches.
        app.filter_text = "xyz".to_string();
        app.filtered_indices = vec![];

        let buf = render_packages(&app, 80, 10);
        let all_text: String = (0..10)
            .map(|y| buffer_line(&buf, y, 80))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_text.contains("No packages match \"xyz\""),
            "Expected 'No packages match' message, got: {all_text}"
        );
        assert!(
            all_text.contains("Packages (0/1)"),
            "Expected 'Packages (0/1)' in title, got: {all_text}"
        );
    }
}
