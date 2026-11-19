use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

#[derive(Debug, Clone)]
pub struct AuditDraft {
    pub project_id: Option<Uuid>,
    pub actor_type: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub input_digest: Option<Vec<u8>>,
    pub result: String,
    pub detail: Value,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuditRow {
    pub id: i64,
    pub actor_type: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub result: String,
    pub trace_id: Option<String>,
    pub detail: Value,
    pub occurred_at: DateTime<Utc>,
}

impl Store {
    pub async fn append_audit(
        &self,
        tx: &mut Tx<'_>,
        draft: &AuditDraft,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO core.audit_log
               (project_id, actor_type, actor_id, action, target_type, target_id,
                input_digest, result, detail, trace_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(draft.project_id)
        .bind(&draft.actor_type)
        .bind(&draft.actor_id)
        .bind(&draft.action)
        .bind(&draft.target_type)
        .bind(&draft.target_id)
        .bind(&draft.input_digest)
        .bind(&draft.result)
        .bind(&draft.detail)
        .bind(&draft.trace_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_audit(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<AuditRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, actor_type, actor_id, action, target_type, target_id,
                    result, trace_id, detail, occurred_at
               FROM core.audit_log
              WHERE project_id = $1
              ORDER BY occurred_at DESC, id DESC
              LIMIT $2",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| AuditRow {
                id: r.get("id"),
                actor_type: r.get("actor_type"),
                actor_id: r.get("actor_id"),
                action: r.get("action"),
                target_type: r.get("target_type"),
                target_id: r.get("target_id"),
                result: r.get("result"),
                trace_id: r.get("trace_id"),
                detail: r.get("detail"),
                occurred_at: r.get("occurred_at"),
            })
            .collect())
    }
    /// 監査ログの月次パーティションを維持する。
    ///
    /// 期限切れの月は **DROP** する（DELETE ではない）。`core.audit_log` は
    /// 追記専用で app / worker から DELETE を剥がしてあるので、保持期間のために
    /// その約束を崩さない。関数は SECURITY DEFINER で、対象を
    /// `audit_log_YYYYMM` に限定している。
    pub async fn maintain_audit_partitions(
        &self,
        tx: &mut Tx<'_>,
        retention_months: i32,
        ahead_months: i32,
    ) -> Result<Value, StoreError> {
        let row = sqlx::query("SELECT core.maintain_audit_partitions($1, $2) AS result")
            .bind(retention_months)
            .bind(ahead_months)
            .fetch_one(&mut **tx)
            .await?;
        Ok(row.get("result"))
    }
}
