use crossterm::event::KeyCode;

/// Application state for the TUI.
///
/// The state machine has three phases:
/// - `Idle`: workspace loaded, user navigates packages/commands
/// - `Running`: a command is executing, live progress displayed
/// - `Done`: results displayed, user can scroll or return to Idle
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Running and Done are used in tests; wired in Batch 51
pub enum AppState {
    Idle,
    Running,
    Done,
}

/// Top-level application state.
pub struct App {
    /// Current state machine phase.
    pub state: AppState,
    /// Whether the user has requested to quit.
    quit: bool,
}

impl App {
    /// Create a new App in the Idle state.
    pub fn new() -> Self {
        Self {
            state: AppState::Idle,
            quit: false,
        }
    }

    /// Returns true if the app should exit.
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// Handle a key press event.
    pub fn handle_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => self.handle_esc(),
            _ => {}
        }
    }

    /// Handle the Escape key based on current state.
    fn handle_esc(&mut self) {
        match self.state {
            AppState::Running => {
                // Cancel execution, return to Idle.
                self.state = AppState::Idle;
            }
            AppState::Done => {
                // Dismiss results, return to Idle.
                self.state = AppState::Idle;
            }
            AppState::Idle => {
                // In Idle, Esc quits.
                self.quit = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_app_is_idle() {
        let app = App::new();
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_q_quits() {
        let mut app = App::new();
        app.handle_key(KeyCode::Char('q'));
        assert!(app.should_quit());
    }

    #[test]
    fn test_esc_in_idle_quits() {
        let mut app = App::new();
        app.handle_key(KeyCode::Esc);
        assert!(app.should_quit());
    }

    #[test]
    fn test_esc_in_done_returns_to_idle() {
        let mut app = App::new();
        app.state = AppState::Done;
        app.handle_key(KeyCode::Esc);
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_esc_in_running_returns_to_idle() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.handle_key(KeyCode::Esc);
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_unknown_key_does_nothing() {
        let mut app = App::new();
        app.handle_key(KeyCode::Char('x'));
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }
}
