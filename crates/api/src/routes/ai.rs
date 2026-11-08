//! Phase 7 の HTTP 境界（Agent / MCP / Workflow / 承認）。
//!
//! MCP Server は project-scoped の `/projects/{pid}/mcp` に置く。
//! 外部 Agent は PAT でプロジェクトを指定し、Gateway が `project_id` を注入する。

use app::Authenticated;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use contracts::{
    AgentListResponse, AgentRunAcceptedResponse, AgentRunListResponse, AgentRunView,
    AiOverviewResponse, ApprovalListResponse, ApprovalView, CreateAgentRunRequest,
    CreateWorkflowRequest, CreateWorkflowRunRequest, DecideApprovalRequest, ErrorCode,
    McpServerListResponse, McpServerView, McpToolsResponse, RegisterAgentRequest,
    RegisterMcpServerRequest, WorkflowListResponse, WorkflowRunAcceptedResponse,
    WorkflowRunListResponse, WorkflowRunView, WorkflowView,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/projects/{pid}/agents",
            get(list_agents).post(register_agent),
        )
        .route(
            "/api/v1/projects/{pid}/agents/{code}/runs",
            post(create_agent_run),
        )
        .route("/api/v1/projects/{pid}/agent-runs", get(list_agent_runs))
        .route("/api/v1/projects/{pid}/agent-runs/{rid}", get(agent_run))
        .route("/api/v1/projects/{pid}/ai/overview", get(ai_overview))
        .route(
            "/api/v1/projects/{pid}/mcp/servers",
            get(list_mcp_servers).post(register_mcp_server),
        )
        .route(
            "/api/v1/projects/{pid}/mcp/servers/{name}/discover",
            post(discover_mcp_server),
        )
        .route("/api/v1/projects/{pid}/mcp/tools", get(mcp_tools))
        .route("/api/v1/projects/{pid}/mcp", post(mcp_rpc))
        .route(
            "/api/v1/projects/{pid}/workflows",
            get(list_workflows).post(create_workflow),
        )
        .route(
            "/api/v1/projects/{pid}/workflows/{wid}",
            get(workflow_detail),
        )
        .route(
            "/api/v1/projects/{pid}/workflows/{wid}/runs",
            post(create_workflow_run),
        )
        .route(
            "/api/v1/projects/{pid}/workflow-runs",
            get(list_workflow_runs),
        )
        .route(
            "/api/v1/projects/{pid}/workflow-runs/{rid}",
            get(workflow_run),
        )
        .route("/api/v1/projects/{pid}/approvals", get(list_approvals))
        .route(
            "/api/v1/projects/{pid}/approvals/{aid}/decide",
            post(decide_approval),
        )
}

fn parse_uuid(raw: &str, trace: &TraceId) -> Result<Uuid, ApiError> {
    raw.parse()
        .map_err(|_| ApiError::new(ErrorCode::ValidationSchema, trace))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/agents",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = RegisterAgentRequest,
    responses((status = 200, body = contracts::AgentView), (status = 400, body = contracts::Problem))
)]
pub(crate) async fn register_agent(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<RegisterAgentRequest>,
) -> Result<Json<contracts::AgentView>, ApiError> {
    let response = state
        .app
        .register_agent(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/agents",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = AgentListResponse))
)]
pub(crate) async fn list_agents(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<AgentListResponse>, ApiError> {
    state
        .app
        .list_agents(&auth, scope.project)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/agents/{code}/runs",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("code" = String, Path, description = "Agent code")
    ),
    request_body = CreateAgentRunRequest,
    responses((status = 202, body = AgentRunAcceptedResponse))
)]
pub(crate) async fn create_agent_run(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, code)): Path<(String, String)>,
    Json(request): Json<CreateAgentRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let response = state
        .app
        .create_agent_run(&auth, scope.project, &code, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/agent-runs",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = AgentRunListResponse))
)]
pub(crate) async fn list_agent_runs(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<AgentRunListResponse>, ApiError> {
    state
        .app
        .list_agent_runs(&auth, scope.project)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/agent-runs/{rid}",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("rid" = uuid::Uuid, Path, description = "Agent run ID")
    ),
    responses((status = 200, body = AgentRunView), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn agent_run(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, rid)): Path<(String, String)>,
) -> Result<Json<AgentRunView>, ApiError> {
    let run_id = parse_uuid(&rid, &trace)?;
    state
        .app
        .agent_run_view(&auth, scope.project, run_id)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/ai/overview",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = AiOverviewResponse))
)]
pub(crate) async fn ai_overview(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<AiOverviewResponse>, ApiError> {
    state
        .app
        .ai_overview(&auth, scope.project)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/mcp/servers",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = RegisterMcpServerRequest,
    responses((status = 200, body = McpServerView))
)]
pub(crate) async fn register_mcp_server(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<RegisterMcpServerRequest>,
) -> Result<Json<McpServerView>, ApiError> {
    state
        .app
        .register_mcp_server(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/mcp/servers",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = McpServerListResponse))
)]
pub(crate) async fn list_mcp_servers(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<McpServerListResponse>, ApiError> {
    state
        .app
        .list_mcp_servers(&auth, scope.project)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/mcp/servers/{name}/discover",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("name" = String, Path, description = "MCP server name")
    ),
    responses((status = 200, body = McpServerView))
)]
pub(crate) async fn discover_mcp_server(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, name)): Path<(String, String)>,
) -> Result<Json<McpServerView>, ApiError> {
    state
        .app
        .discover_mcp_server(&auth, scope.project, &name)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/mcp/tools",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = McpToolsResponse))
)]
pub(crate) async fn mcp_tools(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<McpToolsResponse>, ApiError> {
    // 権限は overview と同じ（AgentRun の read）。
    state
        .app
        .list_agents(&auth, scope.project)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(state.app.tools()))
}

/// MCP JSON-RPC。`initialize` / `tools/list` / `tools/call` のみ。
pub(crate) async fn mcp_rpc(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let error = |code: i32, message: String| {
        ApiError::new(ErrorCode::ValidationSchema, &trace).detail(format!("{code}: {message}"))
    };
    match method {
        "initialize" => Ok(Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "systivra", "version": "0.1.0"},
                "capabilities": {"tools": {}}
            }
        }))),
        "tools/list" => {
            let tools = state.app.tools();
            Ok(Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"tools": tools.tools, "schema_hash": tools.schema_hash}
            })))
        }
        "tools/call" => {
            let name = request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let tool = state
                .app
                .tools()
                .tools
                .into_iter()
                .find(|tool| tool.name == name)
                .ok_or_else(|| error(-32601, format!("unknown tool: {name}")))?;
            // 権限（Gateway）を承認より先に判定する。権限が無い人に
            // 「承認が必要です」と返すと、存在と要件が漏れる。
            state
                .app
                .authorize_tool(&auth, scope.project, &tool.capability)
                .await
                .map_err(|error| ApiError::from_app(error, &trace))?;
            if tool.approval_required {
                // 承認は「同意ボタン」ではなく 1 回の実行への許可証。
                // 未消費の承認があればここで消費して通し、無ければ起票して 423。
                let pass = state
                    .app
                    .take_tool_approval(
                        &auth,
                        scope.project,
                        &tool.name,
                        &tool.capability,
                        &arguments,
                    )
                    .await
                    .map_err(|error| ApiError::from_app(error, &trace))?;
                if pass.is_none() {
                    let approval_id = state
                        .app
                        .request_tool_approval(
                            &auth,
                            scope.project,
                            &tool.name,
                            &tool.capability,
                            &arguments,
                            Some(trace.0.clone()),
                        )
                        .await
                        .map_err(|error| ApiError::from_app(error, &trace))?;
                    return Err(ApiError::new(ErrorCode::PolicyApprovalRequired, &trace)
                        .detail(format!(
                            "{} は承認が必要です（approval_id={}）。承認後に**同じ引数**で呼ぶと 1 回だけ実行されます",
                            tool.name, approval_id
                        )));
                }
            }
            match state
                .app
                .call_tool(&auth, scope.project, &tool.capability, &arguments)
                .await
            {
                Ok(output) => Ok(Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": output.to_string()}],
                        "isError": false
                    }
                }))),
                Err(app_error) => {
                    let api = ApiError::from_app(app_error, &trace);
                    Err(api)
                }
            }
        }
        other => Err(error(-32601, format!("unknown method: {other}"))),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/workflows",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = CreateWorkflowRequest,
    responses((status = 200, body = WorkflowView))
)]
pub(crate) async fn create_workflow(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<CreateWorkflowRequest>,
) -> Result<Json<WorkflowView>, ApiError> {
    state
        .app
        .create_workflow(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/workflows",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = WorkflowListResponse))
)]
pub(crate) async fn list_workflows(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<WorkflowListResponse>, ApiError> {
    state
        .app
        .list_workflows(&auth, scope.project)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/workflows/{wid}/runs",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("wid" = uuid::Uuid, Path, description = "Workflow definition ID")
    ),
    request_body = CreateWorkflowRunRequest,
    responses((status = 202, body = WorkflowRunAcceptedResponse))
)]
pub(crate) async fn create_workflow_run(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, wid)): Path<(String, String)>,
    Json(request): Json<CreateWorkflowRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow_id = parse_uuid(&wid, &trace)?;
    let response = state
        .app
        .create_workflow_run(
            &auth,
            scope.project,
            workflow_id,
            request,
            Some(trace.0.clone()),
        )
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/workflow-runs",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = WorkflowRunListResponse))
)]
pub(crate) async fn list_workflow_runs(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<WorkflowRunListResponse>, ApiError> {
    state
        .app
        .list_workflow_runs(&auth, scope.project)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/workflow-runs/{rid}",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("rid" = uuid::Uuid, Path, description = "Workflow run ID")
    ),
    responses((status = 200, body = WorkflowRunView), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn workflow_run(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, rid)): Path<(String, String)>,
) -> Result<Json<WorkflowRunView>, ApiError> {
    let run_id = parse_uuid(&rid, &trace)?;
    state
        .app
        .workflow_run_view(&auth, scope.project, run_id)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/approvals",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = ApprovalListResponse))
)]
pub(crate) async fn list_approvals(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<ApprovalListResponse>, ApiError> {
    state
        .app
        .list_approvals(&auth, scope.project)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/approvals/{aid}/decide",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("aid" = uuid::Uuid, Path, description = "Approval ID")
    ),
    request_body = DecideApprovalRequest,
    responses((status = 200, body = ApprovalView))
)]
pub(crate) async fn decide_approval(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, aid)): Path<(String, String)>,
    Json(request): Json<DecideApprovalRequest>,
) -> Result<Json<ApprovalView>, ApiError> {
    let approval_id = parse_uuid(&aid, &trace)?;
    state
        .app
        .decide_approval(
            &auth,
            scope.project,
            approval_id,
            request,
            Some(trace.0.clone()),
        )
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/workflows/{wid}",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("wid" = uuid::Uuid, Path, description = "Workflow definition ID")
    ),
    responses((status = 200, body = contracts::WorkflowDetailResponse))
)]
pub(crate) async fn workflow_detail(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, wid)): Path<(String, String)>,
) -> Result<Json<contracts::WorkflowDetailResponse>, ApiError> {
    let workflow_id = parse_uuid(&wid, &trace)?;
    state
        .app
        .workflow_detail(&auth, scope.project, workflow_id)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}
