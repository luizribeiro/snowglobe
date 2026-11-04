use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use async_trait::async_trait;

use super::PlanBackend;
use crate::error::{NixError, Result};
use crate::nix::{Flake, SystemType};
use crate::plan::{ConfiguredSubstituter, DerivationInfo, HostEval, NarInfo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockPlanCall {
    ShowDerivations(Vec<String>),
    QuerySubstituter { url: String, paths: Vec<String> },
}

/// In-memory PlanBackend: evaluations, derivations and substituter contents are set up front.
#[derive(Debug, Clone, Default)]
pub struct MockPlanBackend {
    pub calls: Arc<Mutex<Vec<MockPlanCall>>>,
    pub evals: HashMap<SystemType, Vec<HostEval>>,
    pub failing_eval: Option<SystemType>,
    pub derivations: Vec<DerivationInfo>,
    pub substituters: Vec<ConfiguredSubstituter>,
    pub unreachable: Vec<String>,
    pub failing_queries: Vec<String>,
    pub substituter_paths: HashMap<String, HashMap<String, NarInfo>>,
}

#[async_trait]
impl PlanBackend for MockPlanBackend {
    async fn evaluate(
        &self,
        _flake: &Flake,
        system_type: SystemType,
        hosts: &[String],
    ) -> Result<Vec<HostEval>> {
        if self.failing_eval == Some(system_type) {
            return Err(NixError::EvalFailed {
                stderr: "error: flake 'path:/srv/fleet' does not provide attribute".to_string(),
            }
            .into());
        }
        Ok(self
            .evals
            .get(&system_type)
            .into_iter()
            .flatten()
            .filter(|eval| hosts.contains(&eval.host))
            .cloned()
            .collect())
    }

    async fn show_derivations(&self, drv_paths: &[String]) -> Result<Vec<DerivationInfo>> {
        self.calls
            .lock()
            .push(MockPlanCall::ShowDerivations(drv_paths.to_vec()));
        Ok(self
            .derivations
            .iter()
            .filter(|info| drv_paths.contains(&info.drv_path))
            .cloned()
            .collect())
    }

    async fn substituters(&self) -> Result<Vec<ConfiguredSubstituter>> {
        Ok(self.substituters.clone())
    }

    async fn is_reachable(&self, substituter: &ConfiguredSubstituter) -> bool {
        !self.unreachable.contains(&substituter.url)
    }

    async fn query_substituter(
        &self,
        substituter: &ConfiguredSubstituter,
        paths: &[String],
    ) -> Result<HashMap<String, NarInfo>> {
        self.calls.lock().push(MockPlanCall::QuerySubstituter {
            url: substituter.url.clone(),
            paths: paths.to_vec(),
        });
        if self.failing_queries.contains(&substituter.url) {
            return Err(NixError::CommandFailed {
                source: std::io::Error::other("mock substituter query failure"),
            }
            .into());
        }
        Ok(self
            .substituter_paths
            .get(&substituter.url)
            .into_iter()
            .flatten()
            .filter(|(path, _)| paths.contains(path))
            .map(|(path, info)| (path.clone(), *info))
            .collect())
    }
}
