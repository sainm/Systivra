//! Phase 7: Agent Runtime / MCP / Workflow のユースケース。
//!
//! 境界:
//!   - AI が未設定なら **生成物を作らない**。run は FAILED + `AI_UNAVAILABLE`。
//!   - 能力は文字列で受ける境界（MCP）で必ず `Capability::parse` を通す。
//!   - 予算超過は決定論的に止める（`runtime::budget_exceeded`）。
//!   - Workflow の承認ノードは run を `WAITING_APPROVAL` で止め、人間の判断を待つ。

use std::collections::{BTreeMap, BTreeSet};

use contracts::{
    AgentBudgets, AgentListResponse, AgentRunListResponse, AgentRunView, AgentStepView, AgentView,
    AiOverviewResponse, ApprovalListResponse, ApprovalView, CreateAgentRunRequest,
    CreateWorkflowRequest, CreateWorkflowRunRequest, DecideApprovalRequest, McpServerListResponse,
    McpServerView, McpToolView, McpToolsResponse, NodeRunView, RegisterAgentRequest,
    RegisterMcpServerRequest, WorkflowDefinition, WorkflowDetailResponse, WorkflowListResponse,
    WorkflowRunView, WorkflowView,
};
use domain::{Permission, ProjectId, Role, UserId};
use providers::{AiError, AiMessage, AiRequest, AiStatus, AiTool};
use runtime::{budget_exceeded, BudgetUsage, Capability, CapabilityGateway, Principal};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{App, AppError, Authenticated};

fn stable_uuid(project: ProjectId, parts: &[&str]) -> Uuid {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    Uuid::new_v5(&project.0, &hasher.finalize()[..16])
}

/// 承認済みの許可証を使える時間。承認した人が見た内容と、実行される内容が
/// 同じであることを担保するため、無期限にはしない。
const MCP_APPROVAL_WINDOW_SECS: i64 = 600;

fn mcp_approval_action(tool_name: &str) -> String {
    format!("mcp.call:{tool_name}")
}

/// 承認対象の同定に使う payload。**引数まで含める**ので、
/// 承認した内容と違う引数の呼び出しには許可証が効かない。
fn mcp_approval_payload(tool_name: &str, capability: &str, arguments: &Value) -> Value {
    json!({
        "tool": tool_name,
        "capability": capability,
        "arguments": arguments,
    })
}

/// 対外公開する 12 ツール。`04-コアエンジン` §4.9。
/// **業務能力で組織し、テーブルや内部関数は公開しない。**
pub fn mcp_tools() -> Vec<McpToolView> {
    let tool =
        |name: &str, capability: &str, description: &str, side: &str, approval: bool| McpToolView {
            name: name.to_owned(),
            capability: capability.to_owned(),
            description: description.to_owned(),
            input_schema: json!({"type": "object"}),
            output_schema: json!({}),
            timeout_ms: crate::mcp_client::DEFAULT_MCP_TIMEOUT_MS,
            side_effects: side.to_owned(),
            approval_required: approval,
            server: None,
        };
    vec![
        tool(
            "knowledge_search",
            "knowledge.search",
            "設計書・仕様書を検索し、locator 付き証拠を返す",
            "read",
            false,
        ),
        tool(
            "knowledge_get_evidence",
            "knowledge.get_evidence",
            "証拠原文と文脈",
            "read",
            false,
        ),
        tool("graph_query", "graph.neighbor", "実体と隣接", "read", false),
        tool(
            "graph_trace_path",
            "graph.path",
            "2 実体間の経路",
            "read",
            false,
        ),
        tool(
            "code_get_symbol",
            "code.get_symbol",
            "シンボル定義と参照",
            "read",
            false,
        ),
        tool(
            "impact_analyze",
            "impact.analyze",
            "影響分析（非同期。job_id）",
            "read",
            false,
        ),
        tool(
            "impact_get_report",
            "impact.get_report",
            "影響レポート取得",
            "read",
            false,
        ),
        // 草稿生成は承認不要（04 §4.7「人間との境界」）。正式資産化は
        // `test/cases/{cid}/approve` の人手操作で行う。ここを true にすると
        // 草稿すら作れなくなり、境界が逆転する。
        tool(
            "test_plan_generate",
            "test.plan_generate",
            "テスト計画の草稿（DRAFT。実行は承認後）",
            "write",
            false,
        ),
        tool(
            "test_case_generate",
            "test.case_generate",
            "テストケースの草稿（DRAFT。実行は承認後）",
            "write",
            false,
        ),
        tool(
            "test_run_execute",
            "test.run_execute",
            "テスト実行（承認必須）",
            "destructive",
            true,
        ),
        tool(
            "coverage_get",
            "coverage.get",
            "被覆スナップショット",
            "read",
            false,
        ),
        tool(
            "project_list",
            "project.list",
            "アクセス可能なプロジェクト",
            "read",
            false,
        ),
    ]
    .into_iter()
    .map(|mut tool| {
        tool.input_schema = match tool.name.as_str() {
            "graph_query" => json!({
                "type": "object",
                "properties": {
                    "entity_id": {"type": "string", "description": "起点の実体 ID"},
                    "query": {"type": "string", "description": "実体名の部分一致"}
                },
                "anyOf": [{"required": ["entity_id"]}, {"required": ["query"]}]
            }),
            "graph_trace_path" => json!({
                "type": "object",
                "properties": {
                    "from_id": {"type": "string"},
                    "to_id": {"type": "string"},
                    "max_depth": {"type": "integer", "minimum": 1, "maximum": 6}
                },
                "required": ["from_id", "to_id"]
            }),
            "code_get_symbol" => json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "FQN または単純名"},
                    "revision_id": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50}
                },
                "required": ["query"]
            }),
            "knowledge_search" => json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "top_k": {"type": "integer", "minimum": 1, "maximum": 50}
                },
                "required": ["query"]
            }),
            "impact_analyze" => json!({
                "type": "object",
                "properties": {
                    "entity_ids": {"type": "array", "items": {"type": "string"}, "minItems": 1}
                },
                "required": ["entity_ids"]
            }),
            "test_plan_generate" => json!({
                "type": "object",
                "properties": {"impact_id": {"type": "string"}},
                "required": ["impact_id"]
            }),
            "test_case_generate" => json!({
                "type": "object",
                "description": "既存の計画へケースを追記する（計画は新規作成しない）",
                "properties": {
                    "plan_id": {"type": "string"},
                    "impact_id": {"type": "string"}
                },
                "required": ["plan_id"]
            }),
            "knowledge_get_evidence" => json!({
                "type": "object",
                "properties": {"evidence_id": {"type": "string"}},
                "required": ["evidence_id"]
            }),
            _ => tool.input_schema,
        };
        tool
    })
    .collect()
}

/// ツール記述の安定ハッシュ。Server 差し替え検出に使う。
pub fn mcp_schema_hash() -> String {
    crate::mcp_client::tools_schema_hash(&mcp_tools())
}

#[derive(Debug, Clone)]
struct McpToolRoute {
    tool: McpToolView,
    transport: Option<String>,
    config: Option<Value>,
}

fn available_mcp_tool_routes(servers: &[store::McpServerRow]) -> BTreeMap<String, McpToolRoute> {
    let mut routes = BTreeMap::new();
    for server in servers
        .iter()
        .filter(|server| server.enabled && server.health_status == "UP")
    {
        let Ok(mut tools) = serde_json::from_value::<Vec<McpToolView>>(server.tools.clone()) else {
            continue;
        };
        for tool in &mut tools {
            tool.server = Some(server.name.clone());
        }
        for tool in tools {
            routes
                .entry(tool.name.clone())
                .or_insert_with(|| McpToolRoute {
                    tool,
                    transport: Some(server.transport.clone()),
                    config: Some(server.config.clone()),
                });
        }
    }
    for tool in mcp_tools() {
        routes.entry(tool.name.clone()).or_insert(McpToolRoute {
            tool,
            transport: None,
            config: None,
        });
    }
    routes
}

impl App {
    async fn require_agent_write(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::AgentRun.access_for(role).can_write() {
            return Err(AppError::Forbidden("agent:write がありません".into()));
        }
        Ok(())
    }

    async fn require_agent_read(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::AgentRun.access_for(role).can_read() {
            return Err(AppError::Forbidden("agent:read がありません".into()));
        }
        Ok(())
    }

    async fn require_provider_manage(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::ProviderManage.access_for(role).can_write() {
            return Err(AppError::Forbidden("platform:admin がありません".into()));
        }
        Ok(())
    }

    fn agent_view(row: &store::AgentRow) -> AgentView {
        AgentView {
            id: row.id,
            code: row.code.clone(),
            enabled: row.enabled,
            current_version: row.current_version,
            model_profile: row.model_profile.clone(),
            prompt_ref: row.prompt_ref.clone(),
            capabilities: row.capabilities.clone(),
            budgets: serde_json::from_value::<AgentBudgets>(row.budgets.clone())
                .unwrap_or_default(),
            output_schema: row.output_schema.clone(),
            approval_required: row.approval_required,
            created_at: row.created_at,
        }
    }

    // ── agent 登録 ─────────────────────────────────────────

    pub async fn register_agent(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: RegisterAgentRequest,
        trace: Option<String>,
    ) -> Result<AgentView, AppError> {
        self.require_provider_manage(auth, project).await?;
        if request.code.trim().is_empty() || request.prompt_ref.trim().is_empty() {
            return Err(AppError::Validation("code / prompt_ref は必須です".into()));
        }
        if !matches!(
            request.model_profile.as_str(),
            "fast" | "reasoning" | "long_context"
        ) {
            return Err(AppError::Validation("model_profile が不正です".into()));
        }
        for capability in &request.capabilities {
            if Capability::parse(capability).is_none() {
                return Err(AppError::Validation(format!(
                    "未知の capability: {capability}"
                )));
            }
        }
        let version = {
            let mut tx = self.store().begin_scoped(project).await?;
            let existing = self
                .store()
                .agent_by_code(&mut tx, project.0, &request.code)
                .await?;
            let version = existing
                .as_ref()
                .map(|row| row.current_version + 1)
                .unwrap_or(1);
            let agent_id = existing
                .as_ref()
                .map(|row| row.id)
                .unwrap_or_else(|| stable_uuid(project, &["agent", &request.code]));
            let version_id = stable_uuid(
                project,
                &["agent-version", &request.code, &version.to_string()],
            );
            let budgets = serde_json::to_value(request.budgets).unwrap_or(json!({}));
            self.store()
                .upsert_agent(
                    &mut tx,
                    &store::NewAgent {
                        agent_id,
                        version_id,
                        project_id: project.0,
                        code: request.code.clone(),
                        version,
                        model_profile: request.model_profile.clone(),
                        prompt_ref: request.prompt_ref.clone(),
                        prompt_body: request.prompt_body.clone(),
                        capabilities: request.capabilities.clone(),
                        budgets,
                        output_schema: request.output_schema.clone(),
                        approval_required: request.approval_required,
                    },
                )
                .await?;
            self.store()
                .append_audit(
                    &mut tx,
                    &store::AuditDraft {
                        project_id: Some(project.0),
                        actor_type: "USER".into(),
                        actor_id: auth.user_id.to_string(),
                        action: "ai.agent.register".into(),
                        target_type: Some("ai.agent".into()),
                        target_id: Some(agent_id.to_string()),
                        input_digest: None,
                        result: "OK".into(),
                        detail: json!({"version": version, "capabilities": request.capabilities}),
                        trace_id: trace,
                    },
                )
                .await?;
            store::commit(tx).await?;
            version
        };
        let _ = version;
        let mut tx = self.store().begin_scoped(project).await?;
        let row = self
            .store()
            .agent_by_code(&mut tx, project.0, &request.code)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        store::commit(tx).await?;
        Ok(Self::agent_view(&row))
    }

    pub async fn list_agents(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<AgentListResponse, AppError> {
        self.require_agent_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self.store().list_agents(&mut tx, project.0).await?;
        store::commit(tx).await?;
        Ok(AgentListResponse {
            items: rows.iter().map(Self::agent_view).collect(),
        })
    }

    // ── agent run ──────────────────────────────────────────

    pub async fn create_agent_run(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        agent_code: &str,
        request: CreateAgentRunRequest,
        trace: Option<String>,
    ) -> Result<contracts::AgentRunAcceptedResponse, AppError> {
        self.require_agent_write(auth, project).await?;
        if request.question.trim().is_empty() {
            return Err(AppError::Validation("question は必須です".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let requester_role = self
            .store()
            .role_in(&mut tx, auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let agent = self
            .store()
            .agent_by_code(&mut tx, project.0, agent_code)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let run_id = stable_uuid(
            project,
            &[
                "agent-run",
                &agent.code,
                &request.question,
                &agent.current_version.to_string(),
            ],
        );
        self.store()
            .insert_agent_run(
                &mut tx,
                &store::NewAgentRun {
                    id: run_id,
                    project_id: project.0,
                    agent_version_id: agent.version_id,
                    requested_by: auth.user_id.to_string(),
                    requested_role: requester_role.as_str().to_owned(),
                    input: json!({"question": request.question}),
                    trace_id: trace.clone(),
                },
            )
            .await?;
        let enqueued = self
            .store()
            .enqueue_job(
                &mut tx,
                &store::NewJob {
                    project_id: project.0,
                    kind: "AGENT_RUN".into(),
                    idempotency_key: format!("agent-run:{run_id}"),
                    input: json!({"run_id": run_id}),
                },
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "ai.agent_run.create".into(),
                    target_type: Some("ai.agent_run".into()),
                    target_id: Some(run_id.to_string()),
                    input_digest: None,
                    result: "QUEUED".into(),
                    detail: json!({"agent": agent.code, "job_id": enqueued.job.id}),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(contracts::AgentRunAcceptedResponse {
            run_id,
            job_id: enqueued.job.id,
            status: enqueued.job.status,
        })
    }

    pub async fn agent_run_view(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        run_id: Uuid,
    ) -> Result<AgentRunView, AppError> {
        self.require_agent_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let view = self.agent_run_view_in(&mut tx, project, run_id).await?;
        store::commit(tx).await?;
        view.ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))
    }

    async fn agent_run_view_in(
        &self,
        tx: &mut store::Tx<'_>,
        project: ProjectId,
        run_id: Uuid,
    ) -> Result<Option<AgentRunView>, AppError> {
        let Some(run) = self.store().agent_run(tx, project.0, run_id).await? else {
            return Ok(None);
        };
        let agent_code = self
            .store()
            .agent_code_for_version(tx, project.0, run.agent_version_id)
            .await?;
        let steps = self.store().list_agent_steps(tx, project.0, run_id).await?;
        Ok(Some(AgentRunView {
            id: run.id,
            agent_code,
            status: run.status,
            input: run.input,
            output: run.output,
            cost: run.cost,
            error: run.error,
            steps: steps
                .iter()
                .map(|step| AgentStepView {
                    ordinal: step.ordinal,
                    kind: step.kind.clone(),
                    name: step.name.clone(),
                    status: step.status.clone(),
                    error: step.error.clone(),
                })
                .collect(),
            started_at: run.started_at,
            finished_at: run.finished_at,
        }))
    }

    pub async fn list_agent_runs(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<AgentRunListResponse, AppError> {
        self.require_agent_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let runs = self.store().list_agent_runs(&mut tx, project.0, 20).await?;
        let mut items = Vec::new();
        for run in runs {
            if let Some(view) = self.agent_run_view_in(&mut tx, project, run.id).await? {
                items.push(view);
            }
        }
        store::commit(tx).await?;
        Ok(AgentRunListResponse { items })
    }

    /// worker から呼ぶ。AI が無い場合は FAILED + `AI_UNAVAILABLE` で終わる。
    pub async fn run_agent_from_job(
        &self,
        project: ProjectId,
        job_id: Uuid,
        input: &Value,
    ) -> Result<AgentRunView, AppError> {
        let run_id = input
            .get("run_id")
            .and_then(Value::as_str)
            .and_then(|v| Uuid::parse_str(v).ok())
            .ok_or_else(|| AppError::Validation("run_id がありません".into()))?;
        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .start_agent_run(&mut tx, project.0, run_id)
            .await?;
        let run = self
            .store()
            .agent_run(&mut tx, project.0, run_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let agents = self.store().list_agents(&mut tx, project.0).await?;
        let agent = agents
            .iter()
            .find(|agent| agent.version_id == run.agent_version_id);
        let (Some(agent), Some(question)) = (
            agent,
            run.input
                .get("question")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ) else {
            self.store()
                .insert_agent_step(
                    &mut tx,
                    project.0,
                    run_id,
                    0,
                    "LLM",
                    "config",
                    "FAILED",
                    Some(&json!({"code": "AGENT_CONFIG_MISSING"})),
                )
                .await?;
            self.store()
                .finish_agent_run(
                    &mut tx,
                    project.0,
                    run_id,
                    "FAILED",
                    None,
                    &json!({}),
                    Some(&json!({"code": "AGENT_CONFIG_MISSING"})),
                )
                .await?;
            let view = self
                .agent_run_view_in(&mut tx, project, run_id)
                .await?
                .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
            store::commit(tx).await?;
            return Ok(view);
        };

        let requester_id = run.requested_by.parse::<UserId>().ok();
        let requester_role = Role::parse(&run.requested_role);
        let Some((requester_id, requester_role)) = requester_id.zip(requester_role) else {
            self.store()
                .finish_agent_run(
                    &mut tx,
                    project.0,
                    run_id,
                    "FAILED",
                    None,
                    &json!({}),
                    Some(&json!({"code": "FORBIDDEN_PROJECT_SCOPE"})),
                )
                .await?;
            let view = self
                .agent_run_view_in(&mut tx, project, run_id)
                .await?
                .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
            store::commit(tx).await?;
            return Ok(view);
        };

        // 能力の実行主体は「その run を起動した人間」。Agent は人間の権限を
        // 超えない（Gateway のロール判定もこの身元で行う）。
        let requester_auth = Authenticated::for_job(requester_id);

        let prompt = self
            .store()
            .prompt_body(&mut tx, project.0, &agent.prompt_ref)
            .await?
            .unwrap_or_else(|| "You are a software engineering analyst.".to_owned());
        let budgets: AgentBudgets =
            serde_json::from_value(agent.budgets.clone()).unwrap_or_default();

        let output_schema = "出力は JSON object だけにしてください。\
            {\"narrative\":\"...\",\"hypotheses\":[{\"claim\":\"...\",\
            \"confidence\":0.0,\"basis\":[\"...\"]}]} の形を守り、\
            hypotheses が空でない場合は各項目に 1 件以上の basis を付けてください。";
        let mut messages = vec![
            AiMessage::system(format!("{prompt}\n\n{output_schema}")),
            AiMessage::user(question),
        ];
        let servers = self.store().list_mcp_servers(&mut tx, project.0).await?;
        let tool_routes = available_mcp_tool_routes(&servers);
        let tools: Vec<AiTool> = tool_routes
            .values()
            .filter(|route| agent.capabilities.contains(&route.tool.capability))
            .take(32)
            .map(|route| AiTool {
                name: route.tool.name.clone(),
                description: route.tool.description.clone(),
                parameters: route.tool.input_schema.clone(),
            })
            .collect();
        let mut usage = BudgetUsage::default();
        // 予算 4 項目のうち wall time と cost は、測っていなければ 0 のままで
        // **上限が効かない**。ここで実測を始める。
        let started = std::time::Instant::now();
        let pricing = providers::AiPricing::from_env();
        let mut status = "SUCCEEDED".to_owned();
        let mut output: Option<Value> = None;
        let mut error: Option<Value> = None;
        let mut step_ordinal = 0i32;

        if self.ai().status() == AiStatus::Unavailable {
            status = "FAILED".to_owned();
            error = Some(json!({
                "code": "AI_UNAVAILABLE",
                "detail": "AI_BASE_URL / AI_MODEL_* が未設定。生成物は作りません。"
            }));
        } else {
            loop {
                usage.wall_time_sec = started.elapsed().as_secs() as i32;
                if let Some(reason) = budget_exceeded(&budgets, &usage) {
                    status = "BUDGET_EXCEEDED".to_owned();
                    error = Some(json!({"code": "BUDGET_EXCEEDED", "budget": reason}));
                    break;
                }
                let response = self
                    .ai()
                    .complete(&AiRequest {
                        model_profile: agent.model_profile.clone(),
                        messages: messages.clone(),
                        tools: tools.clone(),
                    })
                    .await;
                let response = match response {
                    Ok(response) => response,
                    Err(AiError::Unavailable(reason)) => {
                        status = "FAILED".to_owned();
                        error = Some(json!({"code": "AI_UNAVAILABLE", "detail": reason}));
                        break;
                    }
                    Err(failure) => {
                        status = "FAILED".to_owned();
                        error = Some(json!({"code": "AI_UPSTREAM", "detail": failure.to_string()}));
                        break;
                    }
                };
                usage.tokens += response.prompt_tokens + response.completion_tokens;
                usage.cost_usd +=
                    pricing.cost_usd(response.prompt_tokens, response.completion_tokens);
                messages.push(AiMessage::assistant(
                    response.text.clone(),
                    response.tool_calls.clone(),
                ));
                if response.tool_calls.is_empty() {
                    match parse_agent_output(&response.text) {
                        Ok(value) => output = Some(value),
                        Err(detail) => {
                            status = "FAILED".to_owned();
                            error = Some(json!({"code": "SCHEMA_VIOLATION", "detail": detail}));
                        }
                    }
                    break;
                }
                let mut terminal_error: Option<Value> = None;
                for call in &response.tool_calls {
                    usage.tool_calls += 1;
                    let route = tool_routes.get(&call.name).cloned();
                    let capability = route
                        .as_ref()
                        .and_then(|route| Capability::parse(&route.tool.capability));
                    let (step_status, content) = match (route, capability) {
                        (None, _) => {
                            terminal_error = Some(json!({
                                "code": "CAPABILITY_UNSUPPORTED",
                                "detail": format!("unknown_tool: {}", call.name),
                            }));
                            (
                                "FAILED",
                                json!({"error": "unknown_tool", "tool": call.name}).to_string(),
                            )
                        }
                        (Some(route), _)
                            if !agent.capabilities.contains(&route.tool.capability) =>
                        {
                            terminal_error = Some(json!({
                                "code": "AGENT_CAPABILITY_DENIED",
                                "capability": route.tool.capability,
                            }));
                            (
                                "FAILED",
                                json!({
                                    "error": "capability_denied",
                                    "capability": route.tool.capability,
                                })
                                .to_string(),
                            )
                        }
                        (Some(route), None) => {
                            terminal_error = Some(json!({
                                "code": "CAPABILITY_UNSUPPORTED",
                                "capability": route.tool.capability,
                            }));
                            (
                                "FAILED",
                                json!({"error": "unsupported_capability"}).to_string(),
                            )
                        }
                        (Some(route), Some(capability)) => {
                            let principal = Principal {
                                user_id: requester_id,
                                project,
                                role: requester_role,
                                agent_capabilities: Some(agent.capabilities.clone()),
                            };
                            if let Err(error) =
                                CapabilityGateway::new().authorize(&principal, capability)
                            {
                                terminal_error = Some(json!({
                                    "code": match error {
                                        runtime::CapabilityError::AgentDenied(_) =>
                                            "AGENT_CAPABILITY_DENIED",
                                        runtime::CapabilityError::Forbidden { .. } =>
                                            "FORBIDDEN_PROJECT_SCOPE",
                                        runtime::CapabilityError::NotFound(_) =>
                                            "CAPABILITY_UNSUPPORTED",
                                    },
                                    "detail": error.to_string(),
                                }));
                                (
                                    "FAILED",
                                    json!({"error": "capability_denied", "detail": error.to_string()})
                                        .to_string(),
                                )
                            } else if route.tool.approval_required {
                                terminal_error = Some(json!({
                                    "code": "POLICY_APPROVAL_REQUIRED",
                                    "capability": route.tool.capability,
                                }));
                                (
                                    "FAILED",
                                    json!({
                                        "error": "approval_required",
                                        "capability": route.tool.capability,
                                    })
                                    .to_string(),
                                )
                            } else {
                                let mut arguments: Value = serde_json::from_str(&call.arguments)
                                    .unwrap_or_else(|_| json!({}));
                                if let Some(object) = arguments.as_object_mut() {
                                    object.insert(
                                        "project_id".to_owned(),
                                        json!(project.0.to_string()),
                                    );
                                }
                                if !arguments.is_object() {
                                    ("FAILED", json!({"error": "invalid_arguments"}).to_string())
                                } else if let (Some(transport), Some(config)) =
                                    (&route.transport, &route.config)
                                {
                                    match crate::mcp_client::call_tool(
                                        transport,
                                        config,
                                        &route.tool,
                                        &arguments,
                                    )
                                    .await
                                    {
                                        Ok(value) => {
                                            if let Some(server) = &route.tool.server {
                                                self.store()
                                                    .update_mcp_health(
                                                        &mut tx, project.0, server, "UP", None,
                                                    )
                                                    .await?;
                                                self.append_mcp_audit(
                                                    &mut tx,
                                                    &requester_id.to_string(),
                                                    project,
                                                    "ai.mcp.call",
                                                    server,
                                                    "PASSED",
                                                    json!({"tool": route.tool.name}),
                                                )
                                                .await?;
                                            }
                                            (
                                                "PASSED",
                                                truncate(
                                                    &json!({
                                                        "untrusted": true,
                                                        "mcp_server": route.tool.server,
                                                        "value": value,
                                                    })
                                                    .to_string(),
                                                    2000,
                                                ),
                                            )
                                        }
                                        Err(error) => {
                                            let health = error.health_status();
                                            let detail = truncate(&error.to_string(), 500);
                                            if let Some(server) = &route.tool.server {
                                                self.store()
                                                    .update_mcp_health(
                                                        &mut tx,
                                                        project.0,
                                                        server,
                                                        health,
                                                        Some(&detail),
                                                    )
                                                    .await?;
                                                self.append_mcp_audit(
                                                    &mut tx,
                                                    &requester_id.to_string(),
                                                    project,
                                                    "ai.mcp.call",
                                                    server,
                                                    health,
                                                    json!({
                                                        "tool": route.tool.name,
                                                        "detail": detail,
                                                    }),
                                                )
                                                .await?;
                                            }
                                            (
                                                "FAILED",
                                                json!({
                                                    "error": "mcp_call_failed",
                                                    "health": health,
                                                    "detail": detail,
                                                    "untrusted": true,
                                                })
                                                .to_string(),
                                            )
                                        }
                                    }
                                } else {
                                    match self
                                        .execute_capability(
                                            Some(&requester_auth),
                                            project,
                                            capability,
                                            &arguments,
                                        )
                                        .await
                                    {
                                        Ok(value) => ("PASSED", truncate(&value.to_string(), 2000)),
                                        Err(failure) => (
                                            "FAILED",
                                            json!({"error": failure.to_string()}).to_string(),
                                        ),
                                    }
                                }
                            }
                        }
                    };
                    step_ordinal += 1;
                    let step_error = (step_status == "FAILED")
                        .then(|| json!({"detail": truncate(&content, 500)}));
                    self.store()
                        .insert_agent_step(
                            &mut tx,
                            project.0,
                            run_id,
                            step_ordinal,
                            "CAPABILITY",
                            &call.name,
                            step_status,
                            step_error.as_ref(),
                        )
                        .await?;
                    messages.push(AiMessage::tool(call.id.clone(), content));
                    if terminal_error.is_some() {
                        break;
                    }
                }
                if let Some(terminal) = terminal_error {
                    status = "FAILED".to_owned();
                    error = Some(terminal);
                    break;
                }
            }
        }

        let cost = json!({
            "tokens": usage.tokens,
            "tool_calls": usage.tool_calls,
            "wall_time_sec": usage.wall_time_sec,
            // `priced=false` は「コスト上限を判定していない」の意味。
            // 0 ドルと書くと、予算内だと誤解される。
            "cost_usd": if pricing.is_priced() { json!(usage.cost_usd) } else { Value::Null },
            "priced": pricing.is_priced(),
        });
        let (status, output, cost, error) = (status, output, cost, error);
        let step_status = if status == "SUCCEEDED" {
            "PASSED"
        } else {
            "FAILED"
        };
        self.store()
            .insert_agent_step(
                &mut tx,
                project.0,
                run_id,
                0,
                "LLM",
                "complete",
                step_status,
                error.as_ref(),
            )
            .await?;
        self.store()
            .finish_agent_run(
                &mut tx,
                project.0,
                run_id,
                &status,
                output.as_ref(),
                &cost,
                error.as_ref(),
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "SYSTEM".into(),
                    actor_id: job_id.to_string(),
                    action: "ai.agent_run.finish".into(),
                    target_type: Some("ai.agent_run".into()),
                    target_id: Some(run_id.to_string()),
                    input_digest: None,
                    result: status.clone(),
                    detail: json!({"agent": agent.code, "error": error}),
                    trace_id: None,
                },
            )
            .await?;
        let view = self
            .agent_run_view_in(&mut tx, project, run_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        store::commit(tx).await?;
        Ok(view)
    }

    // ── MCP ────────────────────────────────────────────────

    pub async fn register_mcp_server(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: RegisterMcpServerRequest,
        trace: Option<String>,
    ) -> Result<McpServerView, AppError> {
        self.require_provider_manage(auth, project).await?;
        if request.name.trim().is_empty() {
            return Err(AppError::Validation("name は必須です".into()));
        }
        if !matches!(request.transport.as_str(), "stdio" | "http") {
            return Err(AppError::Validation("transport が不正です".into()));
        }
        crate::mcp_client::validate_config(&request.transport, &request.config)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        let id = stable_uuid(project, &["mcp", &request.name]);
        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .upsert_mcp_server(
                &mut tx,
                project.0,
                id,
                &request.name,
                &request.transport,
                &request.config,
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "ai.mcp.register".into(),
                    target_type: Some("ai.mcp_server".into()),
                    target_id: Some(id.to_string()),
                    input_digest: None,
                    result: "OK".into(),
                    detail: json!({"name": request.name, "transport": request.transport}),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let servers = self.store().list_mcp_servers(&mut tx, project.0).await?;
        store::commit(tx).await?;
        servers
            .iter()
            .find(|row| row.id == id)
            .map(Self::mcp_view)
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))
    }

    fn mcp_view(row: &store::McpServerRow) -> McpServerView {
        let mut tools: Vec<McpToolView> =
            serde_json::from_value(row.tools.clone()).unwrap_or_default();
        for tool in &mut tools {
            tool.server = Some(row.name.clone());
        }
        McpServerView {
            id: row.id,
            name: row.name.clone(),
            transport: row.transport.clone(),
            enabled: row.enabled,
            schema_hash: row.schema_hash.clone(),
            health_status: row.health_status.clone(),
            health_detail: row.health_detail.clone(),
            tool_count: tools.len() as i64,
            tools,
            last_discovery_at: row.last_discovery_at,
        }
    }

    pub async fn list_mcp_servers(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<McpServerListResponse, AppError> {
        self.require_provider_manage(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self.store().list_mcp_servers(&mut tx, project.0).await?;
        store::commit(tx).await?;
        Ok(McpServerListResponse {
            items: rows.iter().map(Self::mcp_view).collect(),
        })
    }

    /// 外部 Server へ接続し、Tool 定義と `schema_hash` を記録する。
    /// Schema が変わった場合は旧 Tool を保持し、明示的な再登録まで DEGRADED を維持する。
    pub async fn discover_mcp_server(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        name: &str,
    ) -> Result<McpServerView, AppError> {
        self.require_provider_manage(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let server = self
            .store()
            .list_mcp_servers(&mut tx, project.0)
            .await?
            .into_iter()
            .find(|row| row.name == name)
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        store::commit(tx).await?;
        match crate::mcp_client::discover(&server.transport, &server.config).await {
            Ok(tools) => {
                let hash = crate::mcp_client::tools_schema_hash(&tools);
                let changed = server
                    .schema_hash
                    .as_deref()
                    .is_some_and(|previous| previous != hash);
                let still_degraded = server.health_status == "DEGRADED"
                    && server.health_detail.as_deref() == Some("MCP_SCHEMA_CHANGED")
                    && server.schema_hash.as_deref() == Some(hash.as_str());
                let health = if changed || still_degraded {
                    "DEGRADED"
                } else {
                    "UP"
                };
                let detail = if changed || still_degraded {
                    Some("MCP_SCHEMA_CHANGED")
                } else {
                    None
                };
                let tools_value = serde_json::to_value(&tools)
                    .map_err(|error| AppError::Validation(error.to_string()))?;
                let accepted_tools = if changed || still_degraded {
                    None
                } else {
                    Some(&tools_value)
                };
                let mut tx = self.store().begin_scoped(project).await?;
                self.store()
                    .update_mcp_discovery(
                        &mut tx,
                        project.0,
                        name,
                        &hash,
                        health,
                        detail,
                        accepted_tools,
                    )
                    .await?;
                self.append_mcp_audit(
                    &mut tx,
                    &auth.user_id.to_string(),
                    project,
                    "ai.mcp.discover",
                    name,
                    health,
                    json!({
                        "schema_hash": hash,
                        "schema_changed": changed || still_degraded,
                        "tool_count": tools.len(),
                    }),
                )
                .await?;
                store::commit(tx).await?;
            }
            Err(error) => {
                let health = error.health_status();
                let detail = truncate(&error.to_string(), 500);
                let mut tx = self.store().begin_scoped(project).await?;
                self.store()
                    .update_mcp_health(&mut tx, project.0, name, health, Some(&detail))
                    .await?;
                self.append_mcp_audit(
                    &mut tx,
                    &auth.user_id.to_string(),
                    project,
                    "ai.mcp.discover",
                    name,
                    health,
                    json!({"detail": detail}),
                )
                .await?;
                store::commit(tx).await?;
            }
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self.store().list_mcp_servers(&mut tx, project.0).await?;
        store::commit(tx).await?;
        rows.iter()
            .find(|row| row.name == name)
            .map(Self::mcp_view)
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))
    }

    #[allow(clippy::too_many_arguments)]
    async fn append_mcp_audit(
        &self,
        tx: &mut store::Tx<'_>,
        actor_id: &str,
        project: ProjectId,
        action: &str,
        name: &str,
        result: &str,
        detail: Value,
    ) -> Result<(), AppError> {
        self.append_mcp_audit_traced(tx, actor_id, project, action, name, result, detail, None)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn append_mcp_audit_traced(
        &self,
        tx: &mut store::Tx<'_>,
        actor_id: &str,
        project: ProjectId,
        action: &str,
        name: &str,
        result: &str,
        detail: Value,
        trace: Option<String>,
    ) -> Result<(), AppError> {
        self.store()
            .append_audit(
                tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: actor_id.to_owned(),
                    action: action.to_owned(),
                    target_type: Some("ai.mcp_server".into()),
                    target_id: Some(name.to_owned()),
                    input_digest: None,
                    result: result.to_owned(),
                    detail,
                    trace_id: trace,
                },
            )
            .await?;
        Ok(())
    }

    pub fn tools(&self) -> McpToolsResponse {
        McpToolsResponse {
            tools: mcp_tools(),
            schema_hash: mcp_schema_hash(),
        }
    }

    /// MCP `tools/call`。文字列 capability を必ず列挙型へ変換してから権限を見る。
    /// 文字列 capability を列挙型へ変換し、Gateway の許可判定だけを行う。
    /// 承認必須ツールは「実行前に」ここで弾くために公開している。
    pub async fn authorize_tool(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        capability_name: &str,
    ) -> Result<Capability, AppError> {
        let capability = Capability::parse(capability_name).ok_or(AppError::Conflict(
            contracts::ErrorCode::CapabilityUnsupported,
        ))?;
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        runtime::CapabilityGateway::new()
            .authorize(
                &runtime::Principal {
                    user_id: auth.user_id,
                    project,
                    role,
                    agent_capabilities: None,
                },
                capability,
            )
            .map_err(|error| AppError::Forbidden(error.to_string()))?;
        Ok(capability)
    }

    pub async fn call_tool(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        capability_name: &str,
        arguments: &Value,
    ) -> Result<Value, AppError> {
        let capability = self.authorize_tool(auth, project, capability_name).await?;
        self.execute_capability(Some(auth), project, capability, arguments)
            .await
    }

    async fn execute_capability(
        &self,
        auth: Option<&Authenticated>,
        project: ProjectId,
        capability: Capability,
        arguments: &Value,
    ) -> Result<Value, AppError> {
        match capability {
            Capability::GraphNeighbor => {
                let mut tx = self.store().begin_scoped(project).await?;
                let result = if let Some(entity_id) = arguments
                    .get("entity_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                {
                    let neighbors = self
                        .store()
                        .graph_neighbors(&mut tx, project.0, entity_id, &[], 50)
                        .await?;
                    json!(neighbors
                        .iter()
                        .map(|neighbor| json!({
                            "entity_id": neighbor.entity.id,
                            "entity_type": neighbor.entity.entity_type,
                            "name": neighbor.entity.name,
                            "natural_key": neighbor.entity.natural_key,
                            "direction": neighbor.direction,
                            "rel_type": neighbor.rel_type,
                            "depth": neighbor.depth,
                        }))
                        .collect::<Vec<_>>())
                } else {
                    let query = arguments
                        .get("query")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if query.is_empty() {
                        return Err(AppError::Validation("entity_id か query が必要です".into()));
                    }
                    let entities = self
                        .store()
                        .graph_search_entities(&mut tx, project.0, query, None, 20)
                        .await?;
                    json!(entities
                        .iter()
                        .map(|entity| json!({
                            "entity_id": entity.id,
                            "entity_type": entity.entity_type,
                            "name": entity.name,
                            "natural_key": entity.natural_key,
                        }))
                        .collect::<Vec<_>>())
                };
                store::commit(tx).await?;
                Ok(result)
            }
            Capability::GraphPath => {
                let from = arguments
                    .get("from_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                    .ok_or_else(|| AppError::Validation("from_id が必要です".into()))?;
                let to = arguments
                    .get("to_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                    .ok_or_else(|| AppError::Validation("to_id が必要です".into()))?;
                let depth = arguments
                    .get("max_depth")
                    .and_then(Value::as_i64)
                    .unwrap_or(3)
                    .clamp(1, 6) as i32;
                let mut tx = self.store().begin_scoped(project).await?;
                let path = self
                    .store()
                    .graph_path(&mut tx, project.0, from, to, depth)
                    .await?;
                store::commit(tx).await?;
                Ok(match path {
                    Some(path) => json!({
                        "found": true,
                        "nodes": path
                            .nodes
                            .iter()
                            .map(|node| json!({
                                "entity_id": node.id,
                                "name": node.name,
                                "entity_type": node.entity_type,
                            }))
                            .collect::<Vec<_>>(),
                        "relations": path.rel_types,
                    }),
                    None => json!({"found": false}),
                })
            }
            Capability::CodeGetSymbol => {
                let query = arguments
                    .get("query")
                    .or_else(|| arguments.get("fqn"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if query.is_empty() {
                    return Err(AppError::Validation("query が必要です".into()));
                }
                let revision = arguments
                    .get("revision_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok());
                let limit = arguments
                    .get("limit")
                    .and_then(Value::as_i64)
                    .unwrap_or(20)
                    .clamp(1, 50);
                let mut tx = self.store().begin_scoped(project).await?;
                let symbols = self
                    .store()
                    .list_code_symbols(&mut tx, project.0, revision, query, limit)
                    .await?;
                store::commit(tx).await?;
                Ok(json!(symbols
                    .iter()
                    .map(|symbol| json!({
                        "symbol_id": symbol.id,
                        "file_id": symbol.file_id,
                        "kind": symbol.kind,
                        "fqn": symbol.fqn,
                        "signature": symbol.signature,
                        "line_start": symbol.line_start,
                        "line_end": symbol.line_end,
                    }))
                    .collect::<Vec<_>>()))
            }
            Capability::KnowledgeGetEvidence => {
                let evidence_id = arguments
                    .get("evidence_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                    .ok_or_else(|| AppError::Validation("evidence_id が必要です".into()))?;
                let mut tx = self.store().begin_scoped(project).await?;
                let found = self
                    .store()
                    .find_evidence(&mut tx, project.0, evidence_id)
                    .await?;
                store::commit(tx).await?;
                Ok(match found {
                    Some(row) => json!({
                        "found": true,
                        "evidence_id": row.id,
                        "quote": row.quote,
                        "locator": row.locator,
                        "resolution": row.resolution,
                        "document_title": row.document_title,
                    }),
                    None => json!({"found": false}),
                })
            }
            Capability::KnowledgeSearch => {
                let Some(auth) = auth else {
                    return Err(AppError::CapabilityUnsupported(
                        "knowledge.search はリクエスト主体が必要です".into(),
                    ));
                };
                let query = arguments
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if query.is_empty() {
                    return Err(AppError::Validation("query が必要です".into()));
                }
                let top_k = arguments
                    .get("top_k")
                    .and_then(Value::as_i64)
                    .unwrap_or(10)
                    .clamp(1, 50);
                let request: contracts::SearchRequest = serde_json::from_value(json!({
                    "query": query,
                    "top_k": top_k,
                }))
                .map_err(|error| AppError::Validation(error.to_string()))?;
                let response = self.search(auth, project, request).await?;
                Ok(serde_json::to_value(response).unwrap_or(Value::Null))
            }
            Capability::ImpactAnalyze => {
                let Some(auth) = auth else {
                    return Err(AppError::CapabilityUnsupported(
                        "impact.analyze はリクエスト主体が必要です".into(),
                    ));
                };
                let entity_ids: Vec<Uuid> = arguments
                    .get("entity_ids")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                item.as_str().and_then(|value| Uuid::parse_str(value).ok())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if entity_ids.is_empty() {
                    return Err(AppError::Validation("entity_ids が必要です".into()));
                }
                let accepted = self
                    .enqueue_impact_assessment(
                        auth,
                        project,
                        contracts::CreateImpactAssessmentRequest {
                            seed: contracts::ImpactSeedRequest {
                                kind: "ENTITY".to_owned(),
                                entity_ids,
                                base_revision_id: None,
                                head_revision_id: None,
                                paths: Vec::new(),
                                diff_unified: None,
                            },
                            options: Default::default(),
                        },
                        None,
                    )
                    .await?;
                Ok(json!({
                    "impact_id": accepted.impact_id,
                    "job_id": accepted.job_id,
                    "status": accepted.status,
                }))
            }
            Capability::TestPlanGenerate | Capability::TestCaseGenerate => {
                let Some(auth) = auth else {
                    return Err(AppError::CapabilityUnsupported(
                        "test.plan_generate はリクエスト主体が必要です".into(),
                    ));
                };
                let impact_id = arguments
                    .get("impact_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok());
                let plan_id = arguments
                    .get("plan_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok());
                // 計画の新規作成と追記は別の操作。取り違えると
                // 「同じ計画に積んでいるつもりで毎回新しい計画ができる」。
                if capability == Capability::TestPlanGenerate {
                    if plan_id.is_some() {
                        return Err(AppError::Validation(
                            "test.plan_generate は新しい計画を作ります。既存計画への追記は test.case_generate です"
                                .into(),
                        ));
                    }
                    if impact_id.is_none() {
                        return Err(AppError::Validation("impact_id が必要です".into()));
                    }
                } else if plan_id.is_none() {
                    return Err(AppError::Validation(
                        "test.case_generate には追記先の plan_id が必要です".into(),
                    ));
                }
                let response = self
                    .derive_test_plan(
                        auth,
                        project,
                        contracts::DeriveTestPlanRequest {
                            impact_id,
                            plan_id,
                            name: None,
                            max_drafts: Some(20),
                            base_path: None,
                        },
                        None,
                    )
                    .await?;
                Ok(serde_json::to_value(response).unwrap_or(Value::Null))
            }
            Capability::ProjectList => Ok(json!({
                "projects": [{"project_id": project.0}]
            })),
            Capability::CoverageGet => {
                let mut tx = self.store().begin_scoped(project).await?;
                let snapshot = self
                    .store()
                    .latest_coverage_snapshot(&mut tx, project.0)
                    .await?;
                store::commit(tx).await?;
                Ok(match snapshot {
                    Some(row) => json!({
                        "test_run_id": row.test_run_id,
                        "computed_at": row.computed_at,
                        "detail": row.detail,
                    }),
                    None => json!({"detail": null, "degraded": ["coverage_not_computed"]}),
                })
            }
            Capability::ImpactGetReport => {
                let Some(id) = arguments
                    .get("impact_id")
                    .and_then(Value::as_str)
                    .and_then(|v| Uuid::parse_str(v).ok())
                else {
                    return Err(AppError::Validation("impact_id が必要です".into()));
                };
                let mut tx = self.store().begin_scoped(project).await?;
                let draft = self
                    .store()
                    .impact_assessment(&mut tx, project.0, id)
                    .await?;
                store::commit(tx).await?;
                Ok(match draft {
                    Some(row) => json!({"impact_id": row.id, "report": row.report}),
                    None => json!({"report": null, "degraded": ["impact_not_found"]}),
                })
            }
            Capability::TestRunExecute => {
                // 破壊的操作。MCP の `tools/call` は 423 で止めるが、Workflow の
                // 承認ノード通過後はここに来る。引数は run.input をそのまま使う。
                let Some(auth) = auth else {
                    return Err(AppError::CapabilityUnsupported(
                        "test.run.execute はリクエスト主体が必要です".into(),
                    ));
                };
                let request: contracts::CreateTestRunRequest =
                    serde_json::from_value(arguments.clone()).map_err(|error| {
                        AppError::Validation(format!(
                        "test.run.execute の引数が不正です（name / environment が必要）: {error}"
                    ))
                    })?;
                let response = self.create_test_run(auth, project, request, None).await?;
                Ok(serde_json::to_value(response).unwrap_or(Value::Null))
            }
        }
    }

    // ── MCP 承認（1 回の実行への許可証）─────────────────────

    /// 承認必須ツールの呼び出し前に、未消費の承認を**消費して**通す。
    /// `None` は「まだ承認されていない」。呼び出し側は 423 を返して承認を起票する。
    pub async fn take_tool_approval(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        tool_name: &str,
        capability: &str,
        arguments: &Value,
    ) -> Result<Option<Uuid>, AppError> {
        self.require_agent_read(auth, project).await?;
        let action = mcp_approval_action(tool_name);
        let payload = mcp_approval_payload(tool_name, capability, arguments);
        let requester = auth.user_id.to_string();
        let mut tx = self.store().begin_scoped(project).await?;
        let pass = self
            .store()
            .approval_pass(
                &mut tx,
                project.0,
                &action,
                &payload,
                &requester,
                MCP_APPROVAL_WINDOW_SECS,
            )
            .await?;
        let Some(pass) = pass else {
            store::commit(tx).await?;
            return Ok(None);
        };
        // 消費できなければ他が先に使っている。二重実行させない。
        if !self
            .store()
            .consume_approval(&mut tx, project.0, pass.id)
            .await?
        {
            store::commit(tx).await?;
            return Ok(None);
        }
        self.append_mcp_audit(
            &mut tx,
            &requester,
            project,
            "ai.mcp.approval_used",
            tool_name,
            "PASSED",
            json!({"approval_id": pass.id, "decided_by": pass.decided_by}),
        )
        .await?;
        store::commit(tx).await?;
        Ok(Some(pass.id))
    }

    /// 承認を起票する。同じ引数の PENDING があればそれを返す（連打で増やさない）。
    pub async fn request_tool_approval(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        tool_name: &str,
        capability: &str,
        arguments: &Value,
        trace: Option<String>,
    ) -> Result<Uuid, AppError> {
        self.require_agent_read(auth, project).await?;
        let action = mcp_approval_action(tool_name);
        let payload = mcp_approval_payload(tool_name, capability, arguments);
        let requester = auth.user_id.to_string();
        let mut tx = self.store().begin_scoped(project).await?;
        if let Some(existing) = self
            .store()
            .pending_approval(&mut tx, project.0, &action, &payload, &requester)
            .await?
        {
            store::commit(tx).await?;
            return Ok(existing.id);
        }
        // ID にも起票者を混ぜる。別の人が同じ引数を投げたときに
        // `ON CONFLICT DO NOTHING` で相手の承認 ID を返さないようにする。
        let approval_id = stable_uuid(
            project,
            &["mcp-approval", tool_name, &requester, &payload.to_string()],
        );
        self.store()
            .insert_approval(
                &mut tx,
                project.0,
                approval_id,
                None,
                None,
                &action,
                &payload,
                &requester,
            )
            .await?;
        self.append_mcp_audit_traced(
            &mut tx,
            &requester,
            project,
            "ai.mcp.approval_request",
            tool_name,
            "PENDING",
            json!({"approval_id": approval_id, "capability": capability}),
            trace,
        )
        .await?;
        store::commit(tx).await?;
        Ok(approval_id)
    }

    // ── workflow ───────────────────────────────────────────

    pub async fn create_workflow(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: CreateWorkflowRequest,
        trace: Option<String>,
    ) -> Result<WorkflowView, AppError> {
        self.require_agent_write(auth, project).await?;
        validate_workflow(&request.definition)?;
        // 書き込み・破壊的操作は「承認ノードを前に置いた場合だけ」許可する。
        // 承認は同意ボタンではなく、何を・どの引数で実行するかを人が見て決める工程
        // （04 §4.7）。承認なしの自動実行は作らない。
        let tools = mcp_tools();
        let preceded = approval_preceded(&request.definition);
        for node in &request.definition.nodes {
            if node.kind != "capability" {
                continue;
            }
            let name = node.capability.as_deref().unwrap_or("");
            if Capability::parse(name).is_none() {
                return Err(AppError::Validation(format!("未知の capability: {name}")));
            }
            let read_only = tools
                .iter()
                .find(|tool| tool.capability == name)
                .map(|tool| tool.side_effects == "read")
                .unwrap_or(false);
            if !read_only && !preceded.contains(&node.id) {
                return Err(AppError::Validation(format!(
                    "{name} は read ではありません。承認ノードを前に置いてください"
                )));
            }
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let existing = self
            .store()
            .list_workflow_defs(&mut tx, project.0)
            .await?
            .into_iter()
            .filter(|row| row.code == request.code)
            .map(|row| row.version)
            .max()
            .unwrap_or(0);
        let version = existing + 1;
        let id = stable_uuid(project, &["workflow", &request.code, &version.to_string()]);
        self.store()
            .insert_workflow_def(
                &mut tx,
                project.0,
                id,
                &request.code,
                version,
                &serde_json::to_value(&request.definition).unwrap_or(json!({})),
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "ai.workflow.create".into(),
                    target_type: Some("ai.workflow_def".into()),
                    target_id: Some(id.to_string()),
                    input_digest: None,
                    result: "OK".into(),
                    detail: json!({"code": request.code, "version": version}),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(WorkflowView {
            id,
            code: request.code,
            version,
            status: "PUBLISHED".to_owned(),
            node_count: request.definition.nodes.len(),
            created_at: chrono::Utc::now(),
        })
    }

    pub async fn list_workflows(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<WorkflowListResponse, AppError> {
        self.require_agent_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self.store().list_workflow_defs(&mut tx, project.0).await?;
        store::commit(tx).await?;
        Ok(WorkflowListResponse {
            items: rows
                .iter()
                .map(|row| WorkflowView {
                    id: row.id,
                    code: row.code.clone(),
                    version: row.version,
                    status: row.status.clone(),
                    node_count: row
                        .definition
                        .get("nodes")
                        .and_then(Value::as_array)
                        .map(|nodes| nodes.len())
                        .unwrap_or(0),
                    created_at: row.created_at,
                })
                .collect(),
        })
    }

    pub async fn workflow_detail(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        workflow_id: Uuid,
    ) -> Result<WorkflowDetailResponse, AppError> {
        self.require_agent_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let row = self
            .store()
            .workflow_def(&mut tx, project.0, workflow_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        store::commit(tx).await?;
        let definition: WorkflowDefinition = serde_json::from_value(row.definition.clone())
            .map_err(|error| AppError::Validation(error.to_string()))?;
        Ok(WorkflowDetailResponse {
            id: row.id,
            code: row.code,
            version: row.version,
            status: row.status,
            definition,
            created_at: row.created_at,
        })
    }

    pub async fn create_workflow_run(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        workflow_id: Uuid,
        request: CreateWorkflowRunRequest,
        trace: Option<String>,
    ) -> Result<contracts::WorkflowRunAcceptedResponse, AppError> {
        self.require_agent_write(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let def = self
            .store()
            .workflow_def(&mut tx, project.0, workflow_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let run_id = stable_uuid(
            project,
            &[
                "workflow-run",
                &def.code,
                &chrono::Utc::now().timestamp_millis().to_string(),
            ],
        );
        self.store()
            .insert_workflow_run(
                &mut tx,
                &store::NewWorkflowRun {
                    id: run_id,
                    project_id: project.0,
                    workflow_def_id: workflow_id,
                    input: request.input,
                    requested_by: Some(auth.user_id.to_string()),
                },
            )
            .await?;
        let enqueued = self
            .store()
            .enqueue_job(
                &mut tx,
                &store::NewJob {
                    project_id: project.0,
                    kind: "WORKFLOW_RUN".into(),
                    idempotency_key: format!("workflow-run:{run_id}"),
                    input: json!({"run_id": run_id}),
                },
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "ai.workflow_run.create".into(),
                    target_type: Some("ai.workflow_run".into()),
                    target_id: Some(run_id.to_string()),
                    input_digest: None,
                    result: "QUEUED".into(),
                    detail: json!({"workflow": def.code, "job_id": enqueued.job.id}),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(contracts::WorkflowRunAcceptedResponse {
            run_id,
            job_id: enqueued.job.id,
            status: enqueued.job.status,
        })
    }

    pub async fn workflow_run_view(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        run_id: Uuid,
    ) -> Result<WorkflowRunView, AppError> {
        self.require_agent_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let view = self.workflow_view_in(&mut tx, project, run_id).await?;
        store::commit(tx).await?;
        view.ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))
    }

    async fn workflow_view_in(
        &self,
        tx: &mut store::Tx<'_>,
        project: ProjectId,
        run_id: Uuid,
    ) -> Result<Option<WorkflowRunView>, AppError> {
        let Some(run) = self.store().workflow_run(tx, project.0, run_id).await? else {
            return Ok(None);
        };
        let code = self
            .store()
            .workflow_def(tx, project.0, run.workflow_def_id)
            .await?
            .map(|def| def.code)
            .unwrap_or_default();
        let nodes = self.store().list_node_runs(tx, project.0, run_id).await?;
        Ok(Some(WorkflowRunView {
            id: run.id,
            code,
            status: run.status,
            current_node: run.current_node,
            input: run.input,
            output: run.output,
            nodes: nodes
                .iter()
                .map(|node| NodeRunView {
                    node_id: node.node_id.clone(),
                    attempt: node.attempt,
                    status: node.status.clone(),
                    output: node.output.clone(),
                    error: node.error.clone(),
                })
                .collect(),
            created_at: run.created_at,
            finished_at: run.finished_at,
        }))
    }

    pub async fn list_workflow_runs(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<contracts::WorkflowRunListResponse, AppError> {
        self.require_agent_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let runs = self
            .store()
            .list_workflow_runs(&mut tx, project.0, 20)
            .await?;
        let mut items = Vec::new();
        for run in runs {
            if let Some(view) = self.workflow_view_in(&mut tx, project, run.id).await? {
                items.push(view);
            }
        }
        store::commit(tx).await?;
        Ok(contracts::WorkflowRunListResponse { items })
    }

    /// worker から呼ぶ。承認ノードで `WAITING_APPROVAL` にして停止する。
    /// 再開時は同じ関数を再実行し、承認済みなら次へ進む（冪等）。
    pub async fn run_workflow_from_job(
        &self,
        project: ProjectId,
        job_id: Uuid,
        input: &Value,
    ) -> Result<WorkflowRunView, AppError> {
        let run_id = input
            .get("run_id")
            .and_then(Value::as_str)
            .and_then(|v| Uuid::parse_str(v).ok())
            .ok_or_else(|| AppError::Validation("run_id がありません".into()))?;
        let mut tx = self.store().begin_scoped(project).await?;
        let run = self
            .store()
            .workflow_run(&mut tx, project.0, run_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let def = self
            .store()
            .workflow_def(&mut tx, project.0, run.workflow_def_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let definition: WorkflowDefinition = serde_json::from_value(def.definition.clone())
            .map_err(|error| AppError::Validation(error.to_string()))?;
        let approvals = self.store().list_approvals(&mut tx, project.0, 50).await?;

        // capability ノードの実行主体は**起動した人間**。workflow は代行であり、
        // 誰の権限で動くかを曖昧にしない（旧 run は requested_by が NULL なので
        // 身元が要る能力は CAPABILITY_UNSUPPORTED で止まる）。
        let requester_auth = run
            .requested_by
            .as_deref()
            .and_then(|value| value.parse::<UserId>().ok())
            .map(Authenticated::for_job);

        let mut node = definition
            .nodes
            .iter()
            .find(|node| node.kind == "start")
            .cloned()
            .ok_or_else(|| AppError::Validation("start ノードがありません".into()))?;
        let mut status = "SUCCEEDED".to_owned();
        let mut cursor = 0usize;
        loop {
            cursor += 1;
            if cursor > definition.nodes.len() * 2 {
                status = "FAILED".to_owned();
                break;
            }
            self.store()
                .update_workflow_run(&mut tx, project.0, run_id, "RUNNING", Some(&node.id), None)
                .await?;
            match node.kind.as_str() {
                "start" => {
                    self.store()
                        .insert_node_run(&mut tx, project.0, run_id, &node.id, "PASSED", None, None)
                        .await?;
                }
                "capability" => {
                    let capability_name = node.capability.clone().unwrap_or_default();
                    let capability = Capability::parse(&capability_name)
                        .ok_or_else(|| AppError::Validation("未知の capability".into()))?;
                    let result = self
                        .execute_capability(
                            requester_auth.as_ref(),
                            project,
                            capability,
                            &run.input,
                        )
                        .await;
                    match result {
                        Ok(output) => {
                            self.store()
                                .insert_node_run(
                                    &mut tx,
                                    project.0,
                                    run_id,
                                    &node.id,
                                    "PASSED",
                                    Some(&output),
                                    None,
                                )
                                .await?;
                        }
                        Err(error) => {
                            self.store()
                                .insert_node_run(
                                    &mut tx,
                                    project.0,
                                    run_id,
                                    &node.id,
                                    "FAILED",
                                    None,
                                    Some(&json!({"detail": error.to_string()})),
                                )
                                .await?;
                            status = "FAILED".to_owned();
                            break;
                        }
                    }
                }
                "approval" => {
                    let action = node.action.clone().unwrap_or_else(|| node.id.clone());
                    let existing = approvals
                        .iter()
                        .filter(|row| row.workflow_run_id == Some(run_id) && row.action == action)
                        .max_by_key(|row| row.created_at);
                    match existing.map(|row| row.status.as_str()) {
                        Some("APPROVED") => {
                            self.store()
                                .insert_node_run(
                                    &mut tx, project.0, run_id, &node.id, "PASSED", None, None,
                                )
                                .await?;
                        }
                        Some("REJECTED") => {
                            self.store()
                                .insert_node_run(
                                    &mut tx,
                                    project.0,
                                    run_id,
                                    &node.id,
                                    "FAILED",
                                    None,
                                    Some(&json!({"code": "APPROVAL_REJECTED"})),
                                )
                                .await?;
                            status = "FAILED".to_owned();
                            break;
                        }
                        _ => {
                            let approval_id =
                                stable_uuid(project, &["approval", &run_id.to_string(), &action]);
                            let allowed = self
                                .store()
                                .approval(&mut tx, project.0, approval_id)
                                .await?;
                            if allowed.is_none() {
                                self.store()
                                    .insert_approval(
                                        &mut tx,
                                        project.0,
                                        approval_id,
                                        None,
                                        Some(run_id),
                                        &action,
                                        &json!({"workflow": def.code, "node": node.id}),
                                        &format!("workflow:{run_id}"),
                                    )
                                    .await?;
                            }
                            self.store()
                                .insert_node_run(
                                    &mut tx, project.0, run_id, &node.id, "PENDING", None, None,
                                )
                                .await?;
                            self.store()
                                .update_workflow_run(
                                    &mut tx,
                                    project.0,
                                    run_id,
                                    "WAITING_APPROVAL",
                                    Some(&node.id),
                                    None,
                                )
                                .await?;
                            let view = self
                                .workflow_view_in(&mut tx, project, run_id)
                                .await?
                                .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
                            store::commit(tx).await?;
                            return Ok(view);
                        }
                    }
                }
                "end" => {
                    self.store()
                        .insert_node_run(&mut tx, project.0, run_id, &node.id, "PASSED", None, None)
                        .await?;
                    break;
                }
                other => {
                    self.store()
                        .insert_node_run(
                            &mut tx,
                            project.0,
                            run_id,
                            &node.id,
                            "FAILED",
                            None,
                            Some(&json!({"code": "UNKNOWN_NODE_KIND", "kind": other})),
                        )
                        .await?;
                    status = "FAILED".to_owned();
                    break;
                }
            }
            let Some(next) = definition
                .edges
                .iter()
                .find(|edge| edge.from == node.id)
                .and_then(|edge| definition.nodes.iter().find(|n| n.id == edge.to))
            else {
                break;
            };
            node = next.clone();
        }

        self.store()
            .update_workflow_run(
                &mut tx,
                project.0,
                run_id,
                &status,
                Some(&node.id),
                Some(&json!({"status": status})),
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "SYSTEM".into(),
                    actor_id: job_id.to_string(),
                    action: "ai.workflow_run.finish".into(),
                    target_type: Some("ai.workflow_run".into()),
                    target_id: Some(run_id.to_string()),
                    input_digest: None,
                    result: status.clone(),
                    detail: json!({"workflow": def.code}),
                    trace_id: None,
                },
            )
            .await?;
        let view = self
            .workflow_view_in(&mut tx, project, run_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        store::commit(tx).await?;
        Ok(view)
    }

    // ── 承認 ───────────────────────────────────────────────

    pub async fn list_approvals(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<ApprovalListResponse, AppError> {
        self.require_agent_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self.store().list_approvals(&mut tx, project.0, 50).await?;
        store::commit(tx).await?;
        Ok(ApprovalListResponse {
            items: rows.iter().map(approval_view).collect(),
        })
    }

    /// 期限切れの承認依頼を閉じる。worker がティックごとに呼ぶ。
    pub async fn expire_pending_approvals(&self, ttl_secs: i64) -> Result<u64, AppError> {
        let mut tx = self.store().begin_unscoped().await?;
        let count = self
            .store()
            .expire_pending_approvals(&mut tx, ttl_secs)
            .await?;
        store::commit(tx).await?;
        Ok(count)
    }

    pub async fn decide_approval(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        approval_id: Uuid,
        request: DecideApprovalRequest,
        trace: Option<String>,
    ) -> Result<ApprovalView, AppError> {
        self.require_agent_write(auth, project).await?;
        let status = match request.decision.as_str() {
            "APPROVE" => "APPROVED",
            "REJECT" => "REJECTED",
            _ => return Err(AppError::Validation("decision が不正です".into())),
        };
        let mut tx = self.store().begin_scoped(project).await?;
        let approval = self
            .store()
            .approval(&mut tx, project.0, approval_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let changed = self
            .store()
            .decide_approval(
                &mut tx,
                project.0,
                approval_id,
                status,
                &auth.user_id.to_string(),
                request.comment.as_deref(),
            )
            .await?;
        if !changed {
            return Err(AppError::Conflict(
                contracts::ErrorCode::ConflictRevisionSuperseded,
            ));
        }
        let mut resumed = None;
        if status == "APPROVED" {
            if let Some(run_id) = approval.workflow_run_id {
                let enqueued = self
                    .store()
                    .enqueue_job(
                        &mut tx,
                        &store::NewJob {
                            project_id: project.0,
                            kind: "WORKFLOW_RUN".into(),
                            idempotency_key: format!("workflow-resume:{run_id}:{approval_id}"),
                            input: json!({"run_id": run_id}),
                        },
                    )
                    .await?;
                resumed = Some(enqueued.job.id);
            }
        }
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "ai.approval.decide".into(),
                    target_type: Some("ai.approval_request".into()),
                    target_id: Some(approval_id.to_string()),
                    input_digest: None,
                    result: status.to_owned(),
                    detail: json!({"resumed_job_id": resumed}),
                    trace_id: trace,
                },
            )
            .await?;
        let row = self
            .store()
            .approval(&mut tx, project.0, approval_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        store::commit(tx).await?;
        Ok(approval_view(&row))
    }

    // ── 一括取得 ───────────────────────────────────────────

    pub async fn ai_overview(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<AiOverviewResponse, AppError> {
        self.require_agent_read(auth, project).await?;
        let agents = self.list_agents(auth, project).await?.items;
        let mcp_servers = self.list_mcp_servers(auth, project).await?.items;
        let workflows = self.list_workflows(auth, project).await?.items;
        let workflow_runs = self.list_workflow_runs(auth, project).await?.items;
        let approvals = self.list_approvals(auth, project).await?.items;
        let agent_runs = self.list_agent_runs(auth, project).await?.items;
        let configured = matches!(self.ai().status(), AiStatus::Configured);
        // Workflow エディタは SVG で実装済み。以前の `workflow_editor_not_implemented`
        // は実装後に消し忘れていた（画面に「未実装」と出ていた）。
        let mut degraded: Vec<String> = Vec::new();
        if !mcp_servers.is_empty()
            && !mcp_servers
                .iter()
                .any(|server| server.enabled && server.health_status == "UP")
        {
            degraded.push("external_mcp_not_connected".to_owned());
        }
        if !configured {
            degraded.push("ai_unavailable".to_owned());
        }
        Ok(AiOverviewResponse {
            agents,
            mcp_servers,
            workflows,
            workflow_runs,
            approvals,
            agent_runs,
            ai_status: if configured {
                "configured"
            } else {
                "unavailable"
            }
            .to_owned(),
            model_profile_map: json!({
                "fast": std::env::var("AI_MODEL_FAST").ok(),
                "reasoning": std::env::var("AI_MODEL_REASONING").ok(),
                "long_context": std::env::var("AI_MODEL_LONG_CONTEXT").ok(),
            }),
            degraded,
        })
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push_str("...(truncated)");
        out
    }
}

fn approval_view(row: &store::ApprovalRow) -> ApprovalView {
    ApprovalView {
        id: row.id,
        action: row.action.clone(),
        status: row.status.clone(),
        payload: row.payload.clone(),
        requested_by: row.requested_by.clone(),
        decided_by: row.decided_by.clone(),
        comment: row.comment.clone(),
        created_at: row.created_at,
        decided_at: row.decided_at,
        workflow_run_id: row.workflow_run_id,
        run_id: row.run_id,
        consumed_at: row.consumed_at,
    }
}

/// `start` から到達でき、かつ途中に `approval` ノードを 1 つ以上含む node の集合。
///
/// 「承認ノードが定義のどこかに在る」では足りない。start からその node への
/// **経路上**に無ければ、承認されないまま実行されうる。状態を
/// `(node, 承認済みか)` に広げて到達可能性を判定する（定義は小さい）。
fn approval_preceded(definition: &WorkflowDefinition) -> BTreeSet<String> {
    let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &definition.edges {
        next.entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }
    let kind_of: BTreeMap<&str, &str> = definition
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.kind.as_str()))
        .collect();
    let mut out = BTreeSet::new();
    let mut seen: BTreeSet<(String, bool)> = BTreeSet::new();
    let Some(start) = definition.nodes.iter().find(|node| node.kind == "start") else {
        return out;
    };
    let mut stack = vec![(start.id.clone(), false)];
    while let Some((id, approved)) = stack.pop() {
        if !seen.insert((id.clone(), approved)) {
            continue;
        }
        if approved {
            out.insert(id.clone());
        }
        let Some(targets) = next.get(id.as_str()) else {
            continue;
        };
        for target in targets {
            let next_approved = approved || kind_of.get(target).copied() == Some("approval");
            stack.push(((*target).to_owned(), next_approved));
        }
    }
    out
}

fn validate_workflow(definition: &WorkflowDefinition) -> Result<(), AppError> {
    if definition.nodes.is_empty() {
        return Err(AppError::Validation("nodes が空です".into()));
    }
    let mut ids = BTreeMap::new();
    for node in &definition.nodes {
        if ids.insert(node.id.clone(), node.kind.clone()).is_some() {
            return Err(AppError::Validation(format!("重複 node id: {}", node.id)));
        }
        if !matches!(
            node.kind.as_str(),
            "start" | "capability" | "approval" | "end"
        ) {
            return Err(AppError::Validation(format!(
                "未知の node kind: {}",
                node.kind
            )));
        }
    }
    if !ids.values().any(|kind| kind == "start") || !ids.values().any(|kind| kind == "end") {
        return Err(AppError::Validation("start と end の両方が必要です".into()));
    }
    for edge in &definition.edges {
        if !ids.contains_key(&edge.from) || !ids.contains_key(&edge.to) {
            return Err(AppError::Validation(format!(
                "edge が未知の node を参照: {} -> {}",
                edge.from, edge.to
            )));
        }
    }
    Ok(())
}

/// Agent の出力は `narrative` と `hypotheses` だけを受理する。
/// 自由文をそのまま主表へ入れない（`04-コアエンジン` §4.7）。
pub(crate) fn parse_agent_output(text: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("JSON ではありません: {error}"))?;
    let narrative = value
        .get("narrative")
        .and_then(Value::as_str)
        .ok_or_else(|| "narrative がありません".to_owned())?;
    if narrative.trim().is_empty() {
        return Err("narrative が空です".to_owned());
    }
    let hypotheses = value
        .get("hypotheses")
        .and_then(Value::as_array)
        .ok_or_else(|| "hypotheses が配列ではありません".to_owned())?;
    if hypotheses.len() > 5 {
        return Err("hypotheses は最大 5 件です".to_owned());
    }
    let mut normalized = Vec::with_capacity(hypotheses.len());
    for (index, hypothesis) in hypotheses.iter().enumerate() {
        let claim = hypothesis
            .get("claim")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|claim| !claim.is_empty())
            .ok_or_else(|| format!("hypotheses[{index}].claim がありません"))?;
        let confidence = hypothesis
            .get("confidence")
            .and_then(Value::as_f64)
            .filter(|confidence| confidence.is_finite() && (0.0..=1.0).contains(confidence))
            .ok_or_else(|| format!("hypotheses[{index}].confidence が 0〜1 ではありません"))?;
        let basis_values = hypothesis
            .get("basis")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("hypotheses[{index}].basis が配列ではありません"))?;
        let mut basis = Vec::with_capacity(basis_values.len());
        for (basis_index, value) in basis_values.iter().enumerate() {
            let value = value
                .as_str()
                .ok_or_else(|| {
                    format!("hypotheses[{index}].basis[{basis_index}] が文字列ではありません")
                })?
                .trim();
            if value.is_empty() {
                return Err(format!("hypotheses[{index}].basis[{basis_index}] が空です"));
            }
            basis.push(value.to_owned());
        }
        if basis.is_empty() {
            return Err(format!("hypotheses[{index}].basis が空です"));
        }
        if basis.len() > 8 {
            return Err(format!("hypotheses[{index}].basis は最大 8 件です"));
        }
        normalized.push(json!({
            "claim": claim,
            "confidence": confidence,
            "basis": basis,
            "resolution": "INFERRED",
        }));
    }
    Ok(json!({
        "narrative": narrative.chars().take(800).collect::<String>(),
        "hypotheses": normalized,
        "inferred": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{WorkflowEdge, WorkflowNode};

    fn definition() -> WorkflowDefinition {
        WorkflowDefinition {
            nodes: vec![
                WorkflowNode {
                    id: "start".into(),
                    kind: "start".into(),
                    name: "開始".into(),
                    capability: None,
                    action: None,
                    x: None,
                    y: None,
                },
                WorkflowNode {
                    id: "gate".into(),
                    kind: "approval".into(),
                    name: "実行承認".into(),
                    capability: None,
                    action: Some("test.run_execute".into()),
                    x: None,
                    y: None,
                },
                WorkflowNode {
                    id: "end".into(),
                    kind: "end".into(),
                    name: "終了".into(),
                    capability: None,
                    action: None,
                    x: None,
                    y: None,
                },
            ],
            edges: vec![
                WorkflowEdge {
                    from: "start".into(),
                    to: "gate".into(),
                },
                WorkflowEdge {
                    from: "gate".into(),
                    to: "end".into(),
                },
            ],
        }
    }

    #[test]
    fn workflow_requires_start_and_end() {
        let mut broken = definition();
        broken.nodes.retain(|node| node.kind != "end");
        assert!(validate_workflow(&broken).is_err());
    }

    #[test]
    fn workflow_rejects_unknown_edge_target() {
        let mut broken = definition();
        broken.edges.push(WorkflowEdge {
            from: "gate".into(),
            to: "ghost".into(),
        });
        assert!(validate_workflow(&broken).is_err());
    }

    #[test]
    fn agent_output_requires_narrative_and_hypotheses() {
        assert!(parse_agent_output("{\"narrative\":\"x\",\"hypotheses\":[]}").is_ok());
        assert!(parse_agent_output("{\"narrative\":\"x\"}").is_err());
        assert!(parse_agent_output("free text").is_err());
        assert!(parse_agent_output(
            "{\"narrative\":\"x\",\"hypotheses\":[{\"claim\":\"c\",\"confidence\":0.5}]}"
        )
        .is_err());
        assert!(
            parse_agent_output(
                "{\"narrative\":\"x\",\"hypotheses\":[{\"claim\":\"c\",\"confidence\":2,\"basis\":[\"code:x\"]}]}"
            )
            .is_err()
        );
        assert!(
            parse_agent_output(
                "{\"narrative\":\"x\",\"hypotheses\":[{\"claim\":\"c\",\"confidence\":0.5,\"basis\":[1]}]}"
            )
            .is_err()
        );
        let parsed = parse_agent_output(
            "{\"narrative\":\"x\",\"hypotheses\":[{\"claim\":\"c\",\"confidence\":0.5,\"basis\":[\"code:x\"],\"resolution\":\"RESOLVED\"}]}",
        )
        .unwrap();
        assert_eq!(parsed["hypotheses"][0]["resolution"], "INFERRED");
        assert_eq!(parsed["inferred"], true);
    }
}
