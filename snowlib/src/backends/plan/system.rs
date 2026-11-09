use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;

use super::PlanBackend;
use crate::error::Result;
use crate::nix::{Flake, NixCommand, SystemType};
use crate::plan::{
    parse_derivation_show, parse_eval_jobs, parse_path_info, parse_substituters,
    ConfiguredSubstituter, DerivationInfo, HostEval, NarInfo,
};

// An unreachable cache makes nix retry with backoff for a while before giving up.
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

// Keeps argument lists far below ARG_MAX (1 MiB on macOS) for fleets with
// tens of thousands of store paths.
const PATHS_PER_COMMAND: usize = 500;

/// Plans with `nix-eval-jobs` and the local nix daemon, i.e. against the store
/// and substituters a build on this machine would use.
#[derive(Debug, Clone)]
pub struct SystemPlanBackend {
    eval_workers: u32,
    eval_max_memory_mib: u32,
}

impl SystemPlanBackend {
    pub fn new(eval_workers: u32, eval_max_memory_mib: u32) -> Self {
        Self {
            eval_workers,
            eval_max_memory_mib,
        }
    }
}

/// Nix function selecting the toplevel of each named host from a configurations attrset.
///
/// Host names are flake attribute names validated during host selection, so
/// JSON quoting yields valid Nix strings (they cannot contain `${`).
fn select_toplevels(system_type: SystemType, hosts: &[String]) -> String {
    let names: Vec<String> = hosts
        .iter()
        .map(|host| serde_json::to_string(host).expect("strings serialize"))
        .collect();
    format!(
        "configs: builtins.mapAttrs (_: host: host.{}) (builtins.intersectAttrs \
         (builtins.listToAttrs (map (name: {{ inherit name; value = null; }}) [ {} ])) configs)",
        system_type.build_path_suffix(),
        names.join(" ")
    )
}

fn chunked_args<'a>(
    command: &'a [&str],
    paths: &'a [String],
) -> impl Iterator<Item = Vec<String>> + 'a {
    paths.chunks(PATHS_PER_COMMAND).map(move |chunk| {
        command
            .iter()
            .map(|arg| arg.to_string())
            .chain(chunk.iter().cloned())
            .collect()
    })
}

async fn run_nix_chunked<T>(
    command: &[&str],
    paths: &[String],
    parse: impl Fn(&[u8]) -> Result<T>,
) -> Result<Vec<T>> {
    let mut results = Vec::new();
    for args in chunked_args(command, paths) {
        let stdout = NixCommand::tool("nix", args).run().await?;
        results.push(parse(&stdout)?);
    }
    Ok(results)
}

#[async_trait]
impl PlanBackend for SystemPlanBackend {
    async fn evaluate(
        &self,
        flake: &Flake,
        system_type: SystemType,
        hosts: &[String],
    ) -> Result<Vec<HostEval>> {
        let stdout = NixCommand::tool(
            "nix-eval-jobs",
            [
                "--flake".to_string(),
                format!(".#{}", system_type.flake_attr()),
                "--select".to_string(),
                select_toplevels(system_type, hosts),
                "--check-cache-status".to_string(),
                "--workers".to_string(),
                self.eval_workers.to_string(),
                "--max-memory-size".to_string(),
                self.eval_max_memory_mib.to_string(),
            ],
        )
        .in_flake(flake)
        .run()
        .await?;
        parse_eval_jobs(&stdout)
    }

    async fn show_derivations(&self, drv_paths: &[String]) -> Result<Vec<DerivationInfo>> {
        let chunks =
            run_nix_chunked(&["derivation", "show"], drv_paths, parse_derivation_show).await?;
        Ok(chunks.into_iter().flatten().collect())
    }

    async fn substituters(&self) -> Result<Vec<ConfiguredSubstituter>> {
        let stdout = NixCommand::tool("nix", ["config", "show", "--json"])
            .run()
            .await?;
        parse_substituters(&stdout)
    }

    async fn is_reachable(&self, substituter: &ConfiguredSubstituter) -> bool {
        NixCommand::tool(
            "nix",
            ["store", "info", "--json", "--store", &substituter.store_url],
        )
        .timeout(PROBE_TIMEOUT)
        .run()
        .await
        .inspect_err(|e| {
            tracing::warn!(
                "Substituter {} unreachable or probe failed: {}",
                substituter.url,
                e
            )
        })
        .is_ok()
    }

    async fn query_substituter(
        &self,
        substituter: &ConfiguredSubstituter,
        paths: &[String],
    ) -> Result<HashMap<String, NarInfo>> {
        let command = ["path-info", "--json", "--store", &substituter.store_url];
        let chunks = run_nix_chunked(&command, paths, parse_path_info).await?;
        Ok(chunks.into_iter().flatten().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_toplevels_quotes_host_names() {
        let hosts = vec!["raven".to_string(), "limestone".to_string()];

        assert_eq!(
            select_toplevels(SystemType::NixOS, &hosts),
            "configs: builtins.mapAttrs (_: host: host.config.system.build.toplevel) \
             (builtins.intersectAttrs (builtins.listToAttrs (map (name: { inherit name; \
             value = null; }) [ \"raven\" \"limestone\" ])) configs)"
        );
    }

    #[test]
    fn test_select_toplevels_for_darwin_uses_configurations_toplevel() {
        let select = select_toplevels(SystemType::Darwin, &["falcon".to_string()]);

        assert!(select.contains("host.config.system.build.toplevel"));
        assert!(select.contains(r#"[ "falcon" ]"#));
    }

    #[test]
    fn test_chunked_args_split_paths_and_repeat_the_command() {
        let paths: Vec<String> = (0..1_200)
            .map(|i| format!("/nix/store/{i:032}-glibc-2.40-66"))
            .collect();
        let command = ["path-info", "--json", "--store", "s3://nix-store-cache"];

        let chunks: Vec<Vec<String>> = chunked_args(&command, &paths).collect();

        let sizes: Vec<usize> = chunks
            .iter()
            .map(|args| args.len() - command.len())
            .collect();
        assert_eq!(sizes, vec![500, 500, 200]);
        assert!(chunks.iter().all(|args| args[..4] == command));
        assert_eq!(chunks[2].last(), paths.last());
    }

    #[test]
    fn test_chunked_args_without_paths_runs_nothing() {
        assert_eq!(chunked_args(&["derivation", "show"], &[]).count(), 0);
    }

    #[tokio::test]
    #[ignore] // Requires nix-eval-jobs and network access to cache.nixos.org
    async fn test_evaluate_fixture_flake_host() {
        // nix-eval-jobs 2.34.3 hangs on a flake in a subdirectory of a dirty git
        // checkout, so evaluate a copy outside the repository.
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/flake.nix"),
            dir.path().join("flake.nix"),
        )
        .unwrap();
        let flake = Flake::new(dir.path().to_str()).unwrap();
        let backend = SystemPlanBackend::new(1, 2048);

        let evals = backend
            .evaluate(&flake, SystemType::NixOS, &["hedgehog".to_string()])
            .await
            .unwrap();

        assert_eq!(evals.len(), 1);
        assert_eq!(evals[0].host, "hedgehog");
        let hedgehog = evals[0].evaluated().unwrap();
        assert!(hedgehog.drv_path.ends_with("-test-hedgehog-toplevel.drv"));
        assert_eq!(hedgehog.cache_status, crate::plan::CacheStatus::NotBuilt);
    }
}
