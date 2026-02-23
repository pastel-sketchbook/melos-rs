use std::io;
use std::sync::Arc;

use anyhow::{Context, Result};
use crossterm::{
    event::{Event, EventStream, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use melos_core::commands::PackageResults;
use melos_core::workspace::Workspace;
use ratatui::prelude::*;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

mod app;
mod dispatch;
mod event;
mod ui;
mod views;

use app::App;
use event::recv_core_event;

#[tokio::main]
async fn main() -> Result<()> {
    // Install panic hook that restores terminal before printing the panic.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original_hook(info);
    }));

    // Try to load the workspace before entering raw mode so errors print normally.
    let workspace = load_workspace();

    // Set up terminal.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the app.
    let result = run(&mut terminal, workspace).await;

    // Always restore terminal, even on error.
    restore_terminal()?;

    result
}

/// Load the workspace, returning Ok(Workspace) or an error message.
fn load_workspace() -> Result<Workspace> {
    Workspace::find_and_load(None).context("Failed to load workspace")
}

/// Main event loop: render, poll events, update state.
async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    workspace: Result<Workspace>,
) -> Result<()> {
    let mut app = App::new();

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
    app.page_size = term_height.saturating_sub(5) as usize;

    // Event sources.
    let mut event_stream = EventStream::new();
    let mut core_rx: Option<mpsc::UnboundedReceiver<melos_core::events::Event>> = None;
    let mut task_handle: Option<JoinHandle<Result<PackageResults>>> = None;
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
                        app.handle_key(key.code, key.modifiers);
                    }
                    Some(Err(_)) | None => {
                        // Event stream closed or errored; exit gracefully.
                        break;
                    }
                    _ => {}
                }
            }

            result = recv_core_event(&mut core_rx) => {
                match result {
                    Some(core_event) => {
                        app.handle_core_event(core_event);
                    }
                    None => {
                        // Channel closed: command task finished. Join the handle.
                        core_rx = None;
                        if let Some(handle) = task_handle.take() {
                            let join_result = handle.await;
                            // Map Result<Result<PackageResults>> to Result<Result<()>>
                            // by discarding PackageResults (already tracked via events).
                            let mapped = match join_result {
                                Ok(Ok(_)) => Ok(Ok(())),
                                Ok(Err(e)) => Ok(Err(e)),
                                Err(e) => Err(e),
                            };
                            app.on_command_finished(mapped);
                        }
                    }
                }
            }

            _ = tick.tick() => {
                // Periodic redraw (handled by loop top).
            }
        }

        // Handle pending cancel request.
        if app.pending_cancel {
            app.pending_cancel = false;
            if let Some(handle) = task_handle.take() {
                handle.abort();
            }
            core_rx = None;
            app.on_command_cancelled();
        }

        // Handle pending command execution request.
        if let Some(cmd_name) = app.pending_command.take() {
            if let Some(ref ws) = workspace {
                let ws = Arc::clone(ws);
                let name = cmd_name.clone();
                let (tx, rx) = mpsc::unbounded_channel();

                let handle =
                    tokio::spawn(async move { dispatch::dispatch_command(&name, &ws, tx).await });

                core_rx = Some(rx);
                task_handle = Some(handle);
                app.start_command(&cmd_name);
            } else {
                app.exec_messages
                    .push("Cannot execute: no workspace loaded".to_string());
            }
        }

        if app.should_quit() {
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
