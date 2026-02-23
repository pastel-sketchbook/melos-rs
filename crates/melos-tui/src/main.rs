use std::io;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use melos_core::workspace::Workspace;
use ratatui::prelude::*;

mod app;
mod ui;
mod views;

use app::App;

fn main() -> Result<()> {
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
    let result = run(&mut terminal, workspace);

    // Always restore terminal, even on error.
    restore_terminal()?;

    result
}

/// Load the workspace, returning Ok(Workspace) or an error message.
fn load_workspace() -> Result<Workspace> {
    Workspace::find_and_load(None).context("Failed to load workspace")
}

/// Main event loop: render, poll events, update state.
fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    workspace: Result<Workspace>,
) -> Result<()> {
    let mut app = App::new();

    // Load workspace data into app state.
    match workspace {
        Ok(ws) => app.load_workspace(&ws),
        Err(e) => {
            // Store error as a warning so it displays in the TUI.
            app.warnings.push(format!("Workspace load failed: {e}"));
        }
    }

    // Set page size from terminal height (body area minus header, footer, table border, header row).
    let term_height = terminal.size()?.height;
    app.page_size = term_height.saturating_sub(5) as usize;

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        // Block until a crossterm event arrives (250ms timeout for responsive quit).
        if event::poll(std::time::Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
        {
            // Only handle key press events (ignore release/repeat on supported terminals).
            if key.kind == KeyEventKind::Press {
                app.handle_key(key.code, key.modifiers);
            }
        }

        if app.should_quit() {
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
