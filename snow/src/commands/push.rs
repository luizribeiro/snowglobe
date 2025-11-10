use crate::error::Result;
use tabled::{Table, Tabled};

use crate::deploy::{ActivationMode, ssh::SshDeployer};
use crate::nix::{self, DeployTarget};
use crate::push::{self, PushOptions};

#[derive(Debug, Tabled)]
struct DeploymentResult {
    #[tabled(rename = "Host")]
    host: String,

    #[tabled(rename = "Status")]
    status: String,

    #[tabled(rename = "Message")]
    message: String,
}

pub fn run(
    hosts: Vec<String>,
    dry_run: bool,
    boot: bool,
    _concurrency: usize,
) -> Result<()> {
    // Determine activation mode
    let activation_mode = if dry_run {
        ActivationMode::DryActivate
    } else if boot {
        ActivationMode::Boot
    } else {
        ActivationMode::Switch
    };

    println!("→ Building configurations...");

    // Build all hosts and gather metadata
    let mut host_data = Vec::new();

    for host in &hosts {
        let metadata = nix::get_host_metadata(host)?;

        let store_path = nix::build_host(host)?;

        let target = DeployTarget {
            user: "root".to_string(),
            hostname: metadata.fqdn.clone(),
            port: None,
        };

        host_data.push((metadata, target, store_path));
    }

    println!("✓ Built {} configuration(s)", hosts.len());

    // Deploy
    println!("\n→ Deploying...");

    let deployer = SshDeployer::new();
    let opts = PushOptions {
        activation_mode,
        concurrency: 3, // TODO: use from args
    };

    let results = push::push_sequential(&deployer, host_data, &opts)?;

    // Format results
    let table_data: Vec<DeploymentResult> = results
        .results
        .iter()
        .map(|(host, result)| DeploymentResult {
            host: host.clone(),
            status: if result.success { "✓" } else { "✗" }.to_string(),
            message: result.message.clone(),
        })
        .collect();

    println!("\nPush Results\n");
    let table = Table::new(&table_data).to_string();
    println!("{}", table);

    // Summary
    println!(
        "\n{}/{} hosts deployed successfully",
        results.success_count(),
        results.results.len()
    );

    if !results.all_succeeded() {
        return Err(crate::error::DeployError::Failed {
            host: "multiple".to_string(),
            source: std::io::Error::new(std::io::ErrorKind::Other, "Some deployments failed"),
        }
        .into());
    }

    Ok(())
}
