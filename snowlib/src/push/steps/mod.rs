//! Individual deployment steps
//!
//! Each module implements a specific step in the deployment pipeline:
//! - `build`: Builds NixOS configurations
//! - `copy`: Copies closures to remote hosts
//! - `plan`: Creates deployment plans with reboot detection
//! - `activate`: Activates configurations on remote hosts
//! - `reboot`: Reboots hosts that require it

pub mod activate;
pub mod build;
pub mod copy;
pub mod plan;
pub mod reboot;

// Re-export step implementations
pub use activate::ActivateStep;
pub use build::BuildStep;
pub use copy::CopyStep;
pub use plan::PlanningStep;
pub use reboot::RebootStep;

// Re-export input/output types
pub use activate::{ActivateInput, ActivateOutput};
pub use build::{BuildInput, BuildOutput};
pub use copy::{CopyInput, CopyOutput};
pub use plan::{PlanningInput, PlanningOutput};
pub use reboot::{RebootInput, RebootOutput};
