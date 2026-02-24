use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use melos_core::commands::health::HealthReport;

use crate::app::App;
use crate::theme::Theme;

/// Tab labels for the health dashboard.
const TAB_LABELS: &[&str] = &["Version Drift", "Missing Fields", "SDK Consistency"];

/// Draw the health dashboard with three tabs.
///
/// The active tab is determined by `app.health_tab`. Tab/BackTab cycle through
/// the tabs in the Done state key handler.
pub fn draw_health(frame: &mut Frame, area: Rect, app: &App) {
    let report = match &app.health_report {
        Some(r) => r,
        None => return,
    };
    let theme = &app.theme;

    // Layout: tab bar (1) + spacer (1) + content (fill).
    let [tab_area, _spacer, content_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);

    draw_tab_bar(frame, tab_area, app.health_tab, theme);

    match app.health_tab {
        0 => draw_version_drift(frame, content_area, report, theme),
        1 => draw_missing_fields(frame, content_area, report, theme),
        _ => draw_sdk_consistency(frame, content_area, report, theme),
    }
}

/// Render the tab bar with three tabs, highlighting the active one.
fn draw_tab_bar(frame: &mut Frame, area: Rect, active: usize, theme: &Theme) {
    let mut spans = Vec::new();
    for (i, label) in TAB_LABELS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(theme.text_muted)));
        }
        let style = if i == active {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(theme.text_muted)
        };
        spans.push(Span::styled(*label, style));
    }
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Render the Version Drift tab content.
fn draw_version_drift(frame: &mut Frame, area: Rect, report: &HealthReport, theme: &Theme) {
    let mut lines: Vec<Line<'_>> = Vec::new();

    match &report.version_drift {
        None => {
            lines.push(Line::from(Span::styled(
                "Version drift check was not enabled.",
                Style::default().fg(theme.text_muted),
            )));
        }
        Some(drifts) if drifts.is_empty() => {
            lines.push(Line::from(Span::styled(
                "No version drift detected.",
                Style::default().fg(theme.success),
            )));
        }
        Some(drifts) => {
            lines.push(Line::from(Span::styled(
                format!("{} dependencies with version drift:", drifts.len()),
                Style::default()
                    .fg(theme.header)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));

            for drift in drifts {
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {} ({} constraints):",
                        drift.dependency,
                        drift.constraints.len()
                    ),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                )));
                for usage in &drift.constraints {
                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(&usage.constraint, Style::default().fg(theme.accent)),
                        Span::styled(
                            format!("  used by: {}", usage.packages.join(", ")),
                            Style::default().fg(theme.text_muted),
                        ),
                    ]));
                }
                lines.push(Line::from(""));
            }
        }
    }

    let block = Block::default()
        .borders(Borders::TOP)
        .title(" Version Drift ");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Render the Missing Fields tab content.
fn draw_missing_fields(frame: &mut Frame, area: Rect, report: &HealthReport, theme: &Theme) {
    let mut lines: Vec<Line<'_>> = Vec::new();

    match &report.missing_fields {
        None => {
            lines.push(Line::from(Span::styled(
                "Missing fields check was not enabled.",
                Style::default().fg(theme.text_muted),
            )));
        }
        Some(issues) if issues.is_empty() => {
            lines.push(Line::from(Span::styled(
                "All packages have required fields.",
                Style::default().fg(theme.success),
            )));
        }
        Some(issues) => {
            lines.push(Line::from(Span::styled(
                format!("{} packages with missing fields:", issues.len()),
                Style::default()
                    .fg(theme.header)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));

            for issue in issues {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {}: ", issue.package),
                        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(issue.missing.join(", "), Style::default().fg(theme.error)),
                ]));
            }
        }
    }

    let block = Block::default()
        .borders(Borders::TOP)
        .title(" Missing Fields ");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Render the SDK Consistency tab content.
fn draw_sdk_consistency(frame: &mut Frame, area: Rect, report: &HealthReport, theme: &Theme) {
    let mut lines: Vec<Line<'_>> = Vec::new();

    match &report.sdk_consistency {
        None => {
            lines.push(Line::from(Span::styled(
                "SDK consistency check was not enabled.",
                Style::default().fg(theme.text_muted),
            )));
        }
        Some(sdk) => {
            let has_issues = !sdk.missing_sdk.is_empty()
                || !sdk.dart_sdk_drift.is_empty()
                || !sdk.flutter_sdk_drift.is_empty();

            if !has_issues {
                lines.push(Line::from(Span::styled(
                    "SDK constraints are consistent.",
                    Style::default().fg(theme.success),
                )));
            } else {
                if !sdk.missing_sdk.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("{} packages missing SDK constraint:", sdk.missing_sdk.len()),
                        Style::default()
                            .fg(theme.header)
                            .add_modifier(Modifier::BOLD),
                    )));
                    for name in &sdk.missing_sdk {
                        lines.push(Line::from(Span::styled(
                            format!("  {name}"),
                            Style::default().fg(theme.error),
                        )));
                    }
                    lines.push(Line::from(""));
                }

                if !sdk.dart_sdk_drift.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "Dart SDK constraint drift:",
                        Style::default()
                            .fg(theme.header)
                            .add_modifier(Modifier::BOLD),
                    )));
                    for usage in &sdk.dart_sdk_drift {
                        lines.push(Line::from(vec![
                            Span::styled("  ", Style::default()),
                            Span::styled(&usage.constraint, Style::default().fg(theme.accent)),
                            Span::styled(
                                format!("  used by: {}", usage.packages.join(", ")),
                                Style::default().fg(theme.text_muted),
                            ),
                        ]));
                    }
                    lines.push(Line::from(""));
                }

                if !sdk.flutter_sdk_drift.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "Flutter SDK constraint drift:",
                        Style::default()
                            .fg(theme.header)
                            .add_modifier(Modifier::BOLD),
                    )));
                    for usage in &sdk.flutter_sdk_drift {
                        lines.push(Line::from(vec![
                            Span::styled("  ", Style::default()),
                            Span::styled(&usage.constraint, Style::default().fg(theme.accent)),
                            Span::styled(
                                format!("  used by: {}", usage.packages.join(", ")),
                                Style::default().fg(theme.text_muted),
                            ),
                        ]));
                    }
                }
            }
        }
    }

    let block = Block::default()
        .borders(Borders::TOP)
        .title(" SDK Consistency ");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use melos_core::commands::health::{
        ConstraintUsage, MissingFieldsIssue, SdkConsistencyResult, VersionDriftIssue,
    };
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{App, AppState};

    /// Helper: render a frame into a test buffer.
    fn render_frame(
        draw_fn: impl FnOnce(&mut Frame, Rect, &App),
        app: &App,
        width: u16,
        height: u16,
    ) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_fn(frame, area, app);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Extract all lines from the buffer as a single string.
    fn buffer_text(buf: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn make_clean_report() -> HealthReport {
        HealthReport {
            version_drift: Some(vec![]),
            missing_fields: Some(vec![]),
            sdk_consistency: Some(SdkConsistencyResult::default()),
            total_issues: 0,
        }
    }

    fn app_with_health(report: HealthReport, tab: usize) -> App {
        let mut app = App::new(Theme::default());
        app.state = AppState::Done;
        app.running_command = Some("health".to_string());
        app.health_report = Some(report);
        app.health_tab = tab;
        app
    }

    #[test]
    fn test_health_no_report_renders_nothing() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Done;
        // No health_report set.
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        // All blank.
        assert!(
            text.trim().is_empty(),
            "Expected empty render without report, got:\n{text}"
        );
    }

    #[test]
    fn test_health_tab_bar_shows_labels() {
        let app = app_with_health(make_clean_report(), 0);
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("Version Drift"),
            "Expected tab label, got:\n{text}"
        );
        assert!(
            text.contains("Missing Fields"),
            "Expected tab label, got:\n{text}"
        );
        assert!(
            text.contains("SDK Consistency"),
            "Expected tab label, got:\n{text}"
        );
    }

    #[test]
    fn test_health_clean_version_drift() {
        let app = app_with_health(make_clean_report(), 0);
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("No version drift"),
            "Expected clean message, got:\n{text}"
        );
    }

    #[test]
    fn test_health_version_drift_with_issues() {
        let report = HealthReport {
            version_drift: Some(vec![VersionDriftIssue {
                dependency: "http".to_string(),
                constraints: vec![
                    ConstraintUsage {
                        constraint: "^0.13.0".to_string(),
                        packages: vec!["pkg_a".to_string()],
                    },
                    ConstraintUsage {
                        constraint: "^1.0.0".to_string(),
                        packages: vec!["pkg_b".to_string()],
                    },
                ],
            }]),
            missing_fields: None,
            sdk_consistency: None,
            total_issues: 1,
        };
        let app = app_with_health(report, 0);
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("http"),
            "Expected dependency name, got:\n{text}"
        );
        assert!(
            text.contains("^0.13.0"),
            "Expected constraint, got:\n{text}"
        );
        assert!(
            text.contains("pkg_a"),
            "Expected package name, got:\n{text}"
        );
    }

    #[test]
    fn test_health_clean_missing_fields() {
        let app = app_with_health(make_clean_report(), 1);
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("All packages have required fields"),
            "Expected clean message, got:\n{text}"
        );
    }

    #[test]
    fn test_health_missing_fields_with_issues() {
        let report = HealthReport {
            version_drift: None,
            missing_fields: Some(vec![MissingFieldsIssue {
                package: "my_pkg".to_string(),
                missing: vec!["description".to_string(), "homepage".to_string()],
            }]),
            sdk_consistency: None,
            total_issues: 1,
        };
        let app = app_with_health(report, 1);
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("my_pkg"),
            "Expected package name, got:\n{text}"
        );
        assert!(
            text.contains("description"),
            "Expected missing field, got:\n{text}"
        );
        assert!(
            text.contains("homepage"),
            "Expected missing field, got:\n{text}"
        );
    }

    #[test]
    fn test_health_clean_sdk_consistency() {
        let app = app_with_health(make_clean_report(), 2);
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("consistent"),
            "Expected clean message, got:\n{text}"
        );
    }

    #[test]
    fn test_health_sdk_missing_packages() {
        let report = HealthReport {
            version_drift: None,
            missing_fields: None,
            sdk_consistency: Some(SdkConsistencyResult {
                missing_sdk: vec!["orphan_pkg".to_string()],
                dart_sdk_drift: vec![],
                flutter_sdk_drift: vec![],
            }),
            total_issues: 1,
        };
        let app = app_with_health(report, 2);
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("orphan_pkg"),
            "Expected missing sdk package, got:\n{text}"
        );
    }

    #[test]
    fn test_health_sdk_dart_drift() {
        let report = HealthReport {
            version_drift: None,
            missing_fields: None,
            sdk_consistency: Some(SdkConsistencyResult {
                missing_sdk: vec![],
                dart_sdk_drift: vec![ConstraintUsage {
                    constraint: ">=3.0.0 <4.0.0".to_string(),
                    packages: vec!["pkg_x".to_string()],
                }],
                flutter_sdk_drift: vec![],
            }),
            total_issues: 1,
        };
        let app = app_with_health(report, 2);
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("Dart SDK"),
            "Expected Dart SDK drift section, got:\n{text}"
        );
        assert!(
            text.contains(">=3.0.0"),
            "Expected constraint, got:\n{text}"
        );
    }

    #[test]
    fn test_health_disabled_check_shows_message() {
        let report = HealthReport {
            version_drift: None,
            missing_fields: None,
            sdk_consistency: None,
            total_issues: 0,
        };
        let app = app_with_health(report, 0);
        let buf = render_frame(draw_health, &app, 80, 20);
        let text = buffer_text(&buf, 80, 20);
        assert!(
            text.contains("not enabled"),
            "Expected disabled message, got:\n{text}"
        );
    }
}
