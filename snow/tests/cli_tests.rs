use assert_cmd::Command;
use predicates::prelude::*;

fn snow_cmd() -> Command {
    Command::cargo_bin("snow").unwrap()
}

#[test]
fn test_help_command() {
    snow_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("NixOS cluster orchestration tool"))
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
