use clap::Args;
use serde::Serialize;
use snowlib::{list_hosts, Result, SnowContext};
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
}

#[derive(Debug, Tabled, Serialize)]
struct HostStatus {
    #[tabled(rename = "Host")]
    name: String,

    #[tabled(rename = "Status")]
    status: String,
}

impl StatusArgs {
    pub fn run(self, ctx: &SnowContext) -> Result<()> {
        let hosts = list_hosts(ctx, self.filter.as_deref())?;

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
