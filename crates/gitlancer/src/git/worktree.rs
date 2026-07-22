use crate::domain::paths::WorktreeRoot;
use crate::domain::refs::{BranchName, CommitId, WorktreeIdentityToken};
use crate::domain::repo::Repository;
use crate::domain::worktree::{WorktreeHandle, WorktreeKind};
use crate::error::{DomainError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use std::io::{Read, Write};

const WORKTREE_IDENTITY_MARKER: &str = "ora-worktree-identity";
const MAX_WORKTREE_IDENTITY_TOKEN_BYTES: u64 = 128;

/// Carries the information needed to select one worktree from a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_name: &'a str,
}

/// Carries the information needed to locate which worktree contains a caller path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub candidate_path: &'a std::path::Path,
}

/// Carries the information needed to discover a worktree whose checkout root exactly matches a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindWorktreeRootRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_root: &'a std::path::Path,
}

/// Returns the complete list of worktrees associated with one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListWorktreesResponse {
    pub worktrees: Vec<WorktreeHandle>,
}

/// Represents the internal result shape produced before it is wrapped for the public response type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListWorktreesResult {
    pub worktrees: Vec<WorktreeHandle>,
}

/// Carries the information needed to create one linked worktree from a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_root: WorktreeRoot,
    pub branch_name: BranchName,
    pub identity_token: WorktreeIdentityToken,
}

/// Returns the linked worktree created by the runtime API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorktreeResponse {
    pub worktree: WorktreeHandle,
    pub head_commit_id: CommitId,
}

/// Describes how worktree deletion should behave when Git would otherwise protect the checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeDeletionMode {
    Checked,
    Force,
}

/// Carries the information needed to delete one linked worktree from its owning repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree: &'a WorktreeHandle,
    pub mode: WorktreeDeletionMode,
}

/// Returns the linked worktree root removed by the runtime API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteWorktreeResponse {
    pub worktree_root: WorktreeRoot,
    pub outcome: WorktreeDeletionOutcome,
}

/// Carries the repository whose stale worktree administration records should be pruned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneWorktreesRequest<'a> {
    pub repository: &'a Repository,
}

/// Reports whether deletion removed a checkout or found the requested identity already absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeDeletionOutcome {
    Removed,
    AlreadyAbsent,
}

impl From<ListWorktreesResult> for ListWorktreesResponse {
    /// Converts the internal result shape into the stable public response type.
    fn from(value: ListWorktreesResult) -> Self {
        Self {
            worktrees: value.worktrees,
        }
    }
}

impl<R: GitRunner> Git<R> {
    /// Resolves one named linked worktree without assigning a conflicting synthetic name to the main checkout.
    pub fn resolve_worktree(
        &self,
        request: ResolveWorktreeRequest<'_>,
    ) -> Result<WorktreeHandle, GitlancerError> {
        let worktrees = self
            .list_worktrees(crate::git::repository::ListWorktreesRequest {
                repository: request.repository,
            })?
            .worktrees;

        worktrees
            .into_iter()
            .find(|worktree| {
                matches!(
                    worktree.kind(),
                    WorktreeKind::Linked { name } if name == request.worktree_name
                )
            })
            .ok_or_else(|| {
                GitlancerError::Domain(DomainError::NotAWorktree(
                    request.repository.root().as_path().to_path_buf(),
                ))
            })
    }

    /// Finds which worktree contains a candidate path by choosing the deepest worktree root prefix match.
    pub fn find_worktree(
        &self,
        request: FindWorktreeRequest<'_>,
    ) -> Result<WorktreeHandle, GitlancerError> {
        let candidate = normalize_path_for_worktree_match(request.candidate_path);
        let worktrees = self
            .list_worktrees(crate::git::repository::ListWorktreesRequest {
                repository: request.repository,
            })?
            .worktrees;

        worktrees
            .into_iter()
            .filter(|worktree| {
                candidate.starts_with(normalize_path_for_worktree_match(
                    worktree.worktree_root().as_path(),
                ))
            })
            .max_by_key(|worktree| {
                normalize_path_for_worktree_match(worktree.worktree_root().as_path())
                    .components()
                    .count()
            })
            .ok_or(GitlancerError::Domain(DomainError::NotAWorktree(candidate)))
    }

    /// Finds a worktree only when the supplied path is its exact checkout root.
    pub fn find_worktree_root(
        &self,
        request: FindWorktreeRootRequest<'_>,
    ) -> Result<WorktreeHandle, GitlancerError> {
        let requested_root = normalize_path_for_worktree_match(request.worktree_root);

        self.list_worktrees(crate::git::repository::ListWorktreesRequest {
            repository: request.repository,
        })?
        .worktrees
        .into_iter()
        .find(|worktree| {
            normalize_path_for_worktree_match(worktree.worktree_root().as_path()) == requested_root
        })
        .ok_or(GitlancerError::Domain(DomainError::NotAWorktree(
            requested_root,
        )))
    }

    /// Creates one linked worktree and returns the resulting typed worktree handle.
    pub fn create_worktree(
        &self,
        request: CreateWorktreeRequest<'_>,
    ) -> Result<CreateWorktreeResponse, GitlancerError> {
        let head_output = self.runner().run(&build_head_commit_command(&request))?;
        let head_commit_id = head_output.stdout.trim();
        if head_commit_id.is_empty() {
            return Err(crate::ParseError::MissingLine.into());
        }
        let head_commit_id = CommitId::new(head_commit_id)?;
        let command = build_create_worktree_command(&request, &head_commit_id);
        let _output = self.runner().run(&command)?;
        let worktree = match self.find_worktree_root(FindWorktreeRootRequest {
            repository: request.repository,
            worktree_root: request.worktree_root.as_path(),
        }) {
            Ok(worktree) => worktree,
            Err(error) => {
                self.cleanup_failed_worktree_creation(&request)?;
                return Err(error);
            }
        };
        let worktree = match write_worktree_identity(&worktree, &request.identity_token) {
            Ok(()) => worktree.with_identity_token(request.identity_token.clone()),
            Err(error) => {
                self.cleanup_failed_worktree_creation(&request)?;
                return Err(error);
            }
        };

        Ok(CreateWorktreeResponse {
            worktree,
            head_commit_id,
        })
    }

    /// Removes both resources created by `git worktree add` when response discovery fails.
    fn cleanup_failed_worktree_creation(
        &self,
        request: &CreateWorktreeRequest<'_>,
    ) -> Result<(), GitlancerError> {
        let worktree_cleanup = self
            .runner()
            .run(&build_failed_create_worktree_cleanup_command(request));
        let branch_cleanup = self
            .runner()
            .run(&build_failed_create_branch_cleanup_command(request));

        // Both resources are independent cleanup targets, so a failure removing one must not
        // prevent an attempt to remove the other. Preserve the first failure when both fail.
        worktree_cleanup?;
        branch_cleanup?;
        Ok(())
    }

    /// Deletes one trusted linked-worktree identity and treats a previously removed identity as success.
    pub fn delete_worktree(
        &self,
        request: DeleteWorktreeRequest<'_>,
    ) -> Result<DeleteWorktreeResponse, GitlancerError> {
        if request.worktree.repo_root() != request.repository.root() {
            return Err(GitlancerError::Domain(DomainError::WorktreeMismatch {
                worktree: request.worktree.worktree_root().as_path().to_path_buf(),
                repo: request.repository.root().as_path().to_path_buf(),
            }));
        }
        if matches!(request.worktree.kind(), WorktreeKind::Main) {
            return Err(GitlancerError::Domain(
                DomainError::MainWorktreeDeletionUnsupported(
                    request.repository.root().as_path().to_path_buf(),
                ),
            ));
        }
        if request.worktree.identity_token().is_none() {
            return Err(GitlancerError::Domain(
                DomainError::UnmanagedWorktreeDeletionUnsupported(
                    request.worktree.worktree_root().as_path().to_path_buf(),
                ),
            ));
        }

        let current_worktree = self
            .list_worktrees(crate::git::repository::ListWorktreesRequest {
                repository: request.repository,
            })?
            .worktrees
            .into_iter()
            .find(|worktree| {
                normalize_path_for_worktree_match(worktree.worktree_root().as_path())
                    == normalize_path_for_worktree_match(request.worktree.worktree_root().as_path())
            });
        let Some(current_worktree) = current_worktree else {
            return Ok(DeleteWorktreeResponse {
                worktree_root: request.worktree.worktree_root().clone(),
                outcome: WorktreeDeletionOutcome::AlreadyAbsent,
            });
        };
        if current_worktree.git_dir() != request.worktree.git_dir()
            || current_worktree.branch() != request.worktree.branch()
            || current_worktree.identity_token() != request.worktree.identity_token()
        {
            return Err(GitlancerError::Domain(
                DomainError::WorktreeIdentityChanged {
                    worktree: Box::new(current_worktree.worktree_root().as_path().to_path_buf()),
                    expected_git_dir: Box::new(request.worktree.git_dir().as_path().to_path_buf()),
                    actual_git_dir: Box::new(current_worktree.git_dir().as_path().to_path_buf()),
                    expected_branch: request
                        .worktree
                        .branch()
                        .map(|branch| branch.as_str().to_string()),
                    actual_branch: current_worktree
                        .branch()
                        .map(|branch| branch.as_str().to_string()),
                    expected_identity_token: request
                        .worktree
                        .identity_token()
                        .map(|token| Box::<str>::from(token.as_str())),
                    actual_identity_token: current_worktree
                        .identity_token()
                        .map(|token| Box::<str>::from(token.as_str())),
                },
            ));
        }
        if matches!(current_worktree.kind(), WorktreeKind::Main) {
            return Err(GitlancerError::Domain(
                DomainError::MainWorktreeDeletionUnsupported(
                    request.repository.root().as_path().to_path_buf(),
                ),
            ));
        }

        let command = build_delete_worktree_command_for_handle(
            request.repository,
            &current_worktree,
            request.mode,
        );
        let _output = self.runner().run(&command)?;

        Ok(DeleteWorktreeResponse {
            worktree_root: request.worktree.worktree_root().clone(),
            outcome: WorktreeDeletionOutcome::Removed,
        })
    }

    /// Removes Git administration records for worktrees whose checkout paths no longer exist.
    pub fn prune_worktrees(
        &self,
        request: PruneWorktreesRequest<'_>,
    ) -> Result<(), GitlancerError> {
        let command = GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "worktree".to_string(),
                "prune".to_string(),
                "--expire=now".to_string(),
            ],
            GitEnv::default(),
            GitIntent::Mutating,
        );
        self.runner().run(&command)?;
        Ok(())
    }
}

/// Reads the bounded Ora marker stored inside a linked worktree's private Git directory.
pub(crate) fn read_worktree_identity(
    git_dir: &std::path::Path,
) -> Result<Option<WorktreeIdentityToken>, GitlancerError> {
    let marker_path = git_dir.join(WORKTREE_IDENTITY_MARKER);
    let file = match std::fs::File::open(&marker_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(GitlancerError::WorktreeIdentityIo {
                path: marker_path,
                source,
            });
        }
    };
    let mut bytes = Vec::new();
    file.take(MAX_WORKTREE_IDENTITY_TOKEN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| GitlancerError::WorktreeIdentityIo {
            path: marker_path.clone(),
            source,
        })?;
    if bytes.len() as u64 > MAX_WORKTREE_IDENTITY_TOKEN_BYTES {
        return Err(DomainError::InvalidWorktreeIdentityToken(
            "<marker exceeds 128 bytes>".to_string(),
        )
        .into());
    }
    let token = String::from_utf8(bytes).map_err(|error| {
        DomainError::InvalidWorktreeIdentityToken(
            String::from_utf8_lossy(error.as_bytes()).into_owned(),
        )
    })?;

    WorktreeIdentityToken::new(token)
        .map(Some)
        .map_err(Into::into)
}

/// Persists the unique Ora marker before a created worktree is exposed to upper layers.
fn write_worktree_identity(
    worktree: &WorktreeHandle,
    identity_token: &WorktreeIdentityToken,
) -> Result<(), GitlancerError> {
    let marker_path = worktree.git_dir().as_path().join(WORKTREE_IDENTITY_MARKER);
    let mut marker = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker_path)
        .map_err(|source| GitlancerError::WorktreeIdentityIo {
            path: marker_path.clone(),
            source,
        })?;
    marker
        .write_all(identity_token.as_str().as_bytes())
        .and_then(|()| marker.sync_all())
        .map_err(|source| GitlancerError::WorktreeIdentityIo {
            path: marker_path,
            source,
        })
}

/// Normalizes a candidate path lexically so worktree comparisons do not depend on filesystem canonicalization.
fn normalize_candidate_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = normalized.pop();
            }
            std::path::Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

/// Resolves existing path prefixes so Windows short names and Git-reported long paths compare identically.
fn normalize_path_for_worktree_match(path: &std::path::Path) -> std::path::PathBuf {
    let mut current = path;
    let mut suffix_parts = Vec::new();

    loop {
        if let Ok(canonical_path) = std::fs::canonicalize(current) {
            let mut normalized = normalize_candidate_path(&canonical_path);
            for suffix_part in suffix_parts.iter().rev() {
                normalized.push(suffix_part);
            }

            return normalize_candidate_path(&normalized);
        }

        match (current.parent(), current.file_name()) {
            (Some(parent), Some(file_name)) => {
                suffix_parts.push(file_name.to_os_string());
                current = parent;
            }
            _ => return normalize_candidate_path(path),
        }
    }
}

/// Reads the exact start point before mutation so the recorded baseline cannot drift concurrently.
fn build_head_commit_command(request: &CreateWorktreeRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec!["rev-parse".to_string(), "HEAD".to_string()],
        GitEnv::default(),
        GitIntent::ReadOnly,
    )
}

/// Builds a stable `git worktree add` command pinned to the baseline captured before mutation.
pub fn build_create_worktree_command(
    request: &CreateWorktreeRequest<'_>,
    start_point: &CommitId,
) -> GitCommand {
    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec![
            "worktree".to_string(),
            "add".to_string(),
            "-b".to_string(),
            request.branch_name.as_str().to_string(),
            request
                .worktree_root
                .as_path()
                .to_string_lossy()
                .into_owned(),
            start_point.as_str().to_string(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Builds the forced worktree removal used only to compensate a partially completed creation.
fn build_failed_create_worktree_cleanup_command(request: &CreateWorktreeRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec![
            "worktree".to_string(),
            "remove".to_string(),
            request
                .worktree_root
                .as_path()
                .to_string_lossy()
                .into_owned(),
            "--force".to_string(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Deletes the new branch after its failed worktree has been removed successfully.
fn build_failed_create_branch_cleanup_command(request: &CreateWorktreeRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec![
            "branch".to_string(),
            "-D".to_string(),
            request.branch_name.as_str().to_string(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Builds a stable `git worktree remove` command so deletion mode remains visible in one place.
pub fn build_delete_worktree_command(request: &DeleteWorktreeRequest<'_>) -> GitCommand {
    build_delete_worktree_command_for_handle(request.repository, request.worktree, request.mode)
}

/// Builds deletion from the freshly rediscovered identity used immediately before mutation.
fn build_delete_worktree_command_for_handle(
    repository: &Repository,
    worktree: &WorktreeHandle,
    mode: WorktreeDeletionMode,
) -> GitCommand {
    let mut args = vec![
        "worktree".to_string(),
        "remove".to_string(),
        worktree
            .worktree_root()
            .as_path()
            .to_string_lossy()
            .into_owned(),
    ];
    if matches!(mode, WorktreeDeletionMode::Force) {
        args.push("--force".to_string());
    }

    GitCommand::new(
        repository.root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CreateWorktreeRequest, DeleteWorktreeRequest, Git, Repository, WorktreeDeletionMode,
    };
    use crate::{
        BranchName, CommitId, GitCommand, GitDir, GitExecError, GitOutput, GitRunner,
        GitlancerError, RepoRoot, WorktreeHandle, WorktreeIdentityToken, WorktreeKind,
        WorktreeRoot,
    };
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    const BASE_COMMIT_ID: &str = "0123456789abcdef0123456789abcdef01234567";

    /// Creates a validated branch fixture for worktree lifecycle requests.
    fn branch_name(value: &str) -> BranchName {
        BranchName::new(value).expect("test branch should be valid")
    }

    /// Creates a validated identity fixture for managed worktree lifecycle requests.
    fn identity_token(value: &str) -> WorktreeIdentityToken {
        WorktreeIdentityToken::new(value).expect("test identity token should be valid")
    }

    /// Supplies a deterministic command sequence so post-mutation rollback can be tested in isolation.
    struct SequencedRunner {
        results: RefCell<VecDeque<Result<GitOutput, GitExecError>>>,
        commands: RefCell<Vec<GitCommand>>,
    }

    impl SequencedRunner {
        /// Creates a runner whose results are consumed in command order.
        fn new(results: Vec<Result<GitOutput, GitExecError>>) -> Self {
            Self {
                results: RefCell::new(results.into()),
                commands: RefCell::new(Vec::new()),
            }
        }
    }

    impl GitRunner for SequencedRunner {
        /// Records each command and returns the next configured process result.
        fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
            self.commands.borrow_mut().push(command.clone());
            match self.results.borrow_mut().pop_front() {
                Some(result) => result,
                None => panic!("missing fake result for {:?}", command.args),
            }
        }
    }

    /// Verifies baseline lookup failures occur before `git worktree add` can mutate repository state.
    #[test]
    fn does_not_create_worktree_when_baseline_lookup_fails() {
        let runner = SequencedRunner::new(vec![Err(GitExecError::NonZeroExit {
            code: Some(128),
            args: vec!["rev-parse".to_string(), "HEAD".to_string()],
            stdout: String::new(),
            stderr: "missing HEAD".to_string(),
        })]);
        let git = Git::new(runner);
        let repository = Repository::new(RepoRoot::new("/repo"));

        let result = git.create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new("/worktrees/task"),
            branch_name: branch_name("ora/task"),
            identity_token: identity_token("worktree-task"),
        });

        assert!(result.is_err());
        assert_eq!(
            git.runner()
                .commands
                .borrow()
                .iter()
                .map(|command| command.args.clone())
                .collect::<Vec<_>>(),
            vec![vec!["rev-parse", "HEAD"]]
        );
    }

    /// Verifies metadata discovery failures remove both resources created by `git worktree add`.
    #[test]
    fn cleans_up_worktree_and_branch_when_creation_discovery_fails() {
        let runner = SequencedRunner::new(vec![
            Ok(successful_output(&format!("{BASE_COMMIT_ID}\n"))),
            Ok(successful_output("")),
            Err(GitExecError::NonZeroExit {
                code: Some(1),
                args: vec![
                    "worktree".to_string(),
                    "list".to_string(),
                    "--porcelain".to_string(),
                    "-z".to_string(),
                ],
                stdout: String::new(),
                stderr: "list failed".to_string(),
            }),
            Ok(successful_output("")),
            Ok(successful_output("")),
        ]);
        let git = Git::new(runner);
        let repository = Repository::new(RepoRoot::new("/repo"));
        let result = git.create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new("/worktrees/task"),
            branch_name: branch_name("ora/task"),
            identity_token: identity_token("worktree-task"),
        });

        let error = match result {
            Err(GitlancerError::Exec(GitExecError::NonZeroExit {
                code, args, stderr, ..
            })) => (code, args, stderr),
            other => panic!("expected discovery failure after successful cleanup, got {other:?}"),
        };
        assert_eq!(
            error,
            (
                Some(1),
                vec![
                    "worktree".to_string(),
                    "list".to_string(),
                    "--porcelain".to_string(),
                    "-z".to_string(),
                ],
                "list failed".to_string(),
            )
        );
        assert_eq!(
            git.runner()
                .commands
                .borrow()
                .iter()
                .map(|command| command.args.clone())
                .collect::<Vec<_>>(),
            vec![
                vec!["rev-parse", "HEAD"],
                vec![
                    "worktree",
                    "add",
                    "-b",
                    "ora/task",
                    "/worktrees/task",
                    BASE_COMMIT_ID,
                ],
                vec!["worktree", "list", "--porcelain", "-z"],
                vec!["worktree", "remove", "/worktrees/task", "--force"],
                vec!["branch", "-D", "ora/task"],
            ]
        );
    }

    /// Verifies branch cleanup is still attempted when forced worktree removal fails.
    #[test]
    fn attempts_branch_cleanup_after_worktree_cleanup_fails() {
        let cleanup_error = GitExecError::NonZeroExit {
            code: Some(1),
            args: vec!["worktree".to_string(), "remove".to_string()],
            stdout: String::new(),
            stderr: "worktree cleanup failed".to_string(),
        };
        let runner = SequencedRunner::new(vec![
            Ok(successful_output(&format!("{BASE_COMMIT_ID}\n"))),
            Ok(successful_output("")),
            Err(GitExecError::NonZeroExit {
                code: Some(1),
                args: vec![
                    "worktree".to_string(),
                    "list".to_string(),
                    "--porcelain".to_string(),
                    "-z".to_string(),
                ],
                stdout: String::new(),
                stderr: "list failed".to_string(),
            }),
            Err(cleanup_error),
            Ok(successful_output("")),
        ]);
        let git = Git::new(runner);
        let repository = Repository::new(RepoRoot::new("/repo"));

        let result = git.create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new("/worktrees/task"),
            branch_name: branch_name("ora/task"),
            identity_token: identity_token("worktree-task"),
        });

        assert!(result.is_err());
        assert_eq!(
            git.runner()
                .commands
                .borrow()
                .iter()
                .map(|command| command.args.clone())
                .collect::<Vec<_>>(),
            vec![
                vec!["rev-parse", "HEAD"],
                vec![
                    "worktree",
                    "add",
                    "-b",
                    "ora/task",
                    "/worktrees/task",
                    BASE_COMMIT_ID,
                ],
                vec!["worktree", "list", "--porcelain", "-z"],
                vec!["worktree", "remove", "/worktrees/task", "--force"],
                vec!["branch", "-D", "ora/task"],
            ]
        );
    }

    /// Verifies a stale trusted handle cannot delete a replacement branch at the same path and git directory.
    #[test]
    fn rejects_reused_path_with_changed_git_identity() {
        let repository = Repository::new(RepoRoot::new("/repo"));
        let stale_worktree = WorktreeHandle::discovered(
            repository.root().clone(),
            WorktreeRoot::new("/worktrees/task"),
            GitDir::new("/repo/.git/worktrees/task-old"),
            WorktreeKind::Linked {
                name: "task-old".to_string(),
            },
            CommitId::new("0123456789abcdef0123456789abcdef01234567")
                .expect("test commit id should be valid"),
            Some(branch_name("ora/task")),
            None,
        )
        .with_identity_token(identity_token("worktree-old"));
        let runner = SequencedRunner::new(vec![
            Ok(successful_output(concat!(
                "worktree /repo\0",
                "HEAD 0123456789abcdef0123456789abcdef01234567\0",
                "branch refs/heads/main\0\0",
                "worktree /worktrees/task\0",
                "HEAD 89abcdef0123456789abcdef0123456789abcdef\0",
                "branch refs/heads/ora/task-new\0\0",
            ))),
            Ok(successful_output("/repo/.git\n")),
            Ok(successful_output("/repo/.git/worktrees/task-old\n")),
        ]);
        let git = Git::new(runner);

        let error = git
            .delete_worktree(DeleteWorktreeRequest {
                repository: &repository,
                worktree: &stale_worktree,
                mode: WorktreeDeletionMode::Force,
            })
            .expect_err("changed identity must be rejected");

        assert!(matches!(
            error,
            GitlancerError::Domain(crate::DomainError::WorktreeIdentityChanged {
                expected_git_dir,
                actual_git_dir,
                expected_branch,
                actual_branch,
                ..
            }) if expected_git_dir.as_path() == std::path::Path::new("/repo/.git/worktrees/task-old")
                && actual_git_dir.as_path() == std::path::Path::new("/repo/.git/worktrees/task-old")
                && expected_branch.as_deref() == Some("ora/task")
                && actual_branch.as_deref() == Some("ora/task-new")
        ));
        assert_eq!(git.runner().commands.borrow().len(), 3);
    }

    /// Verifies the safe deletion API refuses handles that have no durable instance marker.
    #[test]
    fn rejects_unmanaged_worktree_deletion() {
        let repository = Repository::new(RepoRoot::new("/repo"));
        let unmanaged_worktree = WorktreeHandle::discovered(
            repository.root().clone(),
            WorktreeRoot::new("/worktrees/task"),
            GitDir::new("/repo/.git/worktrees/task"),
            WorktreeKind::Linked {
                name: "task".to_string(),
            },
            CommitId::new(BASE_COMMIT_ID).expect("test commit id should be valid"),
            Some(branch_name("ora/task")),
            None,
        );
        let git = Git::new(SequencedRunner::new(Vec::new()));

        let error = git
            .delete_worktree(DeleteWorktreeRequest {
                repository: &repository,
                worktree: &unmanaged_worktree,
                mode: WorktreeDeletionMode::Force,
            })
            .expect_err("unmanaged worktrees must not enter destructive deletion");

        assert!(matches!(
            error,
            GitlancerError::Domain(
                crate::DomainError::UnmanagedWorktreeDeletionUnsupported(path)
            ) if path == std::path::Path::new("/worktrees/task")
        ));
        assert_eq!(git.runner().commands.borrow().len(), 0);
    }

    /// Creates one successful fake output without repeating process metadata in test setup.
    fn successful_output(stdout: &str) -> GitOutput {
        GitOutput::new(Some(0), stdout.to_string(), String::new(), 0)
    }
}
