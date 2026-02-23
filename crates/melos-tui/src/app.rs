use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use melos_core::config::ScriptEntry;
use melos_core::events::Event as CoreEvent;
use melos_core::package::Package;
use melos_core::workspace::Workspace;

/// Application state for the TUI.
///
/// The state machine has three phases:
/// - `Idle`: workspace loaded, user navigates packages/commands
/// - `Running`: a command is executing, live progress displayed
/// - `Done`: results displayed, user can scroll or return to Idle
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Strip ANSI escape sequences from a string.
///
/// Handles CSI sequences (ESC [ ... final_byte) and OSC sequences
/// (ESC ] ... ST) which are the most common in terminal command output.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: consume until final byte (0x40-0x7E).
                    chars.next();
                    for ch in chars.by_ref() {
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC sequence: consume until ST (ESC \ or BEL).
                    chars.next();
                    while let Some(ch) = chars.next() {
                        if ch == '\x07' {
                            break;
                        }
                        if ch == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {
                    // Other escape: skip the next char.
                    chars.next();
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

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

    // --- Execution state (Batch 51) ---
    /// Command name requested by Enter key, consumed by the main loop.
    pub pending_command: Option<String>,
    /// Whether a cancel was requested (Esc during Running), consumed by the main loop.
    pub pending_cancel: bool,
    /// Name of the currently running command.
    pub running_command: Option<String>,
    /// Packages that have started but not yet finished.
    pub running_packages: Vec<String>,
    /// Finished package results: (name, success, duration).
    pub finished_packages: Vec<(String, bool, Duration)>,
    /// Progress state: (completed, total, message).
    pub progress: Option<(usize, usize, String)>,
    /// Output log lines: (package_name, line, is_stderr).
    pub output_log: Vec<(String, String, bool)>,
    /// Messages/warnings from the current command execution.
    pub exec_messages: Vec<String>,
    /// Error message if the command failed (shown in Done state).
    pub command_error: Option<String>,
    /// Scroll offset into output_log for Done state viewing.
    pub output_scroll: usize,
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
            pending_command: None,
            pending_cancel: false,
            running_command: None,
            running_packages: Vec::new(),
            finished_packages: Vec::new(),
            progress: None,
            output_log: Vec::new(),
            exec_messages: Vec::new(),
            command_error: None,
            output_scroll: 0,
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

        // During Running, only Esc (cancel) and Ctrl+C (quit) are active.
        if self.state == AppState::Running {
            match (code, modifiers.contains(KeyModifiers::CONTROL)) {
                (KeyCode::Esc, _) => self.pending_cancel = true,
                (KeyCode::Char('c'), true) => self.quit = true,
                _ => {}
            }
            return;
        }

        // In Done state, Esc/Enter/q return to Idle; scroll keys navigate output.
        if self.state == AppState::Done {
            let ctrl = modifiers.contains(KeyModifiers::CONTROL);
            let half_page = (self.page_size / 2).max(1);
            match (code, ctrl) {
                (KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q'), false) => {
                    self.state = AppState::Idle;
                }
                (KeyCode::Char('c'), true) => self.quit = true,
                // Scroll navigation in output log
                (KeyCode::Up | KeyCode::Char('k'), false) => {
                    self.output_scroll = self.output_scroll.saturating_sub(1);
                }
                (KeyCode::Down | KeyCode::Char('j'), false) => {
                    self.scroll_output_down(1);
                }
                (KeyCode::Home | KeyCode::Char('g'), false) => {
                    self.output_scroll = 0;
                }
                (KeyCode::End | KeyCode::Char('G'), false) => {
                    self.scroll_output_end();
                }
                (KeyCode::Char('d'), true) => {
                    self.scroll_output_down(half_page);
                }
                (KeyCode::Char('u'), true) => {
                    self.output_scroll = self.output_scroll.saturating_sub(half_page);
                }
                (KeyCode::PageDown, _) | (KeyCode::Char('f'), _) => {
                    self.scroll_output_down(self.page_size);
                }
                (KeyCode::PageUp, _) | (KeyCode::Char('b'), _) => {
                    self.output_scroll = self.output_scroll.saturating_sub(self.page_size);
                }
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

            // Execute selected command
            (KeyCode::Enter, _) => self.request_execute(),

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

    /// Handle the Escape key in Idle state (quit).
    ///
    /// Running and Done states are handled by the early-return blocks in
    /// `handle_key()` before this is ever reached.
    fn handle_esc(&mut self) {
        self.quit = true;
    }

    /// Scroll output log down by `n` lines, clamping to the end.
    fn scroll_output_down(&mut self, n: usize) {
        let max_scroll = self.output_log.len().saturating_sub(1);
        self.output_scroll = (self.output_scroll + n).min(max_scroll);
    }

    /// Scroll output log to the very end.
    fn scroll_output_end(&mut self) {
        self.output_scroll = self.output_log.len().saturating_sub(1);
    }

    /// Request execution of the currently selected command.
    ///
    /// Sets `pending_command` which the main loop consumes to spawn the task.
    /// Only fires when the Commands panel is active and a valid command is selected.
    fn request_execute(&mut self) {
        if self.state != AppState::Idle || self.active_panel != ActivePanel::Commands {
            return;
        }
        if let Some(cmd) = self.command_rows.get(self.selected_command) {
            self.pending_command = Some(cmd.name.clone());
        }
    }

    /// Transition to Running state for the given command.
    ///
    /// Called by the main loop after spawning the command task.
    pub fn start_command(&mut self, name: &str) {
        self.state = AppState::Running;
        self.running_command = Some(name.to_string());
        self.running_packages.clear();
        self.finished_packages.clear();
        self.progress = None;
        self.output_log.clear();
        self.exec_messages.clear();
        self.command_error = None;
        self.output_scroll = 0;
    }

    /// Process a core event received from the running command.
    pub fn handle_core_event(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::CommandStarted {
                package_count,
                command,
            } => {
                self.progress = Some((0, package_count, command));
            }
            CoreEvent::PackageStarted { name } => {
                self.running_packages.push(name);
            }
            CoreEvent::PackageFinished {
                name,
                success,
                duration,
            } => {
                self.running_packages.retain(|n| n != &name);
                self.finished_packages.push((name, success, duration));
                if let Some((completed, _, _)) = &mut self.progress {
                    *completed += 1;
                }
            }
            CoreEvent::PackageOutput {
                name,
                line,
                is_stderr,
            } => {
                self.output_log.push((name, strip_ansi(&line), is_stderr));
            }
            CoreEvent::Progress {
                completed,
                total,
                message,
            } => {
                self.progress = Some((completed, total, message));
            }
            CoreEvent::Warning(msg) => {
                self.exec_messages.push(format!("warn: {msg}"));
            }
            CoreEvent::Info(msg) => {
                self.exec_messages.push(msg);
            }
            CoreEvent::CommandFinished { .. } => {
                // The actual state transition to Done happens in on_command_finished()
                // when the channel closes (sender dropped after this event).
            }
        }
    }

    /// Handle command completion after the channel closes and the task handle resolves.
    ///
    /// `result` is the `JoinHandle` result wrapping the command `Result<PackageResults>`.
    pub fn on_command_finished(
        &mut self,
        result: Result<anyhow::Result<()>, tokio::task::JoinError>,
    ) {
        self.state = AppState::Done;
        self.running_command = None;
        self.running_packages.clear();

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                self.command_error = Some(format!("{e}"));
            }
            Err(e) if e.is_cancelled() => {
                // Task was aborted (user pressed Esc); not an error.
            }
            Err(e) => {
                self.command_error = Some(format!("task panicked: {e}"));
            }
        }
    }

    /// Handle cancel: transition back to Idle, clear execution state.
    pub fn on_command_cancelled(&mut self) {
        self.state = AppState::Idle;
        self.running_command = None;
        self.running_packages.clear();
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
                if abs == 1 { len - 1 } else { 0 }
            } else {
                current - abs
            }
        } else {
            let abs = delta as usize;
            if current + abs >= len {
                // Wrap: single-step down wraps to start; page-down clamps to end.
                if abs == 1 { 0 } else { len - 1 }
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
    fn test_enter_in_done_returns_to_idle() {
        let mut app = App::new();
        app.state = AppState::Done;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_q_in_done_returns_to_idle() {
        let mut app = App::new();
        app.state = AppState::Done;
        press(&mut app, KeyCode::Char('q'));
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_esc_in_running_requests_cancel() {
        let mut app = App::new();
        app.state = AppState::Running;
        press(&mut app, KeyCode::Esc);
        // Running state sets pending_cancel; main loop handles actual cancellation.
        assert!(app.pending_cancel);
        assert_eq!(app.state, AppState::Running);
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

    // --- Enter / execute tests ---

    #[test]
    fn test_enter_on_commands_panel_sets_pending_command() {
        let mut app = App::new();
        app.active_panel = ActivePanel::Commands;
        app.selected_command = 0; // "analyze"
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.pending_command.as_deref(), Some("analyze"));
    }

    #[test]
    fn test_enter_on_packages_panel_does_nothing() {
        let mut app = App::new();
        app.active_panel = ActivePanel::Packages;
        press(&mut app, KeyCode::Enter);
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn test_enter_in_running_does_nothing() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Enter);
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn test_enter_with_empty_commands_does_nothing() {
        let mut app = App::new();
        app.command_rows.clear();
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Enter);
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn test_enter_selects_correct_command_by_index() {
        let mut app = App::new();
        app.active_panel = ActivePanel::Commands;
        // Select "format" (index 5 in BUILTIN_COMMANDS: analyze,bootstrap,build,clean,exec,format)
        app.selected_command = 5;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.pending_command.as_deref(), Some("format"));
    }

    // --- start_command tests ---

    #[test]
    fn test_start_command_transitions_to_running() {
        let mut app = App::new();
        app.start_command("analyze");
        assert_eq!(app.state, AppState::Running);
        assert_eq!(app.running_command.as_deref(), Some("analyze"));
    }

    #[test]
    fn test_start_command_clears_previous_state() {
        let mut app = App::new();
        app.finished_packages
            .push(("old".to_string(), true, Duration::from_secs(1)));
        app.output_log
            .push(("pkg".to_string(), "line".to_string(), false));
        app.exec_messages.push("old msg".to_string());
        app.command_error = Some("old error".to_string());
        app.start_command("test");
        assert!(app.finished_packages.is_empty());
        assert!(app.output_log.is_empty());
        assert!(app.exec_messages.is_empty());
        assert!(app.command_error.is_none());
    }

    // --- handle_core_event tests ---

    #[test]
    fn test_handle_command_started_sets_progress() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::CommandStarted {
            command: "analyze".to_string(),
            package_count: 5,
        });
        assert_eq!(app.progress, Some((0, 5, "analyze".to_string())));
    }

    #[test]
    fn test_handle_package_started_adds_to_running() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::PackageStarted {
            name: "pkg_a".to_string(),
        });
        assert_eq!(app.running_packages, vec!["pkg_a"]);
    }

    #[test]
    fn test_handle_package_finished_moves_to_results() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.running_packages.push("pkg_a".to_string());
        app.progress = Some((0, 3, String::new()));

        app.handle_core_event(CoreEvent::PackageFinished {
            name: "pkg_a".to_string(),
            success: true,
            duration: Duration::from_millis(100),
        });

        assert!(app.running_packages.is_empty());
        assert_eq!(app.finished_packages.len(), 1);
        assert_eq!(app.finished_packages[0].0, "pkg_a");
        assert!(app.finished_packages[0].1);
        assert_eq!(app.progress, Some((1, 3, String::new())));
    }

    #[test]
    fn test_handle_package_output_appends_to_log() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::PackageOutput {
            name: "pkg_a".to_string(),
            line: "hello world".to_string(),
            is_stderr: false,
        });
        assert_eq!(app.output_log.len(), 1);
        assert_eq!(app.output_log[0].1, "hello world");
        assert!(!app.output_log[0].2);
    }

    #[test]
    fn test_handle_progress_updates_progress() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::Progress {
            completed: 3,
            total: 10,
            message: "analyzing".to_string(),
        });
        assert_eq!(app.progress, Some((3, 10, "analyzing".to_string())));
    }

    #[test]
    fn test_handle_warning_appends_message() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::Warning("something wrong".to_string()));
        assert_eq!(app.exec_messages.len(), 1);
        assert!(app.exec_messages[0].contains("something wrong"));
    }

    #[test]
    fn test_handle_info_appends_message() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::Info("info msg".to_string()));
        assert_eq!(app.exec_messages, vec!["info msg"]);
    }

    // --- on_command_finished tests ---

    #[test]
    fn test_on_command_finished_success_transitions_to_done() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.running_command = Some("analyze".to_string());
        app.on_command_finished(Ok(Ok(())));
        assert_eq!(app.state, AppState::Done);
        assert!(app.running_command.is_none());
        assert!(app.command_error.is_none());
    }

    #[test]
    fn test_on_command_finished_error_records_message() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.on_command_finished(Ok(Err(anyhow::anyhow!("command failed"))));
        assert_eq!(app.state, AppState::Done);
        assert!(
            app.command_error
                .as_ref()
                .is_some_and(|e| e.contains("command failed"))
        );
    }

    // --- on_command_cancelled tests ---

    #[test]
    fn test_on_command_cancelled_returns_to_idle() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.running_command = Some("test".to_string());
        app.running_packages.push("pkg_a".to_string());
        app.on_command_cancelled();
        assert_eq!(app.state, AppState::Idle);
        assert!(app.running_command.is_none());
        assert!(app.running_packages.is_empty());
    }

    // --- Running state key blocking tests ---

    #[test]
    fn test_running_blocks_navigation_keys() {
        let mut app = app_with_packages(10);
        app.state = AppState::Running;
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_package, 0);
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit());
    }

    #[test]
    fn test_running_allows_ctrl_c_quit() {
        let mut app = App::new();
        app.state = AppState::Running;
        ctrl(&mut app, KeyCode::Char('c'));
        assert!(app.should_quit());
    }

    // --- Full lifecycle test ---

    #[test]
    fn test_idle_to_running_to_done_lifecycle() {
        let mut app = App::new();
        app.active_panel = ActivePanel::Commands;
        assert_eq!(app.state, AppState::Idle);

        // Press Enter to request command.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.pending_command.as_deref(), Some("analyze"));

        // Main loop consumes pending_command and calls start_command.
        let cmd = app.pending_command.take().unwrap();
        app.start_command(&cmd);
        assert_eq!(app.state, AppState::Running);

        // Core events arrive.
        app.handle_core_event(CoreEvent::CommandStarted {
            command: "analyze".to_string(),
            package_count: 2,
        });
        app.handle_core_event(CoreEvent::PackageStarted {
            name: "pkg_a".to_string(),
        });
        app.handle_core_event(CoreEvent::PackageFinished {
            name: "pkg_a".to_string(),
            success: true,
            duration: Duration::from_millis(50),
        });
        app.handle_core_event(CoreEvent::PackageStarted {
            name: "pkg_b".to_string(),
        });
        app.handle_core_event(CoreEvent::PackageFinished {
            name: "pkg_b".to_string(),
            success: false,
            duration: Duration::from_millis(100),
        });
        app.handle_core_event(CoreEvent::CommandFinished {
            command: "analyze".to_string(),
            duration: Duration::from_millis(150),
        });

        // Channel closes, main loop calls on_command_finished.
        app.on_command_finished(Ok(Ok(())));
        assert_eq!(app.state, AppState::Done);
        assert_eq!(app.finished_packages.len(), 2);
        assert!(app.finished_packages[0].1); // pkg_a success
        assert!(!app.finished_packages[1].1); // pkg_b failure

        // Esc returns to Idle.
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.state, AppState::Idle);
    }

    // --- Done state scroll tests ---

    fn app_in_done_with_output(line_count: usize) -> App {
        let mut app = App::new();
        app.state = AppState::Done;
        app.page_size = 10;
        for i in 0..line_count {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        app
    }

    #[test]
    fn test_done_j_scrolls_down() {
        let mut app = app_in_done_with_output(30);
        assert_eq!(app.output_scroll, 0);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.output_scroll, 1);
        assert_eq!(app.state, AppState::Done);
    }

    #[test]
    fn test_done_k_scrolls_up() {
        let mut app = app_in_done_with_output(30);
        app.output_scroll = 5;
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.output_scroll, 4);
    }

    #[test]
    fn test_done_k_clamps_at_zero() {
        let mut app = app_in_done_with_output(30);
        app.output_scroll = 0;
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.output_scroll, 0);
    }

    #[test]
    fn test_done_g_jumps_to_start() {
        let mut app = app_in_done_with_output(30);
        app.output_scroll = 20;
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.output_scroll, 0);
    }

    #[test]
    fn test_done_shift_g_jumps_to_end() {
        let mut app = app_in_done_with_output(30);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.output_scroll, 29);
    }

    #[test]
    fn test_done_f_pages_down() {
        let mut app = app_in_done_with_output(30);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.output_scroll, 10);
    }

    #[test]
    fn test_done_b_pages_up() {
        let mut app = app_in_done_with_output(30);
        app.output_scroll = 20;
        press(&mut app, KeyCode::Char('b'));
        assert_eq!(app.output_scroll, 10);
    }

    #[test]
    fn test_done_ctrl_d_half_page_down() {
        let mut app = app_in_done_with_output(30);
        ctrl(&mut app, KeyCode::Char('d'));
        assert_eq!(app.output_scroll, 5);
    }

    #[test]
    fn test_done_ctrl_u_half_page_up() {
        let mut app = app_in_done_with_output(30);
        app.output_scroll = 15;
        ctrl(&mut app, KeyCode::Char('u'));
        assert_eq!(app.output_scroll, 10);
    }

    #[test]
    fn test_done_scroll_clamped_to_max() {
        let mut app = app_in_done_with_output(5);
        press(&mut app, KeyCode::Char('f'));
        // max_scroll = 5 - 1 = 4
        assert_eq!(app.output_scroll, 4);
    }

    // --- ANSI stripping tests ---

    #[test]
    fn test_strip_ansi_plain_text_unchanged() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn test_strip_ansi_removes_color_codes() {
        // Bold red "error" then reset
        assert_eq!(strip_ansi("\x1b[1;31merror\x1b[0m"), "error");
    }

    #[test]
    fn test_strip_ansi_removes_sgr_sequences() {
        assert_eq!(strip_ansi("\x1b[32mOK\x1b[0m: all good"), "OK: all good");
    }

    #[test]
    fn test_strip_ansi_handles_osc_with_bel() {
        // OSC title-set terminated by BEL
        assert_eq!(strip_ansi("\x1b]0;my title\x07rest"), "rest");
    }

    #[test]
    fn test_strip_ansi_handles_osc_with_st() {
        // OSC terminated by ESC backslash
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\rest"), "rest");
    }

    #[test]
    fn test_strip_ansi_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn test_strip_ansi_mixed_content() {
        let input = "  \x1b[34mAnalyzing\x1b[0m package \x1b[1mfoo\x1b[0m...";
        assert_eq!(strip_ansi(input), "  Analyzing package foo...");
    }

    #[test]
    fn test_handle_package_output_strips_ansi() {
        let mut app = App::new();
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::PackageOutput {
            name: "pkg_a".to_string(),
            line: "\x1b[32mSUCCESS\x1b[0m".to_string(),
            is_stderr: false,
        });
        assert_eq!(app.output_log[0].1, "SUCCESS");
    }
}
