mod common;

use std::path::Path;

use common::TestScaffold;
use gitlancer::git::branch::{
    BranchDeletionMode, CreateBranchRequest, DeleteBranchRequest, ListBranchesRequest,
};
use gitlancer::git::commit::{AddRequest, CommitRequest};
use gitlancer::git::diff::DiffRequest;
use gitlancer::git::repository::ListWorktreesRequest;
use gitlancer::git::status::{RenameOrCopy, StatusEntry, StatusRequest};
use gitlancer::git::worktree::{
    CreateWorktreeRequest, DeleteWorktreeRequest, FindWorktreeRequest, FindWorktreeRootRequest,
    PruneWorktreesRequest, ResolveWorktreeRequest, WorktreeDeletionMode, WorktreeDeletionOutcome,
};
use gitlancer::{
    BranchName, CliGitRunner, CommitId, Git, RepoRoot, WorktreeIdentityToken, WorktreeKind,
    WorktreeRoot,
};
use pretty_assertions::assert_eq;

/// Creates an initial commit so linked worktrees can be created from a valid repository history.
fn seed_repository(scaffold: &TestScaffold) {
    scaffold
        .write_file(scaffold.repo_path(), "README.md", "seed repository\n")
        .expect("write seed file");
    scaffold
        .stage_all_and_commit("chore: seed repository")
        .expect("create initial commit");
}

/// Returns a typed runtime and repository handle for one scaffold so lifecycle tests can focus on behavior.
fn runtime_repository(scaffold: &TestScaffold) -> (Git<CliGitRunner>, gitlancer::Repository) {
    let git = Git::new(CliGitRunner::default());
    let repository = git
        .discover_repository(RepoRoot::new(scaffold.repo_path()))
        .expect("discover repository");

    (git, repository)
}

/// Creates a validated branch fixture while keeping lifecycle tests focused on Git behavior.
fn branch_name(value: &str) -> BranchName {
    BranchName::new(value).expect("test branch should be valid")
}

/// Creates a validated managed-worktree identity fixture.
fn identity_token(value: &str) -> WorktreeIdentityToken {
    WorktreeIdentityToken::new(value).expect("test identity token should be valid")
}

/// Verifies fixed-baseline diffs combine committed, staged, unstaged, and untracked changes.
#[test]
fn runtime_builds_complete_task_diff() {
    let scaffold = TestScaffold::new("runtime-builds-task-diff").expect("create scaffold");
    seed_repository(&scaffold);
    let base_commit_id = CommitId::new(
        scaffold
            .run_git(["rev-parse", "HEAD"])
            .expect("read base commit")
            .trim(),
    )
    .expect("Git should return a full commit id");
    scaffold
        .write_file(scaffold.repo_path(), "README.md", "committed change\n")
        .expect("write committed change");
    scaffold
        .stage_all_and_commit("feat: committed task change")
        .expect("commit task change");
    scaffold
        .write_file(scaffold.repo_path(), "staged.txt", "staged change\n")
        .expect("write staged change");
    scaffold
        .run_git(["add", "--", "staged.txt"])
        .expect("stage task change");
    scaffold
        .write_file(
            scaffold.repo_path(),
            "README.md",
            "committed change\nunstaged change\n",
        )
        .expect("write unstaged change");
    scaffold
        .write_file(scaffold.repo_path(), "untracked.txt", "untracked change\n")
        .expect("write untracked change");
    scaffold
        .write_file(scaffold.repo_path(), "empty.txt", "")
        .expect("write empty untracked file");
    scaffold
        .run_git(["config", "filter.guard.clean", "false"])
        .expect("configure failing clean filter");
    scaffold
        .run_git(["config", "filter.guard.required", "true"])
        .expect("require clean filter");
    scaffold
        .run_git(["config", "diff.noprefix", "true"])
        .expect("configure nonstandard diff prefixes");
    scaffold
        .run_git(["config", "diff.srcPrefix", "wrong-old/"])
        .expect("configure custom source prefix");
    scaffold
        .run_git(["config", "diff.dstPrefix", "wrong-new/"])
        .expect("configure custom destination prefix");
    scaffold
        .write_file(
            scaffold.repo_path(),
            ".gitattributes",
            "*.guard filter=guard\n",
        )
        .expect("write filter attributes");
    scaffold
        .write_file(
            scaffold.repo_path(),
            "untracked.guard",
            "filter must not run\n",
        )
        .expect("write filtered untracked file");
    std::fs::write(scaffold.repo_path().join("binary.bin"), b"\0binary\n")
        .expect("write untracked binary file");
    let real_index_before = scaffold
        .run_git(["diff", "--cached", "--binary"])
        .expect("read real index before diff");
    let (git, repository) = runtime_repository(&scaffold);
    let worktree = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: scaffold.repo_path(),
        })
        .expect("find main worktree");

    let response = git
        .diff(DiffRequest {
            worktree: &worktree,
            base_commit_id: &base_commit_id,
        })
        .expect("build task diff");

    assert_ne!(response.head_commit_id, base_commit_id);
    for expected_path in [
        "README.md",
        "empty.txt",
        "staged.txt",
        "untracked.txt",
        "untracked.guard",
        "binary.bin",
    ] {
        assert!(
            response
                .patch
                .contains(&format!("diff --git a/{expected_path} b/{expected_path}")),
            "patch should include {expected_path}"
        );
    }
    assert!(response.patch.contains("+unstaged change"));
    assert!(response.patch.contains("+untracked change"));
    let empty_file_patch = response
        .patch
        .split("diff --git ")
        .find(|section| section.starts_with("a/empty.txt b/empty.txt\n"))
        .expect("empty file should have its own patch section");
    assert!(empty_file_patch.contains("new file mode 100644"));
    assert!(empty_file_patch.contains("index 0000000..e69de29"));
    assert!(
        response
            .patch
            .contains("Binary files /dev/null and b/binary.bin differ")
    );
    assert!(!response.patch.contains("GIT binary patch"));
    let real_index_after = scaffold
        .run_git(["diff", "--cached", "--binary"])
        .expect("read real index after diff");
    assert_eq!(real_index_after, real_index_before);
}

/// Verifies the runtime can discover repositories, list worktrees, resolve linked worktrees, and enumerate branches.
#[test]
fn runtime_discovers_worktrees_and_branches() {
    let scaffold = TestScaffold::new("runtime-discovers-worktrees").expect("create scaffold");
    seed_repository(&scaffold);
    let linked_path = scaffold
        .create_linked_worktree("feature-tree", "feature/runtime")
        .expect("create linked worktree");

    let git = Git::new(CliGitRunner::default());
    let repository = git
        .discover_repository(RepoRoot::new(&linked_path))
        .expect("discover repository");
    let worktrees = git
        .list_worktrees(ListWorktreesRequest {
            repository: &repository,
        })
        .expect("list worktrees");
    let resolved = git
        .resolve_worktree(ResolveWorktreeRequest {
            repository: &repository,
            worktree_name: "feature-tree",
        })
        .expect("resolve linked worktree");
    let nested_path = linked_path.join("src").join("nested.txt");
    let found = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: &nested_path,
        })
        .expect("find worktree");
    let branches = git
        .list_branches(ListBranchesRequest {
            repository: &repository,
        })
        .expect("list branches");

    assert_eq!(
        worktrees.worktrees.len(),
        2,
        "main and linked worktrees should be visible"
    );
    assert!(
        worktrees
            .worktrees
            .iter()
            .any(|worktree| matches!(worktree.kind(), WorktreeKind::Main)),
        "one worktree should be classified as the main checkout"
    );
    assert!(
        matches!(resolved.kind(), WorktreeKind::Linked { name } if name == "feature-tree"),
        "the resolved worktree should match the linked worktree name"
    );
    assert_eq!(
        found.worktree_root().as_path(),
        linked_path.as_path(),
        "nested paths should resolve back to the owning linked worktree"
    );
    assert_eq!(
        resolved.git_dir().as_path(),
        scaffold
            .repo_path()
            .join(".git")
            .join("worktrees")
            .join("feature-tree"),
        "linked handles should expose the resolved administration directory"
    );
    assert!(
        branches
            .branches
            .iter()
            .any(|branch| branch.as_str() == "main"),
        "the seeded repository should keep its main branch"
    );
    assert!(
        branches
            .branches
            .iter()
            .any(|branch| branch.as_str() == "feature/runtime"),
        "the linked worktree branch should be listed as a local branch"
    );
}

/// Verifies a linked worktree can use Git's valid `main` administration name without colliding with the main checkout.
#[test]
fn runtime_resolves_linked_worktree_named_main() {
    let scaffold = TestScaffold::new("runtime-resolves-linked-main").expect("create scaffold");
    seed_repository(&scaffold);
    let linked_path = scaffold
        .create_linked_worktree("main", "feature/named-main")
        .expect("create linked worktree named main");
    let git = Git::new(CliGitRunner::default());
    let repository = git
        .discover_repository(RepoRoot::new(scaffold.repo_path()))
        .expect("discover repository");

    let resolved = git
        .resolve_worktree(ResolveWorktreeRequest {
            repository: &repository,
            worktree_name: "main",
        })
        .expect("resolve linked worktree named main");

    assert_eq!(resolved.worktree_root().as_path(), linked_path);
    assert!(matches!(
        resolved.kind(),
        WorktreeKind::Linked { name } if name == "main"
    ));
}

/// Verifies linked worktrees remain discoverable when their owning repository is bare and has no checkout HEAD record.
#[test]
fn runtime_discovers_linked_worktrees_from_bare_repositories() {
    let scaffold = TestScaffold::new("runtime-discovers-bare-repository").expect("create scaffold");
    seed_repository(&scaffold);
    let bare_repository = scaffold.sandbox_root().join("bare.git");
    let linked_path = scaffold.linked_worktree_path("bare-linked");
    scaffold
        .run_git_in(
            scaffold.sandbox_root(),
            vec![
                "clone".to_string(),
                "--bare".to_string(),
                scaffold.repo_path().to_string_lossy().into_owned(),
                bare_repository.to_string_lossy().into_owned(),
            ],
        )
        .expect("clone bare repository");
    scaffold
        .run_git_in(
            &bare_repository,
            vec![
                "worktree".to_string(),
                "add".to_string(),
                "-b".to_string(),
                "feature/bare".to_string(),
                linked_path.to_string_lossy().into_owned(),
                "HEAD".to_string(),
            ],
        )
        .expect("create linked worktree from bare repository");
    let git = Git::new(CliGitRunner::default());

    let repository = git
        .discover_repository(RepoRoot::new(&linked_path))
        .expect("discover owning bare repository");
    let worktrees = git
        .list_worktrees(ListWorktreesRequest {
            repository: &repository,
        })
        .expect("list linked worktrees from bare repository");

    assert_eq!(repository.root().as_path(), bare_repository);
    assert_eq!(worktrees.worktrees.len(), 1);
    assert_eq!(
        worktrees.worktrees[0].worktree_root().as_path(),
        linked_path
    );
    assert!(matches!(
        worktrees.worktrees[0].kind(),
        WorktreeKind::Linked { name } if name == "bare-linked"
    ));
}

/// Verifies status, add, and commit flows return typed results when operating inside a linked worktree.
#[test]
fn runtime_reports_status_and_commit_metadata() {
    let scaffold = TestScaffold::new("runtime-status-and-commit").expect("create scaffold");
    seed_repository(&scaffold);
    let linked_path = scaffold
        .create_linked_worktree("feature-tree", "feature/runtime")
        .expect("create linked worktree");
    scaffold
        .write_file(&linked_path, "linked.txt", "linked worktree change\n")
        .expect("write linked file");

    let git = Git::new(CliGitRunner::default());
    let repository = git
        .discover_repository(RepoRoot::new(scaffold.repo_path()))
        .expect("discover repository");
    let worktree = git
        .resolve_worktree(ResolveWorktreeRequest {
            repository: &repository,
            worktree_name: "feature-tree",
        })
        .expect("resolve linked worktree");
    let status_before_add = git
        .status(StatusRequest {
            worktree: &worktree,
        })
        .expect("read worktree status before add");
    let add_result = git
        .add(AddRequest {
            worktree: &worktree,
            paths: vec![
                worktree
                    .resolve_repo_relative_path(Path::new("linked.txt"))
                    .expect("resolve linked file path"),
            ],
        })
        .expect("stage linked file");
    let commit_result = git
        .commit(CommitRequest {
            worktree: &worktree,
            message: "feat: commit linked worktree change",
            allow_empty: false,
        })
        .expect("commit linked worktree change");

    assert!(
        status_before_add
            .entries
            .iter()
            .any(|entry| entry.path().as_path() == Path::new("linked.txt")),
        "status should include the untracked linked file before staging"
    );
    assert_eq!(
        add_result.staged_paths[0].as_path(),
        Path::new("linked.txt"),
        "the staged path should remain repo-relative"
    );
    assert_eq!(
        commit_result.summary, "feat: commit linked worktree change",
        "commit should return the latest summary"
    );
    assert_eq!(
        commit_result.commit_id.as_str().len(),
        40,
        "commit should return a full object ID"
    );
}

/// Verifies real porcelain `-z` output associates a renamed path with its second NUL-delimited original path.
#[test]
fn runtime_reports_rename_paths() {
    let scaffold = TestScaffold::new("runtime-status-rename").expect("create scaffold");
    seed_repository(&scaffold);
    scaffold
        .write_file(scaffold.repo_path(), "old name.txt", "rename me\n")
        .expect("write original file");
    scaffold
        .stage_all_and_commit("test: add rename source")
        .expect("commit original file");
    scaffold
        .run_git(["mv", "old name.txt", "new name.txt"])
        .expect("rename tracked file");
    let (git, repository) = runtime_repository(&scaffold);
    let worktree = git
        .find_worktree(FindWorktreeRequest {
            repository: &repository,
            candidate_path: scaffold.repo_path(),
        })
        .expect("find main worktree");

    let response = git
        .status(StatusRequest {
            worktree: &worktree,
        })
        .expect("read renamed path status");

    assert!(matches!(
        response.entries.as_slice(),
        [StatusEntry::RenamedOrCopied {
            operation: RenameOrCopy::Rename,
            similarity,
            path,
            original_path,
            ..
        }] if similarity.get() == 100
            && path.as_path() == Path::new("new name.txt")
            && original_path.as_path() == Path::new("old name.txt")
    ));
}

/// Verifies repo-relative path resolution rejects traversal attempts that escape the worktree root.
#[test]
fn worktree_rejects_paths_outside_the_checkout() {
    let scaffold = TestScaffold::new("runtime-rejects-outside-paths").expect("create scaffold");
    seed_repository(&scaffold);
    let linked_path = scaffold
        .create_linked_worktree("feature-tree", "feature/runtime")
        .expect("create linked worktree");

    let git = Git::new(CliGitRunner::default());
    let repository = git
        .discover_repository(RepoRoot::new(&linked_path))
        .expect("discover repository");
    let worktree = git
        .resolve_worktree(ResolveWorktreeRequest {
            repository: &repository,
            worktree_name: "feature-tree",
        })
        .expect("resolve linked worktree");
    let outside = scaffold.sandbox_root().join("outside.txt");

    let error = worktree
        .resolve_repo_relative_path(&outside)
        .expect_err("outside paths must be rejected");

    assert!(
        matches!(error, gitlancer::DomainError::PathOutsideWorktree { .. }),
        "paths outside the worktree should fail with PathOutsideWorktree"
    );
}

/// Verifies branch lifecycle APIs create and delete local branches through typed repository requests.
#[test]
fn runtime_creates_and_deletes_local_branches() {
    let scaffold = TestScaffold::new("runtime-branch-lifecycle").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);

    let created = git
        .create_branch(CreateBranchRequest {
            repository: &repository,
            branch_name: branch_name("feature/runtime"),
        })
        .expect("create branch");
    let branches_after_create = git
        .list_branches(ListBranchesRequest {
            repository: &repository,
        })
        .expect("list branches after create");
    let deleted = git
        .delete_branch(DeleteBranchRequest {
            repository: &repository,
            branch_name: branch_name("feature/runtime"),
            mode: BranchDeletionMode::Checked,
        })
        .expect("delete branch");
    let branches_after_delete = git
        .list_branches(ListBranchesRequest {
            repository: &repository,
        })
        .expect("list branches after delete");

    assert_eq!(created.branch, branch_name("feature/runtime"));
    assert!(
        branches_after_create
            .branches
            .iter()
            .any(|branch| branch.as_str() == "feature/runtime"),
        "created branches should be visible through list_branches"
    );
    assert_eq!(deleted.branch, branch_name("feature/runtime"));
    assert!(
        !branches_after_delete
            .branches
            .iter()
            .any(|branch| branch.as_str() == "feature/runtime"),
        "deleted branches should no longer be visible through list_branches"
    );
}

/// Verifies linked worktree lifecycle APIs create and delete linked worktrees through typed runtime requests.
#[test]
fn runtime_creates_and_deletes_linked_worktrees() {
    let scaffold = TestScaffold::new("runtime-worktree-lifecycle").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let worktree_path = scaffold.linked_worktree_path("feature-tree");

    let created = git
        .create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new(&worktree_path),
            branch_name: branch_name("feature/runtime"),
            identity_token: identity_token("worktree-runtime"),
        })
        .expect("create worktree");
    let worktrees_after_create = git
        .list_worktrees(ListWorktreesRequest {
            repository: &repository,
        })
        .expect("list worktrees after create");
    let deleted = git
        .delete_worktree(DeleteWorktreeRequest {
            repository: &repository,
            worktree: &created.worktree,
            mode: WorktreeDeletionMode::Checked,
        })
        .expect("delete linked worktree");
    let deleted_again = git
        .delete_worktree(DeleteWorktreeRequest {
            repository: &repository,
            worktree: &created.worktree,
            mode: WorktreeDeletionMode::Checked,
        })
        .expect("repeat linked worktree deletion");
    let worktrees_after_delete = git
        .list_worktrees(ListWorktreesRequest {
            repository: &repository,
        })
        .expect("list worktrees after delete");

    assert!(
        matches!(created.worktree.kind(), WorktreeKind::Linked { name } if name == "feature-tree"),
        "created worktrees should come back as linked worktrees"
    );
    assert!(
        worktrees_after_create
            .worktrees
            .iter()
            .any(|worktree| worktree.worktree_root().as_path() == worktree_path.as_path()),
        "created worktrees should be visible through list_worktrees"
    );
    let listed_identity = worktrees_after_create
        .worktrees
        .iter()
        .find(|worktree| worktree.worktree_root().as_path() == worktree_path.as_path())
        .and_then(gitlancer::WorktreeHandle::identity_token)
        .map(WorktreeIdentityToken::as_str);
    assert_eq!(listed_identity, Some("worktree-runtime"));
    assert_eq!(deleted.worktree_root, WorktreeRoot::new(&worktree_path));
    assert_eq!(deleted.outcome, WorktreeDeletionOutcome::Removed);
    assert_eq!(
        deleted_again.outcome,
        WorktreeDeletionOutcome::AlreadyAbsent
    );
    assert!(
        !worktrees_after_delete
            .worktrees
            .iter()
            .any(|worktree| worktree.worktree_root().as_path() == worktree_path.as_path()),
        "deleted worktrees should no longer be visible through list_worktrees"
    );
}

/// Verifies stale Git administration metadata is pruned after a checkout disappears externally.
#[test]
fn runtime_prunes_missing_linked_worktree_metadata() {
    let scaffold = TestScaffold::new("runtime-prunable-worktree").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let worktree_path = scaffold.linked_worktree_path("missing-tree");
    git.create_worktree(CreateWorktreeRequest {
        repository: &repository,
        worktree_root: WorktreeRoot::new(&worktree_path),
        branch_name: branch_name("feature/missing"),
        identity_token: identity_token("worktree-missing"),
    })
    .expect("create worktree");
    std::fs::remove_dir_all(&worktree_path).expect("remove checkout outside Git");
    let before = scaffold
        .run_git(["worktree", "list", "--porcelain"])
        .expect("list stale worktree metadata");
    assert!(before.contains("refs/heads/feature/missing"));

    git.prune_worktrees(PruneWorktreesRequest {
        repository: &repository,
    })
    .expect("prune stale worktree metadata");

    let after = scaffold
        .run_git(["worktree", "list", "--porcelain"])
        .expect("list worktrees after prune");
    assert!(!after.contains("refs/heads/feature/missing"));
}

/// Verifies a stale handle cannot delete a different branch recreated at the same checkout path.
#[test]
fn runtime_rejects_recreated_worktree_at_the_same_path() {
    let scaffold = TestScaffold::new("runtime-recreated-worktree").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let worktree_path = scaffold.linked_worktree_path("reused-tree");
    let stale = git
        .create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new(&worktree_path),
            branch_name: branch_name("feature/original"),
            identity_token: identity_token("worktree-original"),
        })
        .expect("create original worktree")
        .worktree;
    git.delete_worktree(DeleteWorktreeRequest {
        repository: &repository,
        worktree: &stale,
        mode: WorktreeDeletionMode::Force,
    })
    .expect("remove original worktree");
    let replacement = git
        .create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new(&worktree_path),
            branch_name: branch_name("feature/replacement"),
            identity_token: identity_token("worktree-replacement"),
        })
        .expect("create replacement worktree")
        .worktree;

    let error = git
        .delete_worktree(DeleteWorktreeRequest {
            repository: &repository,
            worktree: &stale,
            mode: WorktreeDeletionMode::Force,
        })
        .expect_err("stale handle must not delete replacement worktree");

    assert!(matches!(
        error,
        gitlancer::GitlancerError::Domain(gitlancer::DomainError::WorktreeIdentityChanged {
            expected_branch,
            actual_branch,
            ..
        }) if expected_branch.as_deref() == Some("feature/original")
            && actual_branch.as_deref() == Some("feature/replacement")
    ));
    assert!(
        worktree_path.exists(),
        "replacement checkout must remain present"
    );
    git.delete_worktree(DeleteWorktreeRequest {
        repository: &repository,
        worktree: &replacement,
        mode: WorktreeDeletionMode::Force,
    })
    .expect("clean up replacement worktree");
}

/// Verifies path, branch, and Git administration-name reuse cannot satisfy a stale handle.
#[test]
fn runtime_rejects_recreated_worktree_with_the_same_branch() {
    let scaffold = TestScaffold::new("runtime-recreated-same-branch").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let worktree_path = scaffold.linked_worktree_path("reused-same-branch");
    let stale = git
        .create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new(&worktree_path),
            branch_name: branch_name("feature/same"),
            identity_token: identity_token("worktree-same-original"),
        })
        .expect("create original worktree")
        .worktree;
    git.delete_worktree(DeleteWorktreeRequest {
        repository: &repository,
        worktree: &stale,
        mode: WorktreeDeletionMode::Force,
    })
    .expect("remove original worktree");
    let worktree_arg = worktree_path.to_string_lossy().into_owned();
    scaffold
        .run_git(["worktree", "add", &worktree_arg, "feature/same"])
        .expect("recreate the same branch outside the managed runtime");

    let error = git
        .delete_worktree(DeleteWorktreeRequest {
            repository: &repository,
            worktree: &stale,
            mode: WorktreeDeletionMode::Force,
        })
        .expect_err("stale marker must not match an unmanaged replacement");

    assert!(matches!(
        error,
        gitlancer::GitlancerError::Domain(gitlancer::DomainError::WorktreeIdentityChanged {
            expected_identity_token,
            actual_identity_token,
            ..
        }) if expected_identity_token.as_deref() == Some("worktree-same-original")
            && actual_identity_token.is_none()
    ));
    assert!(
        worktree_path.exists(),
        "replacement checkout must remain present"
    );
    scaffold
        .run_git(["worktree", "remove", "--force", &worktree_arg])
        .expect("clean up unmanaged replacement worktree");
}

/// Verifies exact-root discovery does not mistake the main checkout for an absent nested worktree.
#[test]
fn runtime_exact_root_discovery_rejects_absent_nested_worktree() {
    let scaffold = TestScaffold::new("runtime-exact-worktree-root").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let absent_root = scaffold.repo_path().join(".worktrees").join("missing");

    let result = git.find_worktree_root(FindWorktreeRootRequest {
        repository: &repository,
        worktree_root: &absent_root,
    });

    assert!(matches!(
        result,
        Err(gitlancer::GitlancerError::Domain(
            gitlancer::DomainError::NotAWorktree(_)
        ))
    ));
}

/// Verifies main-worktree deletion is rejected before Git attempts a destructive worktree removal.
#[test]
fn runtime_rejects_main_worktree_deletion() {
    let scaffold =
        TestScaffold::new("runtime-rejects-main-worktree-delete").expect("create scaffold");
    seed_repository(&scaffold);
    let (git, repository) = runtime_repository(&scaffold);
    let worktrees = git
        .list_worktrees(ListWorktreesRequest {
            repository: &repository,
        })
        .expect("list worktrees");
    let main_worktree = worktrees
        .worktrees
        .into_iter()
        .find(|worktree| matches!(worktree.kind(), WorktreeKind::Main))
        .expect("main worktree");

    let error = git
        .delete_worktree(DeleteWorktreeRequest {
            repository: &repository,
            worktree: &main_worktree,
            mode: WorktreeDeletionMode::Checked,
        })
        .expect_err("main worktree deletion should be rejected");

    assert!(
        matches!(
            error,
            gitlancer::GitlancerError::Domain(
                gitlancer::DomainError::MainWorktreeDeletionUnsupported(repo)
            ) if repo == repository.root().as_path()
        ),
        "main worktree deletion should fail with MainWorktreeDeletionUnsupported"
    );
}

/// Verifies worktree deletion rejects linked worktrees that do not belong to the supplied repository.
#[test]
fn runtime_rejects_cross_repository_worktree_deletion() {
    let left = TestScaffold::new("runtime-worktree-mismatch-left").expect("create left scaffold");
    let right =
        TestScaffold::new("runtime-worktree-mismatch-right").expect("create right scaffold");
    seed_repository(&left);
    seed_repository(&right);

    let (left_git, left_repository) = runtime_repository(&left);
    let (_, right_repository) = runtime_repository(&right);
    let linked_path = left
        .create_linked_worktree("feature-tree", "feature/runtime")
        .expect("create linked worktree");
    let linked_worktree = left_git
        .resolve_worktree(ResolveWorktreeRequest {
            repository: &left_repository,
            worktree_name: "feature-tree",
        })
        .expect("resolve linked worktree");

    let error = left_git
        .delete_worktree(DeleteWorktreeRequest {
            repository: &right_repository,
            worktree: &linked_worktree,
            mode: WorktreeDeletionMode::Checked,
        })
        .expect_err("cross-repository worktree deletion should be rejected");

    assert!(
        matches!(
            error,
            gitlancer::GitlancerError::Domain(gitlancer::DomainError::WorktreeMismatch {
                worktree,
                repo,
            }) if worktree == linked_path && repo == right_repository.root().as_path()
        ),
        "cross-repository deletions should fail with WorktreeMismatch"
    );
}
