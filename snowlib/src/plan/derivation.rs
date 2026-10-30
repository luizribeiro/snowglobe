use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::error::{NixError, Result};

// nix 2.34 omits the store dir from derivation keys; only the default store is supported.
const STORE_DIR: &str = "/nix/store";

/// Facts about a derivation that will be built, read from `nix derivation show`.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivationInfo {
    pub drv_path: String,
    pub name: String,
    pub pname: Option<String>,
    pub version: Option<String>,
    pub system: String,
    /// Marked by its author as cheap to build locally (`preferLocalBuild` or
    /// `allowSubstitutes = false`), as nixpkgs does for `writeText`, `runCommandLocal` etc.
    pub trivial: bool,
}

// nix 2.34 wraps derivations in a versioned object keyed by store path basename;
// older versions return a bare map keyed by full store path.
#[derive(Deserialize)]
#[serde(untagged)]
enum DerivationShowOutput {
    Versioned {
        derivations: HashMap<String, RawDerivation>,
    },
    Legacy(HashMap<String, RawDerivation>),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDerivation {
    name: String,
    system: String,
    #[serde(default)]
    env: HashMap<String, String>,
    structured_attrs: Option<Map<String, Value>>,
}

impl RawDerivation {
    fn bool_attr(&self, key: &str) -> Option<bool> {
        match &self.structured_attrs {
            Some(attrs) => attrs.get(key).and_then(Value::as_bool),
            None => self.env.get(key).map(|value| value == "1"),
        }
    }

    fn string_attr(&self, key: &str) -> Option<String> {
        match &self.structured_attrs {
            Some(attrs) => attrs.get(key).and_then(Value::as_str).map(str::to_string),
            None => self.env.get(key).cloned(),
        }
    }

    fn is_trivial(&self) -> bool {
        self.bool_attr("preferLocalBuild") == Some(true)
            || self.bool_attr("allowSubstitutes") == Some(false)
    }

    fn into_info(self, drv_path: String) -> DerivationInfo {
        DerivationInfo {
            drv_path,
            pname: self.string_attr("pname"),
            version: self.string_attr("version"),
            trivial: self.is_trivial(),
            name: self.name,
            system: self.system,
        }
    }
}

fn full_store_path(key: String) -> String {
    if key.starts_with('/') {
        key
    } else {
        format!("{STORE_DIR}/{key}")
    }
}

/// Parse `nix derivation show` output, sorted by derivation path.
pub fn parse_derivation_show(output: &[u8]) -> Result<Vec<DerivationInfo>> {
    let parsed: DerivationShowOutput =
        serde_json::from_slice(output).map_err(|source| NixError::OutputParseFailed {
            output: String::from_utf8_lossy(output).to_string(),
            source,
        })?;

    let derivations = match parsed {
        DerivationShowOutput::Versioned { derivations } => derivations,
        DerivationShowOutput::Legacy(derivations) => derivations,
    };

    let mut infos: Vec<DerivationInfo> = derivations
        .into_iter()
        .map(|(key, raw)| raw.into_info(full_store_path(key)))
        .collect();
    infos.sort_by(|a, b| a.drv_path.cmp(&b.drv_path));
    Ok(infos)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NIX_2_31: &str = include_str!("../../tests/fixtures/plan/derivation-show-2.31.json");
    const NIX_2_34: &str = include_str!("../../tests/fixtures/plan/derivation-show-2.34.json");
    const STRUCTURED: &str =
        include_str!("../../tests/fixtures/plan/derivation-show-structured.json");

    const HELLO_DRV: &str = "/nix/store/3d1b6iqjqj12mj13l1c9g920llcv5n0l-hello-snowtest-2.12.2.drv";
    const STAGE2_DRV: &str = "/nix/store/my8fv7s5hvjgjqfa3j18bajh4hymlzz0-stage-2-init.sh.drv";

    fn expected_fixture_infos() -> Vec<DerivationInfo> {
        vec![
            DerivationInfo {
                drv_path: HELLO_DRV.to_string(),
                name: "hello-snowtest-2.12.2".to_string(),
                pname: Some("hello-snowtest".to_string()),
                version: Some("2.12.2".to_string()),
                system: "x86_64-linux".to_string(),
                trivial: false,
            },
            DerivationInfo {
                drv_path: STAGE2_DRV.to_string(),
                name: "stage-2-init.sh".to_string(),
                pname: None,
                version: None,
                system: "x86_64-linux".to_string(),
                trivial: true,
            },
        ]
    }

    #[test]
    fn test_parse_nix_2_31_output() {
        let infos = parse_derivation_show(NIX_2_31.as_bytes()).unwrap();

        assert_eq!(infos, expected_fixture_infos());
    }

    #[test]
    fn test_parse_nix_2_34_versioned_output() {
        let infos = parse_derivation_show(NIX_2_34.as_bytes()).unwrap();

        assert_eq!(infos, expected_fixture_infos());
    }

    #[test]
    fn test_parse_structured_attrs_derivation() {
        let infos = parse_derivation_show(STRUCTURED.as_bytes()).unwrap();

        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "snowtest-structured");
        assert!(infos[0].trivial);
        assert_eq!(infos[0].pname, None);
    }

    #[test]
    fn test_trivial_when_only_one_attribute_is_set() {
        let output = r#"{
          "/nix/store/aaa-local.drv": {"name": "local", "system": "x86_64-linux",
            "env": {"preferLocalBuild": "1"}},
          "/nix/store/bbb-nosubst.drv": {"name": "nosubst", "system": "x86_64-linux",
            "env": {"allowSubstitutes": ""}},
          "/nix/store/ccc-substitutable.drv": {"name": "substitutable", "system": "x86_64-linux",
            "env": {"preferLocalBuild": "", "allowSubstitutes": "1"}}
        }"#;

        let trivial: Vec<bool> = parse_derivation_show(output.as_bytes())
            .unwrap()
            .iter()
            .map(|info| info.trivial)
            .collect();

        assert_eq!(trivial, vec![true, true, false]);
    }

    #[test]
    fn test_parse_structured_attrs_reads_pname_and_version() {
        let output = r#"{"version": 4, "derivations": {
          "aaa-wine-wow-staging-10.12.drv": {"name": "wine-wow-staging-10.12",
            "system": "x86_64-linux", "env": {"out": "/nix/store/bbb-wine"},
            "structuredAttrs": {"pname": "wine-wow-staging", "version": "10.12"}}
        }}"#;

        let infos = parse_derivation_show(output.as_bytes()).unwrap();

        assert_eq!(
            infos[0].drv_path,
            "/nix/store/aaa-wine-wow-staging-10.12.drv"
        );
        assert_eq!(infos[0].pname.as_deref(), Some("wine-wow-staging"));
        assert_eq!(infos[0].version.as_deref(), Some("10.12"));
        assert!(!infos[0].trivial);
    }

    #[test]
    fn test_parse_empty_output_in_both_formats() {
        assert!(parse_derivation_show(b"{}").unwrap().is_empty());
        assert!(
            parse_derivation_show(br#"{"version": 4, "derivations": {}}"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_parse_invalid_output_is_an_error() {
        let err = parse_derivation_show(b"error: path is not valid").unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Nix(NixError::OutputParseFailed { .. })
        ));
    }
}
