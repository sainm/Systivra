//! PG アクセス全般。マイグレーション、Outbox、ジョブキュー。
//!
//! 境界（`02-アーキテクチャ` §3）：`store` は **モジュール横断の業務判断を持たない**。
//! ここに置けるのは SQL と、その結果を領域型へ移す変換だけ。
//!
//! テナント分離は RLS で強制する。アプリはトランザクション開始時に
//! `SET LOCAL app.project_id` を設定する（`detail/05` §5.1）。設定を忘れると
//! **0 行になる**（エラーにならない）。そのため `begin_scoped` 以外で
//! 業務クエリを書かない。

pub mod ai;
pub mod audit;
pub mod auth;
pub mod code;
pub mod document;
pub mod graph;
pub mod impact;
pub mod inquiry;
pub mod job;
pub mod knowledge;
pub mod pool;
pub mod scim;
pub mod test;

pub use ai::{
    AgentRow, AgentRunRow, AgentStepRow, ApprovalRow, McpServerRow, NewAgent, NewAgentRun,
    NewWorkflowRun, NodeRunRow, WorkflowDefRow, WorkflowRunRow,
};
pub use audit::{AuditDraft, AuditRow};
pub use auth::{MembershipRow, NewSession, SessionIdentity, SessionRow, TokenRow, UserRow};
pub use code::{
    CodeAnalysisRunRow, CodeEvidenceRow, CodeFactRow, CodeRepositoryRow, CodeRevisionRow,
    CodeSourceFileRow, CodeSymbolRow, NewCodeAnalysisRun, NewCodeEvidence, NewCodeFact,
    NewCodeRepository, NewCodeRevision, NewCodeSymbol, NewSourceFile,
};
pub use document::{
    DocumentFilter, DocumentListRow, DocumentRow, NewDocument, NewRevision, RevisionRow,
};
pub use graph::{
    GraphEdgeRow, GraphEntityRow, GraphNeighborRow, GraphPathRow, GraphSubgraph,
    NewProjectionDrift, OntologyRow, OutboxEvent,
};
pub use impact::{
    ChangedCodeSymbolRow, ImpactAssessmentRow, ImpactCompletion, ImpactDocumentMentionRow,
    ImpactEvidenceRow, NewImpactAssessment,
};
pub use inquiry::{InquiryRow, NewInquiry};
pub use job::{EnqueuedJob, JobCompletion, JobEvent, JobRow, NewJob};
pub use knowledge::{
    ChunkDraft, ChunkListRow, ChunkRow, EvidenceDraft, EvidenceRow, IndexBuildRow, NodeDraft,
    RetrievalLogDraft, SearchFilter, SearchHit, TreeNodeRow,
};
pub use pool::{commit, Store, StoreError, Tx};
pub use scim::{MemberRow, ScimGroupRow, ScimUserRow};
pub use test::{
    CoverageSnapshotRow, DefectRow, NewTestCase, NewTestDataRow, NewTestDataSet, NewTestExecution,
    NewTestPlan, NewTestRun, NewTestScenario, NewTestStep, NewValidationRule, TestCaseRow,
    TestDataRow, TestExecutionRow, TestGraphEntity, TestPlanRow, TestRunRow, ValidationRuleRow,
};
