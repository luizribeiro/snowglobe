//! Backend implementations for Snowglobe operations
//!
//! This module provides pluggable backend traits for core operations:
//! - `NixBackend`: Nix evaluation and building
//! - `DeployBackend`: Deployment and activation
//! - `RebootBackend`: Reboot detection and execution
//! - `HookBackend`: Lifecycle hook execution
//! - `KeyGenBackend`: SSH key generation and age key derivation
//! - `Provisioner`: Image key injection
//! - `SopsBackend`: SOPS secrets management integration
//!
//! Each backend has both production (system) and mock implementations for testing.

pub mod deploy;
pub mod hook;
pub mod keygen;
pub mod nix;
pub mod provisioner;
pub mod reboot;
pub mod sops;
