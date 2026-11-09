use crate::host_selection::HostSelectionArgs;
use clap::Args;
use serde::Serialize;
use snowlib::{Flake, HostMetadata, RebootPolicy, Result};
use tabled::settings::{Padding, Style};
use tabled::{Table, Tabled};

/// List the hosts in nixosConfigurations and darwinConfigurations, with their phase and tags
#[derive(Debug, Args)]
#[command(mut_arg("hosts", |hosts| hosts.help("Hosts to list (defaults to every host)")))]
pub struct ListArgs {
    #[command(flatten)]
    pub selection: HostSelectionArgs,

    /// Print the hosts and their deploy settings as a JSON array
    #[arg(long)]
    pub json: bool,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock backend (debug builds only)
    #[arg(long, hide = true)]
    pub mock_fixture: Option<String>,
}

impl ListArgs {
    pub async fn run(self, flake: &Flake) -> Result<()> {
        let nix_backend = crate::backend_factory::create_hookless_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
        )?;

        let hostnames = self
            .selection
            .resolve_for_listing(flake, nix_backend.as_ref())
            .await?;
        let hosts = nix_backend.load_host_metadata(flake, &hostnames).await?;

        if self.json {
            println!("{}", to_json(&hosts)?);
        } else if !hosts.is_empty() {
            println!("{}", table(&hosts));
        }

        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListedHost<'a> {
    name: &'a str,
    system: Option<&'a str>,
    phase: Option<&'a str>,
    tags: &'a [String],
    reboot_policy: &'a RebootPolicy,
    optional: bool,
    serial_group: Option<&'a str>,
}

fn to_json(hosts: &[HostMetadata]) -> Result<String> {
    let listed: Vec<ListedHost> = hosts
        .iter()
        .map(|host| ListedHost {
            name: &host.name,
            system: host.system.as_deref(),
            phase: host.phase.as_deref(),
            tags: &host.tags,
            reboot_policy: &host.reboot_policy,
            optional: host.optional,
            serial_group: host.serial_group.as_deref(),
        })
        .collect();
    serde_json::to_string_pretty(&listed)
        .map_err(|source| snowlib::error::IoError::JsonSerialize { source }.into())
}

#[derive(Tabled)]
#[tabled(rename_all = "UPPERCASE")]
struct HostRow<'a> {
    host: &'a str,
    phase: &'a str,
    tags: String,
}

/// Empty cells show as "-", so each row keeps one word per column
fn table(hosts: &[HostMetadata]) -> String {
    let rows = hosts.iter().map(|host| HostRow {
        host: &host.name,
        phase: host.phase.as_deref().unwrap_or("-"),
        tags: if host.tags.is_empty() {
            "-".to_string()
        } else {
            host.tags.join(",")
        },
    });
    Table::new(rows)
        .with(Style::blank())
        .with(Padding::new(0, 2, 0, 0))
        .to_string()
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{fixtures_dir, snow_cmd, snow_cmd_on};
    use predicates::prelude::*;
    use snowlib::SystemType;

    fn host(name: &str) -> HostMetadata {
        HostMetadata {
            name: name.to_string(),
            phase: None,
            tags: vec![],
            fqdn: format!("{name}.example.com"),
            reboot_policy: RebootPolicy::Auto,
            system_type: SystemType::NixOS,
            system: None,
            hooks: vec![],
            serial_group: None,
            ssh_port: None,
            optional: false,
            ignored_failed_units: Vec::new(),
        }
    }

    fn json_value(hosts: &[HostMetadata]) -> serde_json::Value {
        serde_json::from_str(&to_json(hosts).unwrap()).unwrap()
    }

    #[test]
    fn test_json_has_each_hosts_deploy_settings() {
        let argon = HostMetadata {
            system: Some("aarch64-linux".to_string()),
            phase: Some("p1".to_string()),
            tags: vec!["web".to_string(), "dns".to_string()],
            reboot_policy: RebootPolicy::Never,
            optional: true,
            serial_group: Some("dns".to_string()),
            ..host("argon")
        };

        assert_eq!(
            json_value(&[argon]),
            serde_json::json!([{
                "name": "argon",
                "system": "aarch64-linux",
                "phase": "p1",
                "tags": ["web", "dns"],
                "rebootPolicy": "never",
                "optional": true,
                "serialGroup": "dns",
            }])
        );
    }

    #[test]
    fn test_json_writes_unset_settings_as_null() {
        assert_eq!(
            json_value(&[host("boron")]),
            serde_json::json!([{
                "name": "boron",
                "system": null,
                "phase": null,
                "tags": [],
                "rebootPolicy": "auto",
                "optional": false,
                "serialGroup": null,
            }])
        );
    }

    #[test]
    fn test_json_without_hosts_is_an_empty_array() {
        assert_eq!(to_json(&[]).unwrap(), "[]");
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
        let hosts: Vec<&str> = stdout
            .lines()
            .skip(1)
            .filter_map(|row| row.split_whitespace().next())
            .collect();
        assert_eq!(hosts, ["hydrogen", "oxygen"]);
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

    /// snow list on a flake whose hosts set every deploy option
    fn list_cmd(args: &[&str]) -> assert_cmd::Command {
        let mut cmd = snow_cmd_on(&fixtures_dir().join("list"));
        cmd.arg("list").args(args);
        cmd
    }

    fn list_stdout(args: &[&str]) -> String {
        let output = list_cmd(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(output).unwrap()
    }

    /// The host column of the table, without its header
    fn listed_hosts(args: &[&str]) -> Vec<String> {
        list_stdout(args)
            .lines()
            .skip(1)
            .map(|row| row.split_whitespace().next().unwrap().to_string())
            .collect()
    }

    #[test]
    fn test_list_shows_each_hosts_phase_and_tags() {
        assert_eq!(
            list_stdout(&[]),
            "HOST     PHASE   TAGS\n\
             argon    p1      web,dns\n\
             boron    p2      web\n\
             curium   never   web\n\
             ferrum   -       -\n"
        );
    }

    #[test]
    fn test_list_without_matches_prints_nothing() {
        assert_eq!(list_stdout(&["-e", "c: false"]), "");
    }

    #[test]
    fn test_list_without_a_selection_shows_every_host_even_in_the_never_phase() {
        assert_eq!(listed_hosts(&[]), ["argon", "boron", "curium", "ferrum"]);
    }

    #[test]
    fn test_list_selections_leave_out_the_never_phase_like_deploy() {
        assert_eq!(listed_hosts(&["--tag", "web"]), ["argon", "boron"]);
        assert_eq!(
            listed_hosts(&["-e", "c: true"]),
            ["argon", "boron", "ferrum"]
        );
    }

    #[test]
    fn test_list_selects_the_never_phase_by_phase_or_name_like_deploy() {
        assert_eq!(listed_hosts(&["--phases", "p2,never"]), ["boron", "curium"]);
        assert_eq!(listed_hosts(&["ferrum", "curium"]), ["ferrum", "curium"]);
    }

    #[test]
    fn test_list_rejects_unknown_hosts() {
        list_cmd(&["xenon"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Unknown host: xenon"));
    }

    fn listed_json(args: &[&str]) -> serde_json::Value {
        let args: Vec<&str> = std::iter::once("--json")
            .chain(args.iter().copied())
            .collect();
        serde_json::from_str(&list_stdout(&args)).unwrap()
    }

    #[test]
    fn test_list_json_shows_each_hosts_deploy_settings() {
        assert_eq!(
            listed_json(&[]),
            serde_json::json!([
                {
                    "name": "argon",
                    "system": "x86_64-linux",
                    "phase": "p1",
                    "tags": ["web", "dns"],
                    "rebootPolicy": "never",
                    "optional": false,
                    "serialGroup": "dns",
                },
                {
                    "name": "boron",
                    "system": "aarch64-linux",
                    "phase": "p2",
                    "tags": ["web"],
                    "rebootPolicy": "auto",
                    "optional": true,
                    "serialGroup": null,
                },
                {
                    "name": "curium",
                    "system": "x86_64-linux",
                    "phase": "never",
                    "tags": ["web"],
                    "rebootPolicy": "auto",
                    "optional": false,
                    "serialGroup": null,
                },
                {
                    "name": "ferrum",
                    "system": "aarch64-darwin",
                    "phase": null,
                    "tags": [],
                    "rebootPolicy": "auto",
                    "optional": false,
                    "serialGroup": null,
                },
            ])
        );
    }

    #[test]
    fn test_list_json_lists_only_the_selected_hosts() {
        let names = |args: &[&str]| -> Vec<String> {
            listed_json(args)
                .as_array()
                .unwrap()
                .iter()
                .map(|host| host["name"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(names(&["--tag", "web"]), ["argon", "boron"]);
        assert_eq!(names(&["--phase", "never"]), ["curium"]);
    }

    #[test]
    fn test_list_json_without_matches_is_an_empty_array() {
        assert_eq!(listed_json(&["-e", "c: false"]), serde_json::json!([]));
    }
}
