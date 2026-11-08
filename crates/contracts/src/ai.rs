//! Phase 7: Agent Runtime / MCP / Workflow の API 契約。
//!
//! `model_profile` は論理名（fast / reasoning / long_context）で、具体モデル名ではない。
//! `capabilities` は能力名で、MCP の Tool 名ではない（04-コアエンジン §4.7）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentBudgets {
    pub max_tool_calls: i32,
    pub max_tokens: i32,
    pub max_wall_time_sec: i32,
    pub max_cost_usd: f64,
}

impl Default for AgentBudgets {
    fn default() -> Self {
        Self {
            max_tool_calls: 40,
            max_tokens: 120_000,
            max_wall_time_sec: 300,
            max_cost_usd: 1.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RegisterAgentRequest {
    pub code: String,
    /// fast / reasoning / long_context
    #[serde(default = "default_profile")]
    pub model_profile: String,
    pub prompt_ref: String,
    pub prompt_body: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub budgets: AgentBudgets,
    pub output_schema: Option<String>,
    #[serde(default)]
    pub approval_required: bool,
}

fn default_profile() -> String {
    "reasoning".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentView {
    pub id: Uuid,
    pub code: String,
    pub enabled: bool,
    pub current_version: i32,
    pub model_profile: String,
    pub prompt_ref: String,
    pub capabilities: Vec<String>,
    pub budgets: AgentBudgets,
    pub output_schema: Option<String>,
    pub approval_required: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentListResponse {
    pub items: Vec<AgentView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateAgentRunRequest {
    pub question: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentRunAcceptedResponse {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentStepView {
    pub ordinal: i32,
    pub kind: String,
    pub name: String,
    pub status: String,
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentRunView {
    pub id: Uuid,
    pub agent_code: String,
    pub status: String,
    pub input: Value,
    pub output: Option<Value>,
    pub cost: Value,
    pub error: Option<Value>,
    pub steps: Vec<AgentStepView>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentRunListResponse {
    pub items: Vec<AgentRunView>,
}

// ── MCP ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RegisterMcpServerRequest {
    pub name: String,
    /// stdio / http
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default)]
    pub config: Value,
}

fn default_transport() -> String {
    "http".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct McpServerView {
    pub id: Uuid,
    pub name: String,
    pub transport: String,
    pub enabled: bool,
    pub schema_hash: Option<String>,
    pub health_status: String,
    pub health_detail: Option<String>,
    pub tool_count: i64,
    #[serde(default)]
    pub tools: Vec<McpToolView>,
    pub last_discovery_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct McpServerListResponse {
    pub items: Vec<McpServerView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct McpToolView {
    pub name: String,
    pub capability: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Value,
    #[serde(default = "default_mcp_timeout")]
    pub timeout_ms: i64,
    pub side_effects: String,
    pub approval_required: bool,
    #[serde(default)]
    pub server: Option<String>,
}

fn default_mcp_timeout() -> i64 {
    30_000
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct McpToolsResponse {
    pub tools: Vec<McpToolView>,
    pub schema_hash: String,
}

// ── Workflow ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowNode {
    pub id: String,
    /// start / capability / approval / end
    pub kind: String,
    pub name: String,
    pub capability: Option<String>,
    pub action: Option<String>,
    /// エディタの配置。実行には使わない。
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowDefinition {
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateWorkflowRequest {
    pub code: String,
    pub definition: WorkflowDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowView {
    pub id: Uuid,
    pub code: String,
    pub version: i32,
    pub status: String,
    pub node_count: usize,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowListResponse {
    pub items: Vec<WorkflowView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateWorkflowRunRequest {
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowRunAcceptedResponse {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NodeRunView {
    pub node_id: String,
    pub attempt: i16,
    pub status: String,
    pub output: Option<Value>,
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowRunView {
    pub id: Uuid,
    pub code: String,
    pub status: String,
    pub current_node: Option<String>,
    pub input: Value,
    pub output: Option<Value>,
    pub nodes: Vec<NodeRunView>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowRunListResponse {
    pub items: Vec<WorkflowRunView>,
}

// ── 承認 ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApprovalView {
    pub id: Uuid,
    pub action: String,
    pub status: String,
    pub payload: Value,
    pub requested_by: String,
    pub decided_by: Option<String>,
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    /// Workflow の承認ノードが起票した場合の run。MCP の承認は `None`。
    pub workflow_run_id: Option<Uuid>,
    /// Agent の実行が起票した場合の run。いまは未使用（将来の承認フロー用）。
    pub run_id: Option<Uuid>,
    /// MCP の許可証として消費された時刻。`status=APPROVED` でも
    /// `consumed_at` があれば**もう使えない**。
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApprovalListResponse {
    pub items: Vec<ApprovalView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DecideApprovalRequest {
    /// APPROVE / REJECT
    pub decision: String,
    pub comment: Option<String>,
}

// ── 一括取得 ──────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AiOverviewResponse {
    pub agents: Vec<AgentView>,
    pub mcp_servers: Vec<McpServerView>,
    pub workflows: Vec<WorkflowView>,
    pub workflow_runs: Vec<WorkflowRunView>,
    pub approvals: Vec<ApprovalView>,
    pub agent_runs: Vec<AgentRunView>,
    /// configured / unavailable
    pub ai_status: String,
    pub model_profile_map: Value,
    #[serde(default)]
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowDetailResponse {
    pub id: Uuid,
    pub code: String,
    pub version: i32,
    pub status: String,
    pub definition: WorkflowDefinition,
    pub created_at: DateTime<Utc>,
}
