use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

#[derive(Debug, Clone)]
pub struct NewImpactAssessment {
    pub id: Uuid,
    pub project_id: Uuid,
    pub input: Value,
}

#[derive(Debug, Clone)]
pub struct ImpactAssessmentRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub input: Value,
    pub status: String,
    pub report: Option<Value>,
    pub engine_version: Option<String>,
    pub ontology_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ChangedCodeSymbolRow {
    pub id: Uuid,
    pub file_id: Uuid,
    pub path: String,
    pub kind: String,
    pub fqn: String,
    pub simple_name: String,
    pub line_start: i32,
    pub line_end: i32,
}

#[derive(Debug, Clone)]
pub struct ImpactEvidenceRow {
    pub evidence_id: Option<Uuid>,
    pub quote: String,
    pub locator: Value,
    pub resolution: String,
    pub confidence: f32,
    pub source_type: String,
    pub relation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImpactDocumentMentionRow {
    pub document_id: Uuid,
    pub document_title: String,
    pub doc_type: String,
    pub chunk_id: Uuid,
    pub quote: String,
    pub locator: Value,
    pub evidence_id: Option<Uuid>,
}

pub struct ImpactCompletion<'a> {
    pub status: &'a str,
    pub report: Option<&'a Value>,
    pub engine_version: Option<&'a str>,
    pub ontology_version: Option<&'a str>,
    pub error: Option<&'a Value>,
}

impl Store {
    pub async fn create_impact_assessment(
        &self,
        tx: &mut Tx<'_>,
        draft: &NewImpactAssessment,
    ) -> Result<ImpactAssessmentRow, StoreError> {
        let row = sqlx::query(
            "INSERT INTO impact.assessment_draft (id, project_id, input, status)
             VALUES ($1, $2, $3, 'PENDING')
             ON CONFLICT (id) DO UPDATE SET input = impact.assessment_draft.input
             RETURNING id, project_id, input, status, report, engine_version,
                       ontology_version, created_at, started_at, finished_at, error",
        )
        .bind(draft.id)
        .bind(draft.project_id)
        .bind(&draft.input)
        .fetch_one(&mut **tx)
        .await?;
        Ok(impact_assessment_row(&row))
    }

    pub async fn impact_assessment(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        assessment_id: Uuid,
    ) -> Result<Option<ImpactAssessmentRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, project_id, input, status, report, engine_version,
                    ontology_version, created_at, started_at, finished_at, error
               FROM impact.assessment_draft
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(assessment_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(impact_assessment_row))
    }

    pub async fn list_impact_assessments(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ImpactAssessmentRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, project_id, input, status, report, engine_version,
                    ontology_version, created_at, started_at, finished_at, error
               FROM impact.assessment_draft
              WHERE project_id = $1
              ORDER BY created_at DESC, id DESC
              LIMIT $2",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(impact_assessment_row).collect())
    }

    pub async fn start_impact_assessment(
        &self,
        tx: &mut Tx<'_>,
        assessment_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE impact.assessment_draft
                SET status = 'RUNNING', started_at = COALESCE(started_at, now())
              WHERE id = $1 AND status IN ('PENDING', 'RUNNING')",
        )
        .bind(assessment_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn finish_impact_assessment(
        &self,
        tx: &mut Tx<'_>,
        assessment_id: Uuid,
        completion: &ImpactCompletion<'_>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE impact.assessment_draft
                SET status = $2,
                    report = $3,
                    engine_version = $4,
                    ontology_version = $5,
                    error = $6,
                    finished_at = now()
              WHERE id = $1",
        )
        .bind(assessment_id)
        .bind(completion.status)
        .bind(completion.report)
        .bind(completion.engine_version)
        .bind(completion.ontology_version)
        .bind(completion.error)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn changed_code_symbols(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        revision_id: Uuid,
        paths: &[String],
    ) -> Result<Vec<ChangedCodeSymbolRow>, StoreError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT s.id, s.file_id, f.path, s.kind, s.fqn, s.simple_name,
                    s.line_start, s.line_end
               FROM code.symbol s
               JOIN code.source_file f ON f.id = s.file_id
              WHERE s.project_id = $1 AND s.revision_id = $2 AND f.path = ANY($3)
              ORDER BY f.path, s.line_start, s.kind, s.fqn",
        )
        .bind(project_id)
        .bind(revision_id)
        .bind(paths)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| ChangedCodeSymbolRow {
                id: row.get("id"),
                file_id: row.get("file_id"),
                path: row.get("path"),
                kind: row.get("kind"),
                fqn: row.get("fqn"),
                simple_name: row.get("simple_name"),
                line_start: row.get("line_start"),
                line_end: row.get("line_end"),
            })
            .collect())
    }

    pub async fn impact_evidence_for_entity(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        natural_key: &str,
        revision_id: Option<Uuid>,
    ) -> Result<Option<ImpactEvidenceRow>, StoreError> {
        let row = sqlx::query(
            "SELECT ev.id AS evidence_id, ev.quote, ev.locator, ev.resolution,
                    ev.confidence, ev.source_type, f.relation
               FROM code.fact f
               JOIN code.fact_evidence fe
                 ON fe.fact_id = f.id AND fe.project_id = f.project_id
               JOIN corpus.evidence ev ON ev.id = fe.evidence_id
              WHERE f.project_id = $1 AND f.valid_to IS NULL
                AND ($2::uuid IS NULL OR f.revision_id = $2)
                AND (f.subject_key = $3 OR f.object_key = $3)
              ORDER BY
                CASE f.resolution
                  WHEN 'RESOLVED' THEN 0
                  WHEN 'PARTIAL' THEN 1
                  WHEN 'INFERRED' THEN 2
                  WHEN 'CONFLICT' THEN 3
                  ELSE 4
                END,
                f.created_at DESC,
                ev.id
              LIMIT 1",
        )
        .bind(project_id)
        .bind(revision_id)
        .bind(natural_key)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(|row| ImpactEvidenceRow {
            evidence_id: row.get("evidence_id"),
            quote: row.get("quote"),
            locator: row.get("locator"),
            resolution: row.get("resolution"),
            confidence: row.get("confidence"),
            source_type: row.get("source_type"),
            relation: row.get("relation"),
        }))
    }

    pub async fn impact_evidence_for_document_entity(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        document_id: Uuid,
        name: &str,
    ) -> Result<Option<ImpactEvidenceRow>, StoreError> {
        let row = sqlx::query(
            "SELECT ev.id AS evidence_id, c.text AS quote, c.locator,
                    COALESCE(ev.resolution, 'INFERRED') AS resolution,
                    COALESCE(ev.confidence, 0.6) AS confidence,
                    COALESCE(ev.source_type, 'DOCUMENT_CHUNK') AS source_type,
                    NULL::text AS relation
               FROM corpus.chunk c
               LEFT JOIN LATERAL (
                 SELECT id, resolution, confidence, source_type
                   FROM corpus.evidence
                  WHERE project_id = $1 AND chunk_id = c.id
                  ORDER BY created_at, id
                  LIMIT 1
               ) ev ON true
              WHERE c.project_id = $1 AND c.document_id = $2 AND c.is_current = true
                AND (
                  length($3) < 3
                  OR position(lower($3) in lower(c.text)) > 0
                )
              ORDER BY c.ordinal, c.id
              LIMIT 1",
        )
        .bind(project_id)
        .bind(document_id)
        .bind(name)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(|row| ImpactEvidenceRow {
            evidence_id: row.get("evidence_id"),
            quote: row.get("quote"),
            locator: row.get("locator"),
            resolution: row.get("resolution"),
            confidence: row.get("confidence"),
            source_type: row.get("source_type"),
            relation: row.get("relation"),
        }))
    }

    pub async fn impact_document_mentions(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        terms: &[String],
        limit: i64,
    ) -> Result<Vec<ImpactDocumentMentionRow>, StoreError> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT d.id AS document_id, d.title AS document_title, d.doc_type,
                    c.id AS chunk_id, c.text AS quote, c.locator,
                    ev.id AS evidence_id
               FROM corpus.chunk c
               JOIN corpus.document d ON d.id = c.document_id
               LEFT JOIN LATERAL (
                 SELECT id
                   FROM corpus.evidence
                  WHERE project_id = $1 AND chunk_id = c.id
                  ORDER BY created_at, id
                  LIMIT 1
               ) ev ON true
              WHERE c.project_id = $1 AND c.is_current = true
                AND d.deleted_at IS NULL
                AND EXISTS (
                  SELECT 1 FROM unnest($2::text[]) term
                   WHERE length(term) >= 3
                     AND position(lower(term) in lower(c.text)) > 0
                )
              ORDER BY d.id, c.ordinal, c.id
              LIMIT $3",
        )
        .bind(project_id)
        .bind(terms)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| ImpactDocumentMentionRow {
                document_id: row.get("document_id"),
                document_title: row.get("document_title"),
                doc_type: row.get("doc_type"),
                chunk_id: row.get("chunk_id"),
                quote: row.get("quote"),
                locator: row.get("locator"),
                evidence_id: row.get("evidence_id"),
            })
            .collect())
    }
}

fn impact_assessment_row(row: &sqlx::postgres::PgRow) -> ImpactAssessmentRow {
    ImpactAssessmentRow {
        id: row.get("id"),
        project_id: row.get("project_id"),
        input: row.get("input"),
        status: row.get("status"),
        report: row.get("report"),
        engine_version: row.get("engine_version"),
        ontology_version: row.get("ontology_version"),
        created_at: row.get("created_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        error: row.get("error"),
    }
}
