//! Progress bar styling for tracing-indicatif spans
//!
//! Extracted from observer.rs to separate presentation concerns from
//! lifecycle management. All ProgressStyle definitions live here.

use tracing_indicatif::style::ProgressStyle;

pub(super) const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Clone, Copy)]
pub(super) enum StepColor {
    Yellow,
    Blue,
    Cyan,
    Magenta,
}

impl StepColor {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            StepColor::Yellow => "yellow",
            StepColor::Blue => "blue",
            StepColor::Cyan => "cyan",
            StepColor::Magenta => "magenta",
        }
    }
}

pub(super) fn phase_style(success: bool, show_progress_bar: bool) -> ProgressStyle {
    let color = if success { "green" } else { "red" };
    let template = if show_progress_bar {
        format!(
            "{{spinner:.{color}.bold}} {{span_name:.{color}.bold}} {{wide_msg:.dim}} {{bar:20.{color}/dim}} {{percent}}% {{elapsed_precise:.dim}}"
        )
    } else {
        format!(
            "{{spinner:.{color}.bold}} {{span_name:.{color}.bold}} {{wide_msg:.dim}} {{elapsed_precise:.dim}}"
        )
    };
    ProgressStyle::default_bar()
        .template(&template)
        .expect("valid template")
        .tick_strings(SPINNER_FRAMES)
        .progress_chars("━╸─")
}

pub(super) fn step_style(color: StepColor, active: bool) -> ProgressStyle {
    let template = if active {
        format!("  {{span_name:.{}.bold}} {{wide_msg:.dim}}", color.as_str())
    } else {
        "  {span_name:.dim}".to_string()
    };
    ProgressStyle::default_bar()
        .template(&template)
        .expect("valid template")
}

pub(super) fn host_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("      {msg} {elapsed:.dim}")
        .expect("valid template")
}

pub(super) fn failed_step_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("  {span_name:.red.bold} {wide_msg:.red.dim}")
        .expect("valid template")
}

pub(super) fn failed_host_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("      {msg:.red} {elapsed:.dim}")
        .expect("valid template")
}
