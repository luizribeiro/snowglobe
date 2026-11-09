use serde::Deserialize;

use crate::error::{NixError, Result};

// A binary cache's own nix-cache-info may advertise a different priority
// (cache.nixos.org says 40); snow does not fetch it, so only an explicit
// `?priority=` reorders substituters.
const DEFAULT_PRIORITY: u32 = 50;

/// A substituter from the local nix configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredSubstituter {
    /// Store URL without query parameters, for display.
    pub url: String,
    /// Full store URL, as passed to `--store`.
    pub store_url: String,
    pub priority: u32,
}

#[derive(Deserialize)]
struct NixConfig {
    substituters: Setting,
}

#[derive(Deserialize)]
struct Setting {
    value: Vec<String>,
}

/// Parse substituters from `nix config show --json`, sorted by priority.
///
/// A cache listed more than once keeps its most preferred entry.
pub fn parse_substituters(config_json: &[u8]) -> Result<Vec<ConfiguredSubstituter>> {
    let config: NixConfig =
        serde_json::from_slice(config_json).map_err(|source| NixError::OutputParseFailed {
            output: String::from_utf8_lossy(config_json).to_string(),
            source,
        })?;

    // Entries differing only in query parameters are the same cache to the reader.
    let mut substituters: Vec<ConfiguredSubstituter> = Vec::new();
    for substituter in config.substituters.value.into_iter().map(parse_store_url) {
        match substituters.iter_mut().find(|s| s.url == substituter.url) {
            Some(existing) if substituter.priority < existing.priority => *existing = substituter,
            Some(_) => {}
            None => substituters.push(substituter),
        }
    }

    substituters.sort_by_key(|sub| sub.priority);
    Ok(substituters)
}

fn parse_store_url(store_url: String) -> ConfiguredSubstituter {
    let (base, query) = store_url.split_once('?').unwrap_or((&store_url, ""));
    let priority = query
        .split('&')
        .find_map(|param| param.strip_prefix("priority="))
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_PRIORITY);
    ConfiguredSubstituter {
        url: base.trim_end_matches('/').to_string(),
        priority,
        store_url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPRUCE_CONFIG: &str = include_str!("../../tests/fixtures/plan/nix-config-show.json");

    fn summary(substituters: &[ConfiguredSubstituter]) -> Vec<(&str, u32)> {
        substituters
            .iter()
            .map(|sub| (sub.url.as_str(), sub.priority))
            .collect()
    }

    #[test]
    fn test_parse_real_config_dedupes_and_sorts_by_priority() {
        let substituters = parse_substituters(SPRUCE_CONFIG.as_bytes()).unwrap();

        assert_eq!(
            summary(&substituters),
            vec![("s3://nix-store-cache", 1), ("https://cache.nixos.org", 2)]
        );
        assert_eq!(
            substituters[0].store_url,
            "s3://nix-store-cache?priority=1&endpoint=https://truenas.i.thepromisedlan.club:9000"
        );
        assert_eq!(
            substituters[1].store_url,
            "https://cache.nixos.org/?priority=2"
        );
    }

    #[test]
    fn test_substituters_without_priority_use_default_and_keep_config_order() {
        let config = r#"{"substituters": {"value": [
            "https://nix-community.cachix.org",
            "https://cache.nixos.org/",
            "https://attic.example.com/main?priority=10"
        ]}}"#;

        let substituters = parse_substituters(config.as_bytes()).unwrap();

        assert_eq!(
            summary(&substituters),
            vec![
                ("https://attic.example.com/main", 10),
                ("https://nix-community.cachix.org", 50),
                ("https://cache.nixos.org", 50),
            ]
        );
    }

    #[test]
    fn test_duplicate_keeps_earlier_entry_when_it_is_preferred() {
        let config = r#"{"substituters": {"value": [
            "https://cache.example.com?priority=1",
            "https://cache.example.com/?priority=5"
        ]}}"#;

        let substituters = parse_substituters(config.as_bytes()).unwrap();

        assert_eq!(
            summary(&substituters),
            vec![("https://cache.example.com", 1)]
        );
        assert_eq!(
            substituters[0].store_url,
            "https://cache.example.com?priority=1"
        );
    }

    #[test]
    fn test_invalid_priority_falls_back_to_default() {
        let config = r#"{"substituters": {"value": ["https://cache.example.com?priority=high"]}}"#;

        let substituters = parse_substituters(config.as_bytes()).unwrap();

        assert_eq!(
            summary(&substituters),
            vec![("https://cache.example.com", 50)]
        );
    }

    #[test]
    fn test_no_substituters() {
        let config = r#"{"substituters": {"value": []}}"#;

        assert!(parse_substituters(config.as_bytes()).unwrap().is_empty());
    }

    #[test]
    fn test_parse_invalid_output_is_an_error() {
        assert!(matches!(
            parse_substituters(b"error: unknown command").unwrap_err(),
            crate::error::SnowError::Nix(NixError::OutputParseFailed { .. })
        ));
    }
}
