use crate::error::Result;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::deploy::{ActivationMode, DeployResult, Deployer};
use crate::nix::{DeployTarget, HostMetadata, StorePath};

/// Options for push operations
#[derive(Debug, Clone)]
pub struct PushOptions {
    pub activation_mode: ActivationMode,
    pub concurrency: usize,
}

impl Default for PushOptions {
    fn default() -> Self {
        Self {
            activation_mode: ActivationMode::Switch,
            concurrency: 3,
        }
    }
}

/// Result of pushing multiple hosts
#[derive(Debug)]
pub struct PushResults {
    pub results: Vec<(String, DeployResult)>, // (hostname, result)
}

impl PushResults {
    pub fn success_count(&self) -> usize {
        self.results.iter().filter(|(_, r)| r.success).count()
    }

    pub fn failure_count(&self) -> usize {
        self.results.iter().filter(|(_, r)| !r.success).count()
    }

    pub fn all_succeeded(&self) -> bool {
        self.results.iter().all(|(_, r)| r.success)
    }
}

/// Push hosts in parallel with concurrency limit
///
/// When concurrency is 1, this effectively runs sequentially.
pub fn push_parallel(
    deployer: &dyn Deployer,
    hosts: Vec<(HostMetadata, DeployTarget, StorePath)>,
    opts: &PushOptions,
) -> Result<PushResults> {
    // Wrap deployer in Arc to share across threads
    // Note: This requires the Deployer trait to be Send + Sync
    let deployer = Arc::new(deployer);

    // Configure rayon to respect concurrency limit
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.concurrency)
        .build()
        .map_err(|e| {
            crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                host: "thread pool".to_string(),
                source: std::io::Error::other(e.to_string()),
            })
        })?;

    let results: Vec<(String, DeployResult)> = pool.install(|| {
        hosts
            .into_par_iter()
            .map(|(metadata, target, store_path)| {
                let deployer = Arc::clone(&deployer);
                let result = deployer
                    .deploy(&target, &store_path, opts.activation_mode)
                    .unwrap_or_else(|e| {
                        // Format the full error chain to include source errors
                        let mut error_msg = e.to_string();
                        let mut source = std::error::Error::source(&e);
                        while let Some(err) = source {
                            error_msg.push_str(&format!("\n  Caused by: {}", err));
                            source = std::error::Error::source(err);
                        }
                        DeployResult::failure(error_msg)
                    });

                (metadata.name, result)
            })
            .collect()
    });

    Ok(PushResults { results })
}

/// Push hosts grouped by phase, sequential phases but parallel within each phase
pub fn push_by_phase(
    deployer: &dyn Deployer,
    hosts: Vec<(HostMetadata, DeployTarget, StorePath)>,
    opts: &PushOptions,
) -> Result<PushResults> {
    // Group hosts by phase
    let mut by_phase: HashMap<String, Vec<(HostMetadata, DeployTarget, StorePath)>> =
        HashMap::new();

    for (metadata, target, store_path) in hosts {
        let phase = metadata
            .phase
            .clone()
            .unwrap_or_else(|| "default".to_string());
        by_phase
            .entry(phase)
            .or_default()
            .push((metadata, target, store_path));
    }

    // Sort phases lexicographically
    // TODO: Implement natural sort for phase names (e.g., "phase2" < "phase10")
    let mut phases: Vec<String> = by_phase.keys().cloned().collect();
    phases.sort();

    let mut all_results = Vec::new();

    // Execute each phase sequentially
    for phase in phases {
        let phase_hosts = by_phase.remove(&phase).unwrap();

        // Within phase, run in parallel (respecting concurrency)
        let phase_results = push_parallel(deployer, phase_hosts, opts)?;

        // If any host in this phase failed, stop
        if !phase_results.all_succeeded() {
            all_results.extend(phase_results.results);
            return Ok(PushResults {
                results: all_results,
            });
        }

        all_results.extend(phase_results.results);
    }

    Ok(PushResults {
        results: all_results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::mock::MockDeployer;

    fn make_host(name: &str, phase: Option<&str>) -> (HostMetadata, DeployTarget, StorePath) {
        let metadata = HostMetadata {
            name: name.to_string(),
            phase: phase.map(|s| s.to_string()),
            tags: vec![],
            fqdn: format!("{}.example.com", name),
        };

        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: format!("{}.example.com", name),
            port: None,
        };

        let store_path = StorePath::new(format!("/nix/store/fake-{}", name));

        (metadata, target, store_path)
    }

    #[test]
    fn test_push_results_counts() {
        let results = PushResults {
            results: vec![
                ("host1".to_string(), DeployResult::success("ok")),
                ("host2".to_string(), DeployResult::failure("failed")),
                ("host3".to_string(), DeployResult::success("ok")),
            ],
        };

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 1);
        assert!(!results.all_succeeded());
    }

    #[test]
    fn test_push_results_all_succeeded() {
        let results = PushResults {
            results: vec![
                ("host1".to_string(), DeployResult::success("ok")),
                ("host2".to_string(), DeployResult::success("ok")),
            ],
        };

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);
        assert!(results.all_succeeded());
    }

    #[test]
    fn test_push_parallel_all_success() {
        let deployer = MockDeployer::new();

        let hosts = vec![make_host("host1", None), make_host("host2", None)];

        let results = push_parallel(&deployer, hosts, &PushOptions::default()).unwrap();

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);
    }

    #[test]
    fn test_push_parallel_with_failure() {
        let deployer = MockDeployer::new().with_failures(vec!["host2.example.com".to_string()]);

        let hosts = vec![make_host("host1", None), make_host("host2", None)];

        let results = push_parallel(&deployer, hosts, &PushOptions::default()).unwrap();

        assert_eq!(results.success_count(), 1);
        assert_eq!(results.failure_count(), 1);
        // Note: parallel execution may return results in any order
        let host_names: Vec<&str> = results.results.iter().map(|(n, _)| n.as_str()).collect();
        assert!(host_names.contains(&"host1"));
        assert!(host_names.contains(&"host2"));

        // Check that one succeeded and one failed
        let host1_result = results.results.iter().find(|(n, _)| n == "host1").unwrap();
        let host2_result = results.results.iter().find(|(n, _)| n == "host2").unwrap();
        assert!(host1_result.1.success);
        assert!(!host2_result.1.success);
    }

    #[test]
    fn test_push_by_phase_ordering() {
        let deployer = MockDeployer::new();

        let hosts = vec![
            make_host("host1", Some("p3")),
            make_host("host2", Some("p1")),
            make_host("host3", Some("p2")),
        ];

        let results = push_by_phase(&deployer, hosts, &PushOptions::default()).unwrap();

        // Should be ordered: p1, p2, p3
        assert_eq!(results.results.len(), 3);
        assert_eq!(results.results[0].0, "host2"); // p1
        assert_eq!(results.results[1].0, "host3"); // p2
        assert_eq!(results.results[2].0, "host1"); // p3
        assert!(results.all_succeeded());
    }

    #[test]
    fn test_push_by_phase_stops_on_failure() {
        let deployer = MockDeployer::new().with_failures(vec!["host3.example.com".to_string()]);

        let hosts = vec![
            make_host("host1", Some("p1")),
            make_host("host2", Some("p3")),
            make_host("host3", Some("p2")), // This will fail
        ];

        let results = push_by_phase(&deployer, hosts, &PushOptions::default()).unwrap();

        // Should deploy p1 (success), then p2 (failure), then stop
        assert_eq!(results.results.len(), 2);
        assert_eq!(results.results[0].0, "host1"); // p1 succeeded
        assert_eq!(results.results[1].0, "host3"); // p2 failed
        assert!(!results.all_succeeded());
    }

    #[test]
    fn test_push_by_phase_default_phase() {
        let deployer = MockDeployer::new();

        let hosts = vec![
            make_host("host1", Some("p1")),
            make_host("host2", None), // No phase specified
        ];

        let results = push_by_phase(&deployer, hosts, &PushOptions::default()).unwrap();

        // Both should succeed (default phase sorts after p1)
        assert_eq!(results.results.len(), 2);
        assert!(results.all_succeeded());
    }

    #[test]
    fn test_push_parallel_error_message_includes_source() {
        // Test that when deployment fails, the error message includes the full error chain
        let deployer = MockDeployer::new().with_failures(vec!["fail.example.com".to_string()]);

        let hosts = vec![make_host("fail", None)];

        let results = push_parallel(&deployer, hosts, &PushOptions::default()).unwrap();

        assert_eq!(results.failure_count(), 1);

        // Verify the error message includes the source error
        let (_, deploy_result) = &results.results[0];
        assert!(!deploy_result.success);
        assert!(
            deploy_result.message.contains("Caused by:"),
            "Error message should include 'Caused by:' to show the error chain. Got: {}",
            deploy_result.message
        );
        assert!(
            deploy_result.message.contains("copy_closure failed"),
            "Error message should include the source error details. Got: {}",
            deploy_result.message
        );
    }

    #[test]
    fn test_push_parallel_error_message_format() {
        // Test the specific format of error messages with the full chain
        use std::collections::HashMap;

        let mut failures = HashMap::new();
        failures.insert(
            "test.example.com".to_string(),
            crate::deploy::mock::FailureSpec {
                phase: "activate".to_string(),
                error: Some("systemd service failed to start".to_string()),
            },
        );

        let fixture = crate::deploy::mock::DeploymentFixture {
            name: "Test error formatting".to_string(),
            description: None,
            failures,
            hosts: vec![],
        };

        let deployer = MockDeployer::from_fixture(fixture);
        let hosts = vec![make_host("test", None)];

        let results = push_parallel(&deployer, hosts, &PushOptions::default()).unwrap();

        assert_eq!(results.failure_count(), 1);

        let (_, deploy_result) = &results.results[0];
        assert!(!deploy_result.success);

        // The error message should be structured as:
        // "Failed to deploy to host 'test.example.com'"
        //   Caused by: Activation failed
        assert!(
            deploy_result.message.contains("Failed to deploy"),
            "Error message should start with main error. Got: {}",
            deploy_result.message
        );
        assert!(
            deploy_result.message.contains("Caused by:"),
            "Error message should include 'Caused by:' separator. Got: {}",
            deploy_result.message
        );
        assert!(
            deploy_result.message.contains("Activation failed"),
            "Error message should include the source error. Got: {}",
            deploy_result.message
        );
    }
}
