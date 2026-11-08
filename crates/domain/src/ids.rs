//! 識別子。すべて UUID。表示用の `code` とは別物（`detail/04` §0.2）。
//!
//! ここに sqlx の実装を置かない。`domain` は IO を知らない（`02-アーキテクチャ` §3）。
//! DB との変換は `store` 側で `Uuid` を経由して行う。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, utoipa::ToSchema)]
        #[serde(transparent)]
        #[schema(value_type = uuid::Uuid)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// DB へ渡すときだけ `Uuid` に開く。逆方向は `From<Uuid>`。
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }

        impl From<Uuid> for $name {
            fn from(v: Uuid) -> Self {
                Self(v)
            }
        }
    };
}

id_type!(
    /// プロジェクト。テナント境界であり、RLS のスコープ。
    ProjectId
);
id_type!(
    /// ワークスペース。プロジェクトの上位の束ね。
    WorkspaceId
);
id_type!(
    /// アプリユーザー。
    UserId
);
id_type!(
    /// 非同期ジョブ。
    JobId
);
id_type!(
    /// ログインセッション。
    SessionId
);
id_type!(
    /// 設計書 1 冊（ファイル 1 つ）。
    DocumentId
);
id_type!(
    /// 文書の版。
    RevisionId
);
id_type!(
    /// 検索単位の chunk。
    ChunkId
);
id_type!(
    /// 証拠。
    EvidenceId
);
id_type!(
    /// 解析実行。
    ParseRunId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrips_through_string() {
        let id = ProjectId::new();
        let parsed: ProjectId = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn distinct_ids_differ() {
        assert_ne!(ProjectId::new(), ProjectId::new());
    }
}
