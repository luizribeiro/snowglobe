use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::full_store_path;
use crate::error::{NixError, Result};

/// Sizes a substituter advertises for a store path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NarInfo {
    /// `FileSize` is optional in narinfo files.
    pub download_size: Option<u64>,
    pub nar_size: u64,
}

// `--json-format 2` (nix ≥ 2.34) wraps entries in `info` keyed by basename;
// format 1 is a bare map keyed by full store path.
#[derive(Deserialize)]
#[serde(untagged)]
enum PathInfoOutput {
    Format2 {
        info: HashMap<String, Option<NarInfo>>,
    },
    Format1(HashMap<String, Option<NarInfo>>),
}

/// Parse `nix path-info --json --store <substituter>` output.
///
/// Paths the substituter does not have are left out of the result.
pub fn parse_path_info(output: &[u8]) -> Result<HashMap<String, NarInfo>> {
    let parsed: PathInfoOutput =
        serde_json::from_slice(output).map_err(|source| NixError::OutputParseFailed {
            output: String::from_utf8_lossy(output).to_string(),
            source,
        })?;

    let entries = match parsed {
        PathInfoOutput::Format2 { info } => info,
        PathInfoOutput::Format1(entries) => entries,
    };

    Ok(entries
        .into_iter()
        .filter_map(|(key, info)| Some((full_store_path(key), info?)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMAT_1: &str = include_str!("../../tests/fixtures/plan/path-info-format-1.json");
    const FORMAT_2: &str = include_str!("../../tests/fixtures/plan/path-info-format-2.json");

    const FFMPEG: &str = "/nix/store/2smwf1lipxngcsimlhd077yzx5zd7vvi-ffmpeg-8.0";
    const HELLO_SRC: &str = "/nix/store/dw402azxjrgrzrk6j0p66wkqrab5mwgw-hello-2.12.2.tar.gz";
    const UNCACHED: &str = "/nix/store/42hnz9y3wkcf8rjjk02psm9yh54ibqqd-hello-snowtest-2.12.2";

    fn expected_cache_nixos_org() -> HashMap<String, NarInfo> {
        HashMap::from([
            (
                FFMPEG.to_string(),
                NarInfo {
                    download_size: Some(112),
                    nar_size: 96,
                },
            ),
            (
                HELLO_SRC.to_string(),
                NarInfo {
                    download_size: Some(1168732),
                    nar_size: 1168632,
                },
            ),
        ])
    }

    #[test]
    fn test_parse_format_1_skips_missing_paths() {
        let infos = parse_path_info(FORMAT_1.as_bytes()).unwrap();

        assert_eq!(infos, expected_cache_nixos_org());
        assert!(!infos.contains_key(UNCACHED));
    }

    #[test]
    fn test_parse_format_2_matches_format_1() {
        let infos = parse_path_info(FORMAT_2.as_bytes()).unwrap();

        assert_eq!(infos, expected_cache_nixos_org());
    }

    #[test]
    fn test_parse_without_download_size() {
        let output = format!(r#"{{"{FFMPEG}": {{"narSize": 96}}}}"#);

        let infos = parse_path_info(output.as_bytes()).unwrap();

        assert_eq!(infos[FFMPEG].download_size, None);
        assert_eq!(infos[FFMPEG].nar_size, 96);
    }

    #[test]
    fn test_parse_nothing_cached() {
        let output = format!(r#"{{"{UNCACHED}": null}}"#);

        assert!(parse_path_info(output.as_bytes()).unwrap().is_empty());
    }

    #[test]
    fn test_parse_invalid_output_is_an_error() {
        let err = parse_path_info(b"error: cannot connect to s3").unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Nix(NixError::OutputParseFailed { .. })
        ));
    }
}
