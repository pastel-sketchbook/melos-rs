use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use crate::events::Event;
use crate::package::Package;
use crate::runner::ProcessRunner;
use crate::workspace::Workspace;

use super::PackageResults;

/// Options for the test command (clap-free).
#[derive(Debug, Clone)]
pub struct TestOpts {
    pub concurrency: usize,
    pub fail_fast: bool,
    pub coverage: bool,
    pub test_randomize_ordering_seed: Option<String>,
    pub update_goldens: bool,
    pub no_run: bool,
    pub extra_args: Vec<String>,
}

/// Build extra flags from test options (coverage, randomize, no-run, update-goldens).
pub fn build_extra_flags(opts: &TestOpts) -> Vec<String> {
    let mut flags = Vec::new();

    if opts.coverage {
        flags.push("--coverage".to_string());
    }

    if let Some(ref seed) = opts.test_randomize_ordering_seed {
        flags.push(format!("--test-randomize-ordering-seed={}", seed));
    }

    if opts.no_run {
        flags.push("--no-run".to_string());
    }

    if opts.update_goldens {
        flags.push("--update-goldens".to_string());
    }

    flags
}

/// Build the full test command string.
pub fn build_test_command(sdk: &str, extra_flags: &[String], extra_args: &[String]) -> String {
    let mut parts = vec![sdk.to_string(), "test".to_string()];
    parts.extend(extra_flags.iter().cloned());
    parts.extend(extra_args.iter().cloned());
    parts.join(" ")
}

/// Run tests across packages, splitting into Flutter and Dart SDKs.
///
/// Returns combined [`PackageResults`] from both SDK runs.
pub async fn run(
    packages: &[Package],
    workspace: &Workspace,
    opts: &TestOpts,
    events: Option<&UnboundedSender<Event>>,
) -> Result<PackageResults> {
    let flutter_pkgs: Vec<_> = packages.iter().filter(|p| p.is_flutter).cloned().collect();
    let dart_pkgs: Vec<_> = packages.iter().filter(|p| !p.is_flutter).cloned().collect();

    let extra_flags = build_extra_flags(opts);
    let runner = ProcessRunner::new(opts.concurrency, opts.fail_fast);
    let mut all_results = Vec::new();

    if !flutter_pkgs.is_empty() {
        let cmd = build_test_command("flutter", &extra_flags, &opts.extra_args);
        if let Some(tx) = events {
            let _ = tx.send(Event::Progress {
                completed: 0,
                total: 0,
                message: "flutter test...".into(),
            });
        }
        let results = runner
            .run_in_packages_with_events(
                &flutter_pkgs,
                &cmd,
                &workspace.env_vars(),
                None,
                events,
                &workspace.packages,
            )
            .await?;
        all_results.extend(results);
    }

    if !dart_pkgs.is_empty() {
        let cmd = build_test_command("dart", &extra_flags, &opts.extra_args);
        if let Some(tx) = events {
            let _ = tx.send(Event::Progress {
                completed: 0,
                total: 0,
                message: "dart test...".into(),
            });
        }
        let results = runner
            .run_in_packages_with_events(
                &dart_pkgs,
                &cmd,
                &workspace.env_vars(),
                None,
                events,
                &workspace.packages,
            )
            .await?;
        all_results.extend(results);
    }

    Ok(PackageResults::from(all_results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_test_command_default() {
        let cmd = build_test_command("dart", &[], &[]);
        assert_eq!(cmd, "dart test");
    }

    #[test]
    fn test_build_test_command_flutter_with_coverage() {
        let flags = vec!["--coverage".to_string()];
        let cmd = build_test_command("flutter", &flags, &[]);
        assert_eq!(cmd, "flutter test --coverage");
    }

    #[test]
    fn test_build_test_command_with_all_flags() {
        let flags = vec![
            "--coverage".to_string(),
            "--test-randomize-ordering-seed=42".to_string(),
            "--no-run".to_string(),
        ];
        let extra = vec!["--reporter=expanded".to_string()];
        let cmd = build_test_command("dart", &flags, &extra);
        assert_eq!(
            cmd,
            "dart test --coverage --test-randomize-ordering-seed=42 --no-run --reporter=expanded"
        );
    }

    #[test]
    fn test_build_extra_flags_empty() {
        let opts = TestOpts {
            concurrency: 1,
            fail_fast: false,
            coverage: false,
            test_randomize_ordering_seed: None,
            no_run: false,
            update_goldens: false,
            extra_args: vec![],
        };
        let flags = build_extra_flags(&opts);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_build_extra_flags_all() {
        let opts = TestOpts {
            concurrency: 5,
            fail_fast: true,
            coverage: true,
            test_randomize_ordering_seed: Some("0".to_string()),
            no_run: true,
            update_goldens: true,
            extra_args: vec![],
        };
        let flags = build_extra_flags(&opts);
        assert_eq!(flags.len(), 4);
        assert!(flags.contains(&"--coverage".to_string()));
        assert!(flags.contains(&"--test-randomize-ordering-seed=0".to_string()));
        assert!(flags.contains(&"--no-run".to_string()));
        assert!(flags.contains(&"--update-goldens".to_string()));
    }

    #[test]
    fn test_build_extra_flags_update_goldens_only() {
        let opts = TestOpts {
            concurrency: 1,
            fail_fast: false,
            coverage: false,
            test_randomize_ordering_seed: None,
            no_run: false,
            update_goldens: true,
            extra_args: vec![],
        };
        let flags = build_extra_flags(&opts);
        assert_eq!(flags, vec!["--update-goldens"]);
    }

    #[test]
    fn test_build_test_command_with_update_goldens() {
        let flags = vec!["--update-goldens".to_string()];
        let cmd = build_test_command("flutter", &flags, &[]);
        assert_eq!(cmd, "flutter test --update-goldens");
    }
}
