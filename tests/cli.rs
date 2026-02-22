//! Integration tests for the melos-rs CLI binary.
//!
//! These tests exercise the compiled binary end-to-end using `assert_cmd`.
//! Fixture workspaces are created in temp directories with `tempfile`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a minimal Melos 6.x workspace in `dir` with the given packages.
/// Each package entry is `(name, version, is_flutter, dependencies)`.
fn create_fixture_workspace(
    dir: &Path,
    workspace_name: &str,
    packages: &[(&str, &str, bool, &[&str])],
) {
    // Write melos.yaml
    let melos_yaml =
        format!("name: {workspace_name}\n\npackages:\n  - packages/*\n\nscripts: {{}}\n");
    fs::write(dir.join("melos.yaml"), melos_yaml).unwrap();

    // Create each package
    for (name, version, is_flutter, deps) in packages {
        let pkg_dir = dir.join("packages").join(name);
        fs::create_dir_all(&pkg_dir).unwrap();

        let mut pubspec = format!("name: {name}\nversion: {version}\n");

        if *is_flutter {
            pubspec.push_str("\ndependencies:\n  flutter:\n    sdk: flutter\n");
        }

        if !deps.is_empty() {
            if !is_flutter {
                pubspec.push_str("\ndependencies:\n");
            }
            for dep in *deps {
                pubspec.push_str(&format!("  {dep}:\n    path: ../{dep}\n"));
            }
        }

        fs::write(pkg_dir.join("pubspec.yaml"), pubspec).unwrap();
    }
}

/// Build a `Command` for the melos-rs binary.
fn melos_cmd() -> Command {
    assert_cmd::cargo_bin_cmd!("melos-rs")
}

// ---------------------------------------------------------------------------
// Basic CLI tests
// ---------------------------------------------------------------------------

#[test]
fn test_help_output() {
    melos_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("melos-rs"))
        .stdout(predicate::str::contains("exec"))
        .stdout(predicate::str::contains("bootstrap"))
        .stdout(predicate::str::contains("list"));
}

#[test]
fn test_version_flag() {
    melos_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("melos-rs"));
}

#[test]
fn test_no_workspace_error() {
    let dir = TempDir::new().unwrap();
    melos_cmd()
        .current_dir(dir.path())
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Could not find melos.yaml"));
}

// ---------------------------------------------------------------------------
// Init command tests
// ---------------------------------------------------------------------------

#[test]
fn test_init_creates_7x_workspace() {
    let dir = TempDir::new().unwrap();
    melos_cmd()
        .current_dir(dir.path())
        .args(["init", "test_ws", "-d", "."])
        .write_stdin("n\n") // decline apps directory
        .assert()
        .success()
        .stdout(predicate::str::contains("Initializing workspace"))
        .stdout(predicate::str::contains("Created:"))
        .stdout(predicate::str::contains("pubspec.yaml"));

    // Verify pubspec.yaml was created with correct content
    let pubspec = fs::read_to_string(dir.path().join("pubspec.yaml")).unwrap();
    assert!(pubspec.contains("name: test_ws"));
    assert!(pubspec.contains("melos:"));
    assert!(pubspec.contains("workspace:"));
    assert!(pubspec.contains("packages/*"));

    // Verify packages/ dir was created
    assert!(dir.path().join("packages").is_dir());
}

#[test]
fn test_init_legacy_creates_melos_yaml() {
    let dir = TempDir::new().unwrap();
    melos_cmd()
        .current_dir(dir.path())
        .args(["init", "legacy_ws", "-d", ".", "--legacy"])
        .write_stdin("n\n") // decline apps directory
        .assert()
        .success()
        .stdout(predicate::str::contains("Created:"))
        .stdout(predicate::str::contains("melos.yaml"));

    // Verify melos.yaml was created
    let melos = fs::read_to_string(dir.path().join("melos.yaml")).unwrap();
    assert!(melos.contains("name: legacy_ws"));
    assert!(melos.contains("packages/*"));

    // Verify pubspec.yaml was also created (6.x root package)
    let pubspec = fs::read_to_string(dir.path().join("pubspec.yaml")).unwrap();
    assert!(pubspec.contains("name: legacy_ws"));
    assert!(pubspec.contains("melos: ^7.0.0"));
}

// ---------------------------------------------------------------------------
// List command tests
// ---------------------------------------------------------------------------

#[test]
fn test_list_packages() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "test_mono",
        &[
            ("core", "1.0.0", false, &[]),
            ("app", "2.0.0", false, &["core"]),
        ],
    );

    melos_cmd()
        .current_dir(dir.path())
        .args(["list", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("core"))
        .stdout(predicate::str::contains("app"));
}

#[test]
fn test_list_json_output() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "json_test",
        &[
            ("alpha", "1.2.3", false, &[]),
            ("beta", "0.1.0", false, &[]),
        ],
    );

    let output = melos_cmd()
        .current_dir(dir.path())
        .args(["list", "--json", "--quiet"])
        .output()
        .expect("command should run");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Invalid JSON output: {e}\nOutput: {stdout}"));

    let arr = parsed.as_array().expect("should be a JSON array");
    assert_eq!(arr.len(), 2);

    // Check fields
    let names: Vec<&str> = arr.iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
}

#[test]
fn test_list_parsable_output() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "parsable_test",
        &[("widget", "3.0.0", false, &[])],
    );

    melos_cmd()
        .current_dir(dir.path())
        .args(["list", "--parsable", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("widget:3.0.0:"));
}

#[test]
fn test_list_graph_output() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "graph_test",
        &[
            ("base", "1.0.0", false, &[]),
            ("derived", "1.0.0", false, &["base"]),
        ],
    );

    melos_cmd()
        .current_dir(dir.path())
        .args(["list", "--graph", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("derived"))
        .stdout(predicate::str::contains("base"));
}

// ---------------------------------------------------------------------------
// Exec command tests
// ---------------------------------------------------------------------------

#[test]
fn test_exec_echo() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "exec_test",
        &[
            ("pkg_a", "1.0.0", false, &[]),
            ("pkg_b", "1.0.0", false, &[]),
        ],
    );

    melos_cmd()
        .current_dir(dir.path())
        .args(["exec", "--quiet", "--", "echo", "hello"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello"));
}

#[test]
fn test_exec_dry_run() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(dir.path(), "dry_test", &[("pkg_x", "1.0.0", false, &[])]);

    melos_cmd()
        .current_dir(dir.path())
        .args([
            "exec",
            "--dry-run",
            "--quiet",
            "--",
            "echo",
            "should_not_run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("pkg_x"))
        .stdout(predicate::str::contains("echo should_not_run"));
}

// ---------------------------------------------------------------------------
// Completion command test
// ---------------------------------------------------------------------------

#[test]
fn test_completion_bash() {
    // Completion doesn't require a workspace
    melos_cmd()
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete"));
}

// ---------------------------------------------------------------------------
// Health command test
// ---------------------------------------------------------------------------

#[test]
fn test_health_check_no_issues() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "health_test",
        &[
            ("svc_a", "1.0.0", false, &[]),
            ("svc_b", "2.0.0", false, &[]),
        ],
    );

    // Only run version-drift check (fixture packages lack description/homepage/sdk)
    melos_cmd()
        .current_dir(dir.path())
        .args(["health", "--version-drift", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No health issues found"));
}

// ---------------------------------------------------------------------------
// Scope filter integration test
// ---------------------------------------------------------------------------

#[test]
fn test_list_with_scope_filter() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "scope_test",
        &[
            ("auth", "1.0.0", false, &[]),
            ("billing", "1.0.0", false, &[]),
            ("core", "1.0.0", false, &[]),
        ],
    );

    // Scope to only "auth"
    melos_cmd()
        .current_dir(dir.path())
        .args(["list", "--scope", "auth", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("auth"))
        .stdout(predicate::str::contains("billing").not())
        .stdout(predicate::str::contains("core").not());
}

// ---------------------------------------------------------------------------
// Clean command tests
// ---------------------------------------------------------------------------

#[test]
fn test_clean_dart_packages() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "clean_test",
        &[
            ("pkg_a", "1.0.0", false, &[]),
            ("pkg_b", "1.0.0", false, &[]),
        ],
    );

    // Create build/ directories to be cleaned
    let build_a = dir.path().join("packages/pkg_a/build");
    let build_b = dir.path().join("packages/pkg_b/build");
    fs::create_dir_all(&build_a).unwrap();
    fs::create_dir_all(&build_b).unwrap();
    fs::write(build_a.join("output.js"), "// compiled").unwrap();

    assert!(build_a.exists());
    assert!(build_b.exists());

    melos_cmd()
        .current_dir(dir.path())
        .args(["clean", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("CLEANED"));

    // build/ directories should be removed for pure Dart packages
    assert!(!build_a.exists(), "pkg_a/build should be removed");
    assert!(!build_b.exists(), "pkg_b/build should be removed");
}

#[test]
fn test_clean_deep() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "deep_clean_test",
        &[("mylib", "1.0.0", false, &[])],
    );

    let pkg_dir = dir.path().join("packages/mylib");

    // Create artifacts that deep clean should remove
    let dart_tool = pkg_dir.join(".dart_tool");
    let build_dir = pkg_dir.join("build");
    let lock_file = pkg_dir.join("pubspec.lock");
    fs::create_dir_all(&dart_tool).unwrap();
    fs::create_dir_all(&build_dir).unwrap();
    fs::write(&lock_file, "# lock file").unwrap();

    melos_cmd()
        .current_dir(dir.path())
        .args(["clean", "--deep", "--quiet"])
        .assert()
        .success();

    assert!(
        !dart_tool.exists(),
        ".dart_tool should be removed by deep clean"
    );
    assert!(!build_dir.exists(), "build should be removed by deep clean");
    assert!(
        !lock_file.exists(),
        "pubspec.lock should be removed by deep clean"
    );
}

// ---------------------------------------------------------------------------
// Exec with scope filter test
// ---------------------------------------------------------------------------

#[test]
fn test_exec_with_scope_filter() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "exec_scope_test",
        &[
            ("alpha", "1.0.0", false, &[]),
            ("beta", "1.0.0", false, &[]),
        ],
    );

    // Only execute in "alpha" package
    let output = melos_cmd()
        .current_dir(dir.path())
        .args([
            "exec", "--scope", "alpha", "--quiet", "--", "echo", "FOUND_IT",
        ])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FOUND_IT"), "Should see echo output");
    // Should only run in alpha, not beta
    assert!(stdout.contains("alpha"), "Should mention alpha package");
}

// ---------------------------------------------------------------------------
// List with ignore filter test
// ---------------------------------------------------------------------------

#[test]
fn test_list_with_ignore_filter() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "ignore_test",
        &[
            ("core", "1.0.0", false, &[]),
            ("internal", "1.0.0", false, &[]),
            ("utils", "1.0.0", false, &[]),
        ],
    );

    // Ignore "internal" package
    melos_cmd()
        .current_dir(dir.path())
        .args(["list", "--ignore", "internal", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("core"))
        .stdout(predicate::str::contains("utils"))
        .stdout(predicate::str::contains("internal").not());
}

// ---------------------------------------------------------------------------
// List with no-private filter test
// ---------------------------------------------------------------------------

#[test]
fn test_list_with_no_private_filter() {
    let dir = TempDir::new().unwrap();
    create_fixture_workspace(
        dir.path(),
        "private_test",
        &[("public_pkg", "1.0.0", false, &[])],
    );

    // Create a private package manually (publish_to: none)
    let private_dir = dir.path().join("packages/private_pkg");
    fs::create_dir_all(&private_dir).unwrap();
    fs::write(
        private_dir.join("pubspec.yaml"),
        "name: private_pkg\nversion: 1.0.0\npublish_to: none\n",
    )
    .unwrap();

    melos_cmd()
        .current_dir(dir.path())
        .args(["list", "--no-private", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("public_pkg"))
        .stdout(predicate::str::contains("private_pkg").not());
}

// ---------------------------------------------------------------------------
// Init with apps directory test
// ---------------------------------------------------------------------------

#[test]
fn test_init_7x_with_apps() {
    let dir = TempDir::new().unwrap();
    melos_cmd()
        .current_dir(dir.path())
        .args(["init", "apps_ws", "-d", "."])
        .write_stdin("y\n") // accept apps directory
        .assert()
        .success()
        .stdout(predicate::str::contains("Created:"));

    // Should have both packages/ and apps/ dirs
    assert!(dir.path().join("packages").is_dir());
    assert!(dir.path().join("apps").is_dir());
}
