//! CLI observer implementation for deployment progress reporting

use snowlib::{ActivationMode, DeployResult, DeploymentObserver, Result};
use std::io::Write;

/// CLI observer that prints deployment progress to stdout
///
/// This observer provides user-friendly progress reporting during deployments:
/// - Phase start/completion notifications
/// - Per-host deployment status (✓ or ✗)
/// - Respects quiet mode for less verbose output
pub struct CliObserver {
    quiet: bool,
}

impl CliObserver {
    /// Create a new CLI observer
    ///
    /// # Arguments
    /// - `quiet`: If true, suppress per-host progress messages
    pub fn new(quiet: bool) -> Self {
        Self { quiet }
    }
}

impl DeploymentObserver for CliObserver {
    fn on_build_phase_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
        // Don't print "default" or "_all" phase explicitly
        if phase == "default" || phase == "_all" {
            println!("\n→ Building configurations for {} host(s)...", hosts.len());
        } else {
            println!(
                "\n→ Building configurations for phase '{}' ({} host(s))...",
                phase,
                hosts.len()
            );
        }
        Ok(())
    }

    fn on_build_start(&self, hostname: &str) -> Result<()> {
        if !self.quiet {
            print!("  Building {}... ", hostname);
            std::io::stdout().flush().ok();
        }
        Ok(())
    }

    fn on_build_complete(&self, _hostname: &str, success: bool) -> Result<()> {
        if !self.quiet {
            if success {
                println!("✓");
            } else {
                println!("✗");
            }
        }
        Ok(())
    }

    fn on_copy_phase_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
        // Don't print "default" or "_all" phase explicitly
        if phase == "default" || phase == "_all" {
            println!("\n→ Copying closures to {} host(s)...", hosts.len());
        } else {
            println!(
                "\n→ Copying closures for phase '{}' ({} host(s))...",
                phase,
                hosts.len()
            );
        }
        Ok(())
    }

    fn on_copy_start(&self, hostname: &str) -> Result<()> {
        if !self.quiet {
            print!("  Copying to {}... ", hostname);
            std::io::stdout().flush().ok();
        }
        Ok(())
    }

    fn on_copy_complete(&self, _hostname: &str, success: bool) -> Result<()> {
        if !self.quiet {
            if success {
                println!("✓");
            } else {
                println!("✗");
            }
        }
        Ok(())
    }

    fn on_phase_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
        // Don't print "default" or "_all" phase explicitly
        if phase == "default" || phase == "_all" {
            println!("\n→ Activating {} host(s)...", hosts.len());
        } else {
            println!(
                "\n→ Activating phase '{}' ({} host(s))...",
                phase,
                hosts.len()
            );
        }
        Ok(())
    }

    fn on_deploy_start(&self, hostname: &str, _mode: ActivationMode) -> Result<()> {
        if !self.quiet {
            print!("  {}... ", hostname);
            std::io::stdout().flush().ok();
        }
        Ok(())
    }

    fn on_deploy_complete(&self, _hostname: &str, result: &DeployResult) -> Result<()> {
        if !self.quiet {
            if result.success {
                println!("✓");
            } else {
                println!("✗");
                // Print error details on failure
                eprintln!("    Error: {}", result.message);
            }
        }
        Ok(())
    }

    fn on_phase_complete(&self, phase: &str, success: bool) -> Result<()> {
        if success {
            if phase == "default" {
                println!("✓ Phase completed successfully");
            } else {
                println!("✓ Phase '{}' completed successfully", phase);
            }
        } else if phase == "default" {
            eprintln!("✗ Phase failed, stopping deployment");
        } else {
            eprintln!("✗ Phase '{}' failed, stopping deployment", phase);
        }
        Ok(())
    }

    fn on_reboot_start(&self, hostname: &str) -> Result<()> {
        print!("  Rebooting {}...", hostname);
        std::io::stdout().flush().ok();
        Ok(())
    }

    fn on_reboot_complete(&self, _hostname: &str, success: bool) -> Result<()> {
        if success {
            println!(" ✓ Back online");
        } else {
            println!(" ✗ Failed");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_observer_creation() {
        let observer = CliObserver::new(false);
        assert!(!observer.quiet);

        let quiet_observer = CliObserver::new(true);
        assert!(quiet_observer.quiet);
    }

    #[test]
    fn test_cli_observer_phase_notifications() {
        let observer = CliObserver::new(false);

        // These just test that the methods don't panic
        assert!(observer
            .on_phase_start("test", &["host1".to_string()])
            .is_ok());
        assert!(observer.on_phase_complete("test", true).is_ok());
    }

    #[test]
    fn test_cli_observer_deploy_notifications() {
        let observer = CliObserver::new(true); // Quiet mode for tests

        assert!(observer
            .on_deploy_start("host1", ActivationMode::Switch)
            .is_ok());
        assert!(observer
            .on_deploy_complete("host1", &DeployResult::success("ok"))
            .is_ok());
    }

    #[test]
    fn test_cli_observer_reboot_notifications() {
        let observer = CliObserver::new(true); // Quiet mode for tests

        assert!(observer.on_reboot_start("host1").is_ok());
        assert!(observer.on_reboot_complete("host1", true).is_ok());
    }
}
