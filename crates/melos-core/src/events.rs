use std::time::Duration;

/// Events emitted by the core process runner during command execution.
///
/// These events decouple the execution engine from the presentation layer,
/// allowing different frontends (CLI progress bars, TUI, JSON output) to
/// consume the same event stream.
#[derive(Debug, Clone)]
pub enum Event {
    /// A command is about to run across packages.
    CommandStarted {
        command: String,
        package_count: usize,
    },
    /// All packages have finished for a command.
    CommandFinished { command: String, duration: Duration },
    /// Execution has started for a specific package.
    PackageStarted { name: String },
    /// A package command has finished.
    PackageFinished {
        name: String,
        success: bool,
        duration: Duration,
    },
    /// A line of output from a package command.
    PackageOutput {
        name: String,
        line: String,
        is_stderr: bool,
    },
    /// Progress update for adjusting progress bar message or position.
    Progress {
        completed: usize,
        total: usize,
        message: String,
    },
    /// A warning message.
    Warning(String),
    /// An informational message.
    Info(String),
}
