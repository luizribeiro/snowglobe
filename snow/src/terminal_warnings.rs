//! Shows tracing warnings on an interactive terminal, which otherwise only
//! gets the log file

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, MakeWriter};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// Writes each warning as `! message`. Errors are left out: the command's own
/// failure report already shows them.
pub fn layer<S, W>(writer: W) -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    tracing_subscriber::fmt::layer()
        .event_format(WarningFormat)
        .with_writer(writer)
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            *metadata.level() == Level::WARN
        }))
}

struct WarningFormat;

impl<S, N> FormatEvent<S, N> for WarningFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let mut message = String::new();
        ctx.format_fields(Writer::new(&mut message), event)?;
        writeln!(writer, "{}", crate::marks::warning_lines(&message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn shown(log: impl FnOnce()) -> String {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::registry().with(layer(move || writer.clone()));
        tracing::subscriber::with_default(subscriber, log);
        let bytes = captured.0.lock().unwrap().clone();
        console::strip_ansi_codes(&String::from_utf8(bytes).unwrap()).into_owned()
    }

    #[test]
    fn test_warnings_are_shown_with_the_warning_mark() {
        let output = shown(|| {
            tracing::warn!(
                "snowglobe-activate not found on radon, falling back to direct activation"
            );
        });

        assert_eq!(
            output,
            "! snowglobe-activate not found on radon, falling back to direct activation\n"
        );
    }

    #[test]
    fn test_later_lines_of_a_warning_sit_under_its_text() {
        let output = shown(|| {
            tracing::warn!("sops updatekeys failed for secrets/radon.yaml:\nno key could decrypt")
        });

        assert_eq!(
            output,
            "! sops updatekeys failed for secrets/radon.yaml:\n  no key could decrypt\n"
        );
    }

    #[test]
    fn test_other_levels_are_left_to_the_log() {
        let output = shown(|| {
            tracing::info!("radon: build started");
            tracing::error!("radon: failed at build: error: builder for wine failed");
        });

        assert_eq!(output, "");
    }
}
