use serde::{Deserialize, Serialize};

use crate::error::{NixError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CacheStatus {
    Local,
    Cached,
    NotBuilt,
}

/// A host's toplevel as reported by `nix-eval-jobs --check-cache-status`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluatedHost {
    pub drv_path: String,
    pub system: String,
    pub cache_status: CacheStatus,
    pub needed_builds: Vec<String>,
    pub needed_substitutes: Vec<String>,
}

impl EvaluatedHost {
    /// Every derivation that must be built for this host.
    ///
    /// nix-eval-jobs (verified on 2.34.3) leaves the job's own derivation out of
    /// `neededBuilds`.
    pub fn builds(&self) -> impl Iterator<Item = &str> {
        let root = (self.cache_status == CacheStatus::NotBuilt).then_some(self.drv_path.as_str());
        self.needed_builds.iter().map(String::as_str).chain(root)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EvalOutcome {
    Evaluated(EvaluatedHost),
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostEval {
    pub host: String,
    pub outcome: EvalOutcome,
}

impl HostEval {
    pub fn evaluated(&self) -> Option<&EvaluatedHost> {
        match &self.outcome {
            EvalOutcome::Evaluated(host) => Some(host),
            EvalOutcome::Failed { .. } => None,
        }
    }
}

// `Failed` is tried first so a line carrying `error` is never mistaken for a
// successful evaluation.
#[derive(Deserialize)]
#[serde(untagged)]
enum EvalJobLine {
    Failed {
        attr: String,
        error: String,
    },
    Evaluated {
        attr: String,
        #[serde(flatten)]
        host: EvaluatedHost,
    },
}

/// Parse the JSON-lines output of `nix-eval-jobs`, one line per host.
pub fn parse_eval_jobs(output: &[u8]) -> Result<Vec<HostEval>> {
    String::from_utf8_lossy(output)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_eval_job_line)
        .collect()
}

fn parse_eval_job_line(line: &str) -> Result<HostEval> {
    let job: EvalJobLine =
        serde_json::from_str(line).map_err(|source| NixError::OutputParseFailed {
            output: line.to_string(),
            source,
        })?;

    Ok(match job {
        EvalJobLine::Failed { attr, error } => HostEval {
            host: attr,
            outcome: EvalOutcome::Failed { error },
        },
        EvalJobLine::Evaluated { attr, host } => HostEval {
            host: attr,
            outcome: EvalOutcome::Evaluated(host),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPS_EVAL_JOBS: &str = include_str!("../../tests/fixtures/plan/eval-jobs.jsonl");

    fn fixture_evals() -> Vec<HostEval> {
        parse_eval_jobs(OPS_EVAL_JOBS.as_bytes()).unwrap()
    }

    fn evaluated<'a>(evals: &'a [HostEval], host: &str) -> &'a EvaluatedHost {
        evals
            .iter()
            .find(|e| e.host == host)
            .and_then(HostEval::evaluated)
            .unwrap()
    }

    #[test]
    fn test_parse_real_eval_jobs_output() {
        let evals = fixture_evals();

        let hosts: Vec<&str> = evals.iter().map(|e| e.host.as_str()).collect();
        assert_eq!(hosts, vec!["bravo", "raven", "spruce", "kestrel"]);

        let spruce = evaluated(&evals, "spruce");
        assert_eq!(spruce.cache_status, CacheStatus::NotBuilt);
        assert_eq!(spruce.system, "x86_64-linux");
        assert_eq!(spruce.needed_builds.len(), 5);
        assert_eq!(spruce.needed_substitutes.len(), 5);
        assert!(spruce.drv_path.contains("-nixos-system-spruce-"));
    }

    #[test]
    fn test_parse_eval_failure_keeps_other_hosts() {
        let evals = fixture_evals();

        let kestrel = evals.iter().find(|e| e.host == "kestrel").unwrap();
        match &kestrel.outcome {
            EvalOutcome::Failed { error } => {
                assert!(error.contains("attribute 'doesNotExist' missing"))
            }
            other => panic!("expected eval failure, got {other:?}"),
        }
        assert_eq!(evals.len(), 4);
    }

    #[test]
    fn test_builds_include_root_when_not_built() {
        let evals = fixture_evals();
        let spruce = evaluated(&evals, "spruce");

        let builds: Vec<&str> = spruce.builds().collect();

        assert_eq!(builds.len(), 6);
        assert_eq!(builds.last().unwrap(), &spruce.drv_path);
    }

    #[test]
    fn test_builds_exclude_root_when_cached() {
        let evals = fixture_evals();
        let bravo = evaluated(&evals, "bravo");

        assert_eq!(bravo.cache_status, CacheStatus::Cached);
        assert_eq!(bravo.builds().count(), 0);
        assert_eq!(bravo.needed_substitutes.len(), 1);
    }

    #[test]
    fn test_parse_local_cache_status() {
        let line = r#"{"attr":"osprey","drvPath":"/nix/store/aaa-nixos-system-osprey.drv","system":"aarch64-linux","cacheStatus":"local","neededBuilds":[],"neededSubstitutes":[]}"#;

        let evals = parse_eval_jobs(line.as_bytes()).unwrap();

        let osprey = evaluated(&evals, "osprey");
        assert_eq!(osprey.cache_status, CacheStatus::Local);
        assert_eq!(osprey.builds().count(), 0);
    }

    #[test]
    fn test_parse_skips_blank_lines() {
        let output = format!("\n{OPS_EVAL_JOBS}\n\n");

        assert_eq!(parse_eval_jobs(output.as_bytes()).unwrap().len(), 4);
    }

    #[test]
    fn test_parse_malformed_line_fails_whole_parse() {
        let output = format!("{OPS_EVAL_JOBS}not json\n");

        assert!(parse_eval_jobs(output.as_bytes()).is_err());
    }

    #[test]
    fn test_parse_empty_output() {
        assert!(parse_eval_jobs(b"").unwrap().is_empty());
    }

    #[test]
    fn test_parse_without_cache_status_is_an_error() {
        let line = r#"{"attr":"osprey","drvPath":"/nix/store/aaa.drv","system":"x86_64-linux"}"#;

        let err = parse_eval_jobs(line.as_bytes()).unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Nix(NixError::OutputParseFailed { .. })
        ));
    }
}
