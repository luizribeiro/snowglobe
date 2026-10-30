//! Reports what building a set of hosts would build versus fetch.

mod derivation;
mod eval;
mod narinfo;

pub use derivation::{parse_derivation_show, DerivationInfo};
pub use eval::{parse_eval_jobs, CacheStatus, EvalOutcome, EvaluatedHost, HostEval};
pub use narinfo::{parse_path_info, NarInfo};

// nix 2.34 omits the store dir from some JSON keys; only the default store is supported.
const STORE_DIR: &str = "/nix/store";

fn full_store_path(key: String) -> String {
    if key.starts_with('/') {
        key
    } else {
        format!("{STORE_DIR}/{key}")
    }
}
