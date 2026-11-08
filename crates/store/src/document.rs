use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

#[derive(Debug, Clone)]
pub struct DocumentRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub doc_type: String,
    pub title: String,
    pub business_domain: Option<String>,
    pub acl_groups: Vec<String>,
    pub current_revision_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RevisionRow {
    pub id: Uuid,
    pub document_id: Uuid,
    pub source_revision: Option<String>,
    pub content_hash: Vec<u8>,
    pub object_key: String,
    pub file_size: i64,
    pub mime_type: String,
    pub status: String,
    pub parser: Option<String>,
    pub parser_version: Option<String>,
    pub acquired_at: DateTime<Utc>,
    pub parsed_at: Option<DateTime<Utc>>,
    pub page_count: i64,
    pub chunk_count: i64,
    pub parse_status: String,
}

#[derive(Debug, Clone)]
pub struct DocumentListRow {
    pub id: Uuid,
    pub title: String,
    pub doc_type: String,
    pub business_domain: Option<String>,
    pub current_revision_id: Option<Uuid>,
    pub page_count: i64,
    pub chunk_count: i64,
    pub parse_status: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewDocument {
    pub id: Uuid,
    pub project_id: Uuid,
    pub doc_type: String,
    pub title: String,
    pub business_domain: Option<String>,
    pub acl_groups: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NewRevision {
    pub id: Uuid,
    pub document_id: Uuid,
    pub source_revision: Option<String>,
    pub content_hash: Vec<u8>,
    pub object_key: String,
    pub file_size: i64,
    pub mime_type: String,
    pub acquired_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct DocumentFilter {
    pub doc_type: Option<String>,
    pub business_domain: Option<String>,
    pub parse_status: Option<String>,
    pub query: Option<String>,
    pub cursor_created_at: Option<DateTime<Utc>>,
    pub cursor_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct RevisionStats {
    pub page_count: i64,
    pub chunk_count: i64,
    pub parse_status: String,
}

impl Store {
    pub async fn insert_document(
        &self,
        tx: &mut Tx<'_>,
        doc: &NewDocument,
    ) -> Result<DocumentRow, StoreError> {
        let row = sqlx::query(
            "INSERT INTO corpus.document (id, project_id, doc_type, title, business_domain, acl_groups)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id, project_id, doc_type, title, business_domain, acl_groups,
                       current_revision_id, created_at",
        )
        .bind(doc.id)
        .bind(doc.project_id)
        .bind(&doc.doc_type)
        .bind(&doc.title)
        .bind(&doc.business_domain)
        .bind(&doc.acl_groups)
        .fetch_one(&mut **tx)
        .await?;
        Ok(document_row(&row))
    }

    pub async fn insert_revision(
        &self,
        tx: &mut Tx<'_>,
        rev: &NewRevision,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO corpus.document_revision
               (id, document_id, source_revision, content_hash, object_key, file_size,
                mime_type, status, acquired_at, parser, parser_version)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'CURRENT', $8,
                CASE
                  WHEN $7 LIKE '%spreadsheetml%' THEN 'xlsx-native'
                  WHEN $7 LIKE '%wordprocessingml%' THEN 'docx-native'
                  WHEN $7 LIKE '%presentationml%' THEN 'pptx-native'
                  ELSE 'in-process-pdf'
                END,
                '0.1.0')",
        )
        .bind(rev.id)
        .bind(rev.document_id)
        .bind(&rev.source_revision)
        .bind(&rev.content_hash)
        .bind(&rev.object_key)
        .bind(rev.file_size)
        .bind(&rev.mime_type)
        .bind(rev.acquired_at)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn set_current_revision(
        &self,
        tx: &mut Tx<'_>,
        document_id: Uuid,
        revision_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE corpus.document SET current_revision_id = $2 WHERE id = $1")
            .bind(document_id)
            .bind(revision_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn find_document(
        &self,
        tx: &mut Tx<'_>,
        document_id: Uuid,
    ) -> Result<Option<DocumentRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, project_id, doc_type, title, business_domain, acl_groups,
                    current_revision_id, created_at
               FROM corpus.document
              WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(document_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(document_row))
    }

    pub async fn find_current_revision(
        &self,
        tx: &mut Tx<'_>,
        document_id: Uuid,
    ) -> Result<Option<RevisionRow>, StoreError> {
        let row = sqlx::query(
            "SELECT r.id, r.document_id, r.source_revision, r.content_hash, r.object_key,
                    r.file_size, r.mime_type, r.status, r.parser, r.parser_version,
                    r.acquired_at, r.parsed_at,
                    COALESCE((SELECT (pr.stats->>'page_count')::bigint
                                FROM corpus.parse_run pr
                               WHERE pr.revision_id = r.id
                               ORDER BY pr.started_at DESC NULLS LAST LIMIT 1), 0) AS page_count,
                    (SELECT count(*) FROM corpus.chunk c
                      WHERE c.revision_id = r.id AND c.is_current) AS chunk_count,
                    COALESCE((SELECT pr.status FROM corpus.parse_run pr
                               WHERE pr.revision_id = r.id
                               ORDER BY pr.started_at DESC NULLS LAST LIMIT 1), 'NOT_PARSED') AS parse_status
               FROM corpus.document_revision r
              WHERE r.document_id = $1 AND r.status = 'CURRENT'",
        )
        .bind(document_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(revision_row))
    }

    pub async fn find_revision(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
    ) -> Result<Option<RevisionRow>, StoreError> {
        let row = sqlx::query(
            "SELECT r.id, r.document_id, r.source_revision, r.content_hash, r.object_key,
                    r.file_size, r.mime_type, r.status, r.parser, r.parser_version,
                    r.acquired_at, r.parsed_at,
                    COALESCE((SELECT (pr.stats->>'page_count')::bigint
                                FROM corpus.parse_run pr
                               WHERE pr.revision_id = r.id
                               ORDER BY pr.started_at DESC NULLS LAST LIMIT 1), 0) AS page_count,
                    (SELECT count(*) FROM corpus.chunk c
                      WHERE c.revision_id = r.id AND c.is_current) AS chunk_count,
                    COALESCE((SELECT pr.status FROM corpus.parse_run pr
                               WHERE pr.revision_id = r.id
                               ORDER BY pr.started_at DESC NULLS LAST LIMIT 1), 'NOT_PARSED') AS parse_status
               FROM corpus.document_revision r
              WHERE r.id = $1",
        )
        .bind(revision_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(revision_row))
    }

    pub async fn find_revision_by_hash(
        &self,
        tx: &mut Tx<'_>,
        content_hash: &[u8],
    ) -> Result<Option<(Uuid, Uuid)>, StoreError> {
        let row = sqlx::query(
            "SELECT d.id AS document_id, r.id AS revision_id
               FROM corpus.document_revision r
               JOIN corpus.document d ON d.id = r.document_id
              WHERE r.content_hash = $1 AND d.deleted_at IS NULL
              ORDER BY r.acquired_at DESC
              LIMIT 1",
        )
        .bind(content_hash)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|r| (r.get("document_id"), r.get("revision_id"))))
    }

    pub async fn revision_by_object_key(
        &self,
        tx: &mut Tx<'_>,
        object_key: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        let row = sqlx::query(
            "SELECT r.id
               FROM corpus.document_revision r
               JOIN corpus.document d ON d.id = r.document_id
              WHERE r.object_key = $1 AND d.deleted_at IS NULL
              LIMIT 1",
        )
        .bind(object_key)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|r| r.get("id")))
    }

    pub async fn list_documents(
        &self,
        tx: &mut Tx<'_>,
        filter: &DocumentFilter,
        limit: i64,
    ) -> Result<Vec<DocumentListRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT d.id, d.title, d.doc_type, d.business_domain, d.current_revision_id,
                    COALESCE(latest.page_count, 0) AS page_count,
                    COALESCE(counts.chunk_count, 0) AS chunk_count,
                    COALESCE(latest.status, 'NOT_PARSED') AS parse_status,
                    d.created_at AS updated_at
               FROM corpus.document d
               LEFT JOIN LATERAL (
                 SELECT (pr.stats->>'page_count')::bigint AS page_count, pr.status
                   FROM corpus.parse_run pr
                  WHERE pr.revision_id = d.current_revision_id
                  ORDER BY pr.started_at DESC NULLS LAST
                  LIMIT 1
               ) latest ON true
               LEFT JOIN LATERAL (
                 SELECT count(*) AS chunk_count
                   FROM corpus.chunk c
                  WHERE c.document_id = d.id AND c.is_current
               ) counts ON true
              WHERE d.deleted_at IS NULL
                AND ($1::text IS NULL OR d.doc_type = $1)
                AND ($2::text IS NULL OR d.business_domain = $2)
                AND ($3::text IS NULL OR COALESCE(latest.status, 'NOT_PARSED') = $3)
                AND ($4::text IS NULL OR d.title ILIKE '%' || $4 || '%')
                AND ($5::timestamptz IS NULL OR (d.created_at, d.id) < ($5, $6))
              ORDER BY d.created_at DESC, d.id DESC
              LIMIT $7",
        )
        .bind(&filter.doc_type)
        .bind(&filter.business_domain)
        .bind(&filter.parse_status)
        .bind(&filter.query)
        .bind(filter.cursor_created_at)
        .bind(filter.cursor_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| DocumentListRow {
                id: r.get("id"),
                title: r.get("title"),
                doc_type: r.get("doc_type"),
                business_domain: r.get("business_domain"),
                current_revision_id: r.get("current_revision_id"),
                page_count: r.get("page_count"),
                chunk_count: r.get("chunk_count"),
                parse_status: r.get("parse_status"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }

    pub async fn logical_delete(
        &self,
        tx: &mut Tx<'_>,
        document_id: Uuid,
    ) -> Result<bool, StoreError> {
        let res = sqlx::query(
            "UPDATE corpus.document
                SET deleted_at = now()
              WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(document_id)
        .execute(&mut **tx)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query(
            "UPDATE corpus.chunk
                SET is_current = false, superseded_at = now()
              WHERE document_id = $1 AND is_current",
        )
        .bind(document_id)
        .execute(&mut **tx)
        .await?;
        Ok(true)
    }

    pub async fn insert_parse_run(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
        parser: &str,
        parser_version: &str,
    ) -> Result<Uuid, StoreError> {
        let row = sqlx::query(
            "INSERT INTO corpus.parse_run (revision_id, parser, parser_version, status)
             VALUES ($1, $2, $3, 'PENDING')
             RETURNING id",
        )
        .bind(revision_id)
        .bind(parser)
        .bind(parser_version)
        .fetch_one(&mut **tx)
        .await?;
        Ok(row.get("id"))
    }

    pub async fn latest_parse_run(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
    ) -> Result<Option<Uuid>, StoreError> {
        let row = sqlx::query(
            "SELECT id FROM corpus.parse_run
              WHERE revision_id = $1
              ORDER BY started_at DESC NULLS LAST
              LIMIT 1",
        )
        .bind(revision_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|r| r.get("id")))
    }

    pub async fn audit_refs(
        &self,
        tx: &mut Tx<'_>,
        document_id: Uuid,
    ) -> Result<Option<(String, Option<Uuid>)>, StoreError> {
        let row =
            sqlx::query("SELECT title, current_revision_id FROM corpus.document WHERE id = $1")
                .bind(document_id)
                .fetch_optional(&mut **tx)
                .await?;
        Ok(row.map(|r| (r.get("title"), r.get("current_revision_id"))))
    }
}

fn document_row(row: &sqlx::postgres::PgRow) -> DocumentRow {
    DocumentRow {
        id: row.get("id"),
        project_id: row.get("project_id"),
        doc_type: row.get("doc_type"),
        title: row.get("title"),
        business_domain: row.get("business_domain"),
        acl_groups: row.get("acl_groups"),
        current_revision_id: row.get("current_revision_id"),
        created_at: row.get("created_at"),
    }
}

fn revision_row(row: &sqlx::postgres::PgRow) -> RevisionRow {
    RevisionRow {
        id: row.get("id"),
        document_id: row.get("document_id"),
        source_revision: row.get("source_revision"),
        content_hash: row.get("content_hash"),
        object_key: row.get("object_key"),
        file_size: row.get("file_size"),
        mime_type: row.get("mime_type"),
        status: row.get("status"),
        parser: row.get("parser"),
        parser_version: row.get("parser_version"),
        acquired_at: row.get("acquired_at"),
        parsed_at: row.get("parsed_at"),
        page_count: row.get("page_count"),
        chunk_count: row.get("chunk_count"),
        parse_status: row.get("parse_status"),
    }
}
