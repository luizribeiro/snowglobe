#[cfg(any(debug_assertions, test, feature = "test-utils"))]
mod mock;

#[cfg(any(debug_assertions, test, feature = "test-utils"))]
pub use mock::{MockPlanBackend, MockPlanCall};

use std::collections::HashMap;

use async_trait::async_trait;

use crate::error::Result;
use crate::nix::{Flake, SystemType};
use crate::plan::{ConfiguredSubstituter, DerivationInfo, HostEval, NarInfo};

/// Nix queries needed to plan a build without building anything.
#[async_trait]
pub trait PlanBackend: Send + Sync {
    /// Evaluate each host's toplevel and report what it needs built or fetched.
    ///
    /// Hosts that fail to evaluate are returned as failures, not errors.
    async fn evaluate(
        &self,
        flake: &Flake,
        system_type: SystemType,
        hosts: &[String],
    ) -> Result<Vec<HostEval>>;

    async fn show_derivations(&self, drv_paths: &[String]) -> Result<Vec<DerivationInfo>>;

    async fn substituters(&self) -> Result<Vec<ConfiguredSubstituter>>;

    async fn is_reachable(&self, substituter: &ConfiguredSubstituter) -> bool;

    /// Sizes for the paths the substituter has; paths it lacks are omitted.
    async fn query_substituter(
        &self,
        substituter: &ConfiguredSubstituter,
        paths: &[String],
    ) -> Result<HashMap<String, NarInfo>>;
}
