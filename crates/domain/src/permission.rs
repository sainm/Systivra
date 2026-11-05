//! 権限マトリクス。正本は `06-API.md` §4.1。
//!
//! **ロールは全順序ではない。** `TESTER` はテスト資産を書けるが Ontology は
//! 読めない。`DEVELOPER` はその逆。したがって「ロールの大小比較」で権限を
//! 決めてはならない。この表が唯一の根拠。
//!
//! `—`（権限なし）と `読`（読み取りのみ）は違う。`—` は存在自体を見せない。

use serde::{Deserialize, Serialize};

use crate::project::Role;

/// 行為。API のエンドポイント群と 1:1 で対応させる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Permission {
    ProjectSettings,
    DocumentIngest,
    Search,
    CodeAnalysis,
    ImpactRun,
    ChangeInquiry,
    TestAssetEdit,
    TestRun,
    TestCaseApprove,
    OntologyPublish,
    AgentRun,
    ProviderManage,
    AuditQuery,
}

/// アクセス水準。`None` は「権限なし」。`Read` と混同しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Access {
    None,
    Read,
    Write,
    /// 承認のみ（自分では公開できず、申請を裁可する）。
    Approve,
}

impl Access {
    pub fn can_read(self) -> bool {
        // `Approve` は読める。承認者は内容を見ないと裁可できない。
        self >= Self::Read
    }

    /// 「自分で書ける」か。`Approve` は**含めない**。
    ///
    /// `Approve` を順序で `Write` の上に置いているのは表示順の都合。
    /// 大小比較で書けると判断すると、承認者が勝手に公開できてしまう。
    pub fn can_write(self) -> bool {
        matches!(self, Self::Write)
    }

    /// 承認できるか（申請を裁可する側）。
    pub fn can_approve(self) -> bool {
        matches!(self, Self::Approve | Self::Write)
    }
}

impl Permission {
    /// 全列挙。`/projects/{pid}/me` が「何ができるか」を返すのに使う。
    pub const ALL: [Permission; 13] = [
        Self::ProjectSettings,
        Self::DocumentIngest,
        Self::Search,
        Self::CodeAnalysis,
        Self::ImpactRun,
        Self::ChangeInquiry,
        Self::TestAssetEdit,
        Self::TestRun,
        Self::TestCaseApprove,
        Self::OntologyPublish,
        Self::AgentRun,
        Self::ProviderManage,
        Self::AuditQuery,
    ];

    /// API / フロントで使う識別子。`Permission` の serde 表記と一致させる。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectSettings => "PROJECT_SETTINGS",
            Self::DocumentIngest => "DOCUMENT_INGEST",
            Self::Search => "SEARCH",
            Self::CodeAnalysis => "CODE_ANALYSIS",
            Self::ImpactRun => "IMPACT_RUN",
            Self::ChangeInquiry => "CHANGE_INQUIRY",
            Self::TestAssetEdit => "TEST_ASSET_EDIT",
            Self::TestRun => "TEST_RUN",
            Self::TestCaseApprove => "TEST_CASE_APPROVE",
            Self::OntologyPublish => "ONTOLOGY_PUBLISH",
            Self::AgentRun => "AGENT_RUN",
            Self::ProviderManage => "PROVIDER_MANAGE",
            Self::AuditQuery => "AUDIT_QUERY",
        }
    }

    /// この行為が書ける権限を持つか。
    pub fn writable_by(self, role: Role) -> bool {
        self.access_for(role).can_write()
    }

    /// `06-API.md` §4.1 の表をそのまま写したもの。
    ///
    /// 表を変えるときは、この関数と `06-API.md` を同時に直す。
    /// 片方だけ直すと、画面と API で可否が食い違う。
    pub fn access_for(self, role: Role) -> Access {
        use Access::{None, Read, Write};
        use Permission::*;
        use Role::*;

        match self {
            ProjectSettings => match role {
                Owner => Write,
                _ => Read,
            },
            DocumentIngest => match role {
                Owner | Architect | Developer => Write,
                Tester | Viewer => Read,
            },
            Search => Read,
            CodeAnalysis => match role {
                Owner | Architect | Developer => Write,
                Tester | Viewer => Read,
            },
            ImpactRun => match role {
                Owner | Architect | Developer => Write,
                Tester | Viewer => Read,
            },
            // Q-01 は TESTER も実行できる（影響の確認は QA の日常作業）。
            ChangeInquiry => match role {
                Owner | Architect | Developer | Tester => Write,
                Viewer => Read,
            },
            TestAssetEdit => match role {
                Owner | Architect | Tester => Write,
                Developer | Viewer => Read,
            },
            TestRun => match role {
                Owner | Architect | Tester => Write,
                Developer | Viewer => Read,
            },
            TestCaseApprove => match role {
                Owner | Architect | Tester => Write,
                Developer | Viewer => None,
            },
            OntologyPublish => match role {
                Owner => Write,
                Architect => Access::Approve,
                Developer | Tester | Viewer => None,
            },
            AgentRun => match role {
                Owner | Architect | Developer | Tester => Write,
                Viewer => None,
            },
            ProviderManage => match role {
                Owner => Write,
                Architect | Developer | Tester | Viewer => None,
            },
            AuditQuery => match role {
                Owner => Write,
                Architect => Read,
                Developer | Tester | Viewer => None,
            },
        }
    }

    pub fn is_allowed(self, role: Role) -> bool {
        self.access_for(role) != Access::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Role::*;

    #[test]
    fn tester_is_not_a_weaker_developer() {
        // テスト資産は TESTER が書き、DEVELOPER は読むだけ。
        assert_eq!(Permission::TestAssetEdit.access_for(Tester), Access::Write);
        assert_eq!(
            Permission::TestAssetEdit.access_for(Developer),
            Access::Read
        );
        // 逆向き：コード解析は DEVELOPER が書き、TESTER は読むだけ。
        assert_eq!(
            Permission::CodeAnalysis.access_for(Developer),
            Access::Write
        );
        assert_eq!(Permission::CodeAnalysis.access_for(Tester), Access::Read);
    }

    #[test]
    fn developer_cannot_see_test_approval_at_all() {
        // `—` は読み取り不可。画面に出さない判断のもとになる。
        assert_eq!(
            Permission::TestCaseApprove.access_for(Developer),
            Access::None
        );
        assert!(!Permission::TestCaseApprove.is_allowed(Developer));
    }

    #[test]
    fn only_owner_manages_providers() {
        assert!(Permission::ProviderManage.is_allowed(Owner));
        for r in [Architect, Developer, Tester, Viewer] {
            assert!(!Permission::ProviderManage.is_allowed(r), "{r:?}");
        }
    }

    #[test]
    fn architect_approves_ontology_but_does_not_publish() {
        assert_eq!(
            Permission::OntologyPublish.access_for(Architect),
            Access::Approve
        );
        assert!(!Permission::OntologyPublish
            .access_for(Architect)
            .can_write());
        assert!(Permission::OntologyPublish.access_for(Owner).can_write());
    }

    #[test]
    fn viewer_cannot_run_agents() {
        assert_eq!(Permission::AgentRun.access_for(Viewer), Access::None);
    }

    #[test]
    fn everyone_can_search() {
        for r in [Owner, Architect, Developer, Tester, Viewer] {
            assert!(Permission::Search.is_allowed(r), "{r:?}");
        }
    }
}
