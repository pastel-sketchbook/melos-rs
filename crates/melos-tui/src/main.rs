use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{Event, EventStream, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use melos_core::workspace::Workspace;
use ratatui::prelude::*;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

mod app;
mod dispatch;
mod event;
mod logging;
mod theme;
mod ui;
mod views;

use app::App;
use event::{poll_task_handle, recv_core_event};
use theme::Theme;

/// Interactive terminal UI for melos-rs workspace management.
#[derive(Parser)]
#[command(name = "melos-tui", version)]
struct Cli {
    /// Path to workspace directory (defaults to current directory).
    #[arg(long, value_name = "DIR")]
    workspace: Option<PathBuf>,

    /// Color theme to use.
    ///
    /// Available themes: dark, light, catppuccin-mocha, catppuccin-latte,
    /// dracula, everforest-dark, everforest-light, gruvbox-dark, gruvbox-light,
    /// nord, nord-light, one-dark, one-light, rose-pine, rose-pine-dawn,
    /// solarized-dark, solarized-light, tokyo-night, tokyo-night-light.
    #[arg(long, value_name = "NAME", default_value = "dark")]
    theme: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize file logging before anything else.
    let _guard = logging::init();

    info!("melos-tui starting");

    let cli = Cli::parse();

    // Respect NO_COLOR / TERM=dumb: TUI requires a capable terminal.
    if std::env::var_os("NO_COLOR").is_some() {
        anyhow::bail!(
            "NO_COLOR is set. The TUI requires color support.\n\
             Use `melos-rs` CLI commands directly instead."
        );
    }
    if std::env::var("TERM").ok().as_deref() == Some("dumb") {
        anyhow::bail!(
            "TERM=dumb detected. The TUI requires a capable terminal.\n\
             Use `melos-rs` CLI commands directly instead."
        );
    }

    // If --workspace is provided, change to that directory first.
    if let Some(ref ws_path) = cli.workspace {
        std::env::set_current_dir(ws_path).with_context(|| {
            format!(
                "Failed to change to workspace directory: {}",
                ws_path.display()
            )
        })?;
        info!(path = %ws_path.display(), "changed to workspace directory");
    }

    // Install panic hook that restores terminal before printing the panic.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original_hook(info);
    }));

    // Try to load the workspace before entering raw mode so errors print normally.
    let workspace = load_workspace();

    // Resolve theme by name (fall back to default dark if unknown).
    let theme_index = Theme::available_names()
        .iter()
        .position(|&n| n == cli.theme)
        .unwrap_or(0);
    let theme = Theme::by_name(&cli.theme).unwrap_or_else(|| {
        eprintln!(
            "Unknown theme '{}'. Available: {}",
            cli.theme,
            Theme::available_names().join(", ")
        );
        eprintln!("Falling back to 'dark' theme.");
        Theme::default()
    });

    // Set up terminal.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    info!("terminal initialized, entering event loop");

    // Run the app.
    let result = run(&mut terminal, workspace, theme, theme_index).await;

    // Always restore terminal, even on error.
    restore_terminal()?;

    match &result {
        Ok(()) => info!("melos-tui exiting normally"),
        Err(e) => error!("melos-tui exiting with error: {e}"),
    }

    result
}

/// Load the workspace, returning Ok(Workspace) or an error message.
fn load_workspace() -> Result<Workspace> {
    let result = Workspace::find_and_load(None).context("Failed to load workspace");
    match &result {
        Ok(ws) => info!(
            name = ws.config.name,
            packages = ws.packages.len(),
            "workspace loaded"
        ),
        Err(e) => warn!("workspace load failed: {e}"),
    }
    result
}

/// Main event loop: render, poll events, update state.
async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    workspace: Result<Workspace>,
    theme: Theme,
    theme_index: usize,
) -> Result<()> {
    let mut app = App::new(theme);

    // Set the theme index so 't' cycling starts from the correct position.
    app.theme_index = theme_index;

    // Wrap workspace in Arc for sharing with spawned command tasks.
    let workspace = match workspace {
        Ok(ws) => {
            app.load_workspace(&ws);
            Some(Arc::new(ws))
        }
        Err(e) => {
            app.warnings.push(format!("Workspace load failed: {e}"));
            None
        }
    };

    // Set page size from terminal height (body area minus header, footer, table border, header row).
    let term_height = terminal.size()?.height;
    app.update_page_size(term_height);
    debug!(
        height = term_height,
        page_size = app.page_size,
        "terminal size"
    );

    // Event sources.
    let mut event_stream = EventStream::new();
    let mut core_rx: Option<mpsc::UnboundedReceiver<melos_core::events::Event>> = None;
    let mut task_handle: Option<JoinHandle<Result<dispatch::DispatchResult>>> = None;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        // Adjust tick rate: faster during command execution for responsive updates.
        let tick_duration = match app.state {
            app::AppState::Running => std::time::Duration::from_millis(66),
            _ => std::time::Duration::from_millis(250),
        };
        tick.reset_after(tick_duration);

        tokio::select! {
            maybe_event = event_stream.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        debug!(
                            code = ?key.code,
                            modifiers = ?key.modifiers,
                            state = ?app.state,
                            "key press"
                        );
                        app.handle_key(key.code, key.modifiers);
                    }
                    Some(Ok(Event::Resize(_w, h))) => {
                        debug!(height = h, "terminal resized");
                        app.update_page_size(h);
                    }
                    Some(Err(e)) => {
                        error!("event stream error: {e}");
                        break;
                    }
                    None => {
                        debug!("event stream closed");
                        break;
                    }
                    _ => {}
                }
            }

            result = recv_core_event(&mut core_rx) => {
                match result {
                    Some(core_event) => {
                        debug!(event = ?core_event, "core event received");
                        app.handle_core_event(core_event);
                    }
                    None => {
                        // Channel closed: sender dropped. Mark channel as done
                        // but do NOT await the task handle here -- that would
                        // block the entire event loop. The separate
                        // `poll_task_handle` branch will pick it up.
                        debug!("core event channel closed");
                        core_rx = None;
                    }
                }
            }

            join_result = poll_task_handle(&mut task_handle) => {
                // Task handle resolved. Extract health report and notify app.
                task_handle = None;

                // Re-enable raw mode in case a child process corrupted
                // terminal settings (e.g. by opening /dev/tty directly).
                let _ = enable_raw_mode();

                let mapped = match join_result {
                    Ok(Ok(dr)) => {
                        info!(
                            results = dr.package_results.results.len(),
                            has_health = dr.health_report.is_some(),
                            "command task completed successfully"
                        );
                        if let Some(report) = dr.health_report {
                            app.set_health_report(report);
                        }
                        Ok(Ok(()))
                    }
                    Ok(Err(e)) => {
                        error!("command task returned error: {e}");
                        Ok(Err(e))
                    }
                    Err(e) => {
                        if e.is_cancelled() {
                            debug!("command task was cancelled");
                        } else {
                            error!("command task panicked: {e}");
                        }
                        Err(e)
                    }
                };
                app.on_command_finished(mapped);
            }

            _ = tick.tick() => {
                // Periodic redraw (handled by loop top).
            }
        }

        // Handle pending cancel request.
        // Guard: only cancel if the task is still running (handle present).
        // If poll_task_handle already resolved, on_command_finished was called
        // and the state transitioned to Done; cancelling would incorrectly
        // revert to Idle.
        if app.pending_cancel {
            app.pending_cancel = false;
            if let Some(handle) = task_handle.take() {
                info!("cancel requested, aborting command task");
                handle.abort();
                core_rx = None;
                let _ = enable_raw_mode();
                app.on_command_cancelled();
            }
        }

        // Handle pending command execution request.
        if let Some(cmd_name) = app.pending_command.take() {
            if let Some(ref ws) = workspace {
                info!(command = cmd_name, "dispatching command");
                let ws = Arc::clone(ws);
                let name = cmd_name.clone();
                let opts = app.command_opts.take();
                let (tx, rx) = mpsc::unbounded_channel();

                let handle =
                    tokio::spawn(
                        async move { dispatch::dispatch_command(&name, &ws, tx, opts).await },
                    );

                core_rx = Some(rx);
                task_handle = Some(handle);
                app.start_command(&cmd_name);
            } else {
                warn!(command = cmd_name, "cannot execute: no workspace loaded");
                app.exec_messages
                    .push("Cannot execute: no workspace loaded".to_string());
            }
        }

        if app.should_quit() {
            info!("quit requested");
            // Abort any running command before exiting.
            if let Some(handle) = task_handle.take() {
                handle.abort();
            }
            break;
        }
    }

    Ok(())
}

/// Restore terminal to normal mode.
fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}
