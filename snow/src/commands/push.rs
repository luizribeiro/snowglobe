use clap::Args;
use snowlib::{
    build_host, get_host_metadata, push_parallel, ActivationMode, DeployTarget, PushOptions,
    Result, SnowContext, SshDeployer,
};
use tabled::{Table, Tabled};

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
    pub fn run(self, ctx: &SnowContext) -> Result<()> {
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
        // TODO: Build all hosts in parallel with single `nix build` command for better performance
        let mut host_data = Vec::new();

        for host in &self.hosts {
            let metadata = get_host_metadata(ctx, host)?;

            let store_path = build_host(ctx, host)?;

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
            concurrency: self.concurrency,
        };

        let results = push_parallel(&deployer, host_data, &opts)?;

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
            return Err(snowlib::error::DeployError::Failed {
                host: "multiple".to_string(),
                source: std::io::Error::other("Some deployments failed"),
            }
            .into());
        }

        Ok(())
    }
}
