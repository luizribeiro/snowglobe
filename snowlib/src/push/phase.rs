//! Push phase names

use crate::error::{FlakeError, Result};
use std::cmp::Ordering;

/// The phase of hosts without one, so hosts may not set it themselves
pub const UNPHASED: &str = "default";

/// The one phase of a push that ignores phases
pub const ALL_HOSTS: &str = "_all";

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

/// The order phases run in: named phases first, then hosts without one
pub(crate) fn run_order(a: &str, b: &str) -> Ordering {
    (a == UNPHASED)
        .cmp(&(b == UNPHASED))
        .then_with(|| natural_cmp(a, b))
}

/// Orders names by their text, but by their numbers' values where both have
/// a number, so p2 comes before p10
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a_rest, mut b_rest) = (a, b);
    while !a_rest.is_empty() && !b_rest.is_empty() {
        let (a_chunk, a_next) = split_chunk(a_rest);
        let (b_chunk, b_next) = split_chunk(b_rest);
        let order = if starts_with_digit(a_chunk) && starts_with_digit(b_chunk) {
            cmp_numbers(a_chunk, b_chunk)
        } else {
            a_chunk.cmp(b_chunk)
        };
        if order != Ordering::Equal {
            return order;
        }
        (a_rest, b_rest) = (a_next, b_next);
    }
    // "p01" and "p1" tie on value; their text breaks the tie
    a_rest.len().cmp(&b_rest.len()).then_with(|| a.cmp(b))
}

fn starts_with_digit(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_digit())
}

/// Splits off the leading run of digits, or of anything but digits
fn split_chunk(s: &str) -> (&str, &str) {
    let digits = starts_with_digit(s);
    let end = s
        .find(|c: char| c.is_ascii_digit() != digits)
        .unwrap_or(s.len());
    s.split_at(end)
}

/// Compares runs of digits by value, however long they are
fn cmp_numbers(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        names.sort_by(|a, b| natural_cmp(a, b));
        names
    }

    #[test]
    fn test_hosts_without_a_phase_run_after_every_named_phase() {
        let mut phases = vec![UNPHASED, "p10", "zone-z", "p2", "aaa"];
        phases.sort_by(|a, b| run_order(a, b));

        assert_eq!(phases, vec!["aaa", "p2", "p10", "zone-z", UNPHASED]);
    }

    #[test]
    fn test_natural_cmp_orders_numbers_by_value() {
        assert_eq!(
            sorted(&["p10", "p2", "p1", "p3-k8s", "p20", "p3"]),
            vec!["p1", "p2", "p3", "p3-k8s", "p10", "p20"]
        );
    }

    #[test]
    fn test_natural_cmp_orders_text_alphabetically() {
        assert_eq!(
            sorted(&["staging", "production", "canary", "never"]),
            vec!["canary", "never", "production", "staging"]
        );
    }

    #[test]
    fn test_natural_cmp_compares_every_number_in_a_name() {
        assert_eq!(
            sorted(&["k8s-p10", "k8s-p2", "k8s-p1"]),
            vec!["k8s-p1", "k8s-p2", "k8s-p10"]
        );
    }

    #[test]
    fn test_natural_cmp_breaks_ties_between_padded_numbers_by_text() {
        assert_eq!(
            sorted(&["p10", "p1", "p02", "p01", "p001"]),
            vec!["p001", "p01", "p1", "p02", "p10"]
        );
    }

    #[test]
    fn test_natural_cmp_handles_numbers_too_big_for_integers() {
        assert_eq!(
            sorted(&[
                "p99999999999999999999999",
                "p100000000000000000000000",
                "p2"
            ]),
            vec![
                "p2",
                "p99999999999999999999999",
                "p100000000000000000000000"
            ]
        );
    }

    #[test]
    fn test_natural_cmp_is_antisymmetric() {
        let pairs = [
            ("p2", "p10"),
            ("p", "p1"),
            ("1", "p"),
            ("p01", "p1"),
            ("p2", "p2"),
        ];
        for (a, b) in pairs {
            assert_eq!(natural_cmp(a, b), natural_cmp(b, a).reverse(), "{a} {b}");
        }
        assert_eq!(natural_cmp("p2", "p2"), Ordering::Equal);
    }

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
