use snowlib::{Flake, NixBackend, Result};
use std::collections::HashSet;

/// Bridge sync code to async during migration
fn block_on_async<F: std::future::Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => tokio::runtime::Runtime::new()
            .expect("Failed to create tokio runtime")
            .block_on(future),
    }
}

/// Helper function for commands with hosts and filter arguments.
///
/// Automatically handles the empty vec case, making it convenient for commands
/// to call with `&self.hosts` without checking if empty first.
pub fn resolve_hosts_from_args(
    flake: &Flake,
    nix_backend: &dyn NixBackend,
    filter: Option<&str>,
    hosts: &[String],
) -> Result<Vec<String>> {
    let explicit_hosts = if hosts.is_empty() { None } else { Some(hosts) };

    resolve_hosts(flake, nix_backend, filter, explicit_hosts)
}

/// Resolves the final list of hosts to operate on, combining filtering and explicit selection.
///
/// When both filter and explicit hosts are provided, returns the intersection (hosts that
/// are both in the explicit list AND match the filter). This allows you to specify a broad
/// set of hosts and then narrow them with a filter.
///
/// # Arguments
/// * `flake` - The flake to query
/// * `nix_backend` - Backend for Nix operations
/// * `filter_expr` - Optional Nix expression to filter hosts
/// * `explicit_hosts` - Optional list of specific hosts to select
///
/// # Examples
///
/// ```ignore
/// // Get all hosts matching a filter
/// let hosts = resolve_hosts(&flake, backend, Some("c: c.services.nginx.enable"), None)?;
///
/// // Get specific hosts (validates they exist)
/// let hosts = resolve_hosts(&flake, backend, None, Some(&["web1", "web2"]))?;
///
/// // Get intersection: hosts in explicit list that also match the filter
/// // - If web1 has nginx but db1 doesn't: returns ["web1"]
/// // - If neither has nginx: returns []
/// // - If web1 doesn't exist: returns Err (validation before intersection)
/// let hosts = resolve_hosts(&flake, backend, Some("c: c.services.nginx.enable"), Some(&["web1", "db1"]))?;
/// ```
pub fn resolve_hosts(
    flake: &Flake,
    nix_backend: &dyn NixBackend,
    filter_expr: Option<&str>,
    explicit_hosts: Option<&[String]>,
) -> Result<Vec<String>> {
    match (filter_expr, explicit_hosts) {
        // Note: Empty vec treated as None to allow passing args.hosts.as_slice() directly

        // No filter, no explicit hosts: return all hosts
        (None, None) | (None, Some(&[])) => {
            let all_hosts = block_on_async(nix_backend.list_all_hosts(flake, None))?;
            Ok(all_hosts.iter().map(|h| h.name.clone()).collect())
        }

        // Filter only: return all hosts matching filter
        (Some(filter), None) | (Some(filter), Some(&[])) => {
            let filtered_hosts = block_on_async(nix_backend.list_all_hosts(flake, Some(filter)))?;
            Ok(filtered_hosts.iter().map(|h| h.name.clone()).collect())
        }

        // Explicit hosts only: validate they exist, return them
        (None, Some(requested)) => {
            let all_hosts = block_on_async(nix_backend.list_all_hosts(flake, None))?;
            let valid_set: HashSet<String> = all_hosts.iter().map(|h| h.name.clone()).collect();

            let invalid_hosts: Vec<String> = requested
                .iter()
                .filter(|h| !valid_set.contains(*h))
                .cloned()
                .collect();

            if !invalid_hosts.is_empty() {
                return Err(snowlib::error::FlakeError::InvalidHosts {
                    invalid_hosts: invalid_hosts.join(", "),
                    available_hosts: valid_set.iter().cloned().collect::<Vec<_>>().join(", "),
                }
                .into());
            }

            Ok(requested.to_vec())
        }

        // Both filter and explicit hosts: return intersection
        (Some(filter), Some(requested)) => {
            // Two list_all_hosts calls for clear error messages:
            // - With filter: determines which hosts match (for intersection)
            // - Without filter: validates explicit hosts exist (better errors on typos)
            // Could optimize to one call but would complicate validation logic
            let filtered_hosts = block_on_async(nix_backend.list_all_hosts(flake, Some(filter)))?;
            let all_hosts = block_on_async(nix_backend.list_all_hosts(flake, None))?;

            let filtered_set: HashSet<String> =
                filtered_hosts.iter().map(|h| h.name.clone()).collect();
            let all_hosts_set: HashSet<String> = all_hosts.iter().map(|h| h.name.clone()).collect();

            // First check that all explicit hosts exist (regardless of filter)
            let invalid_hosts: Vec<String> = requested
                .iter()
                .filter(|h| !all_hosts_set.contains(*h))
                .cloned()
                .collect();

            if !invalid_hosts.is_empty() {
                return Err(snowlib::error::FlakeError::InvalidHosts {
                    invalid_hosts: invalid_hosts.join(", "),
                    available_hosts: all_hosts_set.iter().cloned().collect::<Vec<_>>().join(", "),
                }
                .into());
            }

            // Return intersection: explicit hosts that also match the filter
            Ok(requested
                .iter()
                .filter(|h| filtered_set.contains(*h))
                .cloned()
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snowlib::{DeploymentFixture, HostFixture, MockNixBackend, OperationFixture};

    fn create_mock_backend_with_hosts(hosts: Vec<&str>) -> MockNixBackend {
        let host_fixtures: Vec<HostFixture> = hosts
            .iter()
            .map(|name| HostFixture {
                hostname: name.to_string(),
                metadata: None,
                build: None,
                copy_closure: OperationFixture {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: 0,
                },
                activate: OperationFixture {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: 0,
                },
            })
            .collect();

        let fixture = DeploymentFixture {
            name: "test".to_string(),
            description: None,
            failures: std::collections::HashMap::new(),
            hosts: host_fixtures,
        };

        MockNixBackend::from_fixture(fixture)
    }

    #[test]
    fn test_resolve_hosts_no_filter_no_explicit() {
        let flake = Flake::new(None).unwrap();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2", "host3"]);

        let result = resolve_hosts(&flake, &backend, None, None).unwrap();

        assert!(result.contains(&"host1".to_string()));
        assert!(result.contains(&"host2".to_string()));
        assert!(result.contains(&"host3".to_string()));
    }

    #[test]
    fn test_resolve_hosts_with_explicit_empty_vec() {
        let flake = Flake::new(None).unwrap();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2"]);

        let result = resolve_hosts(&flake, &backend, None, Some(&[])).unwrap();

        assert!(result.contains(&"host1".to_string()));
        assert!(result.contains(&"host2".to_string()));
    }

    #[test]
    fn test_resolve_hosts_with_explicit_valid() {
        let flake = Flake::new(None).unwrap();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2", "host3"]);

        let explicit = vec!["host1".to_string(), "host3".to_string()];
        let result = resolve_hosts(&flake, &backend, None, Some(&explicit)).unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains(&"host1".to_string()));
        assert!(result.contains(&"host3".to_string()));
    }

    #[test]
    fn test_resolve_hosts_with_explicit_invalid() {
        let flake = Flake::new(None).unwrap();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2"]);

        let explicit = vec!["host1".to_string(), "nonexistent".to_string()];
        let result = resolve_hosts(&flake, &backend, None, Some(&explicit));

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("nonexistent"));
        assert!(err_msg.contains("host1"));
        assert!(err_msg.contains("host2"));
    }

    #[test]
    fn test_resolve_hosts_intersection_with_filter_and_explicit() {
        let flake = Flake::new(None).unwrap();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2", "host3"]);

        // In a real scenario, the filter would narrow down the hosts
        // For this test, the mock always returns all hosts regardless of filter
        // So we can only test that explicit hosts are validated for existence
        let explicit = vec!["host1".to_string(), "host2".to_string()];
        let result = resolve_hosts(&flake, &backend, Some("c: true"), Some(&explicit));

        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert!(hosts.contains(&"host1".to_string()));
        assert!(hosts.contains(&"host2".to_string()));
    }

    #[test]
    fn test_resolve_hosts_intersection_validates_host_existence() {
        let flake = Flake::new(None).unwrap();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2"]);

        let explicit = vec!["host1".to_string(), "nonexistent".to_string()];
        let result = resolve_hosts(&flake, &backend, Some("c: true"), Some(&explicit));

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("nonexistent"));
    }
}
