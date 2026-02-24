use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::{App, OptionRow};

/// Draw the command options overlay as a centered popup.
///
/// Shows per-command options with the selected row highlighted.
/// Boolean options show [x] / [ ], numeric options show the value
/// with +/- hints. The last row is a "Run" action button.
pub fn draw_options(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let opts = match &app.command_opts {
        Some(o) => o,
        None => return,
    };

    let cmd_name = app
        .command_rows
        .get(app.selected_command)
        .map(|c| c.name.as_str())
        .unwrap_or("command");

    let rows = opts.option_rows();
    // Popup height: border (2) + title line (0, in border) + rows + 1 empty + 1 run button + 1 hint.
    let content_lines = rows.len() + 3;
    let popup_height = (content_lines as u16 + 2).min(area.height);
    let popup_width = 44.min(area.width);

    let popup = centered_rect_fixed(popup_width, popup_height, area);

    frame.render_widget(Clear, popup);

    let title = format!(" {cmd_name} options ");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(theme.accent).bold())
        .border_style(Style::default().fg(theme.accent));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines: Vec<Line<'_>> = Vec::with_capacity(content_lines);

    for (i, row) in rows.iter().enumerate() {
        let selected = i == app.selected_option;
        let highlight = if selected {
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_secondary)
        };
        let cursor = if selected { ">> " } else { "   " };

        match row {
            OptionRow::Bool(label, val) => {
                let check = if *val { "[x]" } else { "[ ]" };
                lines.push(Line::from(vec![
                    Span::styled(cursor, highlight),
                    Span::styled(format!("{check} {label}"), highlight),
                ]));
            }
            OptionRow::Number(label, val) => {
                lines.push(Line::from(vec![
                    Span::styled(cursor, highlight),
                    Span::styled(format!("{label}: "), highlight),
                    Span::styled(
                        format!("{val}"),
                        Style::default().fg(theme.header).add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                    ),
                    Span::styled(
                        if selected { "  -/+" } else { "" },
                        Style::default().fg(theme.text_muted),
                    ),
                ]));
            }
            OptionRow::OptNumber(label, val) => {
                let display = match val {
                    Some(v) => format!("{v}"),
                    None => "default".to_string(),
                };
                lines.push(Line::from(vec![
                    Span::styled(cursor, highlight),
                    Span::styled(format!("{label}: "), highlight),
                    Span::styled(
                        display,
                        Style::default().fg(theme.header).add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                    ),
                ]));
            }
        }
    }

    // Empty separator line.
    lines.push(Line::from(""));

    // "Run" action row.
    let run_selected = app.selected_option == rows.len();
    let run_style = if run_selected {
        Style::default()
            .fg(theme.success)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.success)
    };
    let run_cursor = if run_selected { ">> " } else { "   " };
    lines.push(Line::from(vec![
        Span::styled(run_cursor, run_style),
        Span::styled("[ Run ]", run_style),
    ]));

    // Hint line.
    lines.push(Line::from(Span::styled(
        "   space:toggle  -/+:adjust  enter:run  esc:cancel",
        Style::default().fg(theme.text_muted),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Calculate a centered rectangle of fixed dimensions within `area`.
fn centered_rect_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let [_, v_center, _] = Layout::vertical([
        Constraint::Length(area.height.saturating_sub(height) / 2),
        Constraint::Length(height),
        Constraint::Min(0),
    ])
    .areas(area);

    let [_, h_center, _] = Layout::horizontal([
        Constraint::Length(area.width.saturating_sub(width) / 2),
        Constraint::Length(width),
        Constraint::Min(0),
    ])
    .areas(v_center);

    h_center
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{ActivePanel, App, CommandOpts};
    use crate::theme::Theme;

    /// Helper: render the options overlay and return the buffer.
    fn render_options(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                draw_options(frame, frame.area(), app);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Extract full buffer text.
    fn all_buffer_text(buf: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app_with_options(command_name: &str) -> App {
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Commands;
        // Find the command index.
        if let Some(idx) = app.command_rows.iter().position(|c| c.name == command_name) {
            app.selected_command = idx;
        }
        app.command_opts = CommandOpts::build_default(command_name);
        app.show_options = true;
        app.selected_option = 0;
        app
    }

    #[test]
    fn test_options_overlay_shows_command_name() {
        let app = app_with_options("analyze");
        let buf = render_options(&app, 80, 30);
        let text = all_buffer_text(&buf, 80, 30);
        assert!(
            text.contains("analyze options"),
            "Expected 'analyze options' in overlay, got: {text}"
        );
    }

    #[test]
    fn test_options_overlay_shows_bool_options() {
        let app = app_with_options("analyze");
        let buf = render_options(&app, 80, 30);
        let text = all_buffer_text(&buf, 80, 30);
        assert!(
            text.contains("fatal-warnings"),
            "Expected 'fatal-warnings' option"
        );
        assert!(
            text.contains("fatal-infos"),
            "Expected 'fatal-infos' option"
        );
        assert!(text.contains("no-fatal"), "Expected 'no-fatal' option");
    }

    #[test]
    fn test_options_overlay_shows_number_options() {
        let app = app_with_options("analyze");
        let buf = render_options(&app, 80, 30);
        let text = all_buffer_text(&buf, 80, 30);
        assert!(
            text.contains("concurrency"),
            "Expected 'concurrency' option"
        );
    }

    #[test]
    fn test_options_overlay_shows_run_button() {
        let app = app_with_options("analyze");
        let buf = render_options(&app, 80, 30);
        let text = all_buffer_text(&buf, 80, 30);
        assert!(text.contains("Run"), "Expected 'Run' button in overlay");
    }

    #[test]
    fn test_options_overlay_no_render_without_opts() {
        let mut app = App::new(Theme::default());
        app.command_opts = None;
        app.show_options = true;
        let buf = render_options(&app, 80, 30);
        let text = all_buffer_text(&buf, 80, 30);
        // Should be empty since command_opts is None.
        assert!(
            !text.contains("options"),
            "Should not render overlay without command_opts"
        );
    }

    #[test]
    fn test_options_overlay_hint_line() {
        let app = app_with_options("clean");
        let buf = render_options(&app, 80, 30);
        let text = all_buffer_text(&buf, 80, 30);
        assert!(
            text.contains("space:toggle"),
            "Expected hint line with keybindings"
        );
    }
}
