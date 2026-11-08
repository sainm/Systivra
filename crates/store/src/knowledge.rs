use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{QueryBuilder, Row};
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

pub const EMBEDDING_DIM: usize = 1024;
pub const PARTITION_COUNT: i32 = 32;

#[derive(Debug, Clone)]
pub struct NodeDraft {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub node_type: String,
    pub locator: Value,
    pub ordinal: i32,
    pub text: Option<String>,
    pub meta: Value,
}

#[derive(Debug, Clone)]
pub struct ChunkDraft {
    pub id: Uuid,
    pub project_id: Uuid,
    pub document_id: Uuid,
    pub revision_id: Uuid,
    pub chunk_type: String,
    pub ordinal: i32,
    pub text: String,
    pub tokenized: String,
    pub token_count: i32,
    pub char_count: i32,
    pub content_hash: Vec<u8>,
    pub locator: Value,
    pub node_path: Vec<String>,
    pub metadata: Value,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct EvidenceDraft {
    pub id: Uuid,
    pub project_id: Uuid,
    pub source_type: String,
    pub document_id: Option<Uuid>,
    pub revision_id: Option<Uuid>,
    pub chunk_id: Option<Uuid>,
    pub locator: Value,
    pub quote: String,
    pub quote_hash: Vec<u8>,
    pub resolution: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub doc_types: Vec<String>,
    pub business_domain: Option<String>,
    pub revision_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub document_title: String,
    pub doc_type: String,
    pub revision_id: Uuid,
    pub source_revision: Option<String>,
    pub locator: Value,
    pub quote: String,
    pub score: f64,
    pub rerank_score: Option<f64>,
    pub sources: Vec<String>,
    pub evidence_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct IndexBuildRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub model_id: Uuid,
    pub model_name: String,
    pub revision: String,
    pub status: String,
    pub chunk_count: i64,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct TreeNodeRow {
    pub id: Uuid,
    pub node_type: String,
    pub locator: Value,
    pub ordinal: i32,
    pub text: Option<String>,
    pub child_count: i64,
}

#[derive(Debug, Clone)]
pub struct ChunkListRow {
    pub id: Uuid,
    pub chunk_type: String,
    pub ordinal: i32,
    pub text: String,
    pub locator: Value,
    pub token_count: i32,
}

#[derive(Debug, Clone)]
pub struct ChunkRow {
    pub id: Uuid,
    pub chunk_type: String,
    pub text: String,
    pub locator: Value,
    pub token_count: i32,
    pub char_count: i32,
    pub ordinal: i32,
    pub revision_id: Uuid,
    pub node_path: Vec<String>,
    pub parser: String,
    pub parser_version: String,
}

#[derive(Debug, Clone)]
pub struct EvidenceRow {
    pub id: Uuid,
    pub source_type: String,
    pub document_id: Option<Uuid>,
    pub revision_id: Option<Uuid>,
    pub locator: Value,
    pub quote: String,
    pub quote_hash: Vec<u8>,
    pub resolution: String,
    pub confidence: f32,
    pub document_title: Option<String>,
    pub mime_type: Option<String>,
    pub page_count: Option<i64>,
}

pub struct RetrievalLogDraft {
    pub project_id: Uuid,
    pub query: String,
    pub query_tokens: Vec<String>,
    pub filters: Value,
    pub lexical_mode: String,
    pub candidates: Value,
    pub selected: Vec<Uuid>,
    pub rerank_used: bool,
    pub latency_ms: i32,
}

impl Store {
    pub async fn configure_vector_search(&self, tx: &mut Tx<'_>) -> Result<(), StoreError> {
        sqlx::query("SET LOCAL hnsw.ef_search = 80")
            .execute(&mut **tx)
            .await?;
        sqlx::query("SET LOCAL hnsw.iterative_scan = relaxed_order")
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn embedding_model_id(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Uuid, StoreError> {
        Ok(self.active_embedding_model(tx, project_id).await?.0)
    }

    pub async fn active_embedding_model(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<(Uuid, String, String), StoreError> {
        let row = sqlx::query(
            "SELECT m.id, m.name, m.revision
               FROM corpus.index_build b
               JOIN corpus.embedding_model m ON m.id = b.model_id
              WHERE b.project_id = $1 AND b.is_active AND b.status = 'READY'
              LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(row) = row {
            return Ok((row.get("id"), row.get("name"), row.get("revision")));
        }
        let row = sqlx::query(
            "SELECT id, name, revision FROM corpus.embedding_model
              WHERE name = 'local-hash-1024' AND revision = '1'",
        )
        .fetch_one(&mut **tx)
        .await?;
        Ok((row.get("id"), row.get("name"), row.get("revision")))
    }

    pub async fn embedding_model_by_name(
        &self,
        tx: &mut Tx<'_>,
        name: &str,
    ) -> Result<Option<(Uuid, String, String)>, StoreError> {
        let row = sqlx::query(
            "SELECT id, name, revision FROM corpus.embedding_model
              WHERE name = $1 AND enabled
              ORDER BY revision DESC LIMIT 1",
        )
        .bind(name)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|row| (row.get("id"), row.get("name"), row.get("revision"))))
    }

    pub async fn create_index_build(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        model_id: Uuid,
    ) -> Result<Uuid, StoreError> {
        let row = sqlx::query(
            "INSERT INTO corpus.index_build (project_id, model_id, status, started_at)
             VALUES ($1, $2, 'BUILDING', now())
             RETURNING id",
        )
        .bind(project_id)
        .bind(model_id)
        .fetch_one(&mut **tx)
        .await?;
        Ok(row.get("id"))
    }

    pub async fn update_index_build(
        &self,
        tx: &mut Tx<'_>,
        build_id: Uuid,
        status: &str,
        chunk_count: i64,
        error: Option<&Value>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE corpus.index_build
                SET status = $2, chunk_count = $3, error = $4,
                    finished_at = CASE WHEN $2 IN ('READY','FAILED') THEN now() ELSE finished_at END
              WHERE id = $1",
        )
        .bind(build_id)
        .bind(status)
        .bind(chunk_count)
        .bind(error)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn index_builds(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<IndexBuildRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT b.id, b.project_id, b.model_id, m.name AS model_name, m.revision,
                    b.status, b.chunk_count, b.started_at, b.finished_at, b.is_active
               FROM corpus.index_build b
               JOIN corpus.embedding_model m ON m.id = b.model_id
              WHERE b.project_id = $1
              ORDER BY b.started_at DESC NULLS LAST, b.id",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(index_build_row).collect())
    }

    pub async fn index_build(
        &self,
        tx: &mut Tx<'_>,
        build_id: Uuid,
    ) -> Result<Option<IndexBuildRow>, StoreError> {
        let row = sqlx::query(
            "SELECT b.id, b.project_id, b.model_id, m.name AS model_name, m.revision,
                    b.status, b.chunk_count, b.started_at, b.finished_at, b.is_active
               FROM corpus.index_build b
               JOIN corpus.embedding_model m ON m.id = b.model_id
              WHERE b.id = $1",
        )
        .bind(build_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(index_build_row))
    }

    pub async fn activate_index_build(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        build_id: Uuid,
    ) -> Result<bool, StoreError> {
        sqlx::query(
            "UPDATE corpus.index_build SET is_active = false
              WHERE project_id = $1 AND is_active",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
        let result = sqlx::query(
            "UPDATE corpus.index_build SET is_active = true
              WHERE id = $2 AND project_id = $1 AND status = 'READY'",
        )
        .bind(project_id)
        .bind(build_id)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn chunks_pending_embedding(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        model_id: Uuid,
        limit: i64,
    ) -> Result<Vec<(Uuid, String)>, StoreError> {
        let rows = sqlx::query(
            "SELECT c.id, c.text
               FROM corpus.chunk c
              WHERE c.project_id = $1 AND c.is_current
                AND NOT EXISTS (
                  SELECT 1 FROM corpus.embedding_1024 e
                   WHERE e.chunk_id = c.id AND e.model_id = $2
                )
              ORDER BY c.id
              LIMIT $3",
        )
        .bind(project_id)
        .bind(model_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| (row.get("id"), row.get("text")))
            .collect())
    }

    pub async fn upsert_embedding(
        &self,
        tx: &mut Tx<'_>,
        chunk_id: Uuid,
        project_id: Uuid,
        model_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), StoreError> {
        let vector = format_vector(embedding);
        sqlx::query(
            "INSERT INTO corpus.embedding_1024 (chunk_id, project_id, model_id, vec)
             VALUES ($1, $2, $3, $4::vector)
             ON CONFLICT (chunk_id, model_id)
             DO UPDATE SET vec = EXCLUDED.vec, created_at = now()",
        )
        .bind(chunk_id)
        .bind(project_id)
        .bind(model_id)
        .bind(&vector)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn embedding_count(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        model_id: Uuid,
    ) -> Result<i64, StoreError> {
        let row = sqlx::query(
            "SELECT count(*) AS count
               FROM corpus.embedding_1024 e
               JOIN corpus.chunk c ON c.id = e.chunk_id
              WHERE e.project_id = $1 AND e.model_id = $2 AND c.is_current",
        )
        .bind(project_id)
        .bind(model_id)
        .fetch_one(&mut **tx)
        .await?;
        Ok(row.get("count"))
    }

    pub async fn tree_nodes(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
        parent: Option<Uuid>,
        after: Option<(i32, Uuid)>,
        limit: i64,
    ) -> Result<Vec<TreeNodeRow>, StoreError> {
        let parent = match parent {
            Some(id) => Some(id),
            None => sqlx::query(
                "SELECT id FROM corpus.doc_node
                  WHERE revision_id = $1 AND node_type = 'DOCUMENT'
                  LIMIT 1",
            )
            .bind(revision_id)
            .fetch_optional(&mut **tx)
            .await?
            .map(|row| row.get::<Uuid, _>("id")),
        };
        let Some(parent) = parent else {
            return Ok(Vec::new());
        };
        let (after_ordinal, after_id) = after.unwrap_or((-1, Uuid::nil()));
        let rows = sqlx::query(
            "SELECT n.id, n.node_type, n.locator, n.ordinal, n.text,
                    (SELECT count(*) FROM corpus.doc_node c WHERE c.parent_id = n.id) AS child_count
               FROM corpus.doc_node n
              WHERE n.revision_id = $1 AND n.parent_id = $2
                AND (n.ordinal, n.id) > ($3, $4)
              ORDER BY n.ordinal, n.id
              LIMIT $5",
        )
        .bind(revision_id)
        .bind(parent)
        .bind(after_ordinal)
        .bind(after_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| TreeNodeRow {
                id: row.get("id"),
                node_type: row.get("node_type"),
                locator: row.get("locator"),
                ordinal: row.get("ordinal"),
                text: row.get("text"),
                child_count: row.get("child_count"),
            })
            .collect())
    }

    pub async fn document_chunks(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        document_id: Uuid,
        revision_id: Uuid,
        after_ordinal: Option<i32>,
        limit: i64,
    ) -> Result<Vec<ChunkListRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, chunk_type, ordinal, text, locator, token_count
               FROM corpus.chunk
              WHERE project_id = $1 AND document_id = $2 AND revision_id = $3
                AND is_current AND ($4::integer IS NULL OR ordinal > $4)
              ORDER BY ordinal
              LIMIT $5",
        )
        .bind(project_id)
        .bind(document_id)
        .bind(revision_id)
        .bind(after_ordinal)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| ChunkListRow {
                id: row.get("id"),
                chunk_type: row.get("chunk_type"),
                ordinal: row.get("ordinal"),
                text: row.get("text"),
                locator: row.get("locator"),
                token_count: row.get("token_count"),
            })
            .collect())
    }

    pub async fn purge_revision(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "DELETE FROM corpus.embedding_1024
              WHERE chunk_id IN (SELECT id FROM corpus.chunk WHERE revision_id = $1)",
        )
        .bind(revision_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM corpus.chunk WHERE revision_id = $1")
            .bind(revision_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM corpus.evidence WHERE revision_id = $1")
            .bind(revision_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM corpus.doc_node WHERE revision_id = $1")
            .bind(revision_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn insert_nodes(
        &self,
        tx: &mut Tx<'_>,
        nodes: &[NodeDraft],
    ) -> Result<(), StoreError> {
        for n in nodes {
            sqlx::query(
                "INSERT INTO corpus.doc_node
                   (id, revision_id, parent_id, node_type, locator, ordinal, text, meta)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (id) DO UPDATE
                   SET parent_id = EXCLUDED.parent_id, locator = EXCLUDED.locator,
                       ordinal = EXCLUDED.ordinal, text = EXCLUDED.text, meta = EXCLUDED.meta",
            )
            .bind(n.id)
            .bind(n.revision_id)
            .bind(n.parent_id)
            .bind(&n.node_type)
            .bind(&n.locator)
            .bind(n.ordinal)
            .bind(&n.text)
            .bind(&n.meta)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// 再 tokenize が必要な行だけを数える。
    /// 条件は `chunks_for_retokenize` と**必ず同じ**にする（remaining が減らない
    /// バグを防ぐ）。現行 tokenizer は日本語語を `tokenized` に残すので、
    /// 「本文に和文があるのに tokenized に和文が無い」行が古い。
    pub async fn count_stale_chunks(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<i64, StoreError> {
        let row = sqlx::query(
            "SELECT count(*) AS n FROM corpus.chunk
              WHERE project_id = $1 AND is_current
                AND text ~ '[ぁ-んァ-ヶ一-龥]'
                AND tokenized !~ '[ぁ-んァ-ヶ一-龥]'",
        )
        .bind(project_id)
        .fetch_one(&mut **tx)
        .await?;
        Ok(row.get("n"))
    }

    /// 再 tokenize 用。**古い行だけ**を最大 `limit` 件読む。
    /// 全件を毎回読むと、更新済みの先頭 5000 件がまた返ってきて
    /// `remaining` が減らず、呼び出し側が永久にループする。
    pub async fn chunks_for_retokenize(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<(Uuid, String, String)>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, text, tokenized FROM corpus.chunk
              WHERE project_id = $1 AND is_current
                AND text ~ '[ぁ-んァ-ヶ一-龥]'
                AND tokenized !~ '[ぁ-んァ-ヶ一-龥]'
              ORDER BY ordinal LIMIT $2",
        )
        .bind(project_id)
        .bind(limit.clamp(1, 10_000))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| (row.get("id"), row.get("text"), row.get("tokenized")))
            .collect())
    }

    pub async fn update_chunk_tokenized(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        chunk_id: Uuid,
        tokenized: &str,
    ) -> Result<bool, StoreError> {
        // `fts` は tokenized の生成列なので、これだけで索引も追随する。
        let result = sqlx::query(
            "UPDATE corpus.chunk SET tokenized = $3
              WHERE project_id = $1 AND id = $2 AND tokenized IS DISTINCT FROM $3",
        )
        .bind(project_id)
        .bind(chunk_id)
        .bind(tokenized)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn insert_chunks(
        &self,
        tx: &mut Tx<'_>,
        model_id: Uuid,
        chunks: &[ChunkDraft],
    ) -> Result<(), StoreError> {
        for c in chunks {
            sqlx::query(
                "INSERT INTO corpus.chunk
                   (id, project_id, document_id, revision_id, chunk_type, ordinal, text,
                    tokenized, token_count, char_count, content_hash, locator, node_path,
                    metadata, is_current)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, true)
                 ON CONFLICT (id, project_id) DO UPDATE
                   SET text = EXCLUDED.text, tokenized = EXCLUDED.tokenized,
                       token_count = EXCLUDED.token_count, char_count = EXCLUDED.char_count,
                       content_hash = EXCLUDED.content_hash, locator = EXCLUDED.locator,
                       node_path = EXCLUDED.node_path, metadata = EXCLUDED.metadata,
                       is_current = true, superseded_at = NULL",
            )
            .bind(c.id)
            .bind(c.project_id)
            .bind(c.document_id)
            .bind(c.revision_id)
            .bind(&c.chunk_type)
            .bind(c.ordinal)
            .bind(&c.text)
            .bind(&c.tokenized)
            .bind(c.token_count)
            .bind(c.char_count)
            .bind(&c.content_hash)
            .bind(&c.locator)
            .bind(&c.node_path)
            .bind(&c.metadata)
            .execute(&mut **tx)
            .await?;

            let vector = format_vector(&c.embedding);
            sqlx::query(
                "INSERT INTO corpus.embedding_1024 (chunk_id, project_id, model_id, vec)
                 VALUES ($1, $2, $3, $4::vector)
                 ON CONFLICT (chunk_id, model_id) DO UPDATE SET vec = EXCLUDED.vec",
            )
            .bind(c.id)
            .bind(c.project_id)
            .bind(model_id)
            .bind(vector)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    pub async fn insert_evidence(
        &self,
        tx: &mut Tx<'_>,
        evidence: &EvidenceDraft,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO corpus.evidence
               (id, project_id, source_type, document_id, revision_id, chunk_id,
                locator, quote, quote_hash, resolution, confidence)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (id) DO UPDATE
               SET locator = EXCLUDED.locator, quote = EXCLUDED.quote,
                   quote_hash = EXCLUDED.quote_hash, resolution = EXCLUDED.resolution,
                   confidence = EXCLUDED.confidence",
        )
        .bind(evidence.id)
        .bind(evidence.project_id)
        .bind(&evidence.source_type)
        .bind(evidence.document_id)
        .bind(evidence.revision_id)
        .bind(evidence.chunk_id)
        .bind(&evidence.locator)
        .bind(&evidence.quote)
        .bind(&evidence.quote_hash)
        .bind(&evidence.resolution)
        .bind(evidence.confidence)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn update_parse_run(
        &self,
        tx: &mut Tx<'_>,
        parse_run_id: Uuid,
        status: &str,
        stats: &Value,
        error: Option<&Value>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE corpus.parse_run
                SET status = $2,
                    stats = $3,
                    error = $4,
                    started_at = COALESCE(started_at, now()),
                    finished_at = CASE WHEN $2 IN ('SUCCEEDED','FAILED','PARTIAL')
                                       THEN now() ELSE finished_at END
              WHERE id = $1",
        )
        .bind(parse_run_id)
        .bind(status)
        .bind(stats)
        .bind(error)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn mark_revision_parsed(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE corpus.document_revision SET parsed_at = now() WHERE id = $1")
            .bind(revision_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn search_lexical(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        tsquery: &str,
        filter: &SearchFilter,
        limit: i64,
    ) -> Result<Vec<SearchHit>, StoreError> {
        let mut q = QueryBuilder::new(
            "SELECT c.id AS chunk_id, c.document_id, d.title AS document_title, d.doc_type,
                    c.revision_id, r.source_revision, c.locator, c.text,
                    ts_rank_cd(c.fts, to_tsquery('simple', ",
        );
        q.push_bind(tsquery);
        q.push(
            "))::float8 AS score,
                 (SELECT ev.id FROM corpus.evidence ev WHERE ev.chunk_id = c.id
                   ORDER BY ev.created_at DESC LIMIT 1) AS evidence_id
            FROM corpus.chunk c
            JOIN corpus.document d ON d.id = c.document_id
            JOIN corpus.document_revision r ON r.id = c.revision_id
            WHERE c.project_id = ",
        );
        q.push_bind(project_id);
        if filter.revision_id.is_none() {
            q.push(" AND c.is_current");
        }
        q.push(" AND c.fts @@ to_tsquery('simple', ");
        q.push_bind(tsquery);
        q.push(")");
        push_filter(&mut q, filter);
        q.push(" ORDER BY score DESC, c.ordinal LIMIT ");
        q.push_bind(limit);

        let rows = q.build().fetch_all(&mut **tx).await?;
        Ok(rows.iter().map(|r| search_hit(r, "fulltext")).collect())
    }

    pub async fn search_vector(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        model_id: Uuid,
        embedding: &[f32],
        filter: &SearchFilter,
        limit: i64,
    ) -> Result<Vec<SearchHit>, StoreError> {
        let vector = format_vector(embedding);
        let mut q = QueryBuilder::new("SELECT * FROM (");
        for partition in 0..PARTITION_COUNT {
            if partition > 0 {
                q.push(" UNION ALL ");
            }
            q.push(
                "(SELECT c.id AS chunk_id, c.document_id, d.title AS document_title,
                          d.doc_type, c.revision_id, r.source_revision, c.locator, c.text,
                          (1 - (e.vec <=> ",
            );
            q.push_bind(&vector);
            q.push(
                "::vector))::float8 AS score,
                          (SELECT ev.id FROM corpus.evidence ev WHERE ev.chunk_id = c.id
                            ORDER BY ev.created_at DESC LIMIT 1) AS evidence_id
                     FROM corpus.embedding_1024_p",
            );
            q.push(format!("{partition:02}"));
            q.push(
                " e
                     JOIN corpus.chunk c ON c.id = e.chunk_id AND c.project_id = e.project_id
                     JOIN corpus.document d ON d.id = c.document_id
                     JOIN corpus.document_revision r ON r.id = c.revision_id
                    WHERE e.project_id = ",
            );
            q.push_bind(project_id);
            q.push(" AND e.model_id = ");
            q.push_bind(model_id);
            if filter.revision_id.is_none() {
                q.push(" AND c.is_current");
            }
            push_filter(&mut q, filter);
            q.push(" ORDER BY e.vec <=> ");
            q.push_bind(&vector);
            q.push("::vector LIMIT ");
            q.push_bind(limit);
            q.push(")");
        }
        q.push(") hit ORDER BY hit.score DESC LIMIT ");
        q.push_bind(limit);

        let rows = q.build().fetch_all(&mut **tx).await?;
        Ok(rows.iter().map(|r| search_hit(r, "vector")).collect())
    }

    pub async fn insert_retrieval_log(
        &self,
        tx: &mut Tx<'_>,
        log: &RetrievalLogDraft,
    ) -> Result<Uuid, StoreError> {
        let row = sqlx::query(
            "INSERT INTO corpus.retrieval_log
               (project_id, query, query_tokens, filters, lexical_mode, candidates,
                selected, rerank_used, latency_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING id",
        )
        .bind(log.project_id)
        .bind(&log.query)
        .bind(&log.query_tokens)
        .bind(&log.filters)
        .bind(&log.lexical_mode)
        .bind(&log.candidates)
        .bind(&log.selected)
        .bind(log.rerank_used)
        .bind(log.latency_ms)
        .fetch_one(&mut **tx)
        .await?;
        Ok(row.get("id"))
    }

    pub async fn find_chunk(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        chunk_id: Uuid,
    ) -> Result<Option<ChunkRow>, StoreError> {
        let row = sqlx::query(
            "SELECT c.id, c.chunk_type, c.text, c.locator, c.token_count, c.char_count,
                    c.ordinal, c.revision_id, c.node_path,
                    COALESCE(r.parser, 'in-process-pdf') AS parser,
                    COALESCE(r.parser_version, '0.1.0') AS parser_version
               FROM corpus.chunk c
               JOIN corpus.document_revision r ON r.id = c.revision_id
              WHERE c.id = $1 AND c.project_id = $2 AND c.is_current",
        )
        .bind(chunk_id)
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|r| ChunkRow {
            id: r.get("id"),
            chunk_type: r.get("chunk_type"),
            text: r.get("text"),
            locator: r.get("locator"),
            token_count: r.get("token_count"),
            char_count: r.get("char_count"),
            ordinal: r.get("ordinal"),
            revision_id: r.get("revision_id"),
            node_path: r.get("node_path"),
            parser: r.get("parser"),
            parser_version: r.get("parser_version"),
        }))
    }

    pub async fn neighbor_chunks(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
        ordinal: i32,
    ) -> Result<(Vec<Uuid>, Vec<Uuid>), StoreError> {
        let rows = sqlx::query(
            "SELECT id, ordinal FROM corpus.chunk
              WHERE revision_id = $1 AND is_current AND ordinal IN ($2, $3)
              ORDER BY ordinal",
        )
        .bind(revision_id)
        .bind(ordinal - 1)
        .bind(ordinal + 1)
        .fetch_all(&mut **tx)
        .await?;
        let mut prev = Vec::new();
        let mut next = Vec::new();
        for r in rows {
            let o: i32 = r.get("ordinal");
            if o < ordinal {
                prev.push(r.get("id"));
            } else {
                next.push(r.get("id"));
            }
        }
        Ok((prev, next))
    }

    pub async fn find_evidence(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        evidence_id: Uuid,
    ) -> Result<Option<EvidenceRow>, StoreError> {
        let row = sqlx::query(
            "SELECT ev.id, ev.source_type, ev.document_id, ev.revision_id, ev.locator,
                    ev.quote, ev.quote_hash, ev.resolution, ev.confidence,
                    d.title AS document_title, r.mime_type,
                    (SELECT (pr.stats->>'page_count')::bigint
                       FROM corpus.parse_run pr
                      WHERE pr.revision_id = ev.revision_id
                      ORDER BY pr.started_at DESC NULLS LAST LIMIT 1) AS page_count
               FROM corpus.evidence ev
               LEFT JOIN corpus.document d ON d.id = ev.document_id
               LEFT JOIN corpus.document_revision r ON r.id = ev.revision_id
              WHERE ev.id = $1 AND ev.project_id = $2",
        )
        .bind(evidence_id)
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|r| EvidenceRow {
            id: r.get("id"),
            source_type: r.get("source_type"),
            document_id: r.get("document_id"),
            revision_id: r.get("revision_id"),
            locator: r.get("locator"),
            quote: r.get("quote"),
            quote_hash: r.get("quote_hash"),
            resolution: r.get("resolution"),
            confidence: r.get("confidence"),
            document_title: r.get("document_title"),
            mime_type: r.get("mime_type"),
            page_count: r.get("page_count"),
        }))
    }

    pub async fn page_text(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
        page: i32,
    ) -> Result<Option<String>, StoreError> {
        let row = sqlx::query(
            "SELECT text FROM corpus.doc_node
              WHERE revision_id = $1 AND node_type = 'SECTION'
                AND (locator->>'page')::int = $2
              LIMIT 1",
        )
        .bind(revision_id)
        .bind(page)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.and_then(|r| r.get::<Option<String>, _>("text")))
    }

    pub async fn project_of(
        &self,
        tx: &mut Tx<'_>,
        kind: &str,
        id: Uuid,
    ) -> Result<Option<Uuid>, StoreError> {
        let project = sqlx::query_scalar::<_, Option<Uuid>>("SELECT core.project_of($1, $2)")
            .bind(kind)
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
        Ok(project)
    }

    pub async fn evidence_count_for_page(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
    ) -> Result<i64, StoreError> {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM corpus.evidence WHERE revision_id = $1")
                .bind(revision_id)
                .fetch_one(&mut **tx)
                .await?;
        Ok(count)
    }

    pub async fn inserted_chunk_count(
        &self,
        tx: &mut Tx<'_>,
        revision_id: Uuid,
    ) -> Result<i64, StoreError> {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM corpus.chunk WHERE revision_id = $1")
                .bind(revision_id)
                .fetch_one(&mut **tx)
                .await?;
        Ok(count)
    }

    pub async fn last_audit_for_document(
        &self,
        tx: &mut Tx<'_>,
        document_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>, StoreError> {
        let ts = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            "SELECT max(occurred_at) FROM core.audit_log
              WHERE target_type = 'document' AND target_id = $1",
        )
        .bind(document_id.to_string())
        .fetch_one(&mut **tx)
        .await?;
        Ok(ts)
    }
}

fn push_filter(q: &mut QueryBuilder<'_, sqlx::Postgres>, filter: &SearchFilter) {
    if !filter.doc_types.is_empty() {
        q.push(" AND d.doc_type = ANY(");
        q.push_bind(filter.doc_types.clone());
        q.push(")");
    }
    if let Some(domain) = &filter.business_domain {
        q.push(" AND d.business_domain = ");
        q.push_bind(domain.clone());
    }
    if let Some(revision) = filter.revision_id {
        q.push(" AND c.revision_id = ");
        q.push_bind(revision);
    }
}

fn index_build_row(row: &PgRow) -> IndexBuildRow {
    IndexBuildRow {
        id: row.get("id"),
        project_id: row.get("project_id"),
        model_id: row.get("model_id"),
        model_name: row.get("model_name"),
        revision: row.get("revision"),
        status: row.get("status"),
        chunk_count: row.get("chunk_count"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        is_active: row.get("is_active"),
    }
}

fn search_hit(row: &PgRow, source: &str) -> SearchHit {
    SearchHit {
        chunk_id: row.get("chunk_id"),
        document_id: row.get("document_id"),
        document_title: row.get("document_title"),
        doc_type: row.get("doc_type"),
        revision_id: row.get("revision_id"),
        source_revision: row.get("source_revision"),
        locator: row.get("locator"),
        quote: row.get("text"),
        score: row.get("score"),
        rerank_score: None,
        sources: vec![source.to_owned()],
        evidence_id: row.get("evidence_id"),
    }
}

fn format_vector(values: &[f32]) -> String {
    let mut out = String::with_capacity(values.len() * 12 + 2);
    out.push('[');
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{v:.6}"));
    }
    out.push(']');
    out
}
