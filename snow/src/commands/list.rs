use crate::host_selection::HostSelectionArgs;
use clap::Args;
use snowlib::{Flake, Result};

/// List hostnames from nixosConfigurations and darwinConfigurations
#[derive(Debug, Args)]
#[command(mut_arg("hosts", |hosts| hosts.help("Hosts to list (defaults to every host)")))]
pub struct ListArgs {
    #[command(flatten)]
    pub selection: HostSelectionArgs,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock backend (debug builds only)
    #[arg(long, hide = true)]
    pub mock_fixture: Option<String>,
}

impl ListArgs {
    pub async fn run(self, flake: &Flake) -> Result<()> {
        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
        )?;

        let hosts = self
            .selection
            .resolve_for_listing(flake, nix_backend.as_ref())
            .await?;

        for host in hosts {
            println!("{host}");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_utils::{fixtures_dir, snow_cmd, snow_cmd_on};
    use predicates::prelude::*;

    #[test]
    fn test_list_outputs_one_per_line() {
        let output = snow_cmd()
            .arg("list")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let stdout = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = stdout.lines().collect();

        // Should output exactly 4 lines, one per host
        assert_eq!(lines.len(), 4);
        assert!(lines.contains(&"hydrogen"));
        assert!(lines.contains(&"nitrogen"));
        assert!(lines.contains(&"oxygen"));
        assert!(lines.contains(&"helium"));
    }

    #[test]
    fn test_list_with_postgresql_filter() {
        let output = snow_cmd()
            .arg("list")
            .arg("-e")
            .arg("config: config.services.postgresql.enable or false")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let stdout = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = stdout.lines().collect();

        // Should output exactly 2 lines (hydrogen and oxygen)
        assert_eq!(lines.len(), 2);
        assert!(lines.contains(&"hydrogen"));
        assert!(lines.contains(&"oxygen"));
    }

    #[test]
    fn test_list_with_no_matches_outputs_nothing() {
        snow_cmd()
            .arg("list")
            .arg("-e")
            .arg("config: false")
            .assert()
            .success()
            .stdout(predicate::str::is_empty());
    }

    #[test]
    fn test_list_with_invalid_filter_shows_error() {
        snow_cmd()
            .arg("list")
            .arg("-e")
            .arg("config: { invalid syntax")
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Failed to evaluate Nix expression",
            ));
    }

    /// snow list on a flake whose hosts set every push option
    fn list_cmd(args: &[&str]) -> assert_cmd::Command {
        let mut cmd = snow_cmd_on(&fixtures_dir().join("list"));
        cmd.arg("list").args(args);
        cmd
    }

    fn listed_lines(args: &[&str]) -> Vec<String> {
        let output = list_cmd(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn test_list_without_a_selection_shows_every_host_even_in_the_never_phase() {
        assert_eq!(listed_lines(&[]), ["argon", "boron", "curium", "ferrum"]);
    }

    #[test]
    fn test_list_selections_leave_out_the_never_phase_like_push() {
        assert_eq!(listed_lines(&["--tag", "web"]), ["argon", "boron"]);
        assert_eq!(
            listed_lines(&["-e", "c: true"]),
            ["argon", "boron", "ferrum"]
        );
    }

    #[test]
    fn test_list_selects_the_never_phase_by_phase_or_name_like_push() {
        assert_eq!(listed_lines(&["--phases", "p2,never"]), ["boron", "curium"]);
        assert_eq!(listed_lines(&["ferrum", "curium"]), ["ferrum", "curium"]);
    }

    #[test]
    fn test_list_rejects_unknown_hosts() {
        list_cmd(&["xenon"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Unknown host: xenon"));
    }
}
