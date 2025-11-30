//! Bootstrap module for host provisioning
//!
//! This module provides functionality for bootstrapping new NixOS hosts,
//! including SSH key generation and age key derivation for secrets management.

mod config;
mod keygen;

pub use config::BootstrapConfig;
pub(crate) use config::BootstrapConfigJson;
pub use keygen::{bootstrap_host, HostKeys};

#[cfg(test)]
mod tests;
