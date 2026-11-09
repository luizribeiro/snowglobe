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
        .stdout(predicate::str::contains("deploy"));
}

#[cfg(not(feature = "unstable"))]
const UNSTABLE_COMMANDS: [&str; 3] = ["bootstrap", "plan", "status"];

#[cfg(not(feature = "unstable"))]
#[test]
fn test_unstable_commands_are_unknown_without_the_feature() {
    for command in UNSTABLE_COMMANDS {
        snow_cmd()
            .arg(command)
            .assert()
            .failure()
            .stderr(predicate::str::contains(format!(
                "unrecognized subcommand '{command}'"
            )));
    }
}

#[cfg(not(feature = "unstable"))]
#[test]
fn test_help_leaves_out_unstable_commands() {
    let help = snow_cmd().arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&help.get_output().stdout).into_owned();
    for command in UNSTABLE_COMMANDS {
        assert!(
            !stdout
                .lines()
                .any(|line| line.split_whitespace().next() == Some(command)),
            "--help lists {command}:\n{stdout}"
        );
    }
}

#[test]
fn test_version_flag_prints_the_crate_version() {
    snow_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("snow {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_fatal_error_shows_its_help_without_a_std_prefix() {
    assert_cmd::cargo::cargo_bin_cmd!("snow")
        .args(["-f", "/srv/no-such-flake", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::starts_with(
            "✗ Flake path '/srv/no-such-flake' does not exist\n  help: Provide a valid path",
        ))
        .stderr(predicate::str::contains("Error:").not());
}

#[test]
fn test_cursor_is_left_alone_when_stderr_is_not_a_terminal() {
    assert_cmd::cargo::cargo_bin_cmd!("snow")
        .args(["-f", "/srv/no-such-flake", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("\x1b[?25").not());
}

#[test]
fn test_deploy_help() {
    snow_cmd()
        .arg("deploy")
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
            "List the hosts in nixosConfigurations",
        ));
}

#[cfg(feature = "unstable")]
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
            "Build the NixOS and nix-darwin configurations of one or more hosts",
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
fn test_deploy_filter_flag_in_help() {
    snow_cmd()
        .arg("deploy")
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
fn test_deploy_phase_flag_in_help() {
    snow_cmd()
        .arg("deploy")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Filter by deployment phase"));
}

#[test]
fn test_deploy_redeploy_flag_in_help() {
    snow_cmd()
        .arg("deploy")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--redeploy"));
}

#[test]
fn test_deploy_tags_flag_in_help() {
    snow_cmd()
        .arg("deploy")
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
fn test_deploy_filter_and_tag_mutually_exclusive() {
    snow_cmd()
        .arg("deploy")
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
fn test_deploy_invalid_tag_name() {
    snow_cmd()
        .arg("deploy")
        .arg("--tag")
        .arg("web.server")
        .assert()
        .failure()
        .stderr(predicate::str::contains("only ASCII alphanumeric"));
}

// --mock-fixture only exists in debug builds
#[cfg(debug_assertions)]
#[test]
fn test_mock_push_keeps_stdout_for_the_report() {
    snow_cmd()
        .args(["deploy", "--switch", "--mock-fixture"])
        .arg(fixtures_dir().join("push_scenarios/build_failure.json"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("Using the mock deployer from"))
        .stderr(predicate::str::contains(
            "building '/nix/store/...-nitrogen.drv'",
        ))
        .stderr(predicate::str::contains("copying "))
        .stdout(predicate::str::contains("Pushing 4 hosts"))
        .stdout(predicate::str::contains("copying ").not())
        .stdout(predicate::str::contains("activation").not())
        .stdout(predicate::str::contains("mock deployer").not())
        .stdout(predicate::str::contains("building '/nix/store").not())
        .stdout(predicate::str::contains("Building ").not());
}

#[cfg(debug_assertions)]
#[test]
fn test_deploy_without_a_terminal_logs_each_step_to_stderr() {
    snow_cmd()
        .env("RUST_LOG", "snowlib=info")
        .args(["deploy", "--switch", "--quiet", "--mock-fixture"])
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

#[test]
fn test_list_help_says_it_lists_every_host_by_default() {
    snow_cmd()
        .args(["list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Hosts to list (defaults to every host)",
        ))
        .stdout(predicate::str::contains("--phase"));
}
