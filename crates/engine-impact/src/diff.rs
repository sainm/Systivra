use serde::{Deserialize, Serialize};

/// A contiguous changed block between two text revisions.
///
/// Line numbers are one-based and inclusive. An insertion-only hunk uses the
/// insertion point for both `old_start` and `old_end`; a deletion-only hunk
/// uses the deletion point for both `new_start` and `new_end`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineHunk {
    pub old_start: i32,
    pub old_end: i32,
    pub new_start: i32,
    pub new_end: i32,
    pub removed: i32,
    pub added: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnifiedHunk {
    pub path: String,
    pub old_start: i32,
    pub old_end: i32,
    pub new_start: i32,
    pub new_end: i32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DiffParseError {
    #[error("unified diff の hunk header が不正です: {0}")]
    HunkHeader(String),
    #[error("unified diff の file header が不正です: {0}")]
    FileHeader(String),
}

/// Parses the changed ranges from a standard Git unified diff.
pub fn parse_unified_diff(text: &str) -> Result<Vec<UnifiedHunk>, DiffParseError> {
    let mut path: Option<String> = None;
    let mut hunks = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("+++ ") {
            let value = value.trim();
            if value == "/dev/null" {
                path = None;
            } else {
                path = Some(
                    value
                        .strip_prefix("b/")
                        .unwrap_or(value)
                        .split('\t')
                        .next()
                        .unwrap_or(value)
                        .to_string(),
                );
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("diff --git ") {
            let parts = value.split_whitespace().collect::<Vec<_>>();
            if parts.len() < 2 {
                return Err(DiffParseError::FileHeader(line.to_string()));
            }
            path = Some(parts[1].strip_prefix("b/").unwrap_or(parts[1]).to_string());
            continue;
        }
        if !line.starts_with("@@ ") {
            continue;
        }
        let (old_start, old_count, new_start, new_count) = parse_hunk_header(line)?;
        let Some(current_path) = path.clone() else {
            continue;
        };
        hunks.push(UnifiedHunk {
            path: current_path,
            old_start,
            old_end: range_end(old_start, old_count),
            new_start,
            new_end: range_end(new_start, new_count),
        });
    }
    Ok(hunks)
}

fn parse_hunk_header(line: &str) -> Result<(i32, i32, i32, i32), DiffParseError> {
    let mut parts = line.split_whitespace();
    let _at = parts.next();
    let old = parts
        .next()
        .and_then(|value| value.strip_prefix('-'))
        .ok_or_else(|| DiffParseError::HunkHeader(line.to_string()))?;
    let new = parts
        .next()
        .and_then(|value| value.strip_prefix('+'))
        .ok_or_else(|| DiffParseError::HunkHeader(line.to_string()))?;
    let (old_start, old_count) = parse_range(old, line)?;
    let (new_start, new_count) = parse_range(new, line)?;
    Ok((old_start, old_count, new_start, new_count))
}

fn parse_range(value: &str, line: &str) -> Result<(i32, i32), DiffParseError> {
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    Ok((
        start
            .parse()
            .map_err(|_| DiffParseError::HunkHeader(line.to_string()))?,
        count
            .parse()
            .map_err(|_| DiffParseError::HunkHeader(line.to_string()))?,
    ))
}

fn range_end(start: i32, count: i32) -> i32 {
    if count == 0 {
        start
    } else {
        start + count - 1
    }
}

/// Deterministic line diff used for line-level impact seeds.
///
/// Equal prefix/suffix lines are removed before the LCS. Very large changed
/// middle sections fall back to one explicit whole-hunk change instead of
/// allocating an unbounded quadratic matrix.
pub fn line_hunks(old: &str, new: &str) -> Vec<LineHunk> {
    let old_lines = old.lines().collect::<Vec<_>>();
    let new_lines = new.lines().collect::<Vec<_>>();

    let mut prefix = 0usize;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while suffix < old_lines.len().saturating_sub(prefix)
        && suffix < new_lines.len().saturating_sub(prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_end = old_lines.len().saturating_sub(suffix);
    let new_end = new_lines.len().saturating_sub(suffix);
    let old_mid = &old_lines[prefix..old_end];
    let new_mid = &new_lines[prefix..new_end];
    if old_mid.is_empty() && new_mid.is_empty() {
        return Vec::new();
    }

    const MAX_LCS_CELLS: usize = 4_000_000;
    if old_mid.len().saturating_mul(new_mid.len()) > MAX_LCS_CELLS {
        return vec![whole_hunk(prefix, old_mid.len(), new_mid.len())];
    }

    let mut dp = vec![vec![0u32; new_mid.len() + 1]; old_mid.len() + 1];
    for i in (0..old_mid.len()).rev() {
        for j in (0..new_mid.len()).rev() {
            dp[i][j] = if old_mid[i] == new_mid[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut hunks = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;
    let mut pending: Option<PendingHunk> = None;
    while i < old_mid.len() || j < new_mid.len() {
        if i < old_mid.len() && j < new_mid.len() && old_mid[i] == new_mid[j] {
            if let Some(value) = pending.take() {
                hunks.push(value.finish(prefix));
            }
            i += 1;
            j += 1;
            continue;
        }

        let entry = pending.get_or_insert_with(|| PendingHunk::new(prefix + i, prefix + j));
        if j >= new_mid.len() || (i < old_mid.len() && dp[i + 1][j] >= dp[i][j + 1]) {
            entry.old_last = Some(prefix + i);
            entry.removed += 1;
            i += 1;
        } else {
            entry.new_last = Some(prefix + j);
            entry.added += 1;
            j += 1;
        }
    }
    if let Some(value) = pending {
        hunks.push(value.finish(prefix));
    }
    hunks
}

fn whole_hunk(prefix: usize, removed: usize, added: usize) -> LineHunk {
    let old_start = prefix + 1;
    let new_start = prefix + 1;
    LineHunk {
        old_start: old_start as i32,
        old_end: (prefix + removed).max(prefix + usize::from(removed == 0)) as i32,
        new_start: new_start as i32,
        new_end: (prefix + added).max(prefix + usize::from(added == 0)) as i32,
        removed: removed as i32,
        added: added as i32,
    }
}

#[derive(Debug)]
struct PendingHunk {
    anchor_old: usize,
    anchor_new: usize,
    old_last: Option<usize>,
    new_last: Option<usize>,
    removed: i32,
    added: i32,
}

impl PendingHunk {
    fn new(anchor_old: usize, anchor_new: usize) -> Self {
        Self {
            anchor_old,
            anchor_new,
            old_last: None,
            new_last: None,
            removed: 0,
            added: 0,
        }
    }

    fn finish(self, _prefix: usize) -> LineHunk {
        let old_start = self.old_last.unwrap_or(self.anchor_old);
        let new_start = self.new_last.unwrap_or(self.anchor_new);
        LineHunk {
            old_start: (old_start + 1) as i32,
            old_end: (self.old_last.unwrap_or(self.anchor_old) + 1) as i32,
            new_start: (new_start + 1) as i32,
            new_end: (self.new_last.unwrap_or(self.anchor_new) + 1) as i32,
            removed: self.removed,
            added: self.added,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_no_hunk_for_equal_input() {
        assert!(line_hunks("a\nb\n", "a\nb\n").is_empty());
    }

    #[test]
    fn reports_replacement_lines() {
        let hunks = line_hunks("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(
            hunks,
            vec![LineHunk {
                old_start: 2,
                old_end: 2,
                new_start: 2,
                new_end: 2,
                removed: 1,
                added: 1,
            }]
        );
    }

    #[test]
    fn reports_insertion_point_without_changing_old_range() {
        let hunks = line_hunks("a\nc\n", "a\nb\nc\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].new_start, hunks[0].new_end), (2, 2));
        assert_eq!(hunks[0].added, 1);
        assert_eq!(hunks[0].removed, 0);
    }

    #[test]
    fn reports_deletion_point() {
        let hunks = line_hunks("a\nb\nc\n", "a\nc\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].old_start, hunks[0].old_end), (2, 2));
        assert_eq!(hunks[0].removed, 1);
        assert_eq!(hunks[0].added, 0);
    }

    #[test]
    fn separates_hunks_around_equal_lines() {
        let hunks = line_hunks("a\nb\nc\nd\n", "a\nB\nc\nD\n");
        assert_eq!(hunks.len(), 2);
        assert_eq!((hunks[0].new_start, hunks[0].new_end), (2, 2));
        assert_eq!((hunks[1].new_start, hunks[1].new_end), (4, 4));
    }

    #[test]
    fn parses_git_unified_diff_ranges() {
        let patch = "\
diff --git a/src/UserService.java b/src/UserService.java
index 1111111..2222222 100644
--- a/src/UserService.java
+++ b/src/UserService.java
@@ -10,2 +10,3 @@ class UserService {
-old
+new
+added
@@ -30,0 +32,1 @@
+tail
";
        let hunks = parse_unified_diff(patch).unwrap();
        assert_eq!(
            hunks,
            vec![
                UnifiedHunk {
                    path: "src/UserService.java".into(),
                    old_start: 10,
                    old_end: 11,
                    new_start: 10,
                    new_end: 12,
                },
                UnifiedHunk {
                    path: "src/UserService.java".into(),
                    old_start: 30,
                    old_end: 30,
                    new_start: 32,
                    new_end: 32,
                },
            ]
        );
    }
}
