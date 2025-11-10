use anyhow::Result;
use crate::nix;

pub fn run(filter: Option<String>) -> Result<()> {
    let hosts = nix::list_hosts(filter.as_deref())?;

    for host in hosts {
        println!("{}", host.name);
    }

    Ok(())
}
