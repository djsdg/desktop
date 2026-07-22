use crate::domain::paths::RepoRelativePath;
use crate::domain::refs::CommitId;
use crate::domain::worktree::WorktreeHandle;
use crate::error::GitlancerError;
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use crate::parse::commit::parse_commit_response;

/// Carries the information needed to stage one or more repo-relative paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub paths: Vec<RepoRelativePath>,
}

/// Returns the paths that were requested for staging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddResponse {
    pub staged_paths: Vec<RepoRelativePath>,
}

/// Carries the information needed to create a commit in one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub message: &'a str,
    pub allow_empty: bool,
}

/// Returns the typed metadata upper layers typically need after a successful commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResponse {
    pub commit_id: CommitId,
    pub summary: String,
}

impl<R: GitRunner> Git<R> {
    /// Stages repo-relative paths so callers never need to build `git add` commands themselves.
    pub fn add(&self, request: AddRequest<'_>) -> Result<AddResponse, GitlancerError> {
        let command = build_add_command(&request);
        let _output = self.runner().run(&command)?;

        Ok(AddResponse {
            staged_paths: request.paths,
        })
    }

    /// Creates one commit and returns validated metadata read back from the resulting `HEAD`.
    pub fn commit(&self, request: CommitRequest<'_>) -> Result<CommitResponse, GitlancerError> {
        let command = build_commit_command(&request);
        let _output = self.runner().run(&command)?;
        let hash_output = self.runner().run(&GitCommand::new(
            request.worktree.worktree_root().as_path().to_path_buf(),
            vec!["rev-parse".to_string(), "HEAD".to_string()],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        let summary_output = self.runner().run(&GitCommand::new(
            request.worktree.worktree_root().as_path().to_path_buf(),
            vec![
                "log".to_string(),
                "-1".to_string(),
                "--pretty=%s".to_string(),
                "HEAD".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        let metadata = format!("{}\n{}", hash_output.stdout, summary_output.stdout);

        parse_commit_response(&metadata).map_err(Into::into)
    }
}

/// Builds a stable `git add` command so staging behavior can be tested independently from process execution.
pub fn build_add_command(request: &AddRequest<'_>) -> GitCommand {
    // `--` ends option parsing but does not disable Git's `:(...)` pathspec magic.
    let mut args = vec![
        "--literal-pathspecs".to_string(),
        "add".to_string(),
        "--".to_string(),
    ];
    args.extend(
        request
            .paths
            .iter()
            .map(|path| path.as_path().to_string_lossy().into_owned()),
    );

    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{AddRequest, build_add_command};
    use crate::{
        BranchName, CommitId, GitDir, RepoRelativePath, RepoRoot, WorktreeHandle, WorktreeKind,
        WorktreeRoot,
    };

    /// Verifies caller paths are always passed through Git's literal pathspec mode.
    #[test]
    fn add_command_disables_pathspec_magic() {
        let worktree = WorktreeHandle::discovered(
            RepoRoot::new("/repo"),
            WorktreeRoot::new("/repo/worktree"),
            GitDir::new("/repo/.git/worktrees/task"),
            WorktreeKind::Linked {
                name: "task".to_string(),
            },
            CommitId::new("0123456789abcdef0123456789abcdef01234567")
                .expect("test commit id should be valid"),
            Some(BranchName::new("ora/task").expect("test branch should be valid")),
            None,
        );
        let path = RepoRelativePath::new(":(glob)*.rs")
            .expect("pathspec-like filename should be repository-relative");

        let command = build_add_command(&AddRequest {
            worktree: &worktree,
            paths: vec![path],
        });

        assert_eq!(
            command.args,
            vec!["--literal-pathspecs", "add", "--", ":(glob)*.rs"]
        );
    }
}

/// Builds a stable `git commit` command so commit policy and options stay centralized.
pub fn build_commit_command(request: &CommitRequest<'_>) -> GitCommand {
    let mut args = vec![
        "commit".to_string(),
        "--no-gpg-sign".to_string(),
        "-m".to_string(),
        request.message.to_string(),
    ];

    if request.allow_empty {
        args.push("--allow-empty".to_string());
    }

    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::Mutating,
    )
}
