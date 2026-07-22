use ora_process::ProcessTree;
use std::io::{ErrorKind, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use std::time::Instant;

use crate::error::GitExecError;
use crate::exec::command::GitCommand;
use crate::exec::output::GitOutput;
use crate::logging;

const DEFAULT_MAX_STDOUT_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_STDERR_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_GIT_TIMEOUT: Duration = Duration::from_secs(300);

/// Identifies why a running Git process had to be terminated before its natural exit.
enum ForcedTermination {
    OutputTooLarge { stream: &'static str, limit: usize },
    TimedOut,
}

/// Executes one prepared Git command and returns normalized process output.
pub trait GitRunner {
    /// Runs a fully assembled Git command so use-case layers stay independent from process details.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError>;

    /// Runs a command while bounding both captured streams before they enter application memory.
    fn run_bounded(
        &self,
        command: &GitCommand,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Result<GitOutput, GitExecError> {
        let output = self.run(command)?;
        if output.stdout.len() > max_stdout_bytes {
            return Err(GitExecError::OutputTooLarge {
                stream: "stdout",
                limit: max_stdout_bytes,
            });
        }
        if output.stderr.len() > max_stderr_bytes {
            return Err(GitExecError::OutputTooLarge {
                stream: "stderr",
                limit: max_stderr_bytes,
            });
        }
        Ok(output)
    }
}

/// Executes Git commands through the system `git` binary.
#[derive(Debug, Clone, Copy)]
pub struct CliGitRunner {
    timeout: Duration,
}

impl CliGitRunner {
    /// Creates a CLI runner with a finite deadline so hooks and subprocesses cannot block Ora forever.
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Default for CliGitRunner {
    /// Uses a conservative five-minute deadline for local repository operations.
    fn default() -> Self {
        Self::new(DEFAULT_GIT_TIMEOUT)
    }
}

impl GitRunner for CliGitRunner {
    /// Spawns the Git CLI with stable automation defaults so upper layers can trust execution semantics.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        run_cli(
            command,
            (DEFAULT_MAX_STDOUT_BYTES, DEFAULT_MAX_STDERR_BYTES),
            self.timeout,
        )
    }

    /// Captures Git output concurrently so neither pipe can deadlock or grow without a bound.
    fn run_bounded(
        &self,
        command: &GitCommand,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Result<GitOutput, GitExecError> {
        run_cli(command, (max_stdout_bytes, max_stderr_bytes), self.timeout)
    }
}

/// Executes one CLI command with a deadline and per-stream capture limits.
fn run_cli(
    command: &GitCommand,
    limits: (usize, usize),
    timeout: Duration,
) -> Result<GitOutput, GitExecError> {
    let logger = logging::get();
    let started_at = Instant::now();

    let full_command = format!("git {}", command.args.join(" "));
    if let Some(ref l) = logger {
        l.log_command(&command.cwd, &full_command);
    }

    let mut process = Command::new("git");
    process.current_dir(&command.cwd);
    process.args(&command.args);

    process.envs(&command.env.variables);
    // Apply protected automation settings after custom variables so callers cannot re-enable prompts or paging.
    process.env("GIT_TERMINAL_PROMPT", "0");
    process.env("LANG", &command.env.lang);
    process.env("GIT_PAGER", &command.env.pager);

    process
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    ProcessTree::configure_command(&mut process);
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(source) => {
            if let Some(ref l) = logger {
                l.log_result(0, false, None);
            }
            return Err(if source.kind() == ErrorKind::NotFound {
                GitExecError::GitNotFound
            } else {
                GitExecError::SpawnFailed {
                    args: command.args.clone(),
                    source,
                }
            });
        }
    };
    let process_tree = match ProcessTree::from_spawned_id(child.id()) {
        Ok(process_tree) => process_tree,
        Err(source) => {
            // A spawned process must never escape without an owner when tree enrollment fails.
            let _ = child.kill();
            let _ = child.wait();
            if let Some(ref logger) = logger {
                logger.log_result(started_at.elapsed().as_millis() as u64, false, None);
            }
            return Err(GitExecError::ProcessTreeSetupFailed {
                args: command.args.clone(),
                source,
            });
        }
    };

    let stdout = child.stdout.take().expect("piped stdout must be available");
    let stderr = child.stderr.take().expect("piped stderr must be available");
    let (stdout_limit, stderr_limit) = limits;
    let stdout_exceeded = Arc::new(AtomicBool::new(false));
    let stderr_exceeded = Arc::new(AtomicBool::new(false));
    let stdout_reader = std::thread::spawn({
        let stdout_exceeded = Arc::clone(&stdout_exceeded);
        move || read_bounded(stdout, stdout_limit, stdout_exceeded)
    });
    let stderr_reader = std::thread::spawn({
        let stderr_exceeded = Arc::clone(&stderr_exceeded);
        move || read_bounded(stderr, stderr_limit, stderr_exceeded)
    });
    let mut child_status = None;
    let (status, forced_termination) = loop {
        if child_status.is_none() {
            child_status = match child.try_wait() {
                Ok(status) => status,
                Err(source) => {
                    let status_error = GitExecError::OutputReadFailed {
                        stream: "process status",
                        source,
                    };
                    if let Err(termination_error) =
                        terminate_process_tree(&process_tree, &mut child, command)
                    {
                        if let Some(ref logger) = logger {
                            logger.log_result(started_at.elapsed().as_millis() as u64, false, None);
                        }
                        return Err(termination_error);
                    }
                    if let Some(ref logger) = logger {
                        logger.log_result(started_at.elapsed().as_millis() as u64, false, None);
                    }
                    return Err(status_error);
                }
            };
        }
        let exceeded_stream = if stdout_exceeded.load(Ordering::Acquire) {
            Some(("stdout", stdout_limit))
        } else if stderr_exceeded.load(Ordering::Acquire) {
            Some(("stderr", stderr_limit))
        } else {
            None
        };
        if let Some((stream, limit)) = exceeded_stream {
            let status = match terminate_process_tree(&process_tree, &mut child, command) {
                Ok(status) => status,
                Err(error) => {
                    if let Some(ref logger) = logger {
                        logger.log_result(started_at.elapsed().as_millis() as u64, false, None);
                    }
                    return Err(error);
                }
            };
            break (
                status,
                Some(ForcedTermination::OutputTooLarge { stream, limit }),
            );
        }
        if let Some(status) = child_status
            && stdout_reader.is_finished()
            && stderr_reader.is_finished()
        {
            break (status, None);
        }
        if started_at.elapsed() >= timeout {
            let status = match terminate_process_tree(&process_tree, &mut child, command) {
                Ok(status) => status,
                Err(error) => {
                    if let Some(ref logger) = logger {
                        logger.log_result(started_at.elapsed().as_millis() as u64, false, None);
                    }
                    return Err(error);
                }
            };
            break (status, Some(ForcedTermination::TimedOut));
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let stdout_result = join_bounded_reader(stdout_reader, "stdout", stdout_limit);
    let stderr_result = join_bounded_reader(stderr_reader, "stderr", stderr_limit);
    let duration_ms = started_at.elapsed().as_millis() as u64;
    if let Some(forced_termination) = forced_termination {
        if let Some(ref logger) = logger {
            logger.log_result(duration_ms, false, status.code());
        }
        return Err(match forced_termination {
            ForcedTermination::OutputTooLarge { stream, limit } => {
                GitExecError::OutputTooLarge { stream, limit }
            }
            ForcedTermination::TimedOut => GitExecError::TimedOut {
                args: command.args.clone(),
                timeout_ms: timeout.as_millis() as u64,
            },
        });
    }
    let (stdout, stderr) = match (stdout_result, stderr_result) {
        (Ok(stdout), Ok(stderr)) => (stdout, stderr),
        (Err(error), _) | (_, Err(error)) => {
            if let Some(ref logger) = logger {
                logger.log_result(duration_ms, false, status.code());
            }
            return Err(error);
        }
    };
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();

    if status.success() {
        if let Some(ref l) = logger {
            l.log_result(duration_ms, true, status.code());
        }
        return Ok(GitOutput::new(status.code(), stdout, stderr, duration_ms));
    }

    let code = status.code();
    if let Some(ref l) = logger {
        l.log_result(duration_ms, false, code);
    }
    Err(GitExecError::NonZeroExit {
        code,
        args: command.args.clone(),
        stdout,
        stderr,
    })
}

/// Terminates Git and every descendant before reaping the direct child process.
fn terminate_process_tree(
    process_tree: &ProcessTree,
    child: &mut Child,
    command: &GitCommand,
) -> Result<ExitStatus, GitExecError> {
    if let Err(source) = process_tree.kill() {
        // Fall back to killing the direct child so a tree-management failure does not leak Git too.
        let _ = child.kill();
        let _ = child.wait();
        return Err(GitExecError::ProcessTreeTerminationFailed {
            args: command.args.clone(),
            source,
        });
    }

    child
        .wait()
        .map_err(|source| GitExecError::OutputReadFailed {
            stream: "process status",
            source,
        })
}

/// Reads one pipe without retaining bytes beyond its limit while continuing to drain the child.
fn read_bounded(
    mut reader: impl Read,
    limit: usize,
    output_exceeded: Arc<AtomicBool>,
) -> Result<(Vec<u8>, bool), std::io::Error> {
    let mut captured = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(captured.len());
        let retained = remaining.min(read);
        captured.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
        if truncated {
            output_exceeded.store(true, Ordering::Release);
        }
    }
    Ok((captured, truncated))
}

/// Maps pipe-reader failures and truncation into the stable Git execution error model.
fn join_bounded_reader(
    reader: std::thread::JoinHandle<Result<(Vec<u8>, bool), std::io::Error>>,
    stream: &'static str,
    limit: usize,
) -> Result<Vec<u8>, GitExecError> {
    let (bytes, truncated) = reader
        .join()
        .map_err(|_| GitExecError::OutputReadFailed {
            stream,
            source: std::io::Error::other("git output reader panicked"),
        })?
        .map_err(|source| GitExecError::OutputReadFailed { stream, source })?;
    if truncated {
        return Err(GitExecError::OutputTooLarge { stream, limit });
    }
    Ok(bytes)
}

/// Records commands without executing them so command-building behavior can be tested in isolation.
#[derive(Debug, Default, Clone)]
pub struct RecordingGitRunner {
    commands: Arc<Mutex<Vec<GitCommand>>>,
}

impl RecordingGitRunner {
    /// Returns a snapshot of every command received by this runner in execution order.
    pub fn commands(&self) -> Vec<GitCommand> {
        self.commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl GitRunner for RecordingGitRunner {
    /// Captures the command and returns a deterministic success output without spawning Git.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        self.commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(command.clone());
        Ok(GitOutput::new(
            Some(0),
            String::new(),
            format!("recorded args: {:?}", command.args),
            0,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{GitRunner, RecordingGitRunner, read_bounded};
    use crate::{GitCommand, GitEnv, GitIntent};
    use pretty_assertions::assert_eq;
    use std::sync::{Arc, atomic::AtomicBool};

    /// Verifies bounded readers drain input while retaining no bytes beyond the budget.
    #[test]
    fn bounds_captured_output() {
        assert_eq!(
            read_bounded("abcdef".as_bytes(), 3, Arc::new(AtomicBool::new(false))).unwrap(),
            (b"abc".to_vec(), true)
        );
    }

    /// Verifies the public recording runner exposes captured commands instead of only echoing arguments.
    #[test]
    fn records_commands_in_execution_order() {
        let runner = RecordingGitRunner::default();
        let command = GitCommand::new(
            "/repo".into(),
            vec!["status".to_string()],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );

        runner.run(&command).expect("recording should succeed");

        assert_eq!(runner.commands(), vec![command]);
    }
}
