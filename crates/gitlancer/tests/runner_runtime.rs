mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gitlancer::{
    CliGitRunner, GitCommand, GitEnv, GitExecError, GitIntent, GitRunner, GitlancerLogger, logging,
};
use pretty_assertions::assert_eq;

use common::TestScaffold;

#[derive(Debug, Clone, PartialEq, Eq)]
enum LogEvent {
    Command { cwd: PathBuf, command: String },
    Result { success: bool },
}

#[derive(Clone)]
struct RecordingLogger {
    events: Arc<Mutex<Vec<LogEvent>>>,
}

impl GitlancerLogger for RecordingLogger {
    /// Records command metadata so integration tests verify the public observability contract.
    fn log_command(&self, cwd: &Path, command: &str) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LogEvent::Command {
                cwd: cwd.to_path_buf(),
                command: command.to_string(),
            });
    }

    /// Records completion state while deliberately ignoring platform-specific killed exit codes.
    fn log_result(&self, _duration_ms: u64, success: bool, _exit_code: Option<i32>) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LogEvent::Result { success });
    }
}

/// Builds a real CLI command rooted in the isolated integration-test repository.
fn command(scaffold: &TestScaffold, args: &[&str]) -> GitCommand {
    GitCommand::new(
        scaffold.repo_path().to_path_buf(),
        args.iter().map(ToString::to_string).collect(),
        GitEnv::default(),
        GitIntent::ReadOnly,
    )
}

/// Verifies real Git execution enforces stream limits and deadlines, kills descendants, and logs every outcome.
#[test]
fn cli_runner_enforces_runtime_guards_and_logs_outcomes() {
    let scaffold = TestScaffold::new("runner-runtime").expect("sandbox should initialize");
    let events = Arc::new(Mutex::new(Vec::new()));
    logging::register(RecordingLogger {
        events: Arc::clone(&events),
    });

    let runner = CliGitRunner::new(Duration::from_secs(2));
    let success = command(&scaffold, &["rev-parse", "--is-inside-work-tree"]);
    let output = runner
        .run(&success)
        .expect("real git command should succeed");
    assert_eq!(output.stdout.trim(), "true");

    let stdout_overflow = command(&scaffold, &["-c", "alias.emit=!printf 0123456789", "emit"]);
    assert!(matches!(
        runner.run_bounded(&stdout_overflow, 4, 1024),
        Err(GitExecError::OutputTooLarge {
            stream: "stdout",
            limit: 4
        })
    ));

    let stderr_overflow = command(
        &scaffold,
        &["-c", "alias.emiterr=!printf 0123456789 >&2", "emiterr"],
    );
    assert!(matches!(
        runner.run_bounded(&stderr_overflow, 1024, 4),
        Err(GitExecError::OutputTooLarge {
            stream: "stderr",
            limit: 4
        })
    ));

    let timeout_runner = CliGitRunner::new(Duration::from_millis(75));
    let timeout = command(
        &scaffold,
        &[
            "-c",
            "alias.wait=!f() { sleep 1; printf leaked > tree-leak-marker; }; f",
            "wait",
        ],
    );
    assert!(matches!(
        timeout_runner.run(&timeout),
        Err(GitExecError::TimedOut { timeout_ms: 75, .. })
    ));

    // A direct-child kill would leave the alias shell alive long enough to create this marker.
    std::thread::sleep(Duration::from_millis(1_200));
    assert!(!scaffold.repo_path().join("tree-leak-marker").exists());

    let inherited_pipe_timeout = command(
        &scaffold,
        &[
            "-c",
            "alias.detach=!f() { sleep 1; printf leaked > inherited-pipe-leak-marker; }; f &",
            "detach",
        ],
    );
    let inherited_pipe_started_at = std::time::Instant::now();
    assert!(matches!(
        timeout_runner.run(&inherited_pipe_timeout),
        Err(GitExecError::TimedOut { timeout_ms: 75, .. })
    ));
    assert!(
        inherited_pipe_started_at.elapsed() < Duration::from_millis(750),
        "the runner must not wait for a detached descendant to close inherited pipes"
    );
    std::thread::sleep(Duration::from_millis(1_200));
    assert!(
        !scaffold
            .repo_path()
            .join("inherited-pipe-leak-marker")
            .exists(),
        "deadline termination must include descendants after the direct Git child exits"
    );

    assert_eq!(
        events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        vec![
            LogEvent::Command {
                cwd: scaffold.repo_path().to_path_buf(),
                command: "git rev-parse --is-inside-work-tree".to_string(),
            },
            LogEvent::Result { success: true },
            LogEvent::Command {
                cwd: scaffold.repo_path().to_path_buf(),
                command: "git -c alias.emit=!printf 0123456789 emit".to_string(),
            },
            LogEvent::Result { success: false },
            LogEvent::Command {
                cwd: scaffold.repo_path().to_path_buf(),
                command: "git -c alias.emiterr=!printf 0123456789 >&2 emiterr".to_string(),
            },
            LogEvent::Result { success: false },
            LogEvent::Command {
                cwd: scaffold.repo_path().to_path_buf(),
                command:
                    "git -c alias.wait=!f() { sleep 1; printf leaked > tree-leak-marker; }; f wait"
                        .to_string(),
            },
            LogEvent::Result { success: false },
            LogEvent::Command {
                cwd: scaffold.repo_path().to_path_buf(),
                command: "git -c alias.detach=!f() { sleep 1; printf leaked > inherited-pipe-leak-marker; }; f & detach"
                    .to_string(),
            },
            LogEvent::Result { success: false },
        ]
    );
}
