use crate::domain::paths::{GitDir, RepoRoot};
use crate::domain::repo::Repository;
use crate::domain::worktree::{WorktreeHandle, WorktreeKind};
use crate::error::{DomainError, GitExecError, GitlancerError, ParseError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use crate::git::worktree::{ListWorktreesResponse, ListWorktreesResult};
use crate::parse;
use crate::parse::worktree::ParsedWorktree;

/// Carries the information needed to list worktrees for one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListWorktreesRequest<'a> {
    pub repository: &'a Repository,
}

impl<R: GitRunner> Git<R> {
    /// Lists worktrees for one repository using Git's porcelain worktree listing format.
    pub fn list_worktrees(
        &self,
        request: ListWorktreesRequest<'_>,
    ) -> Result<ListWorktreesResponse, GitlancerError> {
        let command = GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "worktree".to_string(),
                "list".to_string(),
                "--porcelain".to_string(),
                "-z".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output = self.runner().run(&command)?;
        let parsed_worktrees = parse::worktree::parse_worktree_list(&output.stdout)?;
        let mut worktrees = Vec::with_capacity(parsed_worktrees.len());
        for parsed in parsed_worktrees {
            let ParsedWorktree::Checkout {
                worktree_root,
                head_commit_id,
                branch,
                locked_reason,
                prunable_reason,
                ..
            } = parsed
            else {
                continue;
            };
            if prunable_reason.is_some() {
                continue;
            }

            let git_dir_output = self.runner().run(&GitCommand::new(
                worktree_root.as_path().to_path_buf(),
                vec!["rev-parse".to_string(), "--absolute-git-dir".to_string()],
                GitEnv::default(),
                GitIntent::ReadOnly,
            ))?;
            let git_dir = git_dir_output
                .stdout
                .strip_suffix("\r\n")
                .or_else(|| git_dir_output.stdout.strip_suffix('\n'))
                .unwrap_or(&git_dir_output.stdout);
            if git_dir.is_empty() {
                return Err(ParseError::InvalidWorktreeList.into());
            }
            let kind = if worktree_root.as_path() == request.repository.root().as_path() {
                WorktreeKind::Main
            } else {
                WorktreeKind::Linked {
                    name: linked_worktree_name(git_dir)?,
                }
            };
            let git_dir = GitDir::new(git_dir);
            let identity_token = crate::git::worktree::read_worktree_identity(git_dir.as_path())?;
            worktrees.push(
                WorktreeHandle::discovered(
                    request.repository.root().clone(),
                    worktree_root,
                    git_dir,
                    kind,
                    head_commit_id,
                    branch,
                    locked_reason,
                )
                .with_discovered_identity_token(identity_token),
            );
        }

        Ok(ListWorktreesResult { worktrees }.into())
    }

    /// Discovers the owning repository root from Git's first main-checkout or bare-repository record.
    pub fn discover_repository(&self, root: RepoRoot) -> Result<Repository, GitlancerError> {
        let command = GitCommand::new(
            root.as_path().to_path_buf(),
            vec![
                "worktree".to_string(),
                "list".to_string(),
                "--porcelain".to_string(),
                "-z".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output = self.runner().run(&command).map_err(|error| match error {
            GitExecError::NonZeroExit { .. } => {
                GitlancerError::Domain(DomainError::NotARepository(root.as_path().to_path_buf()))
            }
            other => GitlancerError::Exec(other),
        })?;
        let repository_root = match parse::worktree::parse_worktree_list(&output.stdout)?
            .into_iter()
            .next()
            .ok_or(ParseError::InvalidWorktreeList)?
        {
            ParsedWorktree::Bare { repository_root } => repository_root,
            ParsedWorktree::Checkout { worktree_root, .. } => {
                RepoRoot::new(worktree_root.as_path())
            }
        };

        Ok(Repository::new(repository_root))
    }
}

/// Derives Git's stable linked-worktree administration name from its resolved git directory.
fn linked_worktree_name(git_dir: &str) -> Result<String, ParseError> {
    std::path::Path::new(git_dir)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .ok_or(ParseError::InvalidWorktreeList)
}
