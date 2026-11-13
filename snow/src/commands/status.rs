use clap::Args;
use serde::Serialize;
use snowlib::{Flake, Result};
use tabled::{Table, Tabled};

/// Show cluster status
#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Filter hosts using a Nix expression (e.g., "config: config.services.nginx.enable or false")
    #[arg(short = 'e', long)]
    pub filter: Option<String>,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock backend (debug builds only)
    #[arg(long, hide = true)]
    pub mock_fixture: Option<String>,
}

#[derive(Debug, Tabled, Serialize)]
struct HostStatus {
    #[tabled(rename = "Host")]
    name: String,

    #[tabled(rename = "Status")]
    status: String,
}

impl StatusArgs {
    pub async fn run(self, flake: &Flake) -> Result<()> {
        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
        )?;

        let hosts = nix_backend
            .list_all_hosts(flake, self.filter.as_deref())
            .await?;

        let statuses: Vec<HostStatus> = hosts
            .into_iter()
            .map(|host| HostStatus {
                name: host.name,
                status: "unknown".to_string(),
            })
            .collect();

        if self.json {
            let json_output = serde_json::to_string_pretty(&statuses)
                .map_err(|e| snowlib::error::IoError::JsonSerialize { source: e })?;
            println!("{}", json_output);
        } else if statuses.is_empty() {
            println!("No hosts found in flake.nix");
        } else {
            let table = Table::new(&statuses).to_string();
            println!("\nCluster Status\n");
            println!("{}", table);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_utils::snow_cmd;
    use predicates::prelude::*;

    #[test]
    fn test_status_json_all_hosts() {
        let output = snow_cmd()
            .arg("status")
            .arg("--json")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let stdout = String::from_utf8(output).unwrap();
        let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

        // Should be an array with 4 hosts
        assert!(json.is_array());
        let hosts = json.as_array().unwrap();
        assert_eq!(hosts.len(), 4);

        // Check structure
        assert!(hosts[0]["name"].is_string());
        assert!(hosts[0]["status"].is_string());
        assert_eq!(hosts[0]["status"], "unknown");

        // Check all host names are present
        let names: Vec<&str> = hosts.iter().map(|h| h["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"hedgehog"));
        assert!(names.contains(&"nuthatch"));
        assert!(names.contains(&"osprey"));
        assert!(names.contains(&"wren"));
    }

    #[test]
    fn test_status_json_with_filter() {
        let output = snow_cmd()
            .arg("status")
            .arg("--json")
            .arg("-e")
            .arg("config: config.services.postgresql.enable or false")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let stdout = String::from_utf8(output).unwrap();
        let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

        let hosts = json.as_array().unwrap();
        assert_eq!(hosts.len(), 2);

        let names: Vec<&str> = hosts.iter().map(|h| h["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"hedgehog"));
        assert!(names.contains(&"osprey"));
    }

    #[test]
    fn test_status_json_empty() {
        let output = snow_cmd()
            .arg("status")
            .arg("--json")
            .arg("-e")
            .arg("config: false")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let stdout = String::from_utf8(output).unwrap();
        let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

        let hosts = json.as_array().unwrap();
        assert_eq!(hosts.len(), 0);
    }

    #[test]
    fn test_status_table_output() {
        snow_cmd()
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Cluster Status"))
            .stdout(predicate::str::contains("Host"))
            .stdout(predicate::str::contains("Status"))
            .stdout(predicate::str::contains("hedgehog"));
    }

    #[test]
    fn test_status_table_empty_shows_message() {
        snow_cmd()
            .arg("status")
            .arg("-e")
            .arg("config: false")
            .assert()
            .success()
            .stdout(predicate::str::contains("No hosts found"));
    }

    #[test]
    fn test_status_with_invalid_filter_shows_error() {
        snow_cmd()
            .arg("status")
            .arg("-e")
            .arg("config: { invalid syntax")
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Failed to evaluate Nix expression",
            ));
    }
}
