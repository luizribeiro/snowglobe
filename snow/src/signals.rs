//! Ctrl-C and SIGTERM end snow at once, with the status a shell gives a
//! command killed by them, after showing the cursor again and saying what
//! the command leaves unfinished

use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::signal::unix::{signal, Signal, SignalKind};

pub const INTERRUPTED: i32 = 130;
const TERMINATED: i32 = 143;

/// Plenty for a few lines; past it, stderr is stalled (a terminal paused
/// with Ctrl-S, a full pipe) and snow exits without the report
const REPORT_TIMEOUT: Duration = Duration::from_secs(2);

type ExitReport = Box<dyn Fn() -> Vec<String> + Send + Sync>;

static EXIT_REPORT: OnceLock<ExitReport> = OnceLock::new();

/// Lines to show, and log, when a signal ends snow. snow runs one command,
/// so the first report set is the only one.
pub fn report_on_exit(report: impl Fn() -> Vec<String> + Send + Sync + 'static) {
    let _ = EXIT_REPORT.set(Box::new(report));
}

fn exit_report() -> Vec<String> {
    EXIT_REPORT.get().map(|report| report()).unwrap_or_default()
}

/// Without a log file tracing writes to stderr, so logging the report too
/// would print it twice
pub fn exit_on_signals(logs_to_file: bool) -> std::io::Result<()> {
    let mut signals = Signals::install()?;
    tokio::spawn(async move {
        let status = signals.next().await;
        let reported = report_and_exit(status, logs_to_file);
        // A second signal means the user won't wait for the report
        tokio::select! {
            _ = reported => {}
            _ = signals.next() => {}
            _ = tokio::time::sleep(REPORT_TIMEOUT) => {}
        }
        exit(status)
    });
    Ok(())
}

/// On a thread of its own, since stderr may block. Its receiver resolves
/// only if the thread ends without exiting, as when building the report
/// panics.
fn report_and_exit(status: i32, logs_to_file: bool) -> tokio::sync::oneshot::Receiver<()> {
    let (done, reported) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        let _done = done;
        let lines = exit_report();
        if logs_to_file {
            for line in &lines {
                tracing::info!("{line}");
            }
        }
        // Holding the progress bars keeps a redraw from drawing over the report
        tracing_indicatif::suspend_tracing_indicatif(|| {
            crate::cursor::Cursor::stderr().show();
            write_report(std::io::stderr(), &lines);
            exit(status)
        })
    });
    reported
}

/// Safe to call from two threads at once, which `std::process::exit` isn't
fn exit(status: i32) -> ! {
    static EXITING: Mutex<()> = Mutex::new(());
    let _exiting = EXITING.lock();
    std::process::exit(status)
}

/// Ends with a blank line, like every block snow prints. A failed write
/// can't be reported anywhere this close to exiting.
fn write_report(mut out: impl Write, lines: &[String]) {
    if !lines.is_empty() {
        let _ = writeln!(out, "{}\n", lines.join("\n"));
        let _ = out.flush();
    }
}

struct Signals {
    interrupt: Signal,
    terminate: Signal,
}

impl Signals {
    /// Installs the handlers before returning, so no signal is missed while
    /// waiting to be polled
    fn install() -> std::io::Result<Self> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }

    /// The exit status for the next signal
    async fn next(&mut self) -> i32 {
        tokio::select! {
            _ = self.interrupt.recv() => INTERRUPTED,
            _ = self.terminate.recv() => TERMINATED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::signal::{raise, Signal};

    #[test]
    fn test_report_ends_with_a_blank_line() {
        let mut out = Vec::new();

        write_report(
            &mut out,
            &["Interrupted.".to_string(), "  web1".to_string()],
        );

        assert_eq!(String::from_utf8(out).unwrap(), "Interrupted.\n  web1\n\n");
    }

    #[test]
    fn test_no_report_writes_nothing() {
        let mut out = Vec::new();

        write_report(&mut out, &[]);

        assert!(out.is_empty());
    }

    /// One test, since the report can be set once per process
    #[test]
    fn test_exit_report_is_the_first_one_set() {
        report_on_exit(|| vec!["Interrupted.".to_string()]);
        report_on_exit(|| vec!["Later.".to_string()]);

        assert_eq!(exit_report(), ["Interrupted."]);
    }

    /// One test, since a raised signal reaches every handler in the process
    #[tokio::test]
    async fn test_each_signal_gives_the_shell_status_for_it() {
        let mut signals = Signals::install().unwrap();

        raise(Signal::SIGINT).unwrap();
        assert_eq!(signals.next().await, 130);
        raise(Signal::SIGTERM).unwrap();
        assert_eq!(signals.next().await, 143);
    }
}
