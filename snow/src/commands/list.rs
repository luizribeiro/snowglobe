use clap::Args;
use snowlib::{Result, SnowContext};

/// List hostnames from nixosConfigurations
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Filter hosts using a Nix expression (e.g., "config: config.services.nginx.enable or false")
    #[arg(short = 'e', long)]
    pub filter: Option<String>,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock backend (debug builds only)
    #[arg(long, hide = true)]
    pub mock_fixture: Option<String>,
}

impl ListArgs {
    pub fn run(self, ctx: &SnowContext) -> Result<()> {
        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
        )?;

        let hosts = nix_backend.list_hosts(ctx, self.filter.as_deref())?;

        for host in hosts {
            println!("{}", host.name);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_utils::snow_cmd;
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
}
