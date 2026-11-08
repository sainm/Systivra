//! `impact.change_inquiry` へのアクセス。

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

#[derive(Debug, Clone)]
pub struct NewInquiry {
    pub id: Uuid,
    pub project_id: Uuid,
    pub question: String,
    pub input_kind: String,
    pub status: String,
    pub impact_id: Option<Uuid>,
    pub candidates: Value,
}

#[derive(Debug, Clone)]
pub struct InquiryRow {
    pub id: Uuid,
    pub question: String,
    pub input_kind: String,
    pub status: String,
    pub impact_id: Option<Uuid>,
    pub report: Option<Value>,
    pub candidates: Value,
    pub error: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl Store {
    pub async fn create_inquiry(
        &self,
        tx: &mut Tx<'_>,
        inquiry: &NewInquiry,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO impact.change_inquiry
               (id, project_id, question, input_kind, status, impact_id, candidates)
             VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(inquiry.id)
        .bind(inquiry.project_id)
        .bind(&inquiry.question)
        .bind(&inquiry.input_kind)
        .bind(&inquiry.status)
        .bind(inquiry.impact_id)
        .bind(&inquiry.candidates)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn inquiry(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        inquiry_id: Uuid,
    ) -> Result<Option<InquiryRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, question, input_kind, status, impact_id, report, candidates,
                    error, created_at, finished_at
               FROM impact.change_inquiry WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(inquiry_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(inquiry_row))
    }

    pub async fn list_inquiries(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<InquiryRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, question, input_kind, status, impact_id, report, candidates,
                    error, created_at, finished_at
               FROM impact.change_inquiry WHERE project_id = $1
              ORDER BY created_at DESC LIMIT $2",
        )
        .bind(project_id)
        .bind(limit.clamp(1, 100))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(inquiry_row).collect())
    }

    pub async fn set_inquiry_impact(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        inquiry_id: Uuid,
        impact_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE impact.change_inquiry SET impact_id = $3
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(inquiry_id)
        .bind(impact_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn finish_inquiry(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        inquiry_id: Uuid,
        status: &str,
        report: Option<&Value>,
        error: Option<&Value>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE impact.change_inquiry
                SET status = $3, report = COALESCE($4, report), error = $5,
                    finished_at = CASE WHEN $3 IN ('SUCCEEDED','FAILED','NEEDS_CLARIFICATION')
                                       THEN now() ELSE finished_at END
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(inquiry_id)
        .bind(status)
        .bind(report)
        .bind(error)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}

fn inquiry_row(row: &sqlx::postgres::PgRow) -> InquiryRow {
    InquiryRow {
        id: row.get("id"),
        question: row.get("question"),
        input_kind: row.get("input_kind"),
        status: row.get("status"),
        impact_id: row.get("impact_id"),
        report: row.get("report"),
        candidates: row.get("candidates"),
        error: row.get("error"),
        created_at: row.get("created_at"),
        finished_at: row.get("finished_at"),
    }
}
