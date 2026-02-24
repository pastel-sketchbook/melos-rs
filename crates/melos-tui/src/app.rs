use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use melos_core::commands::health::HealthReport;
use melos_core::config::ScriptEntry;
use melos_core::events::Event as CoreEvent;
use melos_core::package::Package;
use melos_core::workspace::Workspace;

use crate::theme::Theme;

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
    /// Whether this command is supported for TUI execution.
    /// Unsupported commands (exec, run, build, version, list) require CLI args.
    pub is_supported: bool,
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
    "pub",
    "publish",
    "run",
    "test",
    "version",
];

/// Commands that can be dispatched from the TUI (no CLI args required).
const SUPPORTED_COMMANDS: &[&str] = &[
    "analyze",
    "bootstrap",
    "clean",
    "format",
    "health",
    "pub",
    "publish",
    "test",
];

/// Per-command configuration options shown in the options overlay.
#[derive(Debug, Clone)]
pub enum CommandOpts {
    Analyze {
        concurrency: usize,
        fatal_warnings: bool,
        fatal_infos: bool,
        no_fatal: bool,
    },
    Bootstrap {
        concurrency: usize,
        enforce_lockfile: bool,
        offline: bool,
        no_example: bool,
    },
    Clean {
        concurrency: usize,
    },
    Format {
        concurrency: usize,
        set_exit_if_changed: bool,
        line_length: Option<u32>,
    },
    Test {
        concurrency: usize,
        fail_fast: bool,
        coverage: bool,
        update_goldens: bool,
        no_run: bool,
    },
    Publish {
        concurrency: usize,
        dry_run: bool,
    },
    Health {
        version_drift: bool,
        missing_fields: bool,
        sdk_consistency: bool,
    },
    Pub {
        subcommand: usize,
        concurrency: usize,
        major_versions: bool,
    },
}

impl CommandOpts {
    /// Build default options for the given command name.
    ///
    /// Returns `None` for unsupported commands.
    pub fn build_default(name: &str) -> Option<Self> {
        match name {
            "analyze" => Some(Self::Analyze {
                concurrency: 1,
                fatal_warnings: false,
                fatal_infos: false,
                no_fatal: false,
            }),
            "bootstrap" => Some(Self::Bootstrap {
                concurrency: 1,
                enforce_lockfile: false,
                offline: false,
                no_example: false,
            }),
            "clean" => Some(Self::Clean { concurrency: 1 }),
            "format" => Some(Self::Format {
                concurrency: 1,
                set_exit_if_changed: false,
                line_length: None,
            }),
            "test" => Some(Self::Test {
                concurrency: 1,
                fail_fast: false,
                coverage: false,
                update_goldens: false,
                no_run: false,
            }),
            "publish" => Some(Self::Publish {
                concurrency: 1,
                dry_run: true,
            }),
            "health" => Some(Self::Health {
                version_drift: true,
                missing_fields: true,
                sdk_consistency: true,
            }),
            "pub" => Some(Self::Pub {
                subcommand: 1,
                concurrency: 1,
                major_versions: false,
            }),
            _ => None,
        }
    }

    /// Return option labels and their current boolean/numeric values for rendering.
    pub fn option_rows(&self) -> Vec<OptionRow> {
        match self {
            Self::Analyze {
                concurrency,
                fatal_warnings,
                fatal_infos,
                no_fatal,
            } => vec![
                OptionRow::Number("concurrency", *concurrency),
                OptionRow::Bool("fatal-warnings", *fatal_warnings),
                OptionRow::Bool("fatal-infos", *fatal_infos),
                OptionRow::Bool("no-fatal", *no_fatal),
            ],
            Self::Bootstrap {
                concurrency,
                enforce_lockfile,
                offline,
                no_example,
            } => vec![
                OptionRow::Number("concurrency", *concurrency),
                OptionRow::Bool("enforce-lockfile", *enforce_lockfile),
                OptionRow::Bool("offline", *offline),
                OptionRow::Bool("no-example", *no_example),
            ],
            Self::Clean { concurrency } => {
                vec![OptionRow::Number("concurrency", *concurrency)]
            }
            Self::Format {
                concurrency,
                set_exit_if_changed,
                line_length,
            } => vec![
                OptionRow::Number("concurrency", *concurrency),
                OptionRow::Bool("set-exit-if-changed", *set_exit_if_changed),
                OptionRow::OptNumber("line-length", *line_length),
            ],
            Self::Test {
                concurrency,
                fail_fast,
                coverage,
                update_goldens,
                no_run,
            } => vec![
                OptionRow::Number("concurrency", *concurrency),
                OptionRow::Bool("fail-fast", *fail_fast),
                OptionRow::Bool("coverage", *coverage),
                OptionRow::Bool("update-goldens", *update_goldens),
                OptionRow::Bool("no-run", *no_run),
            ],
            Self::Publish {
                concurrency,
                dry_run,
            } => vec![
                OptionRow::Number("concurrency", *concurrency),
                OptionRow::Bool("dry-run", *dry_run),
            ],
            Self::Health {
                version_drift,
                missing_fields,
                sdk_consistency,
            } => vec![
                OptionRow::Bool("version-drift", *version_drift),
                OptionRow::Bool("missing-fields", *missing_fields),
                OptionRow::Bool("sdk-consistency", *sdk_consistency),
            ],
            Self::Pub {
                subcommand,
                concurrency,
                major_versions,
            } => vec![
                OptionRow::Number(
                    "sub-cmd (1=get 2=outdated 3=upgrade 4=downgrade)",
                    *subcommand,
                ),
                OptionRow::Number("concurrency", *concurrency),
                OptionRow::Bool("major-versions (upgrade only)", *major_versions),
            ],
        }
    }

    /// Toggle a boolean option at the given index. No-op if index is out of range
    /// or points to a non-boolean option.
    /// Toggle a boolean option at the given visual row index.
    /// No-op if the index is out of range or points to a non-boolean option.
    pub fn toggle_bool(&mut self, visual_index: usize) {
        let rows = self.option_rows();
        let bool_offset = rows
            .iter()
            .take(visual_index)
            .filter(|r| matches!(r, OptionRow::Bool(..)))
            .count();
        if let Some(OptionRow::Bool(..)) = rows.get(visual_index) {
            let mut fields = self.bool_fields_mut();
            if let Some(val) = fields.get_mut(bool_offset) {
                **val = !**val;
            }
        }
    }

    /// Increment a numeric option at the given visual row index.
    pub fn increment_at(&mut self, visual_index: usize) {
        let rows = self.option_rows();
        let num_offset = rows
            .iter()
            .take(visual_index)
            .filter(|r| matches!(r, OptionRow::Number(..)))
            .count();
        if let Some(OptionRow::Number(..)) = rows.get(visual_index) {
            let mut fields = self.number_fields_mut();
            if let Some(val) = fields.get_mut(num_offset) {
                **val = val.saturating_add(1);
            }
        }
    }

    /// Decrement a numeric option at the given visual row index (minimum 1).
    pub fn decrement_at(&mut self, visual_index: usize) {
        let rows = self.option_rows();
        let num_offset = rows
            .iter()
            .take(visual_index)
            .filter(|r| matches!(r, OptionRow::Number(..)))
            .count();
        if let Some(OptionRow::Number(..)) = rows.get(visual_index) {
            let mut fields = self.number_fields_mut();
            if let Some(val) = fields.get_mut(num_offset)
                && **val > 1
            {
                **val -= 1;
            }
        }
    }

    /// Get mutable references to boolean fields, paired with their option_rows index.
    fn bool_fields_mut(&mut self) -> Vec<&mut bool> {
        match self {
            Self::Analyze {
                fatal_warnings,
                fatal_infos,
                no_fatal,
                ..
            } => vec![fatal_warnings, fatal_infos, no_fatal],
            Self::Bootstrap {
                enforce_lockfile,
                offline,
                no_example,
                ..
            } => vec![enforce_lockfile, offline, no_example],
            Self::Clean { .. } => vec![],
            Self::Format {
                set_exit_if_changed,
                ..
            } => vec![set_exit_if_changed],
            Self::Test {
                fail_fast,
                coverage,
                update_goldens,
                no_run,
                ..
            } => vec![fail_fast, coverage, update_goldens, no_run],
            Self::Publish { dry_run, .. } => vec![dry_run],
            Self::Health {
                version_drift,
                missing_fields,
                sdk_consistency,
            } => vec![version_drift, missing_fields, sdk_consistency],
            Self::Pub { major_versions, .. } => vec![major_versions],
        }
    }

    /// Get mutable references to numeric fields.
    fn number_fields_mut(&mut self) -> Vec<&mut usize> {
        match self {
            Self::Analyze { concurrency, .. }
            | Self::Bootstrap { concurrency, .. }
            | Self::Clean { concurrency }
            | Self::Format { concurrency, .. }
            | Self::Test { concurrency, .. }
            | Self::Publish { concurrency, .. } => vec![concurrency],
            Self::Pub {
                subcommand,
                concurrency,
                ..
            } => vec![subcommand, concurrency],
            Self::Health { .. } => vec![],
        }
    }

    /// Number of option rows for this command.
    pub fn option_count(&self) -> usize {
        self.option_rows().len()
    }
}

/// A single option row for display in the options overlay.
#[derive(Debug, Clone)]
pub enum OptionRow {
    Bool(&'static str, bool),
    Number(&'static str, usize),
    OptNumber(&'static str, Option<u32>),
}

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

/// Maximum number of output lines retained in the scrollback buffer.
/// Oldest lines are dropped when this limit is exceeded.
pub const MAX_SCROLLBACK: usize = 10_000;

/// Top-level application state.
pub struct App {
    /// Current state machine phase.
    pub state: AppState,
    /// Whether the user has requested to quit.
    quit: bool,
    /// Which panel is currently focused.
    pub active_panel: ActivePanel,
    /// Color theme for all rendering.
    pub theme: Theme,
    /// Index into `Theme::available_names()` for theme cycling.
    pub theme_index: usize,
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
    /// Whether auto-scroll is active (tracks tail of output during Running).
    /// Set to `false` when the user manually scrolls up; re-engaged on scroll-to-end.
    pub auto_scroll: bool,
    /// Timestamp when the current command started (for elapsed time display).
    pub command_start: Option<Instant>,

    // --- Results state (Batch 53) ---
    /// Structured health report from the health command (for dashboard rendering).
    pub health_report: Option<HealthReport>,
    /// Currently selected tab in the health dashboard (0=drift, 1=missing, 2=sdk).
    pub health_tab: usize,

    // --- Options overlay state (Batch 51.5) ---
    /// Whether the options overlay is currently visible.
    pub show_options: bool,
    /// Per-command options for the overlay (set when overlay opens).
    pub command_opts: Option<CommandOpts>,
    /// Currently selected option row index in the overlay.
    pub selected_option: usize,

    // --- Filter bar state (Batch 54) ---
    /// Whether the filter input bar is currently active (user is typing).
    pub filter_active: bool,
    /// Current filter text (may be non-empty even when input is inactive = applied filter).
    pub filter_text: String,
    /// Indices into `package_rows` matching the filter. Empty when no filter applied.
    pub filtered_indices: Vec<usize>,
}

impl App {
    /// Create a new App in the Idle state with no workspace loaded.
    pub fn new(theme: Theme) -> Self {
        // Pre-populate built-in commands even without a workspace.
        let command_rows = BUILTIN_COMMANDS
            .iter()
            .map(|name| CommandRow {
                name: (*name).to_string(),
                description: None,
                is_builtin: true,
                is_supported: SUPPORTED_COMMANDS.contains(name),
            })
            .collect();

        Self {
            state: AppState::Idle,
            quit: false,
            active_panel: ActivePanel::Packages,
            theme,
            theme_index: 0,
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
            auto_scroll: true,
            command_start: None,
            health_report: None,
            health_tab: 0,
            show_options: false,
            command_opts: None,
            selected_option: 0,
            filter_active: false,
            filter_text: String::new(),
            filtered_indices: Vec::new(),
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
                is_supported: SUPPORTED_COMMANDS.contains(name),
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
                is_supported: false,
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

    /// Update page size from terminal height (called on resize).
    pub fn update_page_size(&mut self, term_height: u16) {
        self.page_size = term_height.saturating_sub(5) as usize;
    }

    /// Cycle to the next built-in theme.
    pub fn cycle_theme(&mut self) {
        let names = Theme::available_names();
        self.theme_index = (self.theme_index + 1) % names.len();
        // safety: theme_index is always in range due to modulo above
        if let Some(t) = Theme::by_name(names[self.theme_index]) {
            self.theme = t;
        }
    }

    /// Return the name of the currently active theme.
    pub fn theme_name(&self) -> &'static str {
        let names = Theme::available_names();
        names.get(self.theme_index).copied().unwrap_or("dark")
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

        // When options overlay is visible, handle option navigation and toggling.
        if self.show_options {
            self.handle_options_key(code);
            return;
        }

        // When filter input bar is active, intercept all keys for text editing.
        if self.filter_active {
            self.handle_filter_key(code);
            return;
        }

        // During Running, Esc (cancel), Ctrl+C (quit), and scroll keys are active.
        if self.state == AppState::Running {
            let ctrl = modifiers.contains(KeyModifiers::CONTROL);
            let half_page = (self.page_size / 2).max(1);
            match (code, ctrl) {
                (KeyCode::Esc, _) => self.pending_cancel = true,
                (KeyCode::Char('c'), true) => self.quit = true,
                // Scroll navigation in output log (disables auto-scroll).
                (KeyCode::Up | KeyCode::Char('k'), false) => {
                    self.output_scroll = self.output_scroll.saturating_sub(1);
                    self.auto_scroll = false;
                }
                (KeyCode::Down | KeyCode::Char('j'), false) => {
                    self.scroll_output_down(1);
                    self.update_auto_scroll();
                }
                (KeyCode::Home | KeyCode::Char('g'), false) => {
                    self.output_scroll = 0;
                    self.auto_scroll = false;
                }
                (KeyCode::End | KeyCode::Char('G'), false) => {
                    self.scroll_output_end();
                    self.auto_scroll = true;
                }
                (KeyCode::Char('d'), true) => {
                    self.scroll_output_down(half_page);
                    self.update_auto_scroll();
                }
                (KeyCode::Char('u'), true) => {
                    self.output_scroll = self.output_scroll.saturating_sub(half_page);
                    self.auto_scroll = false;
                }
                (KeyCode::PageDown, _) | (KeyCode::Char('f'), false) => {
                    self.scroll_output_down(self.page_size);
                    self.update_auto_scroll();
                }
                (KeyCode::PageUp, _) | (KeyCode::Char('b'), false) => {
                    self.output_scroll = self.output_scroll.saturating_sub(self.page_size);
                    self.auto_scroll = false;
                }
                _ => {}
            }
            return;
        }

        // In Done state, Esc/Enter/q return to Idle; scroll keys navigate output.
        // Tab/BackTab cycle health dashboard tabs when a health report is present.
        if self.state == AppState::Done {
            let ctrl = modifiers.contains(KeyModifiers::CONTROL);
            let half_page = (self.page_size / 2).max(1);
            match (code, ctrl) {
                (KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q'), false) => {
                    self.state = AppState::Idle;
                }
                (KeyCode::Char('c'), true) => self.quit = true,
                // Health dashboard tab cycling.
                (KeyCode::Tab, _) if self.health_report.is_some() => {
                    self.health_tab = (self.health_tab + 1) % 3;
                }
                (KeyCode::BackTab, _) if self.health_report.is_some() => {
                    self.health_tab = if self.health_tab == 0 {
                        2
                    } else {
                        self.health_tab - 1
                    };
                }
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

            // Filter bar activation
            (KeyCode::Char('/'), false) => self.activate_filter(),

            // Theme cycling
            (KeyCode::Char('t'), false) => self.cycle_theme(),

            _ => {}
        }
    }

    /// Handle the Escape key in Idle state.
    ///
    /// If a filter is applied, clears the filter. Otherwise quits.
    /// Running and Done states are handled by the early-return blocks in
    /// `handle_key()` before this is ever reached.
    fn handle_esc(&mut self) {
        if self.has_filter() {
            self.cancel_filter();
        } else {
            self.quit = true;
        }
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

    /// Re-engage auto-scroll if the user has scrolled to the tail of output.
    fn update_auto_scroll(&mut self) {
        let max_scroll = self.output_log.len().saturating_sub(1);
        if self.output_scroll >= max_scroll {
            self.auto_scroll = true;
        }
    }

    /// Return the elapsed time since the command started, if running.
    pub fn elapsed(&self) -> Option<Duration> {
        self.command_start.map(|start| start.elapsed())
    }

    /// Request execution of the currently selected command.
    ///
    /// For supported commands, opens the options overlay so the user can
    /// configure options before running. For unsupported commands, sets
    /// `pending_command` directly (the main loop will show an error).
    /// Only fires when the Commands panel is active and a valid command is selected.
    fn request_execute(&mut self) {
        if self.state != AppState::Idle || self.active_panel != ActivePanel::Commands {
            return;
        }
        if let Some(cmd) = self.command_rows.get(self.selected_command) {
            if cmd.is_supported
                && let Some(opts) = CommandOpts::build_default(&cmd.name)
            {
                self.command_opts = Some(opts);
                self.selected_option = 0;
                self.show_options = true;
                return;
            }
            // Unsupported: dispatch directly (dispatch.rs will bail with error).
            self.pending_command = Some(cmd.name.clone());
        }
    }

    /// Confirm execution from the options overlay.
    ///
    /// Transfers `command_opts` into `pending_opts` and sets `pending_command`.
    fn confirm_options(&mut self) {
        if let Some(cmd) = self.command_rows.get(self.selected_command) {
            self.pending_command = Some(cmd.name.clone());
        }
        self.show_options = false;
    }

    /// Dismiss the options overlay without executing.
    fn dismiss_options(&mut self) {
        self.show_options = false;
        self.command_opts = None;
        self.selected_option = 0;
    }

    /// Handle key presses while the options overlay is visible.
    fn handle_options_key(&mut self, code: KeyCode) {
        let opt_count = self.command_opts.as_ref().map_or(0, |o| o.option_count());

        match code {
            KeyCode::Esc | KeyCode::Char('q') => self.dismiss_options(),
            KeyCode::Enter => self.confirm_options(),

            // Navigation within option rows (+ the "Run" action row at the end).
            KeyCode::Char('j') | KeyCode::Down => {
                // +1 for the "Run" action row.
                let total = opt_count + 1;
                self.selected_option = (self.selected_option + 1) % total;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let total = opt_count + 1;
                self.selected_option = if self.selected_option == 0 {
                    total - 1
                } else {
                    self.selected_option - 1
                };
            }

            // Toggle boolean / adjust numbers at the selected row.
            KeyCode::Char(' ') => {
                if self.selected_option < opt_count {
                    if let Some(opts) = &mut self.command_opts {
                        opts.toggle_bool(self.selected_option);
                    }
                } else {
                    // Space on "Run" row: confirm.
                    self.confirm_options();
                }
            }
            KeyCode::Char('+') | KeyCode::Right => {
                if self.selected_option < opt_count
                    && let Some(opts) = &mut self.command_opts
                {
                    opts.increment_at(self.selected_option);
                }
            }
            KeyCode::Char('-') | KeyCode::Left => {
                if self.selected_option < opt_count
                    && let Some(opts) = &mut self.command_opts
                {
                    opts.decrement_at(self.selected_option);
                }
            }
            _ => {}
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
        self.auto_scroll = true;
        self.command_start = Some(Instant::now());
        self.health_report = None;
        self.health_tab = 0;
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
                // Truncate scrollback buffer from the front when over the limit.
                if self.output_log.len() > MAX_SCROLLBACK {
                    let excess = self.output_log.len() - MAX_SCROLLBACK;
                    self.output_log.drain(..excess);
                    // Adjust scroll offset so it still points at the same content.
                    self.output_scroll = self.output_scroll.saturating_sub(excess);
                }
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
    /// `result` is the `JoinHandle` result wrapping the command `Result<()>`.
    /// Note: `running_command` is intentionally preserved so the Done state
    /// can display the command name in the results header.
    pub fn on_command_finished(
        &mut self,
        result: Result<anyhow::Result<()>, tokio::task::JoinError>,
    ) {
        self.state = AppState::Done;
        self.running_packages.clear();
        self.command_start = None;

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
        self.command_start = None;
    }

    /// Set the structured health report from a completed health command.
    pub fn set_health_report(&mut self, report: HealthReport) {
        self.health_report = Some(report);
    }

    // --- Filter bar methods (Batch 54) ---

    /// Activate the filter input bar.
    fn activate_filter(&mut self) {
        self.filter_active = true;
    }

    /// Apply the current filter text and close the input bar.
    ///
    /// If the filter text is empty, clears any active filter. Otherwise,
    /// performs a case-insensitive substring match on package names.
    fn apply_filter(&mut self) {
        self.filter_active = false;
        if self.filter_text.is_empty() {
            self.filtered_indices.clear();
        } else {
            let query = self.filter_text.to_lowercase();
            self.filtered_indices = self
                .package_rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.name.to_lowercase().contains(&query))
                .map(|(i, _)| i)
                .collect();
        }
        self.selected_package = 0;
    }

    /// Cancel filter input and clear any active filter.
    fn cancel_filter(&mut self) {
        self.filter_active = false;
        self.filter_text.clear();
        self.filtered_indices.clear();
        self.selected_package = 0;
    }

    /// Recompute filtered indices from the current filter text.
    ///
    /// Called on each keystroke during filter input for live preview.
    fn recompute_filter(&mut self) {
        if self.filter_text.is_empty() {
            self.filtered_indices.clear();
        } else {
            let query = self.filter_text.to_lowercase();
            self.filtered_indices = self
                .package_rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.name.to_lowercase().contains(&query))
                .map(|(i, _)| i)
                .collect();
        }
        self.selected_package = 0;
    }

    /// Handle key presses while the filter input bar is active.
    fn handle_filter_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => self.apply_filter(),
            KeyCode::Esc => self.cancel_filter(),
            KeyCode::Backspace => {
                self.filter_text.pop();
                self.recompute_filter();
            }
            KeyCode::Char(c) => {
                self.filter_text.push(c);
                self.recompute_filter();
            }
            _ => {}
        }
    }

    /// Returns true if a package filter is currently applied (non-empty filter text with results).
    pub fn has_filter(&self) -> bool {
        !self.filter_text.is_empty()
    }

    /// Returns the number of visible packages (filtered if active, otherwise total).
    pub fn visible_package_count(&self) -> usize {
        if self.has_filter() {
            self.filtered_indices.len()
        } else {
            self.package_rows.len()
        }
    }

    /// Returns the visible package rows (filtered if active, otherwise all).
    pub fn visible_packages(&self) -> Vec<&PackageRow> {
        if self.has_filter() {
            self.filtered_indices
                .iter()
                .filter_map(|&i| self.package_rows.get(i))
                .collect()
        } else {
            self.package_rows.iter().collect()
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
            ActivePanel::Packages => (self.selected_package, self.visible_package_count()),
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
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
        app.command_rows = (0..count)
            .map(|i| CommandRow {
                name: format!("cmd_{i}"),
                description: None,
                is_builtin: i < 3,
                is_supported: true,
            })
            .collect();
        app.active_panel = ActivePanel::Commands;
        app
    }

    // --- Basic state tests ---

    #[test]
    fn test_new_app_is_idle() {
        let app = App::new(Theme::default());
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
        assert_eq!(app.active_panel, ActivePanel::Packages);
    }

    #[test]
    fn test_new_app_has_builtin_commands() {
        let app = App::new(Theme::default());
        assert_eq!(app.command_rows.len(), BUILTIN_COMMANDS.len());
        assert!(app.command_rows.iter().all(|r| r.is_builtin));
        assert_eq!(app.command_rows[0].name, "analyze");
    }

    #[test]
    fn test_q_quits() {
        let mut app = App::new(Theme::default());
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit());
    }

    #[test]
    fn test_esc_in_idle_quits() {
        let mut app = App::new(Theme::default());
        press(&mut app, KeyCode::Esc);
        assert!(app.should_quit());
    }

    #[test]
    fn test_esc_in_done_returns_to_idle() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Done;
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_enter_in_done_returns_to_idle() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Done;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_q_in_done_returns_to_idle() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Done;
        press(&mut app, KeyCode::Char('q'));
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_esc_in_running_requests_cancel() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        press(&mut app, KeyCode::Esc);
        // Running state sets pending_cancel; main loop handles actual cancellation.
        assert!(app.pending_cancel);
        assert_eq!(app.state, AppState::Running);
    }

    #[test]
    fn test_unknown_key_does_nothing() {
        let mut app = App::new(Theme::default());
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.state, AppState::Idle);
        assert!(!app.should_quit());
    }

    // --- Tab switching tests ---

    #[test]
    fn test_tab_toggles_to_commands() {
        let mut app = App::new(Theme::default());
        assert_eq!(app.active_panel, ActivePanel::Packages);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.active_panel, ActivePanel::Commands);
    }

    #[test]
    fn test_tab_toggles_back_to_packages() {
        let mut app = App::new(Theme::default());
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.active_panel, ActivePanel::Packages);
    }

    #[test]
    fn test_backtab_toggles_panel() {
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
        app.command_rows.clear();
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_command, 0);
    }

    #[test]
    fn test_command_count() {
        let app = App::new(Theme::default());
        assert_eq!(app.command_count(), BUILTIN_COMMANDS.len());
    }

    // --- Ctrl key tests ---

    #[test]
    fn test_ctrl_c_quits() {
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.active_panel, ActivePanel::Packages);
    }

    #[test]
    fn test_l_focuses_commands_panel() {
        let mut app = App::new(Theme::default());
        assert_eq!(app.active_panel, ActivePanel::Packages);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.active_panel, ActivePanel::Commands);
    }

    #[test]
    fn test_h_on_packages_panel_stays() {
        let mut app = App::new(Theme::default());
        assert_eq!(app.active_panel, ActivePanel::Packages);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.active_panel, ActivePanel::Packages);
    }

    #[test]
    fn test_l_on_commands_panel_stays() {
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.active_panel, ActivePanel::Commands);
    }

    // --- Help toggle tests ---

    #[test]
    fn test_question_mark_opens_help() {
        let mut app = App::new(Theme::default());
        assert!(!app.show_help);
        press(&mut app, KeyCode::Char('?'));
        assert!(app.show_help);
    }

    #[test]
    fn test_question_mark_closes_help() {
        let mut app = App::new(Theme::default());
        app.show_help = true;
        press(&mut app, KeyCode::Char('?'));
        assert!(!app.show_help);
    }

    #[test]
    fn test_esc_closes_help() {
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Commands;
        app.selected_command = 0; // "analyze" (supported)

        // First Enter opens the options overlay.
        press(&mut app, KeyCode::Enter);
        assert!(app.show_options);
        assert!(app.command_opts.is_some());
        assert!(app.pending_command.is_none());

        // Second Enter confirms and sets pending_command.
        press(&mut app, KeyCode::Enter);
        assert!(!app.show_options);
        assert_eq!(app.pending_command.as_deref(), Some("analyze"));
    }

    #[test]
    fn test_enter_on_packages_panel_does_nothing() {
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Packages;
        press(&mut app, KeyCode::Enter);
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn test_enter_in_running_does_nothing() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Enter);
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn test_enter_with_empty_commands_does_nothing() {
        let mut app = App::new(Theme::default());
        app.command_rows.clear();
        app.active_panel = ActivePanel::Commands;
        press(&mut app, KeyCode::Enter);
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn test_enter_selects_correct_command_by_index() {
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Commands;
        // Select "format" (index 5 in BUILTIN_COMMANDS: analyze,bootstrap,build,clean,exec,format)
        app.selected_command = 5;

        // First Enter opens options overlay for supported command.
        press(&mut app, KeyCode::Enter);
        assert!(app.show_options);
        assert!(app.pending_command.is_none());

        // Second Enter confirms.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.pending_command.as_deref(), Some("format"));
    }

    // --- start_command tests ---

    #[test]
    fn test_start_command_transitions_to_running() {
        let mut app = App::new(Theme::default());
        app.start_command("analyze");
        assert_eq!(app.state, AppState::Running);
        assert_eq!(app.running_command.as_deref(), Some("analyze"));
    }

    #[test]
    fn test_start_command_clears_previous_state() {
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::CommandStarted {
            command: "analyze".to_string(),
            package_count: 5,
        });
        assert_eq!(app.progress, Some((0, 5, "analyze".to_string())));
    }

    #[test]
    fn test_handle_package_started_adds_to_running() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::PackageStarted {
            name: "pkg_a".to_string(),
        });
        assert_eq!(app.running_packages, vec!["pkg_a"]);
    }

    #[test]
    fn test_handle_package_finished_moves_to_results() {
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::Warning("something wrong".to_string()));
        assert_eq!(app.exec_messages.len(), 1);
        assert!(app.exec_messages[0].contains("something wrong"));
    }

    #[test]
    fn test_handle_info_appends_message() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::Info("info msg".to_string()));
        assert_eq!(app.exec_messages, vec!["info msg"]);
    }

    // --- on_command_finished tests ---

    #[test]
    fn test_on_command_finished_success_transitions_to_done() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.running_command = Some("analyze".to_string());
        app.on_command_finished(Ok(Ok(())));
        assert_eq!(app.state, AppState::Done);
        // running_command is preserved so Done state can display it.
        assert_eq!(app.running_command.as_deref(), Some("analyze"));
        assert!(app.command_error.is_none());
    }

    #[test]
    fn test_on_command_finished_error_records_message() {
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        ctrl(&mut app, KeyCode::Char('c'));
        assert!(app.should_quit());
    }

    // --- Full lifecycle test ---

    #[test]
    fn test_idle_to_running_to_done_lifecycle() {
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Commands;
        assert_eq!(app.state, AppState::Idle);

        // Press Enter to open options overlay (analyze is supported).
        press(&mut app, KeyCode::Enter);
        assert!(app.show_options);
        assert!(app.pending_command.is_none());

        // Press Enter again to confirm options and request command.
        press(&mut app, KeyCode::Enter);
        assert!(!app.show_options);
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
        let mut app = App::new(Theme::default());
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
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.handle_core_event(CoreEvent::PackageOutput {
            name: "pkg_a".to_string(),
            line: "\x1b[32mSUCCESS\x1b[0m".to_string(),
            is_stderr: false,
        });
        assert_eq!(app.output_log[0].1, "SUCCESS");
    }

    // --- CommandOpts tests ---

    #[test]
    fn test_build_default_analyze() {
        let opts = CommandOpts::build_default("analyze").unwrap();
        assert!(matches!(
            opts,
            CommandOpts::Analyze {
                concurrency: 1,
                fatal_warnings: false,
                fatal_infos: false,
                no_fatal: false,
            }
        ));
        assert_eq!(opts.option_count(), 4);
    }

    #[test]
    fn test_build_default_bootstrap() {
        let opts = CommandOpts::build_default("bootstrap").unwrap();
        assert!(matches!(
            opts,
            CommandOpts::Bootstrap {
                concurrency: 1,
                enforce_lockfile: false,
                offline: false,
                no_example: false,
            }
        ));
        assert_eq!(opts.option_count(), 4);
    }

    #[test]
    fn test_build_default_clean() {
        let opts = CommandOpts::build_default("clean").unwrap();
        assert!(matches!(opts, CommandOpts::Clean { concurrency: 1 }));
        assert_eq!(opts.option_count(), 1);
    }

    #[test]
    fn test_build_default_format() {
        let opts = CommandOpts::build_default("format").unwrap();
        assert!(matches!(
            opts,
            CommandOpts::Format {
                concurrency: 1,
                set_exit_if_changed: false,
                line_length: None,
            }
        ));
        assert_eq!(opts.option_count(), 3);
    }

    #[test]
    fn test_build_default_test() {
        let opts = CommandOpts::build_default("test").unwrap();
        assert!(matches!(
            opts,
            CommandOpts::Test {
                concurrency: 1,
                fail_fast: false,
                coverage: false,
                update_goldens: false,
                no_run: false,
            }
        ));
        assert_eq!(opts.option_count(), 5);
    }

    #[test]
    fn test_build_default_publish() {
        let opts = CommandOpts::build_default("publish").unwrap();
        assert!(matches!(
            opts,
            CommandOpts::Publish {
                concurrency: 1,
                dry_run: true,
            }
        ));
        assert_eq!(opts.option_count(), 2);
    }

    #[test]
    fn test_build_default_health() {
        let opts = CommandOpts::build_default("health").unwrap();
        assert!(matches!(
            opts,
            CommandOpts::Health {
                version_drift: true,
                missing_fields: true,
                sdk_consistency: true,
            }
        ));
        assert_eq!(opts.option_count(), 3);
    }

    #[test]
    fn test_build_default_pub() {
        let opts = CommandOpts::build_default("pub").unwrap();
        assert!(matches!(
            opts,
            CommandOpts::Pub {
                subcommand: 1,
                concurrency: 1,
                major_versions: false,
            }
        ));
        assert_eq!(opts.option_count(), 3);
    }

    #[test]
    fn test_pub_option_rows() {
        let opts = CommandOpts::build_default("pub").unwrap();
        let rows = opts.option_rows();
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0], OptionRow::Number(_, 1)));
        assert!(matches!(rows[1], OptionRow::Number(_, 1)));
        assert!(matches!(rows[2], OptionRow::Bool(_, false)));
    }

    #[test]
    fn test_pub_toggle_major_versions() {
        let mut opts = CommandOpts::build_default("pub").unwrap();
        // Index 2 is major-versions (Bool).
        opts.toggle_bool(2);
        match &opts {
            CommandOpts::Pub { major_versions, .. } => assert!(*major_versions),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_pub_increment_subcommand() {
        let mut opts = CommandOpts::build_default("pub").unwrap();
        // Index 0 is sub-cmd (Number), default is 1.
        opts.increment_at(0);
        match &opts {
            CommandOpts::Pub { subcommand, .. } => assert_eq!(*subcommand, 2),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_build_default_unsupported_returns_none() {
        assert!(CommandOpts::build_default("exec").is_none());
        assert!(CommandOpts::build_default("run").is_none());
        assert!(CommandOpts::build_default("build").is_none());
        assert!(CommandOpts::build_default("version").is_none());
        assert!(CommandOpts::build_default("list").is_none());
        assert!(CommandOpts::build_default("nonexistent").is_none());
    }

    #[test]
    fn test_toggle_bool_at_visual_index() {
        let mut opts = CommandOpts::build_default("analyze").unwrap();
        // Index 0 is concurrency (Number), index 1 is fatal-warnings (Bool).
        opts.toggle_bool(1);
        match &opts {
            CommandOpts::Analyze { fatal_warnings, .. } => assert!(*fatal_warnings),
            _ => panic!("wrong variant"),
        }
        // Toggle again to verify it flips back.
        opts.toggle_bool(1);
        match &opts {
            CommandOpts::Analyze { fatal_warnings, .. } => assert!(!*fatal_warnings),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_toggle_bool_on_number_is_noop() {
        let mut opts = CommandOpts::build_default("analyze").unwrap();
        let before = opts.clone();
        // Index 0 is concurrency (Number) -- toggle should be no-op.
        opts.toggle_bool(0);
        assert_eq!(format!("{opts:?}"), format!("{before:?}"));
    }

    #[test]
    fn test_toggle_bool_out_of_range_is_noop() {
        let mut opts = CommandOpts::build_default("clean").unwrap();
        let before = opts.clone();
        opts.toggle_bool(99);
        assert_eq!(format!("{opts:?}"), format!("{before:?}"));
    }

    #[test]
    fn test_toggle_bool_third_field() {
        // Analyze: index 3 = no_fatal (third bool field)
        let mut opts = CommandOpts::build_default("analyze").unwrap();
        opts.toggle_bool(3);
        match &opts {
            CommandOpts::Analyze { no_fatal, .. } => assert!(*no_fatal),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_increment_at_visual_index() {
        let mut opts = CommandOpts::build_default("analyze").unwrap();
        // Index 0 is concurrency (Number).
        opts.increment_at(0);
        match &opts {
            CommandOpts::Analyze { concurrency, .. } => assert_eq!(*concurrency, 2),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_decrement_at_visual_index() {
        let mut opts = CommandOpts::build_default("analyze").unwrap();
        // Increment twice then decrement once: should be 2.
        opts.increment_at(0);
        opts.increment_at(0);
        opts.decrement_at(0);
        match &opts {
            CommandOpts::Analyze { concurrency, .. } => assert_eq!(*concurrency, 2),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_decrement_at_minimum_is_one() {
        let mut opts = CommandOpts::build_default("analyze").unwrap();
        // Default is 1, decrement should stay at 1.
        opts.decrement_at(0);
        match &opts {
            CommandOpts::Analyze { concurrency, .. } => assert_eq!(*concurrency, 1),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_increment_on_bool_is_noop() {
        let mut opts = CommandOpts::build_default("analyze").unwrap();
        let before = opts.clone();
        // Index 1 is a bool field.
        opts.increment_at(1);
        assert_eq!(format!("{opts:?}"), format!("{before:?}"));
    }

    #[test]
    fn test_health_all_bools_toggle() {
        let mut opts = CommandOpts::build_default("health").unwrap();
        // Health has 3 bool rows at indices 0, 1, 2.
        opts.toggle_bool(0);
        opts.toggle_bool(1);
        opts.toggle_bool(2);
        match &opts {
            CommandOpts::Health {
                version_drift,
                missing_fields,
                sdk_consistency,
            } => {
                assert!(!*version_drift);
                assert!(!*missing_fields);
                assert!(!*sdk_consistency);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_option_rows_labels() {
        let opts = CommandOpts::build_default("format").unwrap();
        let rows = opts.option_rows();
        assert_eq!(rows.len(), 3);
        match &rows[0] {
            OptionRow::Number(label, val) => {
                assert_eq!(*label, "concurrency");
                assert_eq!(*val, 1);
            }
            _ => panic!("expected Number"),
        }
        match &rows[1] {
            OptionRow::Bool(label, val) => {
                assert_eq!(*label, "set-exit-if-changed");
                assert!(!val);
            }
            _ => panic!("expected Bool"),
        }
        match &rows[2] {
            OptionRow::OptNumber(label, val) => {
                assert_eq!(*label, "line-length");
                assert!(val.is_none());
            }
            _ => panic!("expected OptNumber"),
        }
    }

    // --- Options overlay key handling tests ---

    fn app_with_options(command: &str) -> App {
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Commands;
        // Find the command index in command_rows.
        let idx = app
            .command_rows
            .iter()
            .position(|c| c.name == command)
            .expect("command not found in builtin list");
        app.selected_command = idx;
        // Open the options overlay.
        press(&mut app, KeyCode::Enter);
        assert!(app.show_options, "options overlay should be visible");
        app
    }

    #[test]
    fn test_options_j_navigates_down() {
        let mut app = app_with_options("analyze");
        assert_eq!(app.selected_option, 0);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_option, 1);
    }

    #[test]
    fn test_options_k_wraps_to_bottom() {
        let mut app = app_with_options("analyze");
        assert_eq!(app.selected_option, 0);
        // k at 0 wraps to last (4 opts + 1 Run row = 5 total, last = 4).
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected_option, 4);
    }

    #[test]
    fn test_options_j_wraps_around() {
        let mut app = app_with_options("clean");
        // Clean has 1 option + 1 Run row = 2 total.
        assert_eq!(app.selected_option, 0);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_option, 1); // "Run" row
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_option, 0); // wrapped
    }

    #[test]
    fn test_options_space_toggles_bool() {
        let mut app = app_with_options("analyze");
        // Move to index 1 (fatal-warnings).
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' '));
        match app.command_opts.as_ref().unwrap() {
            CommandOpts::Analyze { fatal_warnings, .. } => assert!(*fatal_warnings),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_options_space_on_run_confirms() {
        let mut app = app_with_options("clean");
        // Clean: 1 option, Run is at index 1.
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_option, 1);
        press(&mut app, KeyCode::Char(' '));
        assert!(!app.show_options);
        assert!(app.pending_command.is_some());
    }

    #[test]
    fn test_options_plus_increments() {
        let mut app = app_with_options("analyze");
        // Index 0 = concurrency.
        press(&mut app, KeyCode::Char('+'));
        match app.command_opts.as_ref().unwrap() {
            CommandOpts::Analyze { concurrency, .. } => assert_eq!(*concurrency, 2),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_options_minus_decrements() {
        let mut app = app_with_options("analyze");
        // Increment to 3, then decrement to 2.
        press(&mut app, KeyCode::Char('+'));
        press(&mut app, KeyCode::Char('+'));
        press(&mut app, KeyCode::Char('-'));
        match app.command_opts.as_ref().unwrap() {
            CommandOpts::Analyze { concurrency, .. } => assert_eq!(*concurrency, 2),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_options_esc_dismisses() {
        let mut app = app_with_options("analyze");
        press(&mut app, KeyCode::Esc);
        assert!(!app.show_options);
        assert!(app.command_opts.is_none());
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn test_options_q_dismisses() {
        let mut app = app_with_options("format");
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.show_options);
        assert!(app.command_opts.is_none());
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn test_options_enter_confirms() {
        let mut app = app_with_options("test");
        press(&mut app, KeyCode::Enter);
        assert!(!app.show_options);
        assert_eq!(app.pending_command.as_deref(), Some("test"));
    }

    // --- Unsupported command bypass tests ---

    #[test]
    fn test_unsupported_command_skips_overlay() {
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Commands;
        // "exec" is at index 4 (analyze=0, bootstrap=1, build=2, clean=3, exec=4).
        app.selected_command = 4;
        press(&mut app, KeyCode::Enter);
        // Should set pending_command directly without opening overlay.
        assert!(!app.show_options);
        assert!(app.command_opts.is_none());
        assert_eq!(app.pending_command.as_deref(), Some("exec"));
    }

    #[test]
    fn test_unsupported_command_build_skips_overlay() {
        let mut app = App::new(Theme::default());
        app.active_panel = ActivePanel::Commands;
        // "build" is at index 2.
        app.selected_command = 2;
        press(&mut app, KeyCode::Enter);
        assert!(!app.show_options);
        assert_eq!(app.pending_command.as_deref(), Some("build"));
    }

    // --- is_supported field tests ---

    #[test]
    fn test_is_supported_set_on_builtin_commands() {
        let app = App::new(Theme::default());
        for row in &app.command_rows {
            let expected = SUPPORTED_COMMANDS.contains(&row.name.as_str());
            assert_eq!(
                row.is_supported, expected,
                "is_supported mismatch for '{}'",
                row.name
            );
        }
    }

    #[test]
    fn test_supported_commands_match_build_default() {
        // Every supported command must return Some from build_default.
        for &name in SUPPORTED_COMMANDS {
            assert!(
                CommandOpts::build_default(name).is_some(),
                "build_default returned None for supported command '{name}'"
            );
        }
    }

    // --- Scrollback truncation tests ---

    #[test]
    fn test_scrollback_truncation_at_limit() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        // Push exactly MAX_SCROLLBACK + 50 lines.
        for i in 0..MAX_SCROLLBACK + 50 {
            app.handle_core_event(CoreEvent::PackageOutput {
                name: "pkg".to_string(),
                line: format!("line {i}"),
                is_stderr: false,
            });
        }
        assert_eq!(app.output_log.len(), MAX_SCROLLBACK);
        // First retained line should be line 50 (oldest 50 were dropped).
        assert_eq!(app.output_log[0].1, "line 50");
    }

    #[test]
    fn test_scrollback_below_limit_no_truncation() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        for i in 0..100 {
            app.handle_core_event(CoreEvent::PackageOutput {
                name: "pkg".to_string(),
                line: format!("line {i}"),
                is_stderr: false,
            });
        }
        assert_eq!(app.output_log.len(), 100);
        assert_eq!(app.output_log[0].1, "line 0");
    }

    #[test]
    fn test_scrollback_truncation_adjusts_scroll_offset() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        // Push MAX_SCROLLBACK lines.
        for i in 0..MAX_SCROLLBACK {
            app.handle_core_event(CoreEvent::PackageOutput {
                name: "pkg".to_string(),
                line: format!("line {i}"),
                is_stderr: false,
            });
        }
        // Set scroll to line 100.
        app.output_scroll = 100;
        // Push 200 more lines to trigger truncation.
        for i in 0..200 {
            app.handle_core_event(CoreEvent::PackageOutput {
                name: "pkg".to_string(),
                line: format!("extra {i}"),
                is_stderr: false,
            });
        }
        assert_eq!(app.output_log.len(), MAX_SCROLLBACK);
        // Scroll offset should have been reduced by the 200 drained lines.
        assert_eq!(app.output_scroll, 0);
    }

    // --- Auto-scroll tests ---

    #[test]
    fn test_auto_scroll_default_true() {
        let app = App::new(Theme::default());
        assert!(app.auto_scroll);
    }

    #[test]
    fn test_auto_scroll_reset_on_start_command() {
        let mut app = App::new(Theme::default());
        app.auto_scroll = false;
        app.start_command("test");
        assert!(app.auto_scroll);
    }

    #[test]
    fn test_scroll_up_disables_auto_scroll_in_running() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        // Add some output so scroll has room.
        for i in 0..50 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        app.output_scroll = 30;
        assert!(app.auto_scroll);
        press(&mut app, KeyCode::Up);
        assert!(!app.auto_scroll);
        assert_eq!(app.output_scroll, 29);
    }

    #[test]
    fn test_scroll_down_at_end_reengages_auto_scroll() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        for i in 0..10 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        app.output_scroll = 8;
        app.auto_scroll = false;
        // Scroll down past end (max is 9).
        press(&mut app, KeyCode::Down);
        assert_eq!(app.output_scroll, 9);
        assert!(app.auto_scroll);
    }

    #[test]
    fn test_scroll_end_reengages_auto_scroll() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        for i in 0..20 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        app.auto_scroll = false;
        press(&mut app, KeyCode::End);
        assert!(app.auto_scroll);
        assert_eq!(app.output_scroll, 19);
    }

    #[test]
    fn test_home_disables_auto_scroll() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        for i in 0..20 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        press(&mut app, KeyCode::Home);
        assert!(!app.auto_scroll);
        assert_eq!(app.output_scroll, 0);
    }

    // --- Running scroll key tests ---

    #[test]
    fn test_running_j_scrolls_down() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        for i in 0..30 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        assert_eq!(app.output_scroll, 0);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.output_scroll, 1);
    }

    #[test]
    fn test_running_k_scrolls_up() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        for i in 0..30 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        app.output_scroll = 10;
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.output_scroll, 9);
    }

    #[test]
    fn test_running_g_jumps_to_start() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        for i in 0..30 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        app.output_scroll = 20;
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.output_scroll, 0);
    }

    #[test]
    fn test_running_shift_g_jumps_to_end() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        for i in 0..30 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.output_scroll, 29);
    }

    #[test]
    fn test_running_page_down() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.page_size = 10;
        for i in 0..50 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.output_scroll, 10);
    }

    #[test]
    fn test_running_page_up() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.page_size = 10;
        for i in 0..50 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        app.output_scroll = 25;
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.output_scroll, 15);
    }

    #[test]
    fn test_running_ctrl_d_half_page() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.page_size = 10;
        for i in 0..50 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        ctrl(&mut app, KeyCode::Char('d'));
        assert_eq!(app.output_scroll, 5);
    }

    #[test]
    fn test_running_ctrl_u_half_page_up() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.page_size = 10;
        for i in 0..50 {
            app.output_log
                .push(("pkg".to_string(), format!("line {i}"), false));
        }
        app.output_scroll = 20;
        ctrl(&mut app, KeyCode::Char('u'));
        assert_eq!(app.output_scroll, 15);
    }

    #[test]
    fn test_running_q_does_not_quit() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit());
    }

    #[test]
    fn test_running_tab_does_not_switch_panel() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        app.active_panel = ActivePanel::Packages;
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.active_panel, ActivePanel::Packages);
    }

    // --- Elapsed time lifecycle tests ---

    #[test]
    fn test_command_start_none_initially() {
        let app = App::new(Theme::default());
        assert!(app.command_start.is_none());
        assert!(app.elapsed().is_none());
    }

    #[test]
    fn test_start_command_sets_command_start() {
        let mut app = App::new(Theme::default());
        app.start_command("analyze");
        assert!(app.command_start.is_some());
        assert!(app.elapsed().is_some());
    }

    #[test]
    fn test_on_command_finished_clears_command_start() {
        let mut app = App::new(Theme::default());
        app.start_command("analyze");
        assert!(app.command_start.is_some());
        app.on_command_finished(Ok(Ok(())));
        assert!(app.command_start.is_none());
    }

    #[test]
    fn test_on_command_cancelled_clears_command_start() {
        let mut app = App::new(Theme::default());
        app.start_command("analyze");
        assert!(app.command_start.is_some());
        app.on_command_cancelled();
        assert!(app.command_start.is_none());
    }

    #[test]
    fn test_elapsed_returns_positive_duration() {
        let mut app = App::new(Theme::default());
        app.start_command("test");
        // Sleep briefly to ensure elapsed > 0.
        std::thread::sleep(Duration::from_millis(5));
        let elapsed = app.elapsed().unwrap();
        assert!(elapsed >= Duration::from_millis(1));
    }

    // --- Health report and tab cycling tests (Batch 53) ---

    fn make_health_report() -> HealthReport {
        HealthReport {
            version_drift: Some(vec![]),
            missing_fields: Some(vec![]),
            sdk_consistency: Some(melos_core::commands::health::SdkConsistencyResult::default()),
            total_issues: 0,
        }
    }

    #[test]
    fn test_health_report_none_initially() {
        let app = App::new(Theme::default());
        assert!(app.health_report.is_none());
        assert_eq!(app.health_tab, 0);
    }

    #[test]
    fn test_start_command_clears_health_report() {
        let mut app = App::new(Theme::default());
        app.health_report = Some(make_health_report());
        app.health_tab = 2;
        app.start_command("analyze");
        assert!(app.health_report.is_none());
        assert_eq!(app.health_tab, 0);
    }

    #[test]
    fn test_set_health_report() {
        let mut app = App::new(Theme::default());
        app.set_health_report(make_health_report());
        assert!(app.health_report.is_some());
    }

    #[test]
    fn test_done_tab_cycles_health_tab_forward() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Done;
        app.health_report = Some(make_health_report());
        assert_eq!(app.health_tab, 0);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.health_tab, 1);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.health_tab, 2);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.health_tab, 0);
    }

    #[test]
    fn test_done_backtab_cycles_health_tab_backward() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Done;
        app.health_report = Some(make_health_report());
        assert_eq!(app.health_tab, 0);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.health_tab, 2);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.health_tab, 1);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.health_tab, 0);
    }

    #[test]
    fn test_done_tab_without_health_report_does_nothing() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Done;
        // No health report set.
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.health_tab, 0);
        // Should still be in Done state (Tab didn't return to Idle).
        assert_eq!(app.state, AppState::Done);
    }

    #[test]
    fn test_on_command_finished_preserves_running_command() {
        let mut app = App::new(Theme::default());
        app.start_command("health");
        app.on_command_finished(Ok(Ok(())));
        assert_eq!(app.running_command.as_deref(), Some("health"));
    }

    #[test]
    fn test_on_command_cancelled_clears_running_command() {
        let mut app = App::new(Theme::default());
        app.start_command("health");
        app.on_command_cancelled();
        assert!(app.running_command.is_none());
    }

    // --- Filter bar tests (Batch 54) ---

    /// Helper: create an App with named package rows for filter testing.
    fn app_with_named_packages(names: &[&str]) -> App {
        let mut app = App::new(Theme::default());
        app.workspace_name = Some("test".to_string());
        for name in names {
            app.package_rows.push(PackageRow {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                sdk: "Dart",
                path: format!("packages/{name}"),
                is_private: false,
            });
        }
        app
    }

    #[test]
    fn test_slash_activates_filter() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        assert!(!app.filter_active);
        press(&mut app, KeyCode::Char('/'));
        assert!(app.filter_active);
    }

    #[test]
    fn test_filter_typing_appends_chars() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.filter_text, "al");
    }

    #[test]
    fn test_filter_backspace_removes_last_char() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.filter_text, "a");
    }

    #[test]
    fn test_filter_backspace_on_empty_is_noop() {
        let mut app = app_with_named_packages(&["alpha", "beta"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Backspace);
        assert!(app.filter_text.is_empty());
        assert!(app.filter_active);
    }

    #[test]
    fn test_filter_enter_applies_and_closes() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('l'));
        press(&mut app, KeyCode::Enter);
        assert!(!app.filter_active);
        assert!(app.has_filter());
        assert_eq!(app.filter_text, "l");
        // Only "alpha" contains 'l'.
        assert_eq!(app.visible_package_count(), 1);
    }

    #[test]
    fn test_filter_esc_cancels_and_clears() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Esc);
        assert!(!app.filter_active);
        assert!(!app.has_filter());
        assert!(app.filter_text.is_empty());
        assert!(app.filtered_indices.is_empty());
    }

    #[test]
    fn test_filter_live_preview_recomputes_on_keystroke() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        press(&mut app, KeyCode::Char('/'));
        // 'l' matches only "alpha"
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.filtered_indices[0], 0); // index of "alpha"
        // Backspace to clear, then 'e' matches "beta" only
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.filtered_indices[0], 1); // index of "beta"
    }

    #[test]
    fn test_filter_case_insensitive() {
        let mut app = app_with_named_packages(&["Alpha", "BETA", "gamma"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.visible_package_count(), 1);
        let visible = app.visible_packages();
        assert_eq!(visible[0].name, "BETA");
    }

    #[test]
    fn test_has_filter_returns_false_when_empty() {
        let app = App::new(Theme::default());
        assert!(!app.has_filter());
    }

    #[test]
    fn test_has_filter_returns_true_with_text() {
        let mut app = app_with_named_packages(&["alpha"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        assert!(app.has_filter());
    }

    #[test]
    fn test_visible_package_count_without_filter() {
        let app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        assert_eq!(app.visible_package_count(), 3);
    }

    #[test]
    fn test_visible_package_count_with_filter() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('l'));
        press(&mut app, KeyCode::Enter);
        // Only "alpha" contains 'l'
        assert_eq!(app.visible_package_count(), 1);
    }

    #[test]
    fn test_visible_packages_without_filter() {
        let app = app_with_named_packages(&["alpha", "beta"]);
        let visible = app.visible_packages();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].name, "alpha");
        assert_eq!(visible[1].name, "beta");
    }

    #[test]
    fn test_visible_packages_with_filter() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Enter);
        let visible = app.visible_packages();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "beta");
    }

    #[test]
    fn test_filter_resets_selected_package_on_apply() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        app.selected_package = 2;
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_filter_resets_selected_package_on_cancel() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        app.selected_package = 2;
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.selected_package, 0);
    }

    #[test]
    fn test_filter_empty_result() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('z'));
        press(&mut app, KeyCode::Enter);
        assert!(app.has_filter());
        assert_eq!(app.visible_package_count(), 0);
        assert!(app.visible_packages().is_empty());
    }

    #[test]
    fn test_esc_in_idle_clears_filter_instead_of_quit() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma"]);
        // Apply a filter first.
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        assert!(app.has_filter());
        // Esc should clear filter, not quit.
        press(&mut app, KeyCode::Esc);
        assert!(!app.has_filter());
        assert!(!app.quit);
        // Second Esc should now quit.
        press(&mut app, KeyCode::Esc);
        assert!(app.quit);
    }

    #[test]
    fn test_filter_input_blocks_other_keys() {
        let mut app = app_with_named_packages(&["alpha", "beta"]);
        app.active_panel = ActivePanel::Packages;
        press(&mut app, KeyCode::Char('/'));
        // 'j' should go into filter text, not move selection.
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.filter_text, "j");
        assert_eq!(app.selected_package, 0); // Unchanged (recompute resets to 0).
    }

    #[test]
    fn test_filter_navigation_uses_visible_count() {
        let mut app = app_with_named_packages(&["alpha", "beta", "gamma", "delta"]);
        app.active_panel = ActivePanel::Packages;
        // Apply filter matching 2 packages: "alpha" and "delta" (contain 'l').
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('l'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.visible_package_count(), 2);
        // Navigate down: should wrap at 2 (visible count), not 4 (total).
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_package, 1);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_package, 0); // Wrapped.
    }

    #[test]
    fn test_filter_enter_with_empty_text_clears_filter() {
        let mut app = app_with_named_packages(&["alpha", "beta"]);
        // First apply a real filter.
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        assert!(app.has_filter());
        // Now open filter again, clear text, and press Enter.
        press(&mut app, KeyCode::Char('/'));
        // Backspace to clear the existing text.
        press(&mut app, KeyCode::Backspace);
        assert!(app.filter_text.is_empty());
        press(&mut app, KeyCode::Enter);
        assert!(!app.has_filter());
        assert_eq!(app.visible_package_count(), 2);
    }

    #[test]
    fn test_update_page_size() {
        let mut app = App::new(Theme::default());
        app.update_page_size(40);
        assert_eq!(app.page_size, 35); // 40 - 5
    }

    #[test]
    fn test_update_page_size_small_terminal() {
        let mut app = App::new(Theme::default());
        app.update_page_size(3);
        assert_eq!(app.page_size, 0); // saturating_sub(5) from 3
    }

    #[test]
    fn test_slash_only_activates_in_idle() {
        let mut app = app_with_named_packages(&["alpha"]);
        app.state = AppState::Running;
        press(&mut app, KeyCode::Char('/'));
        assert!(!app.filter_active);
    }

    #[test]
    fn test_filter_preserves_text_after_apply() {
        let mut app = app_with_named_packages(&["alpha", "beta"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        // Filter text should persist after apply (user can see what's filtered).
        assert_eq!(app.filter_text, "a");
        assert!(!app.filter_active);
    }

    #[test]
    fn test_reactivate_filter_keeps_existing_text() {
        let mut app = app_with_named_packages(&["alpha", "beta"]);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.filter_text, "a");
        // Reactivate filter -- text should still be 'a'.
        press(&mut app, KeyCode::Char('/'));
        assert!(app.filter_active);
        assert_eq!(app.filter_text, "a");
    }

    // ── Theme cycling tests ───────────────────────────────────────────

    #[test]
    fn test_theme_name_default() {
        let app = App::new(Theme::default());
        assert_eq!(app.theme_name(), "dark");
        assert_eq!(app.theme_index, 0);
    }

    #[test]
    fn test_cycle_theme_advances_index() {
        let mut app = App::new(Theme::default());
        let names = Theme::available_names();
        assert_eq!(app.theme_index, 0);

        app.cycle_theme();
        assert_eq!(app.theme_index, 1);
        assert_eq!(app.theme_name(), names[1]);
    }

    #[test]
    fn test_cycle_theme_wraps_around() {
        let mut app = App::new(Theme::default());
        let count = Theme::available_names().len();
        // Cycle through all themes to wrap back to 0.
        for _ in 0..count {
            app.cycle_theme();
        }
        assert_eq!(app.theme_index, 0);
        assert_eq!(app.theme_name(), "dark");
    }

    #[test]
    fn test_cycle_theme_updates_theme_struct() {
        let mut app = App::new(Theme::default());
        let original_accent = app.theme.accent;

        // Cycle until we find a theme with a different accent (light theme should differ).
        let count = Theme::available_names().len();
        let mut found_different = false;
        for _ in 0..count {
            app.cycle_theme();
            if app.theme.accent != original_accent {
                found_different = true;
                break;
            }
        }
        assert!(
            found_different,
            "Expected at least one theme with a different accent color"
        );
    }

    #[test]
    fn test_t_key_cycles_theme_in_idle() {
        let mut app = App::new(Theme::default());
        assert_eq!(app.theme_index, 0);

        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.theme_index, 1);

        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.theme_index, 2);
    }

    #[test]
    fn test_t_key_ignored_when_not_idle() {
        let mut app = App::new(Theme::default());
        app.state = AppState::Running;
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(
            app.theme_index, 0,
            "Theme should not cycle in Running state"
        );
    }

    #[test]
    fn test_theme_index_set_nonzero() {
        let mut app = App::new(Theme::default());
        let names = Theme::available_names();
        // Simulate --theme flag setting index to last theme.
        let last = names.len() - 1;
        app.theme_index = last;
        if let Some(t) = Theme::by_name(names[last]) {
            app.theme = t;
        }
        assert_eq!(app.theme_name(), names[last]);

        // Cycling should wrap to 0.
        app.cycle_theme();
        assert_eq!(app.theme_index, 0);
        assert_eq!(app.theme_name(), "dark");
    }
}
