use snowlib::{Result, SnowContext, list_hosts};
use clap::Args;

/// List hostnames from nixosConfigurations
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Filter hosts using a Nix expression (e.g., "config: config.services.nginx.enable or false")
    #[arg(short = 'e', long)]
    pub filter: Option<String>,
}

impl ListArgs {
    pub fn run(self, ctx: &SnowContext) -> Result<()> {
        let hosts = list_hosts(ctx, self.filter.as_deref())?;

        for host in hosts {
            println!("{}", host.name);
        }

        Ok(())
    }
}
