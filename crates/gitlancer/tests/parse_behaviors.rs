use gitlancer::parse::commit::{parse_commit_id, parse_commit_response};
use gitlancer::parse::status::parse_status_v2;
use gitlancer::parse::worktree::parse_worktree_list;
use pretty_assertions::assert_eq;

/// Verifies commit metadata parsing returns the commit ID and summary from the readback payload.
#[test]
fn parse_commit_response_reads_commit_id_and_summary() {
    let response = parse_commit_response(
        "0123456789abcdef0123456789abcdef01234567\nfeat: add linked worktree support\n",
    )
    .expect("parse commit response");

    assert_eq!(
        response.commit_id.as_str(),
        "0123456789abcdef0123456789abcdef01234567",
        "commit parser should preserve the full object ID"
    );
    assert_eq!(
        response.summary, "feat: add linked worktree support",
        "commit parser should preserve the latest summary"
    );
}

/// Verifies commit ID parsing trims whitespace and returns the first non-empty line.
#[test]
fn parse_commit_id_reads_first_non_empty_line() {
    let commit_id =
        parse_commit_id("\n 0123456789abcdef0123456789abcdef01234567 \n").expect("parse commit id");

    assert_eq!(
        commit_id.as_str(),
        "0123456789abcdef0123456789abcdef01234567",
        "commit ID parser should trim the selected line"
    );
}

/// Verifies porcelain v2 status parsing returns typed records with repository-relative paths.
#[test]
fn parse_status_v2_returns_typed_records() {
    let object_id = "0123456789abcdef0123456789abcdef01234567";
    let entries = parse_status_v2(&format!(
        "? untracked.txt\0# branch.head main\01 M. N... 100644 100644 100644 {object_id} {object_id} tracked file.txt\0"
    ))
    .expect("parse status");

    assert_eq!(entries.len(), 2, "two status records should be returned");
    assert!(
        matches!(
            entries[0],
            gitlancer::git::status::StatusEntry::Untracked { .. }
        ),
        "the untracked record should retain its semantic kind"
    );
    assert!(
        matches!(
            entries[1],
            gitlancer::git::status::StatusEntry::Ordinary { .. }
        ),
        "the tracked record should retain its semantic kind"
    );
    assert_eq!(
        entries[1].path().as_path(),
        std::path::Path::new("tracked file.txt")
    );
}

/// Verifies worktree parsing preserves machine-readable identity and state fields.
#[test]
fn parse_worktree_list_preserves_nul_delimited_records() {
    let output = concat!(
        "worktree /tmp/repo\0",
        "HEAD 0123456789abcdef0123456789abcdef01234567\0",
        "branch refs/heads/main\0\0",
        "worktree /tmp/worktrees/feature\n tree\0",
        "HEAD 89abcdef0123456789abcdef0123456789abcdef\0",
        "detached\0",
        "locked maintenance\0\0",
    );

    let worktrees = parse_worktree_list(output).expect("parse worktrees");

    assert_eq!(
        worktrees,
        vec![
            gitlancer::parse::worktree::ParsedWorktree::Checkout {
                worktree_root: gitlancer::WorktreeRoot::new("/tmp/repo"),
                head_commit_id: gitlancer::CommitId::new(
                    "0123456789abcdef0123456789abcdef01234567"
                )
                .expect("test commit id should be valid"),
                branch: Some(
                    gitlancer::BranchName::new("main").expect("test branch should be valid"),
                ),
                detached: false,
                locked_reason: None,
                prunable_reason: None,
            },
            gitlancer::parse::worktree::ParsedWorktree::Checkout {
                worktree_root: gitlancer::WorktreeRoot::new("/tmp/worktrees/feature\n tree"),
                head_commit_id: gitlancer::CommitId::new(
                    "89abcdef0123456789abcdef0123456789abcdef"
                )
                .expect("test commit id should be valid"),
                branch: None,
                detached: true,
                locked_reason: Some("maintenance".to_string()),
                prunable_reason: None,
            },
        ]
    );
}

/// Verifies a bare repository record remains usable even though Git omits checkout-only HEAD state.
#[test]
fn parse_worktree_list_preserves_bare_repository_roots() {
    let output = "worktree /tmp/repo.git\0bare\0\0";

    let worktrees = parse_worktree_list(output).expect("parse bare repository");

    assert_eq!(
        worktrees,
        vec![gitlancer::parse::worktree::ParsedWorktree::Bare {
            repository_root: gitlancer::RepoRoot::new("/tmp/repo.git"),
        }]
    );
}
