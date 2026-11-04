use clap::Args;
use snowlib::{Flake, NixBackend, Result};
use std::collections::{BTreeSet, HashSet};

/// Host selection flags shared by every command that operates on a set of hosts.
#[derive(Debug, Default, Args)]
pub struct HostSelectionArgs {
    /// Hosts to operate on (defaults to all hosts if not specified)
    pub hosts: Vec<String>,

    /// Filter hosts using a Nix expression (e.g., "config: config.services.nginx.enable or false")
    #[arg(short = 'e', long, conflicts_with_all = &["phase", "phases", "tag", "tags"])]
    pub filter: Option<String>,

    /// Filter by deployment phase (e.g., "production")
    #[arg(long, conflicts_with_all = &["filter", "phases", "tag", "tags"])]
    pub phase: Option<String>,

    /// Filter by multiple phases (comma-separated, OR logic)
    #[arg(long, conflicts_with_all = &["filter", "phase", "tag", "tags"], value_delimiter = ',')]
    pub phases: Vec<String>,

    /// Filter by tag (e.g., "web")
    #[arg(long, conflicts_with_all = &["filter", "phase", "phases", "tags"])]
    pub tag: Option<String>,

    /// Filter by multiple tags (comma-separated, OR logic)
    #[arg(long, conflicts_with_all = &["filter", "phase", "phases", "tag"], value_delimiter = ',')]
    pub tags: Vec<String>,
}

impl HostSelectionArgs {
    fn filter_expression(&self) -> Result<Option<String>> {
        crate::filter_helpers::build_filter_expression(
            self.filter.as_deref(),
            self.phase.as_deref(),
            &self.phases,
            self.tag.as_deref(),
            &self.tags,
        )
    }

    pub async fn resolve(
        &self,
        flake: &Flake,
        nix_backend: &dyn NixBackend,
    ) -> Result<Vec<String>> {
        resolve_hosts(
            flake,
            nix_backend,
            self.filter_expression()?.as_deref(),
            Some(&self.hosts),
        )
        .await
    }
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
/// let hosts = resolve_hosts(&flake, backend, Some("c: c.services.nginx.enable"), None).await?;
///
/// // Get specific hosts (validates they exist)
/// let hosts = resolve_hosts(&flake, backend, None, Some(&["web1", "web2"])).await?;
///
/// // Get intersection: hosts in explicit list that also match the filter
/// // - If web1 has nginx but db1 doesn't: returns ["web1"]
/// // - If neither has nginx: returns []
/// // - If web1 doesn't exist: returns Err (validation before intersection)
/// let hosts = resolve_hosts(&flake, backend, Some("c: c.services.nginx.enable"), Some(&["web1", "db1"])).await?;
/// ```
pub async fn resolve_hosts(
    flake: &Flake,
    nix_backend: &dyn NixBackend,
    filter_expr: Option<&str>,
    explicit_hosts: Option<&[String]>,
) -> Result<Vec<String>> {
    match (filter_expr, explicit_hosts) {
        // Note: Empty vec treated as None to allow passing args.hosts.as_slice() directly

        // No filter, no explicit hosts: return all hosts
        (None, None) | (None, Some(&[])) => {
            let all_hosts = nix_backend.list_all_hosts(flake, None).await?;
            Ok(all_hosts.iter().map(|h| h.name.clone()).collect())
        }

        // Filter only: return all hosts matching filter
        (Some(filter), None) | (Some(filter), Some(&[])) => {
            let filtered_hosts = nix_backend.list_all_hosts(flake, Some(filter)).await?;
            Ok(filtered_hosts.iter().map(|h| h.name.clone()).collect())
        }

        // Explicit hosts only: validate they exist, return them
        (None, Some(requested)) => {
            let all_hosts = nix_backend.list_all_hosts(flake, None).await?;
            ensure_hosts_exist(requested, &all_hosts)?;
            Ok(requested.to_vec())
        }

        // Both filter and explicit hosts: return intersection
        (Some(filter), Some(requested)) => {
            // Two list_all_hosts calls for clear error messages:
            // - With filter: determines which hosts match (for intersection)
            // - Without filter: validates explicit hosts exist (better errors on typos)
            // Could optimize to one call but would complicate validation logic
            let filtered_hosts = nix_backend.list_all_hosts(flake, Some(filter)).await?;
            let all_hosts = nix_backend.list_all_hosts(flake, None).await?;

            ensure_hosts_exist(requested, &all_hosts)?;

            let filtered_set: HashSet<String> =
                filtered_hosts.iter().map(|h| h.name.clone()).collect();

            // Return intersection: explicit hosts that also match the filter
            Ok(requested
                .iter()
                .filter(|h| filtered_set.contains(*h))
                .cloned()
                .collect())
        }
    }
}

fn ensure_hosts_exist(requested: &[String], all_hosts: &[snowlib::Host]) -> Result<()> {
    let available: BTreeSet<&str> = all_hosts.iter().map(|h| h.name.as_str()).collect();
    let invalid_hosts: Vec<String> = requested
        .iter()
        .filter(|h| !available.contains(h.as_str()))
        .cloned()
        .collect();

    if invalid_hosts.is_empty() {
        return Ok(());
    }
    Err(snowlib::error::FlakeError::InvalidHosts {
        invalid_hosts,
        available_hosts: available.into_iter().collect::<Vec<_>>().join(", "),
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use snowlib::{DeploymentFixture, HostFixture, MockNixBackend, OperationFixture};

    /// The mock backends never read it, but a Flake needs a flake.nix
    fn fixture_flake() -> Flake {
        Flake::new(Some(crate::test_utils::fixtures_dir().to_str().unwrap())).unwrap()
    }

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

    #[derive(clap::Parser)]
    struct SelectionCli {
        #[command(flatten)]
        selection: HostSelectionArgs,
    }

    fn parse_selection(args: &[&str]) -> std::result::Result<HostSelectionArgs, clap::Error> {
        SelectionCli::try_parse_from(std::iter::once("snow").chain(args.iter().copied()))
            .map(|cli| cli.selection)
    }

    #[test]
    fn test_selection_parses_hosts_and_comma_separated_tags() {
        let selection = parse_selection(&["--tags", "web,database", "hydrogen", "oxygen"]).unwrap();

        assert_eq!(selection.hosts, vec!["hydrogen", "oxygen"]);
        assert_eq!(selection.tags, vec!["web", "database"]);
        assert_eq!(
            selection.filter_expression().unwrap().unwrap(),
            r#"config: builtins.any (tag: builtins.elem tag (config.core.tags or [])) ["web" "database"]"#
        );
    }

    #[test]
    fn test_selection_without_flags_has_no_filter() {
        let selection = parse_selection(&[]).unwrap();

        assert!(selection.hosts.is_empty());
        assert!(selection.filter_expression().unwrap().is_none());
    }

    #[test]
    fn test_selection_rejects_conflicting_filters() {
        for args in [
            &["--phase", "production", "--tag", "web"][..],
            &["-e", "c: true", "--phases", "staging"][..],
        ] {
            let err = parse_selection(args).err().unwrap();
            assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    #[tokio::test]
    async fn test_selection_resolve_without_hosts_returns_all_hosts() {
        let backend = create_mock_backend_with_hosts(vec!["hydrogen", "oxygen"]);
        let flake = fixture_flake();

        let result = parse_selection(&[])
            .unwrap()
            .resolve(&flake, &backend)
            .await;

        let hosts: HashSet<String> = result.unwrap().into_iter().collect();
        assert_eq!(hosts, HashSet::from(["hydrogen".into(), "oxygen".into()]));
    }

    #[tokio::test]
    async fn test_selection_resolve_validates_explicit_hosts() {
        let backend = create_mock_backend_with_hosts(vec!["hydrogen", "oxygen"]);
        let flake = fixture_flake();
        let selection = parse_selection(&["hydrogen", "radon"]).unwrap();

        let result = selection.resolve(&flake, &backend).await;

        assert!(result.unwrap_err().to_string().contains("radon"));
    }

    #[tokio::test]
    async fn test_resolve_hosts_no_filter_no_explicit() {
        let flake = fixture_flake();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2", "host3"]);

        let result = resolve_hosts(&flake, &backend, None, None).await.unwrap();

        assert!(result.contains(&"host1".to_string()));
        assert!(result.contains(&"host2".to_string()));
        assert!(result.contains(&"host3".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_hosts_with_explicit_empty_vec() {
        let flake = fixture_flake();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2"]);

        let result = resolve_hosts(&flake, &backend, None, Some(&[]))
            .await
            .unwrap();

        assert!(result.contains(&"host1".to_string()));
        assert!(result.contains(&"host2".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_hosts_with_explicit_valid() {
        let flake = fixture_flake();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2", "host3"]);

        let explicit = vec!["host1".to_string(), "host3".to_string()];
        let result = resolve_hosts(&flake, &backend, None, Some(&explicit))
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains(&"host1".to_string()));
        assert!(result.contains(&"host3".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_hosts_with_explicit_invalid() {
        let flake = fixture_flake();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2"]);

        let explicit = vec!["host1".to_string(), "nonexistent".to_string()];
        let result = resolve_hosts(&flake, &backend, None, Some(&explicit)).await;

        assert_eq!(result.unwrap_err().to_string(), "Unknown host: nonexistent");
    }

    #[tokio::test]
    async fn test_resolve_hosts_names_every_unknown_host() {
        let flake = fixture_flake();
        let backend = create_mock_backend_with_hosts(vec!["argon", "boron"]);

        let explicit = vec!["argon".to_string(), "xenon".to_string(), "neon".to_string()];
        let result = resolve_hosts(&flake, &backend, None, Some(&explicit)).await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Unknown hosts: xenon, neon"
        );
    }

    #[tokio::test]
    async fn test_resolve_hosts_lists_the_available_hosts_in_order() {
        let flake = fixture_flake();
        let backend = create_mock_backend_with_hosts(vec!["radon", "argon", "neon", "boron"]);

        let explicit = vec!["xenon".to_string(), "argon".to_string()];
        for filter in [None, Some("c: true")] {
            let err = resolve_hosts(&flake, &backend, filter, Some(&explicit))
                .await
                .unwrap_err();

            assert_eq!(err.to_string(), "Unknown host: xenon");
            assert_eq!(
                miette::Diagnostic::help(&err).unwrap().to_string(),
                "The flake's hosts are argon, boron, neon, radon. Run `snow list` to see them."
            );
        }
    }

    #[tokio::test]
    async fn test_resolve_hosts_intersection_with_filter_and_explicit() {
        let flake = fixture_flake();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2", "host3"]);

        // In a real scenario, the filter would narrow down the hosts
        // For this test, the mock always returns all hosts regardless of filter
        // So we can only test that explicit hosts are validated for existence
        let explicit = vec!["host1".to_string(), "host2".to_string()];
        let result = resolve_hosts(&flake, &backend, Some("c: true"), Some(&explicit)).await;

        assert!(result.is_ok());
        let hosts = result.unwrap();
        assert!(hosts.contains(&"host1".to_string()));
        assert!(hosts.contains(&"host2".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_hosts_intersection_validates_host_existence() {
        let flake = fixture_flake();
        let backend = create_mock_backend_with_hosts(vec!["host1", "host2"]);

        let explicit = vec!["host1".to_string(), "nonexistent".to_string()];
        let result = resolve_hosts(&flake, &backend, Some("c: true"), Some(&explicit)).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("nonexistent"));
    }
}
