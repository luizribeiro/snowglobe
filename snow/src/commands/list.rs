use crate::error::Result;
use crate::nix;
use clap::Args;

/// List hostnames from nixosConfigurations
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Filter hosts using a Nix expression (e.g., "config: config.services.nginx.enable or false")
    #[arg(short = 'e', long)]
    pub filter: Option<String>,
}

impl ListArgs {
    pub fn run(self, ctx: &nix::SnowContext) -> Result<()> {
        let hosts = nix::list_hosts(ctx, self.filter.as_deref())?;

        for host in hosts {
            println!("{}", host.name);
        }

        Ok(())
    }
}
