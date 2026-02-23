use crossterm::event::KeyCode;
use melos_core::package::Package;
use melos_core::workspace::Workspace;

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

/// A row in the package table, pre-computed for display.
#[derive(Debug, Clone)]
pub struct PackageRow {
    pub name: String,
    pub version: String,
    pub sdk: &'static str,
    pub path: String,
    pub is_private: bool,
}

impl PackageRow {
    /// Build a display row from a Package and workspace root path.
    pub fn from_package(pkg: &Package, root: &std::path::Path) -> Self {
        let rel_path = pkg
            .path
            .strip_prefix(root)
            .unwrap_or(&pkg.path)
            .to_string_lossy()
            .to_string();

        Self {
            name: pkg.name.clone(),
            version: pkg.version.clone().unwrap_or_default(),
            sdk: if pkg.is_flutter { "Flutter" } else { "Dart" },
            path: rel_path,
            is_private: pkg.is_private(),
        }
    }
}

/// Top-level application state.
pub struct App {
    /// Current state machine phase.
    pub state: AppState,
    /// Whether the user has requested to quit.
    quit: bool,
    /// Workspace name (from config).
    pub workspace_name: Option<String>,
    /// Config source label (e.g. "melos.yaml" or "pubspec.yaml").
    pub config_source_label: Option<String>,
    /// Pre-computed package rows for display.
    pub package_rows: Vec<PackageRow>,
    /// Currently selected package index.
    pub selected_package: usize,
    /// Page size for PgUp/PgDown (set from terminal height).
    pub page_size: usize,
    /// Workspace warnings collected during loading.
    pub warnings: Vec<String>,
}

impl App {
    /// Create a new App in the Idle state with no workspace loaded.
    pub fn new() -> Self {
        Self {
            state: AppState::Idle,
            quit: false,
            workspace_name: None,
            config_source_label: None,
            package_rows: Vec::new(),
            selected_package: 0,
            page_size: 20,
            warnings: Vec::new(),
        }
    }

    /// Load workspace data into the app.
    pub fn load_workspace(&mut self, workspace: &Workspace) {
        self.workspace_name = Some(workspace.config.name.clone());
        self.config_source_label = Some(if workspace.config_source.is_legacy() {
            "melos.yaml".to_string()
        } else {
            "pubspec.yaml".to_string()
        });
        self.warnings = workspace.warnings.clone();

        self.package_rows = workspace
            .packages
            .iter()
            .map(|pkg| PackageRow::from_package(pkg, &workspace.root_path))
            .collect();

        // Reset selection to first package.
        self.selected_package = 0;
    }

    /// Returns the number of packages.
    pub fn package_count(&self) -> usize {
        self.package_rows.len()
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
            KeyCode::Up => self.select_previous(),
            KeyCode::Down => self.select_next(),
            KeyCode::Home => self.select_first(),
            KeyCode::End => self.select_last(),
            KeyCode::PageUp => self.select_page_up(),
            KeyCode::PageDown => self.select_page_down(),
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

    /// Move selection to the previous package (wraps to last).
    fn select_previous(&mut self) {
        if self.package_rows.is_empty() {
            return;
        }
        if self.selected_package == 0 {
            self.selected_package = self.package_rows.len() - 1;
        } else {
            self.selected_package -= 1;
        }
    }

    /// Move selection to the next package (wraps to first).
    fn select_next(&mut self) {
        if self.package_rows.is_empty() {
            return;
        }
        if self.selected_package >= self.package_rows.len() - 1 {
            self.selected_package = 0;
        } else {
            self.selected_package += 1;
        }
    }

    /// Jump to the first package.
    fn select_first(&mut self) {
        self.selected_package = 0;
    }

    /// Jump to the last package.
    fn select_last(&mut self) {
        if !self.package_rows.is_empty() {
            self.selected_package = self.package_rows.len() - 1;
        }
    }

    /// Move selection up by one page.
    fn select_page_up(&mut self) {
        if self.package_rows.is_empty() {
            return;
        }
        self.selected_package = self.selected_package.saturating_sub(self.page_size);
    }

    /// Move selection down by one page.
    fn select_page_down(&mut self) {
        if self.package_rows.is_empty() {
            return;
        }
        self.selected_package =
            (self.selected_package + self.page_size).min(self.package_rows.len() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create an App with N fake package rows.
    fn app_with_packages(count: usize) -> App {
        let mut app = App::new();
        for i in 0..count {
            app.package_rows.push(PackageRow {
                name: format!("pkg_{i}"),
                version: "1.0.0".to_string(),
                sdk: "Dart",
                path: format!("packages/pkg_{i}"),
                is_private: false,
            });
        }
        app
    }

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

    // --- Navigation tests ---

    #[test]
    fn test_down_wraps_to_first() {
        let mut app = app_with_packages(3);
        app.selected_package = 2;
        app.handle_key(KeyCode::Down);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_up_wraps_to_last() {
        let mut app = app_with_packages(3);
        app.selected_package = 0;
        app.handle_key(KeyCode::Up);
        assert_eq!(app.selected_package, 2);
    }

    #[test]
    fn test_down_increments() {
        let mut app = app_with_packages(5);
        app.handle_key(KeyCode::Down);
        assert_eq!(app.selected_package, 1);
        app.handle_key(KeyCode::Down);
        assert_eq!(app.selected_package, 2);
    }

    #[test]
    fn test_up_decrements() {
        let mut app = app_with_packages(5);
        app.selected_package = 3;
        app.handle_key(KeyCode::Up);
        assert_eq!(app.selected_package, 2);
    }

    #[test]
    fn test_home_jumps_to_first() {
        let mut app = app_with_packages(10);
        app.selected_package = 7;
        app.handle_key(KeyCode::Home);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_end_jumps_to_last() {
        let mut app = app_with_packages(10);
        app.handle_key(KeyCode::End);
        assert_eq!(app.selected_package, 9);
    }

    #[test]
    fn test_page_down_moves_by_page_size() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        app.handle_key(KeyCode::PageDown);
        assert_eq!(app.selected_package, 10);
        app.handle_key(KeyCode::PageDown);
        assert_eq!(app.selected_package, 20);
    }

    #[test]
    fn test_page_down_clamps_to_last() {
        let mut app = app_with_packages(5);
        app.page_size = 10;
        app.handle_key(KeyCode::PageDown);
        assert_eq!(app.selected_package, 4);
    }

    #[test]
    fn test_page_up_moves_by_page_size() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        app.selected_package = 30;
        app.handle_key(KeyCode::PageUp);
        assert_eq!(app.selected_package, 20);
    }

    #[test]
    fn test_page_up_clamps_to_first() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        app.selected_package = 3;
        app.handle_key(KeyCode::PageUp);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_navigation_on_empty_packages() {
        let mut app = App::new();
        // None of these should panic.
        app.handle_key(KeyCode::Up);
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Home);
        app.handle_key(KeyCode::End);
        app.handle_key(KeyCode::PageUp);
        app.handle_key(KeyCode::PageDown);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_package_count() {
        let app = app_with_packages(7);
        assert_eq!(app.package_count(), 7);
    }
}
