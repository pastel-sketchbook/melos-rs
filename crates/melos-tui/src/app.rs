use crossterm::event::{KeyCode, KeyModifiers};
use melos_core::config::ScriptEntry;
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

/// Which panel is currently focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePanel {
    Packages,
    Commands,
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

/// A row in the command/script list, pre-computed for display.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields read by views/commands.rs in Batch 50
pub struct CommandRow {
    /// Display name (e.g. "analyze" or script name).
    pub name: String,
    /// Optional description (scripts only).
    pub description: Option<String>,
    /// Whether this is a built-in command (vs. a user script).
    pub is_builtin: bool,
}

/// Built-in commands available in the command picker.
const BUILTIN_COMMANDS: &[&str] = &[
    "analyze",
    "bootstrap",
    "build",
    "clean",
    "exec",
    "format",
    "health",
    "list",
    "publish",
    "run",
    "test",
    "version",
];

/// Top-level application state.
pub struct App {
    /// Current state machine phase.
    pub state: AppState,
    /// Whether the user has requested to quit.
    quit: bool,
    /// Which panel is currently focused.
    pub active_panel: ActivePanel,
    /// Workspace name (from config).
    pub workspace_name: Option<String>,
    /// Config source label (e.g. "melos.yaml" or "pubspec.yaml").
    pub config_source_label: Option<String>,
    /// Pre-computed package rows for display.
    pub package_rows: Vec<PackageRow>,
    /// Currently selected package index.
    pub selected_package: usize,
    /// Pre-computed command/script rows for display.
    pub command_rows: Vec<CommandRow>,
    /// Currently selected command index.
    pub selected_command: usize,
    /// Page size for PgUp/PgDown (set from terminal height).
    pub page_size: usize,
    /// Workspace warnings collected during loading.
    pub warnings: Vec<String>,
    /// Whether the help overlay is currently visible.
    pub show_help: bool,
}

impl App {
    /// Create a new App in the Idle state with no workspace loaded.
    pub fn new() -> Self {
        // Pre-populate built-in commands even without a workspace.
        let command_rows = BUILTIN_COMMANDS
            .iter()
            .map(|name| CommandRow {
                name: (*name).to_string(),
                description: None,
                is_builtin: true,
            })
            .collect();

        Self {
            state: AppState::Idle,
            quit: false,
            active_panel: ActivePanel::Packages,
            workspace_name: None,
            config_source_label: None,
            package_rows: Vec::new(),
            selected_package: 0,
            command_rows,
            selected_command: 0,
            page_size: 20,
            warnings: Vec::new(),
            show_help: false,
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

        // Build command rows: built-ins first, then non-private scripts sorted by name.
        let mut commands: Vec<CommandRow> = BUILTIN_COMMANDS
            .iter()
            .map(|name| CommandRow {
                name: (*name).to_string(),
                description: None,
                is_builtin: true,
            })
            .collect();

        let mut scripts: Vec<(&String, &ScriptEntry)> = workspace
            .config
            .scripts
            .iter()
            .filter(|(_, entry)| !entry.is_private())
            .collect();
        scripts.sort_by_key(|(name, _)| name.to_lowercase());

        for (name, entry) in scripts {
            commands.push(CommandRow {
                name: name.clone(),
                description: entry.description().map(String::from),
                is_builtin: false,
            });
        }

        self.command_rows = commands;

        // Reset selections.
        self.selected_package = 0;
        self.selected_command = 0;
    }

    /// Returns the number of packages.
    pub fn package_count(&self) -> usize {
        self.package_rows.len()
    }

    /// Returns the number of commands/scripts.
    #[allow(dead_code)] // Used by views/commands.rs in Batch 50
    pub fn command_count(&self) -> usize {
        self.command_rows.len()
    }

    /// Returns true if the app should exit.
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// Handle a key press event.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // When help overlay is visible, only ? and Esc dismiss it.
        if self.show_help {
            match code {
                KeyCode::Char('?') | KeyCode::Esc => self.show_help = false,
                _ => {}
            }
            return;
        }

        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let half_page = (self.page_size / 2).max(1) as isize;

        match (code, ctrl) {
            (KeyCode::Char('q'), false) => self.quit = true,
            (KeyCode::Char('c'), true) => self.quit = true,
            (KeyCode::Esc, _) => self.handle_esc(),

            // Panel switching
            (KeyCode::Tab | KeyCode::BackTab, _) => self.toggle_panel(),
            (KeyCode::Char('h'), false) | (KeyCode::Left, _) => self.focus_panel_left(),
            (KeyCode::Char('l'), false) | (KeyCode::Right, _) => self.focus_panel_right(),

            // Single-step navigation
            (KeyCode::Up | KeyCode::Char('k'), false) => self.move_selection(-1),
            (KeyCode::Down | KeyCode::Char('j'), false) => self.move_selection(1),

            // Jump to start/end
            (KeyCode::Home | KeyCode::Char('g'), false) => self.jump_selection_start(),
            (KeyCode::End | KeyCode::Char('G'), false) => self.jump_selection_end(),

            // Half-page scroll: Ctrl+d / Ctrl+u
            (KeyCode::Char('d'), true) => self.move_selection(half_page),
            (KeyCode::Char('u'), true) => self.move_selection(-half_page),

            // Full-page scroll: PgUp/PgDn, Ctrl+f/Ctrl+b, or plain f/b
            (KeyCode::PageDown, _) | (KeyCode::Char('f'), true | false) => {
                self.move_selection(self.page_size as isize)
            }
            (KeyCode::PageUp, _) | (KeyCode::Char('b'), true | false) => {
                self.move_selection(-(self.page_size as isize))
            }

            // Help overlay
            (KeyCode::Char('?'), _) => self.show_help = true,

            _ => {}
        }
    }

    /// Handle the Escape key based on current state.
    fn handle_esc(&mut self) {
        match self.state {
            AppState::Running => {
                self.state = AppState::Idle;
            }
            AppState::Done => {
                self.state = AppState::Idle;
            }
            AppState::Idle => {
                self.quit = true;
            }
        }
    }

    /// Toggle between Packages and Commands panels.
    fn toggle_panel(&mut self) {
        self.active_panel = match self.active_panel {
            ActivePanel::Packages => ActivePanel::Commands,
            ActivePanel::Commands => ActivePanel::Packages,
        };
    }

    /// Focus the left panel (Packages).
    fn focus_panel_left(&mut self) {
        self.active_panel = ActivePanel::Packages;
    }

    /// Focus the right panel (Commands).
    fn focus_panel_right(&mut self) {
        self.active_panel = ActivePanel::Commands;
    }

    /// Get (selected_index, list_len) for the active panel.
    fn active_selection(&self) -> (usize, usize) {
        match self.active_panel {
            ActivePanel::Packages => (self.selected_package, self.package_rows.len()),
            ActivePanel::Commands => (self.selected_command, self.command_rows.len()),
        }
    }

    /// Set the selected index for the active panel.
    fn set_active_selection(&mut self, idx: usize) {
        match self.active_panel {
            ActivePanel::Packages => self.selected_package = idx,
            ActivePanel::Commands => self.selected_command = idx,
        }
    }

    /// Move the selection by `delta` (negative = up, positive = down) with wrapping.
    fn move_selection(&mut self, delta: isize) {
        let (current, len) = self.active_selection();
        if len == 0 {
            return;
        }
        let new_idx = if delta < 0 {
            let abs = (-delta) as usize;
            if abs > current {
                // Wrap: single-step up wraps to end; page-up clamps to 0.
                if abs == 1 {
                    len - 1
                } else {
                    0
                }
            } else {
                current - abs
            }
        } else {
            let abs = delta as usize;
            if current + abs >= len {
                // Wrap: single-step down wraps to start; page-down clamps to end.
                if abs == 1 {
                    0
                } else {
                    len - 1
                }
            } else {
                current + abs
            }
        };
        self.set_active_selection(new_idx);
    }

    /// Jump to the first item in the active panel.
    fn jump_selection_start(&mut self) {
        self.set_active_selection(0);
    }

    /// Jump to the last item in the active panel.
    fn jump_selection_end(&mut self) {
        let (_, len) = self.active_selection();
        if len > 0 {
            self.set_active_selection(len - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: press a key with no modifiers.
    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(code, KeyModifiers::NONE);
    }

    /// Helper: press a key with Ctrl held.
    fn ctrl(app: &mut App, code: KeyCode) {
        app.handle_key(code, KeyModifiers::CONTROL);
    }

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

    /// Helper: create an App with N fake command rows (replacing defaults).
    fn app_with_commands(count: usize) -> App {
        let mut app = App::new();
        app.command_rows = (0..count)
            .map(|i| CommandRow {
                name: format!("cmd_{i}"),
                description: None,
                is_builtin: i < 3,
            })
            .collect();
        app.active_panel = ActivePanel::Commands;
        app
    }

    // --- Basic state tests ---

    #[test]
    fn test_new_app_is_idle() {
        let app = App::new();
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
        assert_eq!(app.active_panel, ActivePanel::Packages);
    }

    #[test]
    fn test_new_app_has_builtin_commands() {
        let app = App::new();
        assert_eq!(app.command_rows.len(), BUILTIN_COMMANDS.len());
        assert!(app.command_rows.iter().all(|r| r.is_builtin));
        assert_eq!(app.command_rows[0].name, "analyze");
    }

    #[test]
    fn test_q_quits() {
        let mut app = App::new();
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit());
    }

    #[test]
    fn test_esc_in_idle_quits() {
        let mut app = App::new();
        press(&mut app, KeyCode::Esc);
        assert!(app.should_quit());
    }

    #[test]
    fn test_esc_in_done_returns_to_idle() {
        let mut app = App::new();
        app.state = AppState::Done;
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_esc_in_running_returns_to_idle() {
        let mut app = App::new();
        app.state = AppState::Running;
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_unknown_key_does_nothing() {
        let mut app = App::new();
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    // --- Tab switching tests ---

    #[test]
    fn test_tab_toggles_to_commands() {
        let mut app = App::new();
        assert_eq!(app.active_panel, ActivePanel::Packages);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.active_panel, ActivePanel::Commands);
    }

    #[test]
    fn test_tab_toggles_back_to_packages() {
        let mut app = App::new();
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.active_panel, ActivePanel::Packages);
    }

    #[test]
    fn test_backtab_toggles_panel() {
        let mut app = App::new();
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.active_panel, ActivePanel::Commands);
    }

    #[test]
    fn test_selection_state_independent_per_panel() {
        let mut app = app_with_packages(10);
        // Move package selection to 5.
        for _ in 0..5 {
            press(&mut app, KeyCode::Down);
        }
        assert_eq!(app.selected_package, 5);
        assert_eq!(app.selected_command, 0);

        // Switch to commands, move down 2.
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_command, 2);

        // Package selection unchanged.
        assert_eq!(app.selected_package, 5);
    }

    // --- Package navigation tests ---

    #[test]
    fn test_pkg_down_wraps_to_first() {
        let mut app = app_with_packages(3);
        app.selected_package = 2;
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_pkg_up_wraps_to_last() {
        let mut app = app_with_packages(3);
        app.selected_package = 0;
        press(&mut app, KeyCode::Up);
        assert_eq!(app.selected_package, 2);
    }

    #[test]
    fn test_pkg_down_increments() {
        let mut app = app_with_packages(5);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_package, 1);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_package, 2);
    }

    #[test]
    fn test_pkg_up_decrements() {
        let mut app = app_with_packages(5);
        app.selected_package = 3;
        press(&mut app, KeyCode::Up);
        assert_eq!(app.selected_package, 2);
    }

    #[test]
    fn test_pkg_home_jumps_to_first() {
        let mut app = app_with_packages(10);
        app.selected_package = 7;
        press(&mut app, KeyCode::Home);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_pkg_end_jumps_to_last() {
        let mut app = app_with_packages(10);
        press(&mut app, KeyCode::End);
        assert_eq!(app.selected_package, 9);
    }

    #[test]
    fn test_pkg_page_down_moves_by_page_size() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.selected_package, 10);
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.selected_package, 20);
    }

    #[test]
    fn test_pkg_page_down_clamps_to_last() {
        let mut app = app_with_packages(5);
        app.page_size = 10;
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.selected_package, 4);
    }

    #[test]
    fn test_pkg_page_up_moves_by_page_size() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        app.selected_package = 30;
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.selected_package, 20);
    }

    #[test]
    fn test_pkg_page_up_clamps_to_first() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        app.selected_package = 3;
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_pkg_navigation_on_empty() {
        let mut app = App::new();
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Home);
        press(&mut app, KeyCode::End);
        press(&mut app, KeyCode::PageUp);
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_package_count() {
        let app = app_with_packages(7);
        assert_eq!(app.package_count(), 7);
    }

    // --- Vi-style key tests ---

    #[test]
    fn test_j_moves_down() {
        let mut app = app_with_packages(5);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_package, 1);
    }

    #[test]
    fn test_k_moves_up() {
        let mut app = app_with_packages(5);
        app.selected_package = 3;
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected_package, 2);
    }

    #[test]
    fn test_g_jumps_to_first() {
        let mut app = app_with_packages(10);
        app.selected_package = 7;
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_shift_g_jumps_to_last() {
        let mut app = app_with_packages(10);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.selected_package, 9);
    }

    #[test]
    fn test_f_pages_down() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.selected_package, 10);
    }

    #[test]
    fn test_b_pages_up() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        app.selected_package = 20;
        press(&mut app, KeyCode::Char('b'));
        assert_eq!(app.selected_package, 10);
    }

    // --- Command navigation tests ---

    #[test]
    fn test_cmd_down_wraps_to_first() {
        let mut app = app_with_commands(3);
        app.selected_command = 2;
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_command, 0);
    }

    #[test]
    fn test_cmd_up_wraps_to_last() {
        let mut app = app_with_commands(3);
        app.selected_command = 0;
        press(&mut app, KeyCode::Up);
        assert_eq!(app.selected_command, 2);
    }

    #[test]
    fn test_cmd_home_end() {
        let mut app = app_with_commands(10);
        app.selected_command = 5;
        press(&mut app, KeyCode::Home);
        assert_eq!(app.selected_command, 0);
        press(&mut app, KeyCode::End);
        assert_eq!(app.selected_command, 9);
    }

    #[test]
    fn test_cmd_page_navigation() {
        let mut app = app_with_commands(30);
        app.page_size = 10;
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.selected_command, 10);
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.selected_command, 0);
    }

    #[test]
    fn test_cmd_navigation_on_empty() {
        let mut app = App::new();
        app.command_rows.clear();
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_command, 0);
    }

    #[test]
    fn test_command_count() {
        let app = App::new();
        assert_eq!(app.command_count(), BUILTIN_COMMANDS.len());
    }

    // --- Ctrl key tests ---

    #[test]
    fn test_ctrl_c_quits() {
        let mut app = App::new();
        ctrl(&mut app, KeyCode::Char('c'));
        assert!(app.should_quit());
    }

    #[test]
    fn test_ctrl_d_half_page_down() {
        let mut app = app_with_packages(50);
        app.page_size = 20;
        ctrl(&mut app, KeyCode::Char('d'));
        assert_eq!(app.selected_package, 10);
    }

    #[test]
    fn test_ctrl_u_half_page_up() {
        let mut app = app_with_packages(50);
        app.page_size = 20;
        app.selected_package = 30;
        ctrl(&mut app, KeyCode::Char('u'));
        assert_eq!(app.selected_package, 20);
    }

    #[test]
    fn test_ctrl_f_full_page_down() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        ctrl(&mut app, KeyCode::Char('f'));
        assert_eq!(app.selected_package, 10);
    }

    #[test]
    fn test_ctrl_b_full_page_up() {
        let mut app = app_with_packages(50);
        app.page_size = 10;
        app.selected_package = 20;
        ctrl(&mut app, KeyCode::Char('b'));
        assert_eq!(app.selected_package, 10);
    }

    // --- h/l panel focus tests ---

    #[test]
    fn test_h_focuses_packages_panel() {
        let mut app = App::new();
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.active_panel, ActivePanel::Packages);
    }

    #[test]
    fn test_l_focuses_commands_panel() {
        let mut app = App::new();
        assert_eq!(app.active_panel, ActivePanel::Packages);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.active_panel, ActivePanel::Commands);
    }

    #[test]
    fn test_h_on_packages_panel_stays() {
        let mut app = App::new();
        assert_eq!(app.active_panel, ActivePanel::Packages);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.active_panel, ActivePanel::Packages);
    }

    #[test]
    fn test_l_on_commands_panel_stays() {
        let mut app = App::new();
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.active_panel, ActivePanel::Commands);
    }

    // --- Help toggle tests ---

    #[test]
    fn test_question_mark_opens_help() {
        let mut app = App::new();
        assert!(!app.show_help);
        press(&mut app, KeyCode::Char('?'));
        assert!(app.show_help);
    }

    #[test]
    fn test_question_mark_closes_help() {
        let mut app = App::new();
        app.show_help = true;
        press(&mut app, KeyCode::Char('?'));
        assert!(!app.show_help);
    }

    #[test]
    fn test_esc_closes_help() {
        let mut app = App::new();
        app.show_help = true;
        press(&mut app, KeyCode::Esc);
        assert!(!app.show_help);
        // Should NOT quit (Esc only closes help, doesn't propagate)
        assert!(!app.should_quit());
    }

    #[test]
    fn test_help_blocks_other_keys() {
        let mut app = app_with_packages(10);
        app.show_help = true;
        press(&mut app, KeyCode::Char('j'));
        // Navigation should be blocked when help is open
        assert_eq!(app.selected_package, 0);
        press(&mut app, KeyCode::Char('q'));
        // Quit should be blocked too
        assert!(!app.should_quit());
        // Help still open
        assert!(app.show_help);
    }

    #[test]
    fn test_help_toggle_roundtrip() {
        let mut app = App::new();
        press(&mut app, KeyCode::Char('?'));
        assert!(app.show_help);
        press(&mut app, KeyCode::Char('?'));
        assert!(!app.show_help);
        // App still idle, not quit
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }
}
