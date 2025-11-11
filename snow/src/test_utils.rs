use assert_cmd::Command;
use std::path::PathBuf;

/// Get the path to the test fixtures directory
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Create a snow command configured to use the test fixtures
///
/// Note: Uses deprecated cargo_bin for compatibility with unit tests.
/// Integration tests should use cargo_bin_cmd! directly.
#[allow(deprecated)]
pub fn snow_cmd() -> Command {
    let mut cmd = Command::cargo_bin("snow").unwrap();
    cmd.arg("-f").arg(fixtures_dir());
    cmd
}
