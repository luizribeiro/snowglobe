//! Push phase names

use crate::error::{FlakeError, Result};

/// The phase of hosts without one, so hosts may not set it themselves
pub const UNPHASED: &str = "default";

/// Characters allowed in phase and tag names, as in the module's option type
pub fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn is_valid_phase(phase: &str) -> bool {
    !phase.is_empty() && phase.chars().all(is_name_char) && phase != UNPHASED
}

/// Rejects a phase the module's option type would have, for flakes that set
/// `snowglobe.push.phase` without the module
pub(crate) fn check_phase(host: &str, phase: Option<&str>) -> Result<()> {
    match phase {
        Some(phase) if !is_valid_phase(phase) => Err(FlakeError::InvalidPhase {
            host: host.to_string(),
            phase: phase.to_string(),
        }
        .into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_phase_accepts_names_and_no_phase() {
        for phase in [
            None,
            Some("p1"),
            Some("p10-k8s"),
            Some("never"),
            Some("storage_nodes"),
        ] {
            assert!(check_phase("cerium", phase).is_ok(), "{phase:?}");
        }
    }

    #[test]
    fn test_check_phase_rejects_other_characters_and_default() {
        for phase in ["", "p 1", "p1.k8s", "prod!", "日本", "default"] {
            assert!(check_phase("cerium", Some(phase)).is_err(), "{phase:?}");
        }
    }

    #[test]
    fn test_invalid_phase_names_the_host_and_how_to_fix_it() {
        let error = check_phase("cerium", Some("p1.k8s")).unwrap_err();

        assert_eq!(
            error.to_string(),
            "cerium has an invalid push phase 'p1.k8s'"
        );
        assert_eq!(
            miette::Diagnostic::help(&error).unwrap().to_string(),
            "A phase name may contain only ASCII letters, digits, dashes and underscores, and 'default' is reserved for hosts without a phase. Set snowglobe.push.phase to a name like 'p1', or leave it unset."
        );
    }
}
