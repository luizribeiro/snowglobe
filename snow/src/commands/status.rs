use crate::error::{IoError, Result};
use serde::Serialize;
use tabled::{Table, Tabled};

use crate::nix;

#[derive(Debug, Tabled, Serialize)]
struct HostStatus {
    #[tabled(rename = "Host")]
    name: String,

    #[tabled(rename = "Status")]
    status: String,
}

pub fn run(json: bool, filter: Option<String>) -> Result<()> {
    let hosts = nix::list_hosts(filter.as_deref())?;

    let statuses: Vec<HostStatus> = hosts
        .into_iter()
        .map(|host| HostStatus {
            name: host.name,
            status: "unknown".to_string(),
        })
        .collect();

    if json {
        let json_output = serde_json::to_string_pretty(&statuses)
            .map_err(|e| IoError::JsonSerialize { source: e })?;
        println!("{}", json_output);
    } else {
        if statuses.is_empty() {
            println!("No hosts found in flake.nix");
        } else {
            let table = Table::new(&statuses).to_string();
            println!("\nCluster Status\n");
            println!("{}", table);
        }
    }

    Ok(())
}
