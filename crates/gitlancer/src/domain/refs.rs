use crate::error::DomainError;

/// Identifies a validated local branch name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BranchName(String);

impl BranchName {
    /// Creates a local branch name after enforcing Git's ref-format safety constraints.
    pub fn new(name: impl Into<String>) -> Result<Self, DomainError> {
        let name = name.into();
        if !is_valid_branch_name(&name) {
            return Err(DomainError::InvalidBranchName(name));
        }

        Ok(Self(name))
    }

    /// Exposes the branch name for command assembly and display.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifies a commit by its object ID as returned by Git.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitId(String);

impl CommitId {
    /// Creates a full SHA-1 or SHA-256 commit identifier after validating its hexadecimal shape.
    pub fn new(id: impl Into<String>) -> Result<Self, DomainError> {
        let id = id.into();
        if !matches!(id.len(), 40 | 64) || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainError::InvalidCommitId(id));
        }

        Ok(Self(id))
    }

    /// Exposes the commit identifier for command assembly and display.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifies one application-managed worktree instance across path and branch reuse.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorktreeIdentityToken(String);

impl WorktreeIdentityToken {
    /// Creates a bounded filesystem-safe token suitable for a Git administration marker.
    pub fn new(token: impl Into<String>) -> Result<Self, DomainError> {
        let token = token.into();
        if token.is_empty()
            || token.len() > 128
            || !token.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(DomainError::InvalidWorktreeIdentityToken(token));
        }

        Ok(Self(token))
    }

    /// Exposes the validated token for marker persistence and identity comparisons.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Mirrors Git's local ref restrictions without spawning a validation process at each call site.
fn is_valid_branch_name(name: &str) -> bool {
    if name.is_empty()
        || name == "@"
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.contains("..")
        || name.contains("@{")
        || name.contains("//")
        || name.bytes().any(|byte| {
            byte.is_ascii_control()
                || matches!(byte, b' ' | b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
        })
    {
        return false;
    }

    name.split('/').all(|component| {
        !component.is_empty()
            && component != "."
            && component != ".."
            && !component.starts_with('.')
            && !component.ends_with(".lock")
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{BranchName, CommitId, WorktreeIdentityToken};
    use crate::DomainError;

    /// Verifies branch names cannot smuggle option syntax, traversal-like refs, or forbidden Git characters.
    #[test]
    fn validates_local_branch_names() {
        let branch = BranchName::new("feature/runtime").expect("valid branch should be accepted");
        assert_eq!(branch.as_str(), "feature/runtime");
        for invalid in ["", "-force", "feature..old", "feature.lock", "bad name"] {
            assert!(matches!(
                BranchName::new(invalid),
                Err(DomainError::InvalidBranchName(_))
            ));
        }
    }

    /// Verifies only full Git object identifiers can cross the typed commit boundary.
    #[test]
    fn validates_commit_object_ids() {
        let sha1 = "0123456789abcdef0123456789abcdef01234567";
        let commit = CommitId::new(sha1).expect("valid commit should be accepted");
        assert_eq!(commit.as_str(), sha1);
        assert!(matches!(
            CommitId::new("base-commit"),
            Err(DomainError::InvalidCommitId(_))
        ));
    }

    /// Verifies worktree identity markers remain bounded and safe to persist as plain text.
    #[test]
    fn validates_worktree_identity_tokens() {
        let token = WorktreeIdentityToken::new("worktree-0123_abcd")
            .expect("test identity token should be valid");
        assert_eq!(token.as_str(), "worktree-0123_abcd");
        for invalid in ["", "contains/slash", "contains newline\n"] {
            assert!(matches!(
                WorktreeIdentityToken::new(invalid),
                Err(DomainError::InvalidWorktreeIdentityToken(_))
            ));
        }
    }
}
