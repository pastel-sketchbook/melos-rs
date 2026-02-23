use std::io;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

mod app;
mod ui;

use app::App;

fn main() -> Result<()> {
    // Install panic hook that restores terminal before printing the panic.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original_hook(info);
    }));

    // Set up terminal.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the app.
    let result = run(&mut terminal);

    // Always restore terminal, even on error.
    restore_terminal()?;

    result
}

/// Main event loop: render, poll events, update state.
fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        // Block until a crossterm event arrives (250ms timeout for responsive quit).
        if event::poll(std::time::Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
        {
            // Only handle key press events (ignore release/repeat on supported terminals).
            if key.kind == KeyEventKind::Press {
                app.handle_key(key.code);
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
