use anyhow::Result;
use clap::Args;
use colored::Colorize;

use crate::config::filter::PackageFilters;
use crate::package::filter::apply_filters;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

/// Arguments for the `exec` command
#[derive(Args, Debug)]
pub struct ExecArgs {
    /// Command to execute in each package
    #[arg(trailing_var_arg = true, required = true)]
    pub command: Vec<String>,

    /// Maximum number of concurrent processes
    #[arg(short = 'c', long, default_value = "1")]
    pub concurrency: usize,

    /// Stop execution on first failure
    #[arg(long)]
    pub fail_fast: bool,

    /// Only include packages that depend on this package
    #[arg(long = "depends-on")]
    pub depends_on: Option<String>,

    /// Only include Flutter packages
    #[arg(long)]
    pub flutter: bool,

    /// Only include non-Flutter (Dart) packages
    #[arg(long)]
    pub no_flutter: bool,

    /// Only include packages where this file exists
    #[arg(long = "file-exists")]
    pub file_exists: Option<String>,

    /// Only include packages where this directory exists
    #[arg(long = "dir-exists")]
    pub dir_exists: Option<String>,
}

impl ExecArgs {
    /// Convert CLI flags into PackageFilters
    fn to_package_filters(&self) -> PackageFilters {
        let flutter = if self.flutter {
            Some(true)
        } else if self.no_flutter {
            Some(false)
        } else {
            None
        };

        PackageFilters {
            flutter,
            dir_exists: self.dir_exists.clone(),
            file_exists: self.file_exists.clone(),
            depends_on: self
                .depends_on
                .as_ref()
                .map(|d| d.split(',').map(|s| s.trim().to_string()).collect()),
            ignore: None,
            scope: None,
        }
    }
}

/// Execute a command across all matching packages
pub async fn run(workspace: &Workspace, args: ExecArgs) -> Result<()> {
    let cmd_str = args.command.join(" ");
    println!(
        "\n{} Running '{}' in packages...\n",
        "$".cyan(),
        cmd_str.bold()
    );

    // Apply filters
    let filters = args.to_package_filters();
    let packages = apply_filters(&workspace.packages, &filters);

    if packages.is_empty() {
        println!("{}", "No packages matched the given filters.".yellow());
        return Ok(());
    }

    println!(
        "Running in {} package(s) with concurrency {}:\n",
        packages.len().to_string().cyan(),
        args.concurrency.to_string().cyan()
    );

    for pkg in &packages {
        println!("  {} {}", "->".cyan(), pkg.name);
    }
    println!();

    // Execute command in each package
    let runner = ProcessRunner::new(args.concurrency, args.fail_fast);
    let results = runner
        .run_in_packages(&packages, &cmd_str, &workspace.env_vars())
        .await?;

    // Report results
    let mut failed = 0;
    for (pkg_name, success) in &results {
        if *success {
            println!("  {} {}", "SUCCESS".green(), pkg_name);
        } else {
            println!("  {} {}", "FAILED".red(), pkg_name);
            failed += 1;
        }
    }

    if failed > 0 {
        anyhow::bail!("{} package(s) failed", failed);
    }

    Ok(())
}
