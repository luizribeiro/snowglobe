use std::time::Duration;

/// Polls `check` until it yields a value, panicking with `what` otherwise.
/// Processes may take seconds to start or be reaped on a loaded machine, so
/// the deadline is far beyond what an idle one needs.
pub async fn eventually<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(value) = check() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}"))
}
