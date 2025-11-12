//! Backend implementations for Snowglobe operations
//!
//! This module provides pluggable backend traits for core operations:
//! - `NixBackend`: Nix evaluation and building
//! - `DeployBackend`: Deployment and activation
//! - `RebootBackend`: Reboot detection and execution
//!
//! Each backend has both production (system) and mock implementations for testing.

pub mod deploy;
pub mod nix;
pub mod reboot;

#[cfg(debug_assertions)]
pub mod mock;
