//! Ctrl-C and SIGTERM end snow at once, with the status a shell gives a
//! command killed by them, after showing the cursor again

use std::future::Future;
use tokio::signal::unix::{signal, SignalKind};

pub const INTERRUPTED: i32 = 130;
const TERMINATED: i32 = 143;

pub fn exit_on_signals() -> std::io::Result<()> {
    let status = first_signal()?;
    tokio::spawn(async move { crate::cursor::exit(status.await) });
    Ok(())
}

/// Installs the handlers before returning, so no signal is missed while
/// the future waits to be polled
fn first_signal() -> std::io::Result<impl Future<Output = i32>> {
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => INTERRUPTED,
            _ = terminate.recv() => TERMINATED,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::signal::{raise, Signal};

    async fn status_for(signal: Signal) -> i32 {
        let status = first_signal().unwrap();
        raise(signal).unwrap();
        status.await
    }

    /// One test, since a raised signal reaches every handler in the process
    #[tokio::test]
    async fn test_each_signal_gives_the_shell_status_for_it() {
        assert_eq!(status_for(Signal::SIGINT).await, 130);
        assert_eq!(status_for(Signal::SIGTERM).await, 143);
    }
}
