//! Reports what building a set of hosts would build versus fetch.

mod eval;

pub use eval::{parse_eval_jobs, CacheStatus, EvalOutcome, EvaluatedHost, HostEval};
