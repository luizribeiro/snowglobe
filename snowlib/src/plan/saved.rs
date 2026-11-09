use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::PlanReport;
use crate::error::{IoError, Result};

/// A plan report kept on disk so it can be shown again without re-evaluating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPlan {
    /// Unix timestamp, in seconds.
    pub saved_at: u64,
    pub flake: PathBuf,
    /// Git revision of the flake, suffixed with `-dirty` for uncommitted changes.
    pub revision: Option<String>,
    pub report: PlanReport,
}

/// `$XDG_STATE_HOME/snowglobe/last-plan.json`, falling back to `~/.local/state`.
pub fn last_plan_path(xdg_state_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let state_home = match (xdg_state_home, home) {
        (Some(dir), _) if !dir.is_empty() => PathBuf::from(dir),
        (_, Some(home)) if !home.is_empty() => Path::new(home).join(".local/state"),
        _ => return None,
    };
    Some(state_home.join("snowglobe/last-plan.json"))
}

fn state_file_error(path: &Path) -> impl Fn(std::io::Error) -> IoError + '_ {
    move |source| IoError::StateFile {
        path: path.to_path_buf(),
        source,
    }
}

impl SavedPlan {
    pub fn save(&self, path: &Path) -> Result<()> {
        let json =
            serde_json::to_vec_pretty(self).map_err(|source| IoError::JsonSerialize { source })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(state_file_error(path))?;
        }
        // Written beside the target and renamed so a reader never sees a partial
        // file; the pid keeps concurrent runs from writing the same temporary file.
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        let written = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, path));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written.map_err(state_file_error(path))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => IoError::NoSavedPlan {
                path: path.to_path_buf(),
            },
            _ => state_file_error(path)(e),
        })?;
        Ok(
            serde_json::from_slice(&contents).map_err(|source| IoError::InvalidSavedPlan {
                path: path.to_path_buf(),
                source,
            })?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{build_report, parse_eval_jobs, DerivationInfo, NarInfo, SubstituterLookup};
    use std::collections::HashMap;

    const OPS_EVAL_JOBS: &str = include_str!("../../tests/fixtures/plan/eval-jobs.jsonl");

    fn ops_plan() -> SavedPlan {
        let evals = parse_eval_jobs(OPS_EVAL_JOBS.as_bytes()).unwrap();
        let derivations = vec![DerivationInfo {
            drv_path: "/nix/store/my8fv7s5hvjgjqfa3j18bajh4hymlzz0-stage-2-init.sh.drv".to_string(),
            name: "stage-2-init.sh".to_string(),
            pname: None,
            version: None,
            system: "x86_64-linux".to_string(),
            trivial: true,
        }];
        let substituters = vec![SubstituterLookup {
            url: "https://cache.nixos.org".to_string(),
            priority: 40,
            reachable: true,
            paths: HashMap::from([(
                "/nix/store/cni8h48r0c96xjlndndabai63hz9dmzr-die-hook".to_string(),
                NarInfo {
                    download_size: Some(1_000),
                    nar_size: 4_000,
                },
            )]),
        }];
        SavedPlan {
            saved_at: 1_793_262_060,
            flake: PathBuf::from("/home/alice/ops"),
            revision: Some("f6b7cddd0ffa200cc14834f3aa516908095dcb94".to_string()),
            report: build_report(&evals, &derivations, &substituters),
        }
    }

    #[test]
    fn test_save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snowglobe/last-plan.json");
        let plan = ops_plan();

        plan.save(&path).unwrap();

        let loaded = SavedPlan::load(&path).unwrap();
        assert_eq!(loaded, plan);
        assert!(loaded.report.has_eval_failures());
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn test_save_replaces_previous_plan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("last-plan.json");
        let mut plan = ops_plan();
        plan.save(&path).unwrap();

        plan.revision = Some("e80c5956d4c9fbe7b4d44cbedeacebd89af75663-dirty".to_string());
        plan.save(&path).unwrap();

        assert_eq!(SavedPlan::load(&path).unwrap().revision, plan.revision);
    }

    #[test]
    fn test_round_trip_without_revision_or_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("last-plan.json");
        let plan = SavedPlan {
            revision: None,
            report: build_report(&[], &[], &[]),
            ..ops_plan()
        };

        plan.save(&path).unwrap();

        assert_eq!(SavedPlan::load(&path).unwrap(), plan);
    }

    #[test]
    fn test_save_fails_when_state_dir_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("snowglobe"), b"not a directory").unwrap();

        let err = ops_plan()
            .save(&dir.path().join("snowglobe/last-plan.json"))
            .unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Io(IoError::StateFile { .. })
        ));
    }

    #[test]
    fn test_load_directory_is_a_state_file_error() {
        let dir = tempfile::tempdir().unwrap();

        let err = SavedPlan::load(dir.path()).unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Io(IoError::StateFile { .. })
        ));
    }

    #[test]
    fn test_load_missing_plan() {
        let dir = tempfile::tempdir().unwrap();

        let err = SavedPlan::load(&dir.path().join("last-plan.json")).unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Io(IoError::NoSavedPlan { .. })
        ));
    }

    #[test]
    fn test_load_corrupt_plan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("last-plan.json");
        std::fs::write(&path, b"{\"savedAt\": 1793262060, \"report\":").unwrap();

        let err = SavedPlan::load(&path).unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Io(IoError::InvalidSavedPlan { .. })
        ));
    }

    #[test]
    fn test_last_plan_path_prefers_xdg_state_home() {
        assert_eq!(
            last_plan_path(Some("/var/lib/ci/state"), Some("/home/alice")),
            Some(PathBuf::from("/var/lib/ci/state/snowglobe/last-plan.json"))
        );
    }

    #[test]
    fn test_last_plan_path_falls_back_to_home_when_xdg_is_empty() {
        assert_eq!(
            last_plan_path(Some(""), Some("/home/alice")),
            Some(PathBuf::from(
                "/home/alice/.local/state/snowglobe/last-plan.json"
            ))
        );
    }

    #[test]
    fn test_last_plan_path_without_xdg_or_home() {
        assert_eq!(last_plan_path(None, None), None);
        assert_eq!(last_plan_path(None, Some("")), None);
    }
}
