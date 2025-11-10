use crate::error::Result;
use clap::Args;
use tabled::{Table, Tabled};

use crate::deploy::{ActivationMode, ssh::SshDeployer};
use crate::nix::{self, DeployTarget};
use crate::push::{self, PushOptions};

/// Deploy configurations to hosts
#[derive(Debug, Args)]
pub struct PushArgs {
    /// Hosts to push (one or more)
    pub hosts: Vec<String>,

    /// Dry run - show what would change without activating
    #[arg(long)]
    pub dry_run: bool,

    /// Only set for next boot (don't activate now)
    #[arg(long)]
    pub boot: bool,

    /// Maximum number of hosts to push in parallel
    #[arg(long, default_value = "3")]
    pub concurrency: usize,
}

#[derive(Debug, Tabled)]
struct DeploymentResult {
    #[tabled(rename = "Host")]
    host: String,

    #[tabled(rename = "Status")]
    status: String,

    #[tabled(rename = "Message")]
    message: String,
}

impl PushArgs {
    pub fn run(self) -> Result<()> {
        // Determine activation mode
        let activation_mode = if self.dry_run {
            ActivationMode::DryActivate
        } else if self.boot {
            ActivationMode::Boot
    } else {
        ActivationMode::Switch
    };

        println!("→ Building configurations...");

        // Build all hosts and gather metadata
        let mut host_data = Vec::new();

        for host in &self.hosts {
        let metadata = nix::get_host_metadata(host)?;

        let store_path = nix::build_host(host)?;

        let target = DeployTarget {
            user: "root".to_string(),
            hostname: metadata.fqdn.clone(),
            port: None,
        };

        host_data.push((metadata, target, store_path));
    }

        println!("✓ Built {} configuration(s)", self.hosts.len());

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
}
