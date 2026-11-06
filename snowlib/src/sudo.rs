//! Running commands as root on the machine snow runs on, through sudo
//!
//! sudo asks for the password at most once, before any work starts, while
//! the caller has set the terminal aside for it. A background `sudo -n -v`
//! keeps the remembered password from expiring during long builds, and
//! commands run with `sudo -n`, so sudo never asks under a redrawing view.

use crate::error::{DeployError, Result};
use crate::process::r#async::run_command;
use crate::process::{CommandResult, OutputMode};
use std::io::IsTerminal;
use std::sync::Arc;
use std::time::Duration;

/// Well under sudo's default `timestamp_timeout` of 5 minutes
const KEEPALIVE_EVERY: Duration = Duration::from_secs(60);

/// Why sudo is handed the terminal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SudoPrompt {
    /// sudo asks for the password, then remembers it
    Password,
    /// sudo remembers no password, so it may ask while the command runs
    Command,
}

/// Whatever draws on the terminal: it stops between `pause` and `resume`,
/// so sudo can ask for the password there
pub trait SudoTerminal: Send + Sync {
    fn pause(&self, prompt: SudoPrompt);
    fn resume(&self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// snow runs as root, so commands run as they are
    Root,
    /// `sudo -n` runs commands without asking
    Remembered,
    /// sudo remembers no password (`timestamp_timeout=0`), so every command
    /// may ask for it
    Forgetful,
}

/// How to call sudo: the program, then any arguments before sudo's own
#[derive(Debug, Clone)]
struct Program(Vec<String>);

impl Program {
    fn split(&self) -> (&str, impl Iterator<Item = &str>) {
        let (program, leading) = self.0.split_first().expect("a program to run");
        (program, leading.iter().map(String::as_str))
    }

    async fn run(&self, args: &[&str], output_mode: OutputMode) -> std::io::Result<CommandResult> {
        let (program, leading) = self.split();
        let args: Vec<&str> = leading.chain(args.iter().copied()).collect();
        run_command(program, &args, output_mode, None, None, &[]).await
    }

    async fn succeeds(&self, args: &[&str]) -> std::io::Result<bool> {
        Ok(self.run(args, OutputMode::Capture).await?.status.success())
    }

    /// On the terminal, which sudo asks for the password on
    async fn ask(&self, args: &[&str]) -> std::io::Result<std::process::ExitStatus> {
        let (program, leading) = self.split();
        tokio::process::Command::new(program)
            .args(leading)
            .args(args)
            .status()
            .await
    }
}

struct Options {
    program: Program,
    as_root: bool,
    has_terminal: bool,
    keepalive_every: Duration,
}

impl Options {
    fn detect() -> Self {
        Self {
            program: Program(vec!["sudo".to_string()]),
            as_root: nix::unistd::geteuid().is_root(),
            has_terminal: std::io::stdin().is_terminal(),
            keepalive_every: KEEPALIVE_EVERY,
        }
    }
}

/// Stops refreshing sudo's timestamp when dropped
struct Keepalive(tokio::task::JoinHandle<()>);

impl Keepalive {
    fn start(program: Program, every: Duration) -> Self {
        Self(tokio::spawn(async move {
            let mut ticks = tokio::time::interval(every);
            ticks.tick().await;
            loop {
                ticks.tick().await;
                // A failed refresh is caught before the next command
                let _ = program.succeeds(&["-n", "-v"]).await;
            }
        }))
    }
}

impl Drop for Keepalive {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct Sudo {
    access: Access,
    options: Options,
    terminal: Arc<dyn SudoTerminal>,
    asking: tokio::sync::Mutex<()>,
    _keepalive: Option<Keepalive>,
}

impl Sudo {
    /// Makes sure sudo will run commands, asking for the password on the
    /// terminal if it needs one
    pub async fn start(terminal: Arc<dyn SudoTerminal>) -> Result<Self> {
        Self::start_with(Options::detect(), terminal).await
    }

    async fn start_with(options: Options, terminal: Arc<dyn SudoTerminal>) -> Result<Self> {
        let mut sudo = Self {
            access: Access::Root,
            options,
            terminal,
            asking: tokio::sync::Mutex::new(()),
            _keepalive: None,
        };
        if sudo.options.as_root {
            return Ok(sudo);
        }
        let remembered = sudo.runs_without_asking().await? || {
            sudo.ask_password().await?;
            sudo.runs_without_asking().await?
        };
        if remembered {
            sudo.access = Access::Remembered;
            sudo._keepalive = Some(Keepalive::start(
                sudo.options.program.clone(),
                sudo.options.keepalive_every,
            ));
        } else {
            tracing::info!("sudo -n failed right after sudo -v, so sudo may ask at every step");
            sudo.access = Access::Forgetful;
        }
        Ok(sudo)
    }

    /// Runs `command` as root. Only Inherit output shows what sudo asks
    /// while it runs, so a forgetful sudo turns Display into Inherit.
    ///
    /// The outer error is sudo's, with its own help; the inner one is
    /// starting the command, which callers word for what it does.
    pub async fn run(
        &self,
        command: &[&str],
        output_mode: OutputMode,
    ) -> Result<std::io::Result<CommandResult>> {
        let Some((program, args)) = command.split_first() else {
            return Ok(Err(std::io::Error::other("no command to run as root")));
        };
        Ok(match self.access {
            Access::Root => run_command(program, args, output_mode, None, None, &[]).await,
            Access::Remembered => {
                self.refresh().await?;
                let args: Vec<&str> = ["-n"].iter().chain(command).copied().collect();
                self.options.program.run(&args, output_mode).await
            }
            Access::Forgetful => {
                let _paused = Paused::new(self.terminal.as_ref(), SudoPrompt::Command);
                let output_mode = shown_with_prompts(output_mode);
                self.options.program.run(command, output_mode).await
            }
        })
    }

    /// A user may run commands without a password yet need one for
    /// `sudo -v`, so a failed refresh asks only if commands need it too
    async fn refresh(&self) -> Result<()> {
        // Commands running in parallel would otherwise each ask
        let _asking = self.asking.lock().await;
        if self.succeeds(&["-n", "-v"]).await? || self.runs_without_asking().await? {
            return Ok(());
        }
        tracing::info!("sudo forgot the password");
        self.ask_password().await
    }

    async fn runs_without_asking(&self) -> Result<bool> {
        self.succeeds(&["-n", "true"]).await
    }

    async fn ask_password(&self) -> Result<()> {
        if !self.options.has_terminal {
            return Err(DeployError::SudoNeedsTerminal.into());
        }
        let validated = {
            let _paused = Paused::new(self.terminal.as_ref(), SudoPrompt::Password);
            self.options.program.ask(&["-v"]).await
        };
        match validated {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(refused(std::io::Error::other(format!(
                "`sudo -v` exited with {status}"
            )))),
            Err(source) => Err(refused(source)),
        }
    }

    async fn succeeds(&self, args: &[&str]) -> Result<bool> {
        self.options.program.succeeds(args).await.map_err(refused)
    }
}

/// Resumes the terminal however the wait for sudo ends, cancelled too
struct Paused<'a>(&'a dyn SudoTerminal);

impl<'a> Paused<'a> {
    fn new(terminal: &'a dyn SudoTerminal, prompt: SudoPrompt) -> Self {
        terminal.pause(prompt);
        Self(terminal)
    }
}

impl Drop for Paused<'_> {
    fn drop(&mut self) {
        self.0.resume();
    }
}

/// Display output goes to the log, where nobody sees sudo ask
fn shown_with_prompts(output_mode: OutputMode) -> OutputMode {
    match output_mode {
        OutputMode::Display => OutputMode::Inherit,
        other => other,
    }
}

fn refused(source: std::io::Error) -> crate::error::SnowError {
    DeployError::SudoRefused { source }.into()
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::*;
    use parking_lot::Mutex;

    /// A sudo that remembers the password in `remembers`, forgets it for
    /// good while `forgets` exists, refuses it while `refuses` exists, and
    /// fails `sudo -n -v` while `validate-asks` exists. It logs every call
    /// to `calls`.
    pub struct FakeSudo {
        pub dir: tempfile::TempDir,
    }

    impl FakeSudo {
        pub fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let script = format!(
                r#"dir='{}'
echo "$*" >> "$dir/calls"
if [ "$1" = -v ]; then
  [ -e "$dir/refuses" ] && exit 1
  [ -e "$dir/forgets" ] || touch "$dir/remembers"
  exit 0
fi
if [ "$1" = -n ]; then
  shift
  [ -e "$dir/remembers" ] || {{ echo "sudo: a password is required" >&2; exit 1; }}
  if [ "$1" = -v ]; then
    [ -e "$dir/validate-asks" ] && exit 1
    exit 0
  fi
fi
exec "$@"
"#,
                dir.path().display()
            );
            // Read by sh rather than run: exec fails on a file that a test
            // forking in parallel still holds open for writing
            std::fs::write(dir.path().join("sudo"), script).unwrap();
            Self { dir }
        }

        pub fn with(self, flag: &str) -> Self {
            self.set(flag);
            self
        }

        pub fn set(&self, flag: &str) {
            std::fs::write(self.path(flag), "").unwrap();
        }

        pub fn unset(&self, flag: &str) {
            let _ = std::fs::remove_file(self.path(flag));
        }

        pub fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.path("calls"))
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect()
        }

        fn path(&self, name: &str) -> std::path::PathBuf {
            self.dir.path().join(name)
        }

        pub(in crate::sudo) fn options(&self, as_root: bool, has_terminal: bool) -> Options {
            Options {
                program: Program(vec![
                    "sh".to_string(),
                    self.path("sudo").to_string_lossy().into_owned(),
                ]),
                as_root,
                has_terminal,
                keepalive_every: KEEPALIVE_EVERY,
            }
        }
    }

    /// Records when sudo was handed the terminal
    #[derive(Default)]
    pub struct Pauses(pub Mutex<Vec<String>>);

    impl SudoTerminal for Pauses {
        fn pause(&self, prompt: SudoPrompt) {
            self.0.lock().push(format!("pause {prompt:?}"));
        }

        fn resume(&self) {
            self.0.lock().push("resume".to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_utils::{FakeSudo, Pauses};
    use super::*;

    async fn start(
        fake: &FakeSudo,
        as_root: bool,
        has_terminal: bool,
    ) -> (Result<Sudo>, Arc<Pauses>) {
        let pauses = Arc::new(Pauses::default());
        let sudo = Sudo::start_with(fake.options(as_root, has_terminal), pauses.clone()).await;
        (sudo, pauses)
    }

    fn pauses(pauses: &Pauses) -> Vec<String> {
        pauses.0.lock().clone()
    }

    async fn stdout_of(sudo: &Sudo, command: &[&str]) -> String {
        let result = sudo
            .run(command, OutputMode::Capture)
            .await
            .unwrap()
            .unwrap();
        assert!(result.status.success());
        String::from_utf8_lossy(&result.stdout).trim().to_string()
    }

    #[tokio::test]
    async fn test_root_runs_commands_without_sudo() {
        let fake = FakeSudo::new();
        let (sudo, pauses) = start(&fake, true, true).await;
        let sudo = sudo.unwrap();

        assert_eq!(stdout_of(&sudo, &["echo", "switch"]).await, "switch");
        assert!(fake.calls().is_empty());
        assert!(self::pauses(&pauses).is_empty());
    }

    #[tokio::test]
    async fn test_passwordless_sudo_never_takes_the_terminal() {
        let fake = FakeSudo::new().with("remembers");
        let (sudo, pauses) = start(&fake, false, false).await;
        let sudo = sudo.unwrap();

        assert_eq!(stdout_of(&sudo, &["echo", "switch"]).await, "switch");
        assert!(self::pauses(&pauses).is_empty());
        assert_eq!(fake.calls(), vec!["-n true", "-n -v", "-n echo switch"]);
    }

    #[tokio::test]
    async fn test_password_is_asked_once_before_the_commands() {
        let fake = FakeSudo::new();
        let (sudo, pauses) = start(&fake, false, true).await;
        let sudo = sudo.unwrap();

        assert_eq!(
            stdout_of(&sudo, &["echo", "dry-activate"]).await,
            "dry-activate"
        );
        assert_eq!(stdout_of(&sudo, &["echo", "switch"]).await, "switch");
        assert_eq!(self::pauses(&pauses), vec!["pause Password", "resume"]);
        assert_eq!(fake.calls().iter().filter(|call| *call == "-v").count(), 1);
        assert!(fake.calls().contains(&"-n echo dry-activate".to_string()));
    }

    #[tokio::test]
    async fn test_password_is_asked_again_once_sudo_forgot_it() {
        let fake = FakeSudo::new();
        let (sudo, pauses) = start(&fake, false, true).await;
        let sudo = sudo.unwrap();

        fake.unset("remembers");
        assert_eq!(stdout_of(&sudo, &["echo", "switch"]).await, "switch");

        assert_eq!(
            self::pauses(&pauses),
            vec!["pause Password", "resume", "pause Password", "resume"]
        );
        assert_eq!(fake.calls().last().unwrap(), "-n echo switch");
    }

    #[tokio::test]
    async fn test_sudo_that_remembers_nothing_runs_commands_on_the_terminal() {
        let fake = FakeSudo::new().with("forgets");
        let (sudo, pauses) = start(&fake, false, true).await;
        let sudo = sudo.unwrap();

        assert_eq!(stdout_of(&sudo, &["echo", "switch"]).await, "switch");

        assert_eq!(
            self::pauses(&pauses),
            vec!["pause Password", "resume", "pause Command", "resume"]
        );
        assert_eq!(fake.calls().last().unwrap(), "echo switch");
    }

    #[tokio::test]
    async fn test_password_needed_without_a_terminal_fails_before_asking() {
        let fake = FakeSudo::new();
        let (sudo, pauses) = start(&fake, false, false).await;

        let error = sudo.err().unwrap();
        assert!(matches!(
            error,
            crate::error::SnowError::Deploy(DeployError::SudoNeedsTerminal)
        ));
        assert!(self::pauses(&pauses).is_empty());
        assert!(!fake.calls().contains(&"-v".to_string()));
    }

    #[tokio::test]
    async fn test_refused_password_fails_the_start() {
        let fake = FakeSudo::new().with("refuses");
        let (sudo, pauses) = start(&fake, false, true).await;

        let error = sudo.err().unwrap();
        assert!(matches!(
            error,
            crate::error::SnowError::Deploy(DeployError::SudoRefused { .. })
        ));
        assert_eq!(self::pauses(&pauses), vec!["pause Password", "resume"]);
    }

    #[tokio::test]
    async fn test_commands_that_need_no_password_skip_a_failed_refresh() {
        let fake = FakeSudo::new().with("remembers");
        let (sudo, pauses) = start(&fake, false, true).await;
        let sudo = sudo.unwrap();
        // As for a user with NOPASSWD for everything but also an entry that
        // needs a password, for which `sudo -v` asks
        fake.set("validate-asks");

        assert_eq!(stdout_of(&sudo, &["echo", "switch"]).await, "switch");
        assert!(self::pauses(&pauses).is_empty());
        assert!(!fake.calls().contains(&"-v".to_string()));
    }

    #[tokio::test]
    async fn test_forgotten_password_without_a_terminal_fails_with_sudo_s_error() {
        let fake = FakeSudo::new().with("remembers");
        let (sudo, _) = start(&fake, false, false).await;
        let sudo = sudo.unwrap();

        fake.unset("remembers");
        let error = sudo
            .run(&["echo", "switch"], OutputMode::Capture)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            crate::error::SnowError::Deploy(DeployError::SudoNeedsTerminal)
        ));
        assert!(!fake.calls().contains(&"-n echo switch".to_string()));
    }

    #[tokio::test]
    async fn test_root_reports_a_failed_command_and_an_empty_one() {
        let fake = FakeSudo::new();
        let (sudo, _) = start(&fake, true, true).await;
        let sudo = sudo.unwrap();

        let failed = sudo.run(&["false"], OutputMode::Capture).await.unwrap();
        let empty = sudo.run(&[], OutputMode::Capture).await.unwrap();

        assert!(!failed.unwrap().status.success());
        assert!(empty.is_err());
    }

    #[test]
    fn test_sudo_that_may_ask_shows_display_output_on_the_terminal() {
        assert_eq!(shown_with_prompts(OutputMode::Display), OutputMode::Inherit);
        assert_eq!(shown_with_prompts(OutputMode::Capture), OutputMode::Capture);
        assert_eq!(shown_with_prompts(OutputMode::Inherit), OutputMode::Inherit);
    }

    #[tokio::test]
    async fn test_keepalive_refreshes_until_sudo_is_dropped() {
        let fake = FakeSudo::new().with("remembers");
        let options = Options {
            keepalive_every: Duration::from_millis(50),
            ..fake.options(false, false)
        };
        let sudo = Sudo::start_with(options, Arc::new(Pauses::default()))
            .await
            .unwrap();
        let refreshes = || fake.calls().iter().filter(|call| *call == "-n -v").count();

        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(sudo);
        let at_drop = refreshes();
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(at_drop >= 2, "only {at_drop} refreshes");
        assert!(refreshes() <= at_drop + 1, "kept refreshing after the drop");
    }
}
