//! 能力ゲートウェイ。MCP Server、Agent Tool、内部 UseCase の**唯一の実行入口**。
//!
//! 正本：`04-コアエンジン.md` §4.7 の 10 手順。Phase 1 では 1〜4 だけを実装し、
//! 5 以降（Schema 検証・予算・実行・provenance・監査）は Phase 7 で足す。
//!
//! ```text
//! 1. capability 存在？        → CAPABILITY_NOT_FOUND
//! 2. agent ホワイトリスト？   → AGENT_CAPABILITY_DENIED
//! 3. ユーザー権限？           → FORBIDDEN
//! 4. project_id を ctx から注入（Agent の引数は破棄）  ← 越権防止の中核
//! ```
//!
//! 4 が重要なのは、**Agent が本当の `project_id` を知らない**ようにするため。
//! Agent が見るのは `ctx.project` という不透明参照だけ。

use contracts::ErrorCode;
use domain::{Permission, ProjectId, Role, UserId};
use serde::{Deserialize, Serialize};

/// 能力名。Tool 名ではなく**できること**で名付ける（Provider 差し替えで変わらない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    KnowledgeSearch,
    KnowledgeGetEvidence,
    GraphNeighbor,
    GraphPath,
    CodeGetSymbol,
    ImpactAnalyze,
    ImpactGetReport,
    TestPlanGenerate,
    TestCaseGenerate,
    TestRunExecute,
    CoverageGet,
    ProjectList,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KnowledgeSearch => "knowledge.search",
            Self::KnowledgeGetEvidence => "knowledge.get_evidence",
            Self::GraphNeighbor => "graph.neighbor",
            Self::GraphPath => "graph.path",
            Self::CodeGetSymbol => "code.get_symbol",
            Self::ImpactAnalyze => "impact.analyze",
            Self::ImpactGetReport => "impact.get_report",
            Self::TestPlanGenerate => "test.plan_generate",
            Self::TestCaseGenerate => "test.case_generate",
            Self::TestRunExecute => "test.run_execute",
            Self::CoverageGet => "coverage.get",
            Self::ProjectList => "project.list",
        }
    }

    /// MCP の文字列境界。未知の名前は `None`（勝手に作らない）。
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "knowledge.search" => Self::KnowledgeSearch,
            "knowledge.get_evidence" => Self::KnowledgeGetEvidence,
            "graph.neighbor" => Self::GraphNeighbor,
            "graph.path" => Self::GraphPath,
            "code.get_symbol" => Self::CodeGetSymbol,
            "impact.analyze" => Self::ImpactAnalyze,
            "impact.get_report" => Self::ImpactGetReport,
            "test.plan_generate" => Self::TestPlanGenerate,
            "test.case_generate" => Self::TestCaseGenerate,
            "test.run_execute" => Self::TestRunExecute,
            "coverage.get" => Self::CoverageGet,
            "project.list" => Self::ProjectList,
            _ => return None,
        })
    }

    /// この能力に対応する行為。権限の可否は `Permission` の表で決める。
    /// ロールの大小比較では決めない（`TESTER` と `DEVELOPER` は優劣が無い）。
    pub fn permission(self) -> Permission {
        match self {
            Self::KnowledgeSearch
            | Self::KnowledgeGetEvidence
            | Self::GraphNeighbor
            | Self::GraphPath
            | Self::ProjectList => Permission::Search,
            Self::CodeGetSymbol => Permission::CodeAnalysis,
            Self::ImpactAnalyze | Self::ImpactGetReport => Permission::ImpactRun,
            Self::TestPlanGenerate | Self::TestCaseGenerate | Self::CoverageGet => {
                Permission::TestAssetEdit
            }
            Self::TestRunExecute => Permission::TestRun,
        }
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 呼び出し元。`project` は**サーバー側で埋める**。リクエスト本文から取らない。
#[derive(Debug, Clone)]
pub struct Principal {
    pub user_id: UserId,
    pub project: ProjectId,
    pub role: Role,
    /// Agent 経由の呼び出しなら、その Agent が許可されている能力名。
    /// `None` は人間による直接呼び出し。
    pub agent_capabilities: Option<Vec<String>>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CapabilityError {
    #[error("capability not found: {0}")]
    NotFound(&'static str),

    #[error("agent is not allowed to call: {0}")]
    AgentDenied(&'static str),

    #[error("role {role:?} cannot call: {capability}")]
    Forbidden {
        capability: &'static str,
        role: Role,
    },
}

impl CapabilityError {
    /// HTTP 境界のコードへ写す。`detail/04` §0.1 の表に存在するものだけを返す。
    pub fn to_code(&self) -> ErrorCode {
        match self {
            Self::NotFound(_) => ErrorCode::CapabilityUnsupported,
            Self::AgentDenied(_) => ErrorCode::AgentCapabilityDenied,
            Self::Forbidden { .. } => ErrorCode::ForbiddenProjectScope,
        }
    }
}

/// `AgentRun` の予算消費（手順 6）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BudgetUsage {
    pub tool_calls: i32,
    pub tokens: i32,
    pub wall_time_sec: i32,
    pub cost_usd: f64,
}

/// 超過した項目名を返す。超過なしなら `None`。
/// 判定は単調で、AI の出力に依存しない。
pub fn budget_exceeded(
    budget: &contracts::AgentBudgets,
    used: &BudgetUsage,
) -> Option<&'static str> {
    if used.tool_calls > budget.max_tool_calls {
        return Some("max_tool_calls");
    }
    if used.tokens > budget.max_tokens {
        return Some("max_tokens");
    }
    if used.wall_time_sec > budget.max_wall_time_sec {
        return Some("max_wall_time_sec");
    }
    if used.cost_usd > budget.max_cost_usd {
        return Some("max_cost_usd");
    }
    None
}

/// ゲートウェイ。Phase 1 では許可判定のみ。
#[derive(Debug, Default, Clone, Copy)]
pub struct CapabilityGateway;

impl CapabilityGateway {
    pub fn new() -> Self {
        Self
    }

    /// 手順 1〜3。手順 4（`project_id` の注入）は引数で受ける形にして、
    /// 呼び出し側が本文の値を使えないようにしている。
    pub fn authorize(
        &self,
        principal: &Principal,
        capability: Capability,
    ) -> Result<(), CapabilityError> {
        // 1. capability は列挙型なので「存在しない」は起きない。
        //    文字列で受ける境界（MCP）では Phase 7 で検査する。

        // 2. Agent のホワイトリスト
        if let Some(allowed) = &principal.agent_capabilities {
            if !allowed.iter().any(|name| name == capability.as_str()) {
                return Err(CapabilityError::AgentDenied(capability.as_str()));
            }
        }

        // 3. ユーザー権限（`06-API.md` §4.1 の表で判定）
        if !capability.permission().is_allowed(principal.role) {
            return Err(CapabilityError::Forbidden {
                capability: capability.as_str(),
                role: principal.role,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(role: Role, agent: Option<Vec<String>>) -> Principal {
        Principal {
            user_id: UserId::new(),
            project: ProjectId::new(),
            role,
            agent_capabilities: agent,
        }
    }

    #[test]
    fn viewer_can_search() {
        let gw = CapabilityGateway::new();
        assert!(gw
            .authorize(&principal(Role::Viewer, None), Capability::KnowledgeSearch)
            .is_ok());
    }

    #[test]
    fn agent_outside_whitelist_is_denied() {
        let gw = CapabilityGateway::new();
        let p = principal(Role::Owner, Some(vec!["knowledge.search".to_owned()]));
        let err = gw.authorize(&p, Capability::CodeGetSymbol).unwrap_err();
        assert_eq!(err, CapabilityError::AgentDenied("code.get_symbol"));
        assert_eq!(err.to_code(), ErrorCode::AgentCapabilityDenied);
    }

    #[test]
    fn budget_exhaustion_is_deterministic() {
        let budget = contracts::AgentBudgets::default();
        assert_eq!(budget_exceeded(&budget, &BudgetUsage::default()), None);
        let over = BudgetUsage {
            tool_calls: budget.max_tool_calls + 1,
            ..BudgetUsage::default()
        };
        assert_eq!(budget_exceeded(&budget, &over), Some("max_tool_calls"));
        let tokens = BudgetUsage {
            tokens: budget.max_tokens + 1,
            ..BudgetUsage::default()
        };
        assert_eq!(budget_exceeded(&budget, &tokens), Some("max_tokens"));
    }

    #[test]
    fn unknown_capability_name_is_not_created() {
        assert_eq!(Capability::parse("sql.query"), None);
        assert_eq!(
            Capability::parse("impact.analyze"),
            Some(Capability::ImpactAnalyze)
        );
    }

    #[test]
    fn agent_whitelist_never_expands_role() {
        // Agent のホワイトリストは「上限を削る」方向にだけ働く。
        // 許可されていても、ロールが足りなければ通らない（積集合）。
        let gw = CapabilityGateway::new();
        let p = principal(Role::Viewer, Some(vec!["graph.path".to_owned()]));
        assert!(gw.authorize(&p, Capability::GraphPath).is_ok());

        // ホワイトリストに無い能力は、ロールが足りていても落ちる。
        let strong = principal(Role::Owner, Some(vec!["graph.path".to_owned()]));
        let err = gw
            .authorize(&strong, Capability::KnowledgeSearch)
            .unwrap_err();
        assert_eq!(err, CapabilityError::AgentDenied("knowledge.search"));
    }
}
