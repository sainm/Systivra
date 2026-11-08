//! `ai` schema へのアクセス。SQL と行→構造体の変換だけを持つ。

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

#[derive(Debug, Clone)]
pub struct NewAgent {
    pub agent_id: Uuid,
    pub version_id: Uuid,
    pub project_id: Uuid,
    pub code: String,
    pub version: i32,
    pub model_profile: String,
    pub prompt_ref: String,
    pub prompt_body: String,
    pub capabilities: Vec<String>,
    pub budgets: Value,
    pub output_schema: Option<String>,
    pub approval_required: bool,
}

#[derive(Debug, Clone)]
pub struct AgentRow {
    pub id: Uuid,
    pub code: String,
    pub enabled: bool,
    pub current_version: i32,
    pub version_id: Uuid,
    pub model_profile: String,
    pub prompt_ref: String,
    pub capabilities: Vec<String>,
    pub budgets: Value,
    pub output_schema: Option<String>,
    pub approval_required: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AgentRunRow {
    pub id: Uuid,
    pub agent_version_id: Uuid,
    pub requested_by: String,
    pub requested_role: String,
    pub status: String,
    pub input: Value,
    pub output: Option<Value>,
    pub cost: Value,
    pub error: Option<Value>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct AgentStepRow {
    pub ordinal: i32,
    pub kind: String,
    pub name: String,
    pub status: String,
    pub error: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ApprovalRow {
    pub id: Uuid,
    pub run_id: Option<Uuid>,
    pub workflow_run_id: Option<Uuid>,
    pub action: String,
    pub status: String,
    pub payload: Value,
    pub requested_by: String,
    pub decided_by: Option<String>,
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    /// 承認を「1 回の実行」に使った時刻。未使用なら `None`。
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct WorkflowDefRow {
    pub id: Uuid,
    pub code: String,
    pub version: i32,
    pub definition: Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct WorkflowRunRow {
    pub id: Uuid,
    pub workflow_def_id: Uuid,
    pub status: String,
    pub input: Value,
    pub output: Option<Value>,
    pub current_node: Option<String>,
    pub requested_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NodeRunRow {
    pub node_id: String,
    pub attempt: i16,
    pub status: String,
    pub output: Option<Value>,
    pub error: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct McpServerRow {
    pub id: Uuid,
    pub name: String,
    pub transport: String,
    pub config: Value,
    pub enabled: bool,
    pub schema_hash: Option<String>,
    pub health_status: String,
    pub health_detail: Option<String>,
    pub tools: Value,
    pub last_discovery_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewAgentRun {
    pub id: Uuid,
    pub project_id: Uuid,
    pub agent_version_id: Uuid,
    pub requested_by: String,
    pub requested_role: String,
    pub input: Value,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewWorkflowRun {
    pub id: Uuid,
    pub project_id: Uuid,
    pub workflow_def_id: Uuid,
    pub input: Value,
    /// 起動した人間。capability ノードはこの主体の権限で動く。
    pub requested_by: Option<String>,
}

impl Store {
    // ── agent ───────────────────────────────────────────────

    pub async fn upsert_agent(&self, tx: &mut Tx<'_>, agent: &NewAgent) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO ai.agent (id, project_id, code, current_version)
             VALUES ($1,$2,$3,$4)
             ON CONFLICT (project_id, code)
             DO UPDATE SET current_version = EXCLUDED.current_version",
        )
        .bind(agent.agent_id)
        .bind(agent.project_id)
        .bind(&agent.code)
        .bind(agent.version)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO ai.agent_version
               (id, project_id, agent_id, version, model_profile, prompt_ref,
                capabilities, budgets, output_schema, approval_required)
             VALUES ($1,$2,$3,$4,$5,$6,$7::text[],$8,$9,$10)
             ON CONFLICT (agent_id, version) DO NOTHING",
        )
        .bind(agent.version_id)
        .bind(agent.project_id)
        .bind(agent.agent_id)
        .bind(agent.version)
        .bind(&agent.model_profile)
        .bind(&agent.prompt_ref)
        .bind(&agent.capabilities)
        .bind(&agent.budgets)
        .bind(&agent.output_schema)
        .bind(agent.approval_required)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO ai.prompt_template (id, project_id, ref, version, body, published_at)
             VALUES ($1,$2,$3,$4,$5,now())
             ON CONFLICT (project_id, ref, version) DO UPDATE SET body = EXCLUDED.body",
        )
        .bind(Uuid::new_v5(&agent.version_id, b"prompt"))
        .bind(agent.project_id)
        .bind(&agent.prompt_ref)
        .bind(agent.version)
        .bind(&agent.prompt_body)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_agents(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<AgentRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT a.id, a.code, a.enabled, a.current_version, a.created_at,
                    v.id AS version_id, v.model_profile, v.prompt_ref, v.capabilities,
                    v.budgets, v.output_schema, v.approval_required
               FROM ai.agent a
               JOIN ai.agent_version v ON v.agent_id = a.id AND v.version = a.current_version
              WHERE a.project_id = $1
              ORDER BY a.code",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(agent_row).collect())
    }

    pub async fn agent_by_code(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        code: &str,
    ) -> Result<Option<AgentRow>, StoreError> {
        let row = sqlx::query(
            "SELECT a.id, a.code, a.enabled, a.current_version, a.created_at,
                    v.id AS version_id, v.model_profile, v.prompt_ref, v.capabilities,
                    v.budgets, v.output_schema, v.approval_required
               FROM ai.agent a
               JOIN ai.agent_version v ON v.agent_id = a.id AND v.version = a.current_version
              WHERE a.project_id = $1 AND a.code = $2",
        )
        .bind(project_id)
        .bind(code)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(agent_row))
    }

    pub async fn prompt_body(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        prompt_ref: &str,
    ) -> Result<Option<String>, StoreError> {
        let row = sqlx::query(
            "SELECT body FROM ai.prompt_template
              WHERE project_id = $1 AND ref = $2
              ORDER BY version DESC LIMIT 1",
        )
        .bind(project_id)
        .bind(prompt_ref)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|row| row.get("body")))
    }

    // ── agent run / step ────────────────────────────────────

    pub async fn insert_agent_run(
        &self,
        tx: &mut Tx<'_>,
        run: &NewAgentRun,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO ai.agent_run
               (id, project_id, agent_version_id, status, input, trace_id,
                requested_by, requested_role)
             VALUES ($1,$2,$3,'PENDING',$4,$5,$6,$7)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(run.id)
        .bind(run.project_id)
        .bind(run.agent_version_id)
        .bind(&run.input)
        .bind(&run.trace_id)
        .bind(&run.requested_by)
        .bind(&run.requested_role)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn start_agent_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE ai.agent_run SET status = 'RUNNING', started_at = now()
              WHERE project_id = $1 AND id = $2 AND status = 'PENDING'",
        )
        .bind(project_id)
        .bind(run_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finish_agent_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
        status: &str,
        output: Option<&Value>,
        cost: &Value,
        error: Option<&Value>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE ai.agent_run
                SET status = $3, output = $4, cost = $5, error = $6, finished_at = now()
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(run_id)
        .bind(status)
        .bind(output)
        .bind(cost)
        .bind(error)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn agent_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
    ) -> Result<Option<AgentRunRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, agent_version_id, status, input, output, cost, error,
                    started_at, finished_at, requested_by, requested_role
               FROM ai.agent_run WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(agent_run_row))
    }

    pub async fn list_agent_runs(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<AgentRunRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, agent_version_id, status, input, output, cost, error,
                    started_at, finished_at, requested_by, requested_role
               FROM ai.agent_run WHERE project_id = $1
              ORDER BY started_at DESC LIMIT $2",
        )
        .bind(project_id)
        .bind(limit.clamp(1, 100))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(agent_run_row).collect())
    }

    pub async fn agent_code_for_version(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        version_id: Uuid,
    ) -> Result<String, StoreError> {
        let row = sqlx::query(
            "SELECT a.code FROM ai.agent_version v
               JOIN ai.agent a ON a.id = v.agent_id
              WHERE v.project_id = $1 AND v.id = $2",
        )
        .bind(project_id)
        .bind(version_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|r| r.get("code")).unwrap_or_default())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_agent_step(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
        ordinal: i32,
        kind: &str,
        name: &str,
        status: &str,
        error: Option<&Value>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO ai.agent_step
               (id, project_id, run_id, ordinal, kind, name, status, error)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (run_id, ordinal) DO NOTHING",
        )
        .bind(Uuid::new_v5(&run_id, format!("step:{ordinal}").as_bytes()))
        .bind(project_id)
        .bind(run_id)
        .bind(ordinal)
        .bind(kind)
        .bind(name)
        .bind(status)
        .bind(error)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_agent_steps(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
    ) -> Result<Vec<AgentStepRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT ordinal, kind, name, status, error
               FROM ai.agent_step WHERE project_id = $1 AND run_id = $2
              ORDER BY ordinal",
        )
        .bind(project_id)
        .bind(run_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| AgentStepRow {
                ordinal: row.get("ordinal"),
                kind: row.get("kind"),
                name: row.get("name"),
                status: row.get("status"),
                error: row.get("error"),
            })
            .collect())
    }

    // ── approval ────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_approval(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        id: Uuid,
        run_id: Option<Uuid>,
        workflow_run_id: Option<Uuid>,
        action: &str,
        payload: &Value,
        requested_by: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO ai.approval_request
               (id, project_id, run_id, workflow_run_id, action, payload_digest,
                payload, status, requested_by)
             VALUES ($1,$2,$3,$4,$5,sha256($6::text::bytea),$6,'PENDING',$7)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(project_id)
        .bind(run_id)
        .bind(workflow_run_id)
        .bind(action)
        .bind(payload)
        .bind(requested_by)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_approvals(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ApprovalRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, run_id, workflow_run_id, action, status, payload,
                    requested_by, decided_by, comment, created_at, decided_at,
                    consumed_at
               FROM ai.approval_request WHERE project_id = $1
              ORDER BY created_at DESC LIMIT $2",
        )
        .bind(project_id)
        .bind(limit.clamp(1, 100))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(approval_row).collect())
    }

    pub async fn approval(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        approval_id: Uuid,
    ) -> Result<Option<ApprovalRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, run_id, workflow_run_id, action, status, payload,
                    requested_by, decided_by, comment, created_at, decided_at,
                    consumed_at
               FROM ai.approval_request WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(approval_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(approval_row))
    }

    /// 同じ action と**同じ payload**（jsonb 等価）で、未消費・期限内の承認済みを引く。
    /// 引数が 1 つでも違えば別物として扱う（payload の等価で判定する）。
    pub async fn approval_pass(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        action: &str,
        payload: &Value,
        requested_by: &str,
        within_secs: i64,
    ) -> Result<Option<ApprovalRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, run_id, workflow_run_id, action, status, payload,
                    requested_by, decided_by, comment, created_at, decided_at,
                    consumed_at
               FROM ai.approval_request
              WHERE project_id = $1 AND action = $2 AND payload = $3::jsonb
                AND requested_by = $4
                AND status = 'APPROVED' AND consumed_at IS NULL
                AND decided_at > now() - make_interval(secs => $5)
              ORDER BY decided_at DESC LIMIT 1",
        )
        .bind(project_id)
        .bind(action)
        .bind(payload)
        .bind(requested_by)
        .bind(within_secs as f64)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(approval_row))
    }

    /// 承認を 1 回分だけ消費する。既に消費済みなら false（二重実行を防ぐ）。
    pub async fn consume_approval(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        approval_id: Uuid,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE ai.approval_request SET consumed_at = now()
              WHERE project_id = $1 AND id = $2
                AND status = 'APPROVED' AND consumed_at IS NULL",
        )
        .bind(project_id)
        .bind(approval_id)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 期限を過ぎた PENDING を `EXPIRED` にする。
    ///
    /// 承認は「その時点の diff を見て判断する」ものなので、古い依頼をいつまでも
    /// 承認可能のまま置かない。判断者を立てずに閉じる（decided_by は触らない）。
    pub async fn expire_pending_approvals(
        &self,
        tx: &mut Tx<'_>,
        ttl_secs: i64,
    ) -> Result<u64, StoreError> {
        let result = sqlx::query(
            "UPDATE ai.approval_request
                SET status = 'EXPIRED'
              WHERE status = 'PENDING'
                AND created_at < now() - make_interval(secs => $1)",
        )
        .bind(ttl_secs as f64)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected())
    }

    /// 同じ action / payload の PENDING。連打で承認依頼を増やさないために使う。
    pub async fn pending_approval(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        action: &str,
        payload: &Value,
        requested_by: &str,
    ) -> Result<Option<ApprovalRow>, StoreError> {
        // **起票者で絞る**。同じ action / payload でも他人の承認を返すと、
        // 相手が承認しても `approval_pass` は requested_by が違って使えず、
        // こちらは 423 のまま永久に進まなくなる。
        let row = sqlx::query(
            "SELECT id, run_id, workflow_run_id, action, status, payload,
                    requested_by, decided_by, comment, created_at, decided_at,
                    consumed_at
               FROM ai.approval_request
              WHERE project_id = $1 AND action = $2 AND payload = $3::jsonb
                AND requested_by = $4
                AND status = 'PENDING'
              ORDER BY created_at DESC LIMIT 1",
        )
        .bind(project_id)
        .bind(action)
        .bind(payload)
        .bind(requested_by)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(approval_row))
    }

    pub async fn decide_approval(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        approval_id: Uuid,
        status: &str,
        decided_by: &str,
        comment: Option<&str>,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE ai.approval_request
                SET status = $3, decided_by = $4, decided_at = now(), comment = $5
              WHERE project_id = $1 AND id = $2 AND status = 'PENDING'",
        )
        .bind(project_id)
        .bind(approval_id)
        .bind(status)
        .bind(decided_by)
        .bind(comment)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ── workflow ────────────────────────────────────────────

    pub async fn insert_workflow_def(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        id: Uuid,
        code: &str,
        version: i32,
        definition: &Value,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO ai.workflow_def (id, project_id, code, version, definition, status)
             VALUES ($1,$2,$3,$4,$5,'PUBLISHED')
             ON CONFLICT (project_id, code, version) DO NOTHING",
        )
        .bind(id)
        .bind(project_id)
        .bind(code)
        .bind(version)
        .bind(definition)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_workflow_defs(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<WorkflowDefRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, code, version, definition, status, created_at
               FROM ai.workflow_def WHERE project_id = $1
              ORDER BY code, version DESC",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(workflow_def_row).collect())
    }

    pub async fn workflow_def(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        id: Uuid,
    ) -> Result<Option<WorkflowDefRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, code, version, definition, status, created_at
               FROM ai.workflow_def WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(workflow_def_row))
    }

    pub async fn insert_workflow_run(
        &self,
        tx: &mut Tx<'_>,
        run: &NewWorkflowRun,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO ai.workflow_run
               (id, project_id, workflow_def_id, status, input, requested_by)
             VALUES ($1,$2,$3,'PENDING',$4,$5)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(run.id)
        .bind(run.project_id)
        .bind(run.workflow_def_id)
        .bind(&run.input)
        .bind(run.requested_by.as_deref())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn workflow_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
    ) -> Result<Option<WorkflowRunRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, workflow_def_id, status, input, output, current_node,
                    requested_by, created_at, finished_at
               FROM ai.workflow_run WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(workflow_run_row))
    }

    pub async fn list_workflow_runs(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<WorkflowRunRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, workflow_def_id, status, input, output, current_node,
                    requested_by, created_at, finished_at
               FROM ai.workflow_run WHERE project_id = $1
              ORDER BY created_at DESC LIMIT $2",
        )
        .bind(project_id)
        .bind(limit.clamp(1, 100))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(workflow_run_row).collect())
    }

    pub async fn update_workflow_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
        status: &str,
        current_node: Option<&str>,
        output: Option<&Value>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE ai.workflow_run
                SET status = $3, current_node = $4, output = $5,
                    started_at = COALESCE(started_at, now()),
                    finished_at = CASE WHEN $3 IN ('SUCCEEDED','FAILED','CANCELLED')
                                       THEN now() ELSE finished_at END
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(run_id)
        .bind(status)
        .bind(current_node)
        .bind(output)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_node_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
        node_id: &str,
        status: &str,
        output: Option<&Value>,
        error: Option<&Value>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO ai.node_run
               (id, project_id, run_id, node_id, attempt, status, output, error,
                started_at, finished_at)
             VALUES ($1,$2,$3,$4,1,$5,$6,$7,now(),now())
             ON CONFLICT (run_id, node_id, attempt) DO UPDATE
               SET status = EXCLUDED.status, output = EXCLUDED.output,
                   error = EXCLUDED.error, finished_at = now()",
        )
        .bind(Uuid::new_v5(&run_id, format!("node:{node_id}").as_bytes()))
        .bind(project_id)
        .bind(run_id)
        .bind(node_id)
        .bind(status)
        .bind(output)
        .bind(error)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_node_runs(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
    ) -> Result<Vec<NodeRunRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT node_id, attempt, status, output, error
               FROM ai.node_run WHERE project_id = $1 AND run_id = $2
              ORDER BY started_at NULLS LAST, node_id",
        )
        .bind(project_id)
        .bind(run_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| NodeRunRow {
                node_id: row.get("node_id"),
                attempt: row.get("attempt"),
                status: row.get("status"),
                output: row.get("output"),
                error: row.get("error"),
            })
            .collect())
    }

    // ── mcp ─────────────────────────────────────────────────

    pub async fn upsert_mcp_server(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        id: Uuid,
        name: &str,
        transport: &str,
        config: &Value,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO ai.mcp_server (id, project_id, name, transport, config)
             VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (project_id, name) DO UPDATE
               SET transport = EXCLUDED.transport,
                   config = EXCLUDED.config,
                   schema_hash = NULL,
                   health_status = 'UNKNOWN',
                   health_detail = NULL,
                   tools = '[]'::jsonb,
                   last_discovery_at = NULL",
        )
        .bind(id)
        .bind(project_id)
        .bind(name)
        .bind(transport)
        .bind(config)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_mcp_servers(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<McpServerRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, name, transport, config, enabled, schema_hash,
                    health_status, health_detail, tools, last_discovery_at
               FROM ai.mcp_server WHERE project_id = $1 ORDER BY name",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| McpServerRow {
                id: row.get("id"),
                name: row.get("name"),
                transport: row.get("transport"),
                config: row.get("config"),
                enabled: row.get("enabled"),
                schema_hash: row.get("schema_hash"),
                health_status: row.get("health_status"),
                health_detail: row.get("health_detail"),
                tools: row.get("tools"),
                last_discovery_at: row.get("last_discovery_at"),
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_mcp_discovery(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        name: &str,
        schema_hash: &str,
        health: &str,
        detail: Option<&str>,
        tools: Option<&Value>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE ai.mcp_server
                SET schema_hash = $3,
                    health_status = $4,
                    health_detail = $5,
                    tools = COALESCE($6, tools),
                    last_discovery_at = now()
              WHERE project_id = $1 AND name = $2",
        )
        .bind(project_id)
        .bind(name)
        .bind(schema_hash)
        .bind(health)
        .bind(detail)
        .bind(tools)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn update_mcp_health(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        name: &str,
        health: &str,
        detail: Option<&str>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE ai.mcp_server
                SET health_status = $3, health_detail = $4, last_discovery_at = now()
              WHERE project_id = $1 AND name = $2",
        )
        .bind(project_id)
        .bind(name)
        .bind(health)
        .bind(detail)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}

fn agent_row(row: &sqlx::postgres::PgRow) -> AgentRow {
    AgentRow {
        id: row.get("id"),
        code: row.get("code"),
        enabled: row.get("enabled"),
        current_version: row.get("current_version"),
        version_id: row.get("version_id"),
        model_profile: row.get("model_profile"),
        prompt_ref: row.get("prompt_ref"),
        capabilities: row.get("capabilities"),
        budgets: row.get("budgets"),
        output_schema: row.get("output_schema"),
        approval_required: row.get("approval_required"),
        created_at: row.get("created_at"),
    }
}

fn agent_run_row(row: &sqlx::postgres::PgRow) -> AgentRunRow {
    AgentRunRow {
        id: row.get("id"),
        agent_version_id: row.get("agent_version_id"),
        requested_by: row.get("requested_by"),
        requested_role: row.get("requested_role"),
        status: row.get("status"),
        input: row.get("input"),
        output: row.get("output"),
        cost: row.get("cost"),
        error: row.get("error"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    }
}

fn approval_row(row: &sqlx::postgres::PgRow) -> ApprovalRow {
    ApprovalRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        workflow_run_id: row.get("workflow_run_id"),
        action: row.get("action"),
        status: row.get("status"),
        payload: row.get("payload"),
        requested_by: row.get("requested_by"),
        decided_by: row.get("decided_by"),
        comment: row.get("comment"),
        created_at: row.get("created_at"),
        decided_at: row.get("decided_at"),
        consumed_at: row.get("consumed_at"),
    }
}

fn workflow_def_row(row: &sqlx::postgres::PgRow) -> WorkflowDefRow {
    WorkflowDefRow {
        id: row.get("id"),
        code: row.get("code"),
        version: row.get("version"),
        definition: row.get("definition"),
        status: row.get("status"),
        created_at: row.get("created_at"),
    }
}

fn workflow_run_row(row: &sqlx::postgres::PgRow) -> WorkflowRunRow {
    WorkflowRunRow {
        id: row.get("id"),
        workflow_def_id: row.get("workflow_def_id"),
        status: row.get("status"),
        input: row.get("input"),
        output: row.get("output"),
        current_node: row.get("current_node"),
        requested_by: row.get("requested_by"),
        created_at: row.get("created_at"),
        finished_at: row.get("finished_at"),
    }
}
