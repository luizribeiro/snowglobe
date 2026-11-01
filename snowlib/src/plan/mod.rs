//! Reports what building a set of hosts would build versus fetch.

mod derivation;
mod eval;
mod narinfo;
mod report;
mod run;
mod saved;
mod substituters;

pub use derivation::{parse_derivation_show, DerivationInfo};
pub use eval::{parse_eval_jobs, CacheStatus, EvalOutcome, EvaluatedHost, HostEval};
pub use narinfo::{parse_path_info, NarInfo};
pub use report::{
    build_report, HostReport, HostStatus, PlanReport, PlannedBuild, PlannedFetch,
    SubstituterLookup, SubstituterSummary, Totals,
};
pub use run::{create_plan, PlanTarget};
pub use saved::{last_plan_path, SavedPlan};
pub use substituters::{parse_substituters, ConfiguredSubstituter};

// nix 2.34 omits the store dir from some JSON keys; only the default store is supported.
const STORE_DIR: &str = "/nix/store";

fn full_store_path(key: String) -> String {
    if key.starts_with('/') {
        key
    } else {
        format!("{STORE_DIR}/{key}")
    }
}
