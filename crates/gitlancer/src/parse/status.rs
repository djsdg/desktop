use crate::domain::paths::RepoRelativePath;
use crate::error::ParseError;
use crate::git::status::{
    ChangeKind, FileMode, RenameOrCopy, SimilarityScore, StatusEntry, StatusObjectId,
    SubmoduleStatus, TrackedStatus,
};

/// Parses porcelain-v2 `-z` output into typed status entries while preserving NUL-delimited paths.
pub fn parse_status_v2(stdout: &str) -> Result<Vec<StatusEntry>, ParseError> {
    let records = stdout.split('\0').collect::<Vec<_>>();
    let mut entries = Vec::new();
    let mut cursor = 0;

    while cursor < records.len() {
        let record = records[cursor];
        cursor += 1;

        if record.is_empty() || record.starts_with("# ") {
            continue;
        }

        let entry = match record.as_bytes().first().copied() {
            Some(b'1') => parse_ordinary(record)?,
            Some(b'2') => {
                let original_path = records.get(cursor).ok_or(ParseError::InvalidStatus)?;
                cursor += 1;
                parse_renamed_or_copied(record, original_path)?
            }
            Some(b'u') => parse_unmerged(record)?,
            Some(b'?') => parse_untracked(record)?,
            Some(b'!') => parse_ignored(record)?,
            _ => return Err(ParseError::InvalidStatus),
        };
        entries.push(entry);
    }

    Ok(entries)
}

/// Parses a porcelain v2 type `1` record for an ordinary tracked path.
fn parse_ordinary(record: &str) -> Result<StatusEntry, ParseError> {
    let fields = split_fields::<9>(record)?;
    if fields[0] != "1" {
        return Err(ParseError::InvalidStatus);
    }

    Ok(StatusEntry::Ordinary {
        status: parse_tracked_status(fields[1])?,
        submodule: parse_submodule_status(fields[2])?,
        head_mode: parse_file_mode(fields[3])?,
        index_mode: parse_file_mode(fields[4])?,
        worktree_mode: parse_file_mode(fields[5])?,
        head_object_id: parse_object_id(fields[6])?,
        index_object_id: parse_object_id(fields[7])?,
        path: parse_path(fields[8])?,
    })
}

/// Parses a porcelain v2 type `2` record and consumes its separately NUL-delimited original path.
fn parse_renamed_or_copied(record: &str, original_path: &str) -> Result<StatusEntry, ParseError> {
    let fields = split_fields::<10>(record)?;
    if fields[0] != "2" {
        return Err(ParseError::InvalidStatus);
    }
    let (operation, similarity) = parse_rename_or_copy(fields[8])?;

    Ok(StatusEntry::RenamedOrCopied {
        status: parse_tracked_status(fields[1])?,
        submodule: parse_submodule_status(fields[2])?,
        head_mode: parse_file_mode(fields[3])?,
        index_mode: parse_file_mode(fields[4])?,
        worktree_mode: parse_file_mode(fields[5])?,
        head_object_id: parse_object_id(fields[6])?,
        index_object_id: parse_object_id(fields[7])?,
        operation,
        similarity,
        path: parse_path(fields[9])?,
        original_path: parse_path(original_path)?,
    })
}

/// Parses a porcelain v2 type `u` record while retaining all three conflicted index stages.
fn parse_unmerged(record: &str) -> Result<StatusEntry, ParseError> {
    let fields = split_fields::<11>(record)?;
    if fields[0] != "u" {
        return Err(ParseError::InvalidStatus);
    }

    Ok(StatusEntry::Unmerged {
        status: parse_tracked_status(fields[1])?,
        submodule: parse_submodule_status(fields[2])?,
        stage_one_mode: parse_file_mode(fields[3])?,
        stage_two_mode: parse_file_mode(fields[4])?,
        stage_three_mode: parse_file_mode(fields[5])?,
        worktree_mode: parse_file_mode(fields[6])?,
        stage_one_object_id: parse_object_id(fields[7])?,
        stage_two_object_id: parse_object_id(fields[8])?,
        stage_three_object_id: parse_object_id(fields[9])?,
        path: parse_path(fields[10])?,
    })
}

/// Parses an untracked path while requiring the exact porcelain record prefix.
fn parse_untracked(record: &str) -> Result<StatusEntry, ParseError> {
    let path = record.strip_prefix("? ").ok_or(ParseError::InvalidStatus)?;
    Ok(StatusEntry::Untracked {
        path: parse_path(path)?,
    })
}

/// Parses an ignored path while requiring the exact porcelain record prefix.
fn parse_ignored(record: &str) -> Result<StatusEntry, ParseError> {
    let path = record.strip_prefix("! ").ok_or(ParseError::InvalidStatus)?;
    Ok(StatusEntry::Ignored {
        path: parse_path(path)?,
    })
}

/// Splits a fixed-width record while leaving the final pathname field untouched, including spaces.
fn split_fields<const N: usize>(record: &str) -> Result<[&str; N], ParseError> {
    record
        .splitn(N, ' ')
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| ParseError::InvalidStatus)
}

/// Decodes the index/worktree pair shared by tracked and unmerged records.
fn parse_tracked_status(value: &str) -> Result<TrackedStatus, ParseError> {
    let mut characters = value.chars();
    let index = characters.next().ok_or(ParseError::InvalidStatus)?;
    let worktree = characters.next().ok_or(ParseError::InvalidStatus)?;
    if characters.next().is_some() {
        return Err(ParseError::InvalidStatus);
    }

    Ok(TrackedStatus {
        index: parse_change_kind(index)?,
        worktree: parse_change_kind(worktree)?,
    })
}

/// Maps one porcelain status code to a closed set so malformed codes cannot leak into callers.
fn parse_change_kind(value: char) -> Result<ChangeKind, ParseError> {
    match value {
        '.' => Ok(ChangeKind::Unmodified),
        'M' => Ok(ChangeKind::Modified),
        'T' => Ok(ChangeKind::FileTypeChanged),
        'A' => Ok(ChangeKind::Added),
        'D' => Ok(ChangeKind::Deleted),
        'R' => Ok(ChangeKind::Renamed),
        'C' => Ok(ChangeKind::Copied),
        'U' => Ok(ChangeKind::Unmerged),
        _ => Err(ParseError::InvalidStatus),
    }
}

/// Decodes Git's positional submodule flags into named booleans.
fn parse_submodule_status(value: &str) -> Result<SubmoduleStatus, ParseError> {
    if value == "N..." {
        return Ok(SubmoduleStatus::NotSubmodule);
    }

    let bytes = value.as_bytes();
    if bytes.len() != 4
        || bytes[0] != b'S'
        || !matches!(bytes[1], b'.' | b'C')
        || !matches!(bytes[2], b'.' | b'M')
        || !matches!(bytes[3], b'.' | b'U')
    {
        return Err(ParseError::InvalidStatus);
    }

    Ok(SubmoduleStatus::Submodule {
        commit_changed: bytes[1] == b'C',
        tracked_changes: bytes[2] == b'M',
        untracked_changes: bytes[3] == b'U',
    })
}

/// Validates the six-digit octal mode used by Git's status porcelain.
fn parse_file_mode(value: &str) -> Result<FileMode, ParseError> {
    if value.len() != 6 || !value.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
        return Err(ParseError::InvalidStatus);
    }

    Ok(FileMode::from_validated(value))
}

/// Validates SHA-1 and SHA-256 object IDs while accepting Git's all-zero missing-object sentinel.
fn parse_object_id(value: &str) -> Result<StatusObjectId, ParseError> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ParseError::InvalidStatus);
    }

    Ok(StatusObjectId::from_validated(value))
}

/// Parses the rename/copy classification and enforces Git's similarity percentage range.
fn parse_rename_or_copy(value: &str) -> Result<(RenameOrCopy, SimilarityScore), ParseError> {
    let (operation, score) = match value.as_bytes().first() {
        Some(b'R') => (RenameOrCopy::Rename, &value[1..]),
        Some(b'C') => (RenameOrCopy::Copy, &value[1..]),
        _ => return Err(ParseError::InvalidStatus),
    };
    let score = score.parse::<u8>().map_err(|_| ParseError::InvalidStatus)?;
    if score > 100 {
        return Err(ParseError::InvalidStatus);
    }

    Ok((operation, SimilarityScore::from_validated(score)))
}

/// Wraps a non-empty pathname emitted by Git as repository-relative status data.
fn parse_path(value: &str) -> Result<RepoRelativePath, ParseError> {
    if value.is_empty() {
        return Err(ParseError::InvalidStatus);
    }

    RepoRelativePath::new(value).map_err(|_| ParseError::InvalidStatus)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    const FIRST_OBJECT_ID: &str = "0123456789abcdef0123456789abcdef01234567";
    const SECOND_OBJECT_ID: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const THIRD_OBJECT_ID: &str = "abcdef0123456789abcdef0123456789abcdef01";

    /// Creates a validated test mode so expected values exercise deep equality.
    fn mode(value: &str) -> FileMode {
        FileMode::from_validated(value)
    }

    /// Creates a validated test object ID so expected values exercise deep equality.
    fn object_id(value: &str) -> StatusObjectId {
        StatusObjectId::from_validated(value)
    }

    /// Creates a repository-relative test path for deep status comparisons.
    fn path(value: &str) -> RepoRelativePath {
        RepoRelativePath::new(value).expect("test path should be repository-relative")
    }

    /// Verifies every supported record kind is decoded without losing paths or status metadata.
    #[test]
    fn parses_all_supported_record_kinds() {
        let stdout = format!(
            "# branch.head main\0\
             1 M. N... 100644 100755 100755 {FIRST_OBJECT_ID} {SECOND_OBJECT_ID} ordinary file.txt\0\
             2 R. SCMU 100644 100644 100644 {FIRST_OBJECT_ID} {SECOND_OBJECT_ID} R087 renamed\nfile.txt\0\
             original file.txt\0\
             2 C. N... 100644 100644 100644 {FIRST_OBJECT_ID} {SECOND_OBJECT_ID} C100 copied file.txt\0\
             copy source.txt\0\
             u UU N... 100644 100644 100755 100755 {FIRST_OBJECT_ID} {SECOND_OBJECT_ID} {THIRD_OBJECT_ID} conflicted.txt\0\
             ? untracked file.txt\0\
             ! ignored file.txt\0"
        );

        let entries = parse_status_v2(&stdout).expect("parse all status record kinds");

        assert_eq!(
            entries,
            vec![
                StatusEntry::Ordinary {
                    status: TrackedStatus {
                        index: ChangeKind::Modified,
                        worktree: ChangeKind::Unmodified,
                    },
                    submodule: SubmoduleStatus::NotSubmodule,
                    head_mode: mode("100644"),
                    index_mode: mode("100755"),
                    worktree_mode: mode("100755"),
                    head_object_id: object_id(FIRST_OBJECT_ID),
                    index_object_id: object_id(SECOND_OBJECT_ID),
                    path: path("ordinary file.txt"),
                },
                StatusEntry::RenamedOrCopied {
                    status: TrackedStatus {
                        index: ChangeKind::Renamed,
                        worktree: ChangeKind::Unmodified,
                    },
                    submodule: SubmoduleStatus::Submodule {
                        commit_changed: true,
                        tracked_changes: true,
                        untracked_changes: true,
                    },
                    head_mode: mode("100644"),
                    index_mode: mode("100644"),
                    worktree_mode: mode("100644"),
                    head_object_id: object_id(FIRST_OBJECT_ID),
                    index_object_id: object_id(SECOND_OBJECT_ID),
                    operation: RenameOrCopy::Rename,
                    similarity: SimilarityScore::from_validated(87),
                    path: path("renamed\nfile.txt"),
                    original_path: path("original file.txt"),
                },
                StatusEntry::RenamedOrCopied {
                    status: TrackedStatus {
                        index: ChangeKind::Copied,
                        worktree: ChangeKind::Unmodified,
                    },
                    submodule: SubmoduleStatus::NotSubmodule,
                    head_mode: mode("100644"),
                    index_mode: mode("100644"),
                    worktree_mode: mode("100644"),
                    head_object_id: object_id(FIRST_OBJECT_ID),
                    index_object_id: object_id(SECOND_OBJECT_ID),
                    operation: RenameOrCopy::Copy,
                    similarity: SimilarityScore::from_validated(100),
                    path: path("copied file.txt"),
                    original_path: path("copy source.txt"),
                },
                StatusEntry::Unmerged {
                    status: TrackedStatus {
                        index: ChangeKind::Unmerged,
                        worktree: ChangeKind::Unmerged,
                    },
                    submodule: SubmoduleStatus::NotSubmodule,
                    stage_one_mode: mode("100644"),
                    stage_two_mode: mode("100644"),
                    stage_three_mode: mode("100755"),
                    worktree_mode: mode("100755"),
                    stage_one_object_id: object_id(FIRST_OBJECT_ID),
                    stage_two_object_id: object_id(SECOND_OBJECT_ID),
                    stage_three_object_id: object_id(THIRD_OBJECT_ID),
                    path: path("conflicted.txt"),
                },
                StatusEntry::Untracked {
                    path: path("untracked file.txt"),
                },
                StatusEntry::Ignored {
                    path: path("ignored file.txt"),
                },
            ]
        );
    }

    /// Verifies a rename's original path is mandatory because it is a second NUL-delimited field.
    #[test]
    fn rejects_rename_without_original_path() {
        let stdout = format!(
            "2 R. N... 100644 100644 100644 {FIRST_OBJECT_ID} {SECOND_OBJECT_ID} R100 renamed.txt"
        );

        let result = parse_status_v2(&stdout);

        assert!(matches!(result, Err(ParseError::InvalidStatus)));
    }

    /// Verifies malformed structured fields fail closed rather than leaking partially typed records.
    #[test]
    fn rejects_invalid_structured_fields() {
        let invalid_records = [
            format!(
                "1 X. N... 100644 100644 100644 {FIRST_OBJECT_ID} {SECOND_OBJECT_ID} path.txt\0"
            ),
            format!(
                "1 M. BAD! 100644 100644 100644 {FIRST_OBJECT_ID} {SECOND_OBJECT_ID} path.txt\0"
            ),
            format!(
                "1 M. N... invalid 100644 100644 {FIRST_OBJECT_ID} {SECOND_OBJECT_ID} path.txt\0"
            ),
            format!("1 M. N... 100644 100644 100644 short {SECOND_OBJECT_ID} path.txt\0"),
            format!(
                "2 R. N... 100644 100644 100644 {FIRST_OBJECT_ID} {SECOND_OBJECT_ID} R101 renamed.txt\0original.txt\0"
            ),
            "? ../outside.txt\0".to_string(),
            "? /absolute.txt\0".to_string(),
            "x unsupported\0".to_string(),
        ];

        for record in invalid_records {
            assert!(
                matches!(parse_status_v2(&record), Err(ParseError::InvalidStatus)),
                "record should be rejected: {record:?}"
            );
        }
    }
}
