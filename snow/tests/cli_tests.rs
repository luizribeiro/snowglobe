use assert_cmd::Command;
use predicates::prelude::*;

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// snow run on the fixture flake, whatever directory the tests run in
fn snow_cmd() -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("snow");
    cmd.arg("-f").arg(fixtures_dir());
    cmd
}

#[test]
fn test_help_command() {
    snow_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("NixOS cluster orchestration tool"))
        .stdout(predicate::str::contains("build"))
        .stdout(predicate::str::contains("push"));
}

#[test]
fn test_push_help() {
    snow_cmd()
        .arg("push")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Deploy configurations to hosts"));
}

#[test]
fn test_list_help() {
    snow_cmd()
        .arg("list")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "List hostnames from nixosConfigurations",
        ));
}

#[test]
fn test_status_help() {
    snow_cmd()
        .arg("status")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Show cluster status"));
}

#[test]
fn test_build_help() {
    snow_cmd()
        .arg("build")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Build NixOS configurations for one or more hosts",
        ));
}

#[test]
fn test_build_filter_flag_in_help() {
    snow_cmd()
        .arg("build")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Filter hosts using a Nix expression",
        ));
}

#[test]
fn test_push_filter_flag_in_help() {
    snow_cmd()
        .arg("push")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Filter hosts using a Nix expression",
        ));
}

#[test]
fn test_build_phase_flag_in_help() {
    snow_cmd()
        .arg("build")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Filter by deployment phase"));
}

#[test]
fn test_build_phases_flag_in_help() {
    snow_cmd()
        .arg("build")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Filter by multiple phases"));
}

#[test]
fn test_build_tag_flag_in_help() {
    snow_cmd()
        .arg("build")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Filter by tag"));
}

#[test]
fn test_build_tags_flag_in_help() {
    snow_cmd()
        .arg("build")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Filter by multiple tags"));
}

#[test]
fn test_push_phase_flag_in_help() {
    snow_cmd()
        .arg("push")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Filter by deployment phase"));
}

#[test]
fn test_push_redeploy_flag_in_help() {
    snow_cmd()
        .arg("push")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--redeploy"));
}

#[test]
fn test_push_tags_flag_in_help() {
    snow_cmd()
        .arg("push")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Filter by multiple tags"));
}

#[test]
fn test_build_filter_and_phase_mutually_exclusive() {
    snow_cmd()
        .arg("build")
        .arg("-e")
        .arg("config: true")
        .arg("--phase")
        .arg("production")
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_build_phase_and_phases_mutually_exclusive() {
    snow_cmd()
        .arg("build")
        .arg("--phase")
        .arg("production")
        .arg("--phases")
        .arg("staging")
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_build_tag_and_tags_mutually_exclusive() {
    snow_cmd()
        .arg("build")
        .arg("--tag")
        .arg("web")
        .arg("--tags")
        .arg("database")
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_build_phase_and_tag_mutually_exclusive() {
    snow_cmd()
        .arg("build")
        .arg("--phase")
        .arg("production")
        .arg("--tag")
        .arg("web")
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_push_filter_and_tag_mutually_exclusive() {
    snow_cmd()
        .arg("push")
        .arg("-e")
        .arg("config: true")
        .arg("--tag")
        .arg("web")
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_build_invalid_phase_name() {
    snow_cmd()
        .arg("build")
        .arg("--phase")
        .arg("prod!uction")
        .assert()
        .failure()
        .stderr(predicate::str::contains("only ASCII alphanumeric"));
}

#[test]
fn test_build_empty_phase_name() {
    snow_cmd()
        .arg("build")
        .arg("--phase")
        .arg("!@#$")
        .assert()
        .failure()
        .stderr(predicate::str::contains("must contain at least one"));
}

#[test]
fn test_push_invalid_tag_name() {
    snow_cmd()
        .arg("push")
        .arg("--tag")
        .arg("web.server")
        .assert()
        .failure()
        .stderr(predicate::str::contains("only ASCII alphanumeric"));
}

// --mock-fixture only exists in debug builds
#[cfg(debug_assertions)]
#[test]
fn test_push_without_a_terminal_logs_each_step_to_stderr() {
    snow_cmd()
        .env("RUST_LOG", "snowlib=info")
        .args(["push", "--switch", "--quiet", "--mock-fixture"])
        .arg(fixtures_dir().join("push_scenarios/build_failure.json"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("nitrogen: build started"))
        .stderr(predicate::str::contains("nitrogen: build finished in "))
        .stderr(predicate::str::contains("hydrogen: failed at build: "))
        .stdout(predicate::str::contains(
            "in 1 phase:\n  production  hydrogen",
        ));
}
