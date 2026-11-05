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
/// 権限の対応表は `06-API.md` §4。ここでは**順序**だけを持つ
/// （`OWNER` が最強）。順序を 1 箇所に閉じ込めないと、画面ごとに
/// 「TESTER は DEVELOPER より上か」がぶれる。
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
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "VIEWER",
            Self::Tester => "TESTER",
            Self::Developer => "DEVELOPER",
            Self::Architect => "ARCHITECT",
            Self::Owner => "OWNER",
        }
    }

    /// メンバー管理（招待・ロール変更・削除）は OWNER だけ。
    pub fn can_manage_members(self) -> bool {
        self == Self::Owner
    }

    /// 設計・Ontology の確定は ARCHITECT 以上。
    pub fn can_approve_design(self) -> bool {
        self >= Self::Architect
    }

    /// コード解析の実行・設定変更は DEVELOPER 以上。
    pub fn can_write_code(self) -> bool {
        self >= Self::Developer
    }

    /// テストの承認は TESTER 以上。
    pub fn can_approve_test(self) -> bool {
        self >= Self::Tester
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
    fn role_order_is_explicit() {
        assert!(Role::Owner > Role::Architect);
        assert!(Role::Architect > Role::Developer);
        assert!(Role::Developer > Role::Tester);
        assert!(Role::Tester > Role::Viewer);
    }

    #[test]
    fn only_owner_manages_members() {
        assert!(Role::Owner.can_manage_members());
        assert!(!Role::Architect.can_manage_members());
    }

    #[test]
    fn viewer_can_do_almost_nothing() {
        let v = Role::Viewer;
        assert!(!v.can_manage_members());
        assert!(!v.can_approve_design());
        assert!(!v.can_write_code());
        assert!(!v.can_approve_test());
    }
}
