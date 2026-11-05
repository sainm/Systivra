//! プロジェクトとロール。
//!
//! 列は `detail/02-database-ddl.md` §1 の `core.project` / `core.project_member` に対応する。
//! ここでは DB の都合（`deleted_at` の扱い等）を持ち込まず、不変条件だけを置く。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ProjectId, WorkspaceId};

/// プロジェクトのソース種別。`core.project.source_kind` の CHECK と一致させる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceKind {
    Git,
    Svn,
    Dir,
    Snapshot,
}

impl SourceKind {
    /// DB の text から戻す。未知の値は `None`。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "GIT" => Some(Self::Git),
            "SVN" => Some(Self::Svn),
            "DIR" => Some(Self::Dir),
            "SNAPSHOT" => Some(Self::Snapshot),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Git => "GIT",
            Self::Svn => "SVN",
            Self::Dir => "DIR",
            Self::Snapshot => "SNAPSHOT",
        }
    }
}

/// ロール。`core.project_member.role` の CHECK と一致させる。
///
/// **`Ord` は「強い順」ではない。** 表示順と並べ替えのために付けているだけで、
/// 権限判定には使わない。`TESTER` と `DEVELOPER` に優劣は無い（互いに
/// 書ける領域が違う）。可否は必ず [`Permission`](crate::Permission) の表で決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Role {
    Viewer,
    Tester,
    Developer,
    Architect,
    Owner,
}

impl Role {
    /// DB の text から戻す。未知の値は `None`（勝手に VIEWER に落とさない）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "VIEWER" => Some(Self::Viewer),
            "TESTER" => Some(Self::Tester),
            "DEVELOPER" => Some(Self::Developer),
            "ARCHITECT" => Some(Self::Architect),
            "OWNER" => Some(Self::Owner),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "VIEWER",
            Self::Tester => "TESTER",
            Self::Developer => "DEVELOPER",
            Self::Architect => "ARCHITECT",
            Self::Owner => "OWNER",
        }
    }
}

/// プロジェクト。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub workspace_id: WorkspaceId,
    /// 表示用の短いキー。`id` の代わりに API へ出さない（`detail/04` §0.2）。
    pub code: String,
    pub name: String,
    /// デモ用に作られたプロジェクトか。画面に「SAMPLE」を出す判断に使う（D-06）。
    pub is_sample: bool,
    pub default_locale: String,
    pub source_kind: SourceKind,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Project {
    /// 論理削除済みか。物理削除は別ジョブ（`detail/02` §8）。
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_order_is_stable() {
        // 並べ替え用の順序。権限の強弱ではない。
        assert!(Role::Owner > Role::Architect);
        assert!(Role::Architect > Role::Developer);
        assert!(Role::Developer > Role::Tester);
        assert!(Role::Tester > Role::Viewer);
    }
}
