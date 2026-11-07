use assert_cmd::Command;
use std::path::{Path, PathBuf};

/// Get the path to the test fixtures directory
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Create a snow command configured to use the test fixtures
pub fn snow_cmd() -> Command {
    snow_cmd_on(&fixtures_dir())
}

/// A snow command on the fixture flake in `flake_dir`
///
/// Note: Uses deprecated cargo_bin for compatibility with unit tests.
/// Integration tests should use cargo_bin_cmd! directly.
#[allow(deprecated)]
pub fn snow_cmd_on(flake_dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("snow").unwrap();
    cmd.arg("-f").arg(flake_dir);
    cmd
}

/// Parses `flags` as a command's arguments would be
pub fn try_parse_args<A: clap::Args>(flags: &[&str]) -> Result<A, clap::Error> {
    #[derive(clap::Parser)]
    struct Cli<A: clap::Args> {
        #[command(flatten)]
        args: A,
    }
    <Cli<A> as clap::Parser>::try_parse_from(std::iter::once("snow").chain(flags.iter().copied()))
        .map(|cli| cli.args)
}
