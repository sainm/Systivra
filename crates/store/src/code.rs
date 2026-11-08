use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

#[derive(Debug, Clone)]
pub struct CodeRepositoryRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub url: Option<String>,
    pub default_branch: Option<String>,
    pub settings: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewCodeRepository {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub url: String,
    pub default_branch: Option<String>,
    pub settings: Value,
}

#[derive(Debug, Clone)]
pub struct NewCodeRevision {
    pub id: Uuid,
    pub project_id: Uuid,
    pub repository_id: Uuid,
    pub revision: String,
    pub parent_id: Option<Uuid>,
    pub tree_digest: Vec<u8>,
    pub snapshot_key: String,
    pub committed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CodeRevisionRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub repository_id: Uuid,
    pub revision: String,
    pub parent_id: Option<Uuid>,
    pub tree_digest: Vec<u8>,
    pub snapshot_key: String,
    pub committed_at: Option<DateTime<Utc>>,
    pub is_baseline: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewSourceFile {
    pub id: Uuid,
    pub path: String,
    pub language: String,
    pub content_hash: Vec<u8>,
    pub size_bytes: i64,
    pub encoding: String,
    pub loc: i32,
    pub line_offsets: Vec<u8>,
    pub exact_offsets: bool,
}

#[derive(Debug, Clone)]
pub struct CodeSourceFileRow {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub path: String,
    pub language: String,
    pub content_hash: Vec<u8>,
    pub size_bytes: i64,
    pub encoding: String,
    pub loc: i32,
    pub line_offsets: Option<Vec<u8>>,
    pub exact_offsets: bool,
}

#[derive(Debug, Clone)]
pub struct NewCodeSymbol {
    pub id: Uuid,
    pub project_id: Uuid,
    pub revision_id: Uuid,
    pub file_id: Uuid,
    pub kind: String,
    pub fqn: String,
    pub simple_name: String,
    pub signature: Option<String>,
    pub line_start: i32,
    pub line_end: i32,
    pub byte_start: i64,
    pub byte_end: i64,
    pub modifiers: Vec<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct CodeSymbolRow {
    pub id: Uuid,
    pub file_id: Uuid,
    pub kind: String,
    pub fqn: String,
    pub simple_name: String,
    pub signature: Option<String>,
    pub line_start: i32,
    pub line_end: i32,
    pub byte_start: i64,
    pub byte_end: i64,
    pub modifiers: Vec<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct NewCodeAnalysisRun {
    pub id: Uuid,
    pub project_id: Uuid,
    pub revision_id: Uuid,
    pub engine_versions: Value,
}

#[derive(Debug, Clone)]
pub struct CodeAnalysisRunRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub revision_id: Uuid,
    pub status: String,
    pub engine_versions: Value,
    pub classpath_complete: bool,
    pub stats: Value,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub error: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct NewCodeEvidence {
    pub id: Uuid,
    pub quote: String,
    pub quote_hash: Vec<u8>,
    pub locator: Value,
    pub resolution: String,
    pub confidence: f32,
    pub role: String,
}

#[derive(Debug, Clone)]
pub struct NewCodeFact {
    pub id: Uuid,
    pub project_id: Uuid,
    pub revision_id: Uuid,
    pub subject_kind: String,
    pub subject_key: String,
    pub relation: String,
    pub object_kind: String,
    pub object_key: String,
    pub resolution: String,
    pub confidence: f32,
    pub analysis_engine: String,
    pub engine_version: String,
    pub conflict_group: Option<Uuid>,
    pub unresolved_reason: Option<String>,
    pub candidates: Value,
    pub fact_hash: Vec<u8>,
    pub evidence: Vec<NewCodeEvidence>,
}

#[derive(Debug, Clone)]
pub struct CodeEvidenceRow {
    pub id: Uuid,
    pub role: String,
    pub quote: String,
    pub locator: Value,
    pub resolution: String,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct CodeFactRow {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub subject_kind: String,
    pub subject_key: String,
    pub relation: String,
    pub object_kind: String,
    pub object_key: String,
    pub resolution: String,
    pub confidence: f32,
    pub analysis_engine: String,
    pub engine_version: String,
    pub unresolved_reason: Option<String>,
    pub candidates: Value,
    pub valid_from: DateTime<Utc>,
    pub evidence: Vec<CodeEvidenceRow>,
}

impl Store {
    pub async fn create_code_repository(
        &self,
        tx: &mut Tx<'_>,
        repository: &NewCodeRepository,
    ) -> Result<CodeRepositoryRow, StoreError> {
        let row = sqlx::query(
            "INSERT INTO code.repository
               (id, project_id, kind, url, default_branch, settings)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (project_id, kind, url) WHERE url IS NOT NULL
             DO UPDATE SET default_branch = EXCLUDED.default_branch,
                           settings = EXCLUDED.settings
             RETURNING id, project_id, kind, url, default_branch, settings, created_at",
        )
        .bind(repository.id)
        .bind(repository.project_id)
        .bind(&repository.kind)
        .bind(&repository.url)
        .bind(&repository.default_branch)
        .bind(&repository.settings)
        .fetch_one(&mut **tx)
        .await?;
        Ok(code_repository_row(&row))
    }

    pub async fn list_code_repositories(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<CodeRepositoryRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, project_id, kind, url, default_branch, settings, created_at
               FROM code.repository
              WHERE project_id = $1
              ORDER BY created_at DESC, id",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(code_repository_row).collect())
    }

    pub async fn code_repository(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        repository_id: Uuid,
    ) -> Result<Option<CodeRepositoryRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, project_id, kind, url, default_branch, settings, created_at
               FROM code.repository
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(repository_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(code_repository_row))
    }

    pub async fn create_code_revision(
        &self,
        tx: &mut Tx<'_>,
        revision: &NewCodeRevision,
        files: &[NewSourceFile],
    ) -> Result<CodeRevisionRow, StoreError> {
        let inserted = sqlx::query(
            "INSERT INTO code.code_revision
               (id, project_id, repository_id, revision, parent_id, tree_digest,
                snapshot_key, committed_at, is_baseline)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $5 IS NULL)
             ON CONFLICT (repository_id, revision) DO NOTHING
             RETURNING id, project_id, repository_id, revision, parent_id, tree_digest,
                       snapshot_key, committed_at, is_baseline, created_at",
        )
        .bind(revision.id)
        .bind(revision.project_id)
        .bind(revision.repository_id)
        .bind(&revision.revision)
        .bind(revision.parent_id)
        .bind(&revision.tree_digest)
        .bind(&revision.snapshot_key)
        .bind(revision.committed_at)
        .fetch_optional(&mut **tx)
        .await?;

        let row = if let Some(row) = inserted {
            let created = code_revision_row(&row);
            for file in files {
                sqlx::query(
                    "INSERT INTO code.source_file
                       (id, project_id, revision_id, path, language, content_hash,
                        size_bytes, encoding, loc, line_offsets, exact_offsets)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                )
                .bind(file.id)
                .bind(revision.project_id)
                .bind(created.id)
                .bind(&file.path)
                .bind(&file.language)
                .bind(&file.content_hash)
                .bind(file.size_bytes)
                .bind(&file.encoding)
                .bind(file.loc)
                .bind(&file.line_offsets)
                .bind(file.exact_offsets)
                .execute(&mut **tx)
                .await?;
            }
            created
        } else {
            let row = sqlx::query(
                "SELECT id, project_id, repository_id, revision, parent_id, tree_digest,
                        snapshot_key, committed_at, is_baseline, created_at
                   FROM code.code_revision
                  WHERE repository_id = $1 AND revision = $2",
            )
            .bind(revision.repository_id)
            .bind(&revision.revision)
            .fetch_one(&mut **tx)
            .await?;
            code_revision_row(&row)
        };
        Ok(row)
    }

    pub async fn code_revision(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<CodeRevisionRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, project_id, repository_id, revision, parent_id, tree_digest,
                    snapshot_key, committed_at, is_baseline, created_at
               FROM code.code_revision
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(revision_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(code_revision_row))
    }

    pub async fn latest_code_revision(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        repository_id: Uuid,
    ) -> Result<Option<CodeRevisionRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, project_id, repository_id, revision, parent_id, tree_digest,
                    snapshot_key, committed_at, is_baseline, created_at
               FROM code.code_revision
              WHERE project_id = $1 AND repository_id = $2
              ORDER BY created_at DESC, id DESC
              LIMIT 1",
        )
        .bind(project_id)
        .bind(repository_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(code_revision_row))
    }

    pub async fn list_code_revisions(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        repository_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<CodeRevisionRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, project_id, repository_id, revision, parent_id, tree_digest,
                    snapshot_key, committed_at, is_baseline, created_at
               FROM code.code_revision
              WHERE project_id = $1 AND ($2::uuid IS NULL OR repository_id = $2)
              ORDER BY created_at DESC, id DESC
              LIMIT $3",
        )
        .bind(project_id)
        .bind(repository_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(code_revision_row).collect())
    }

    pub async fn code_source_files(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Vec<CodeSourceFileRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, revision_id, path, language, content_hash, size_bytes, encoding,
                    loc, line_offsets, exact_offsets
               FROM code.source_file
              WHERE project_id = $1 AND revision_id = $2
              ORDER BY path",
        )
        .bind(project_id)
        .bind(revision_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(code_source_file_row).collect())
    }

    pub async fn code_source_file_count(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        revision_id: Uuid,
    ) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM code.source_file
              WHERE project_id = $1 AND revision_id = $2",
        )
        .bind(project_id)
        .bind(revision_id)
        .fetch_one(&mut **tx)
        .await?)
    }

    pub async fn code_source_file(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        file_id: Uuid,
    ) -> Result<Option<CodeSourceFileRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, revision_id, path, language, content_hash, size_bytes, encoding,
                    loc, line_offsets, exact_offsets
               FROM code.source_file
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(file_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(code_source_file_row))
    }

    pub async fn changed_code_paths(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        parent_id: Uuid,
        current_id: Uuid,
    ) -> Result<Vec<String>, StoreError> {
        let rows = sqlx::query(
            "SELECT cur.path
               FROM code.source_file cur
               LEFT JOIN code.source_file old
                 ON old.revision_id = $2 AND old.path = cur.path
              WHERE cur.project_id = $1 AND cur.revision_id = $3
                AND (old.id IS NULL OR old.content_hash <> cur.content_hash)
              UNION
             SELECT old.path
               FROM code.source_file old
               LEFT JOIN code.source_file cur
                 ON cur.revision_id = $3 AND cur.path = old.path
              WHERE old.project_id = $1 AND old.revision_id = $2
                AND cur.id IS NULL
              ORDER BY 1",
        )
        .bind(project_id)
        .bind(parent_id)
        .bind(current_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(|row| row.get("path")).collect())
    }

    pub async fn create_code_analysis_run(
        &self,
        tx: &mut Tx<'_>,
        run: &NewCodeAnalysisRun,
    ) -> Result<CodeAnalysisRunRow, StoreError> {
        let row = sqlx::query(
            "INSERT INTO code.analysis_run
               (id, project_id, revision_id, status, engine_versions)
             VALUES ($1, $2, $3, 'PENDING', $4)
             ON CONFLICT (id) DO UPDATE SET engine_versions = EXCLUDED.engine_versions
             RETURNING id, project_id, revision_id, status, engine_versions,
                       classpath_complete, stats, started_at, finished_at, created_at, error",
        )
        .bind(run.id)
        .bind(run.project_id)
        .bind(run.revision_id)
        .bind(&run.engine_versions)
        .fetch_one(&mut **tx)
        .await?;
        Ok(code_analysis_run_row(&row))
    }

    pub async fn code_analysis_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
    ) -> Result<Option<CodeAnalysisRunRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, project_id, revision_id, status, engine_versions,
                    classpath_complete, stats, started_at, finished_at, created_at, error
               FROM code.analysis_run
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(code_analysis_run_row))
    }

    pub async fn list_code_analysis_runs(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<CodeAnalysisRunRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, project_id, revision_id, status, engine_versions,
                    classpath_complete, stats, started_at, finished_at, created_at, error
               FROM code.analysis_run
              WHERE project_id = $1
              ORDER BY created_at DESC, id DESC
              LIMIT $2",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(code_analysis_run_row).collect())
    }

    pub async fn start_code_analysis(
        &self,
        tx: &mut Tx<'_>,
        run_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE code.analysis_run
                SET status = 'RUNNING', started_at = COALESCE(started_at, now())
              WHERE id = $1 AND status IN ('PENDING', 'RUNNING')",
        )
        .bind(run_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn finish_code_analysis(
        &self,
        tx: &mut Tx<'_>,
        run_id: Uuid,
        status: &str,
        classpath_complete: bool,
        stats: &Value,
        error: Option<&Value>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE code.analysis_run
                SET status = $2,
                    classpath_complete = $3,
                    stats = $4,
                    error = $5,
                    finished_at = now()
              WHERE id = $1",
        )
        .bind(run_id)
        .bind(status)
        .bind(classpath_complete)
        .bind(stats)
        .bind(error)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn insert_code_symbols(
        &self,
        tx: &mut Tx<'_>,
        symbols: &[NewCodeSymbol],
    ) -> Result<(), StoreError> {
        for symbol in symbols {
            sqlx::query(
                "INSERT INTO code.symbol
                   (id, project_id, revision_id, file_id, kind, fqn, simple_name,
                    signature, line_start, line_end, byte_start, byte_end, modifiers, metadata)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(symbol.id)
            .bind(symbol.project_id)
            .bind(symbol.revision_id)
            .bind(symbol.file_id)
            .bind(&symbol.kind)
            .bind(&symbol.fqn)
            .bind(&symbol.simple_name)
            .bind(&symbol.signature)
            .bind(symbol.line_start)
            .bind(symbol.line_end)
            .bind(symbol.byte_start)
            .bind(symbol.byte_end)
            .bind(&symbol.modifiers)
            .bind(&symbol.metadata)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    pub async fn list_code_symbols(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        revision_id: Option<Uuid>,
        fqn: &str,
        limit: i64,
    ) -> Result<Vec<CodeSymbolRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, file_id, kind, fqn, simple_name, signature, line_start, line_end,
                    byte_start, byte_end, modifiers, metadata
               FROM code.symbol
              WHERE project_id = $1
                AND ($2::uuid IS NULL OR revision_id = $2)
                AND ($3 = '' OR fqn ILIKE '%' || $3 || '%')
              ORDER BY revision_id DESC, fqn, line_start
              LIMIT $4",
        )
        .bind(project_id)
        .bind(revision_id)
        .bind(fqn)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| CodeSymbolRow {
                id: row.get("id"),
                file_id: row.get("file_id"),
                kind: row.get("kind"),
                fqn: row.get("fqn"),
                simple_name: row.get("simple_name"),
                signature: row.get("signature"),
                line_start: row.get("line_start"),
                line_end: row.get("line_end"),
                byte_start: row.get("byte_start"),
                byte_end: row.get("byte_end"),
                modifiers: row.get("modifiers"),
                metadata: row.get("metadata"),
            })
            .collect())
    }

    pub async fn code_symbol_count(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        revision_id: Uuid,
    ) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM code.symbol
              WHERE project_id = $1 AND revision_id = $2",
        )
        .bind(project_id)
        .bind(revision_id)
        .fetch_one(&mut **tx)
        .await?)
    }

    pub async fn insert_code_facts(
        &self,
        tx: &mut Tx<'_>,
        revision_label: &str,
        facts: &[NewCodeFact],
    ) -> Result<usize, StoreError> {
        if facts.is_empty() {
            return Ok(0);
        }
        sqlx::query("SET CONSTRAINTS ALL DEFERRED")
            .execute(&mut **tx)
            .await?;
        for batch in facts.chunks(5_000) {
            let mut fact_copy = String::new();
            let mut evidence_copy = String::new();
            for fact in batch {
                fact_copy.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                    fact.id,
                    fact.project_id,
                    fact.revision_id,
                    csv_text(&fact.subject_kind),
                    csv_text(&fact.subject_key),
                    csv_text(&fact.relation),
                    csv_text(&fact.object_kind),
                    csv_text(&fact.object_key),
                    csv_text(&fact.resolution),
                    fact.confidence,
                    csv_text(&fact.analysis_engine),
                    csv_text(&fact.engine_version),
                    csv_optional_uuid(fact.conflict_group),
                    csv_optional_text(fact.unresolved_reason.as_deref()),
                    csv_text(&fact.candidates.to_string()),
                    csv_bytea(&fact.fact_hash),
                ));
                for evidence in &fact.evidence {
                    evidence_copy.push_str(&format!(
                        "{},{},{},{},{},{},{},{},{}\n",
                        fact.project_id,
                        fact.id,
                        evidence.id,
                        csv_text(&evidence.role),
                        csv_text(&evidence.quote),
                        csv_bytea(&evidence.quote_hash),
                        csv_text(&evidence.locator.to_string()),
                        csv_text(&evidence.resolution),
                        evidence.confidence,
                    ));
                }
            }

            sqlx::query(
                "CREATE TEMP TABLE code_fact_stage (
                   id uuid, project_id uuid, revision_id uuid,
                   subject_kind text, subject_key text, relation text,
                   object_kind text, object_key text, resolution text,
                   confidence real, analysis_engine text, engine_version text,
                   conflict_group uuid, unresolved_reason text,
                   candidates jsonb, fact_hash bytea
                 ) ON COMMIT DROP",
            )
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "CREATE TEMP TABLE code_evidence_stage (
                   project_id uuid, fact_id uuid, evidence_id uuid, role text,
                   quote text, quote_hash bytea, locator jsonb,
                   resolution text, confidence real
                 ) ON COMMIT DROP",
            )
            .execute(&mut **tx)
            .await?;
            let mut copy = tx
                .copy_in_raw(
                    "COPY code_fact_stage
                       (id,project_id,revision_id,subject_kind,subject_key,relation,
                        object_kind,object_key,resolution,confidence,analysis_engine,
                        engine_version,conflict_group,unresolved_reason,candidates,fact_hash)
                     FROM STDIN WITH (FORMAT csv, NULL '\\N')",
                )
                .await?;
            copy.send(fact_copy.as_bytes()).await?;
            copy.finish().await?;
            let mut copy = tx
                .copy_in_raw(
                    "COPY code_evidence_stage
                       (project_id,fact_id,evidence_id,role,quote,quote_hash,locator,
                        resolution,confidence)
                     FROM STDIN WITH (FORMAT csv, NULL '\\N')",
                )
                .await?;
            copy.send(evidence_copy.as_bytes()).await?;
            copy.finish().await?;

            sqlx::query(
                "UPDATE code.fact AS old
                    SET valid_to = now(), superseded_by = staged.id
                   FROM code_fact_stage AS staged
                  WHERE old.project_id = staged.project_id
                    AND old.revision_id = staged.revision_id
                    AND old.subject_key = staged.subject_key
                    AND old.relation = staged.relation
                    AND old.object_key = staged.object_key
                    AND old.analysis_engine = staged.analysis_engine
                    AND old.valid_to IS NULL",
            )
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO code.fact
                   (id, project_id, revision_id, subject_kind, subject_key, relation,
                    object_kind, object_key, resolution, confidence, analysis_engine,
                    engine_version, conflict_group, unresolved_reason, candidates, fact_hash)
                 SELECT id, project_id, revision_id, subject_kind, subject_key, relation,
                        object_kind, object_key, resolution, confidence, analysis_engine,
                        engine_version, conflict_group, unresolved_reason, candidates, fact_hash
                   FROM code_fact_stage
                 ON CONFLICT (id) DO NOTHING",
            )
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO corpus.evidence
                   (id, project_id, source_type, locator, quote, quote_hash,
                    resolution, confidence)
                 SELECT evidence_id, project_id, 'CODE', locator, quote, quote_hash,
                        resolution, confidence
                   FROM code_evidence_stage
                 ON CONFLICT (id) DO NOTHING",
            )
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO code.fact_evidence
                   (project_id, fact_id, evidence_id, role)
                 SELECT project_id, fact_id, evidence_id, role
                   FROM code_evidence_stage
                 ON CONFLICT (fact_id, evidence_id) DO NOTHING",
            )
            .execute(&mut **tx)
            .await?;
            sqlx::query("DROP TABLE code_fact_stage, code_evidence_stage")
                .execute(&mut **tx)
                .await?;
            self.project_code_facts_to_graph(tx, revision_label, batch)
                .await?;
        }
        Ok(facts.len())
    }

    pub async fn list_code_facts(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        revision_id: Option<Uuid>,
        subject: &str,
        relation: &str,
        limit: i64,
    ) -> Result<Vec<CodeFactRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, revision_id, subject_kind, subject_key, relation, object_kind,
                    object_key, resolution, confidence, analysis_engine, engine_version,
                    unresolved_reason, candidates, valid_from
               FROM code.fact
              WHERE project_id = $1
                AND ($2::uuid IS NULL OR revision_id = $2)
                AND valid_to IS NULL
                AND ($3 = '' OR subject_key ILIKE '%' || $3 || '%'
                     OR object_key ILIKE '%' || $3 || '%')
                AND ($4 = '' OR relation = $4)
              ORDER BY valid_from DESC, id
              LIMIT $5",
        )
        .bind(project_id)
        .bind(revision_id)
        .bind(subject)
        .bind(relation)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        let mut facts = Vec::with_capacity(rows.len());
        for row in rows {
            let fact_id: Uuid = row.get("id");
            let evidence_rows = sqlx::query(
                "SELECT ev.id, fe.role, ev.quote, ev.locator, ev.resolution, ev.confidence
                   FROM code.fact_evidence fe
                   JOIN corpus.evidence ev ON ev.id = fe.evidence_id
                  WHERE fe.project_id = $1 AND fe.fact_id = $2
                  ORDER BY fe.role, ev.id",
            )
            .bind(project_id)
            .bind(fact_id)
            .fetch_all(&mut **tx)
            .await?;
            let evidence = evidence_rows
                .iter()
                .map(|evidence| CodeEvidenceRow {
                    id: evidence.get("id"),
                    role: evidence.get("role"),
                    quote: evidence.get("quote"),
                    locator: evidence.get("locator"),
                    resolution: evidence.get("resolution"),
                    confidence: evidence.get("confidence"),
                })
                .collect();
            facts.push(CodeFactRow {
                id: fact_id,
                revision_id: row.get("revision_id"),
                subject_kind: row.get("subject_kind"),
                subject_key: row.get("subject_key"),
                relation: row.get("relation"),
                object_kind: row.get("object_kind"),
                object_key: row.get("object_key"),
                resolution: row.get("resolution"),
                confidence: row.get("confidence"),
                analysis_engine: row.get("analysis_engine"),
                engine_version: row.get("engine_version"),
                unresolved_reason: row.get("unresolved_reason"),
                candidates: row.get("candidates"),
                valid_from: row.get("valid_from"),
                evidence,
            });
        }
        Ok(facts)
    }

    pub async fn code_fact_counts(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        revision_id: Uuid,
    ) -> Result<(i64, i64), StoreError> {
        let row = sqlx::query(
            "SELECT count(*) AS total,
                    count(*) FILTER (WHERE resolution <> 'RESOLVED') AS unresolved
               FROM code.fact
              WHERE project_id = $1 AND revision_id = $2 AND valid_to IS NULL",
        )
        .bind(project_id)
        .bind(revision_id)
        .fetch_one(&mut **tx)
        .await?;
        Ok((row.get("total"), row.get("unresolved")))
    }

    pub async fn copy_unchanged_code_analysis(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        parent_revision_id: Uuid,
        current_revision_id: Uuid,
    ) -> Result<(usize, usize), StoreError> {
        let symbols = sqlx::query(
            "INSERT INTO code.symbol
               (id, project_id, revision_id, file_id, kind, fqn, simple_name,
                signature, line_start, line_end, byte_start, byte_end, modifiers, metadata)
             SELECT gen_random_uuid(), cur.project_id, cur.revision_id, cur.id,
                    old.kind, old.fqn, old.simple_name, old.signature,
                    old.line_start, old.line_end, old.byte_start, old.byte_end,
                    old.modifiers, old.metadata
               FROM code.symbol old
               JOIN code.source_file prev
                 ON prev.id = old.file_id AND prev.revision_id = $2
               JOIN code.source_file cur
                 ON cur.revision_id = $3 AND cur.path = prev.path
                AND cur.content_hash = prev.content_hash
                AND cur.project_id = $1
              WHERE old.project_id = $1 AND old.revision_id = $2",
        )
        .bind(project_id)
        .bind(parent_revision_id)
        .bind(current_revision_id)
        .execute(&mut **tx)
        .await?
        .rows_affected() as usize;

        let parent_label = sqlx::query_scalar::<_, String>(
            "SELECT revision FROM code.code_revision WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(parent_revision_id)
        .fetch_one(&mut **tx)
        .await?;
        let current_label = sqlx::query_scalar::<_, String>(
            "SELECT revision FROM code.code_revision WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(current_revision_id)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            "CREATE TEMP TABLE copied_fact_map ON COMMIT DROP AS
             SELECT DISTINCT md5($3::text || ':' || f.id::text)::uuid AS new_id,
                    f.id AS old_id,
                    f.project_id, f.revision_id, f.subject_kind, f.subject_key,
                    f.relation, f.object_kind, f.object_key, f.resolution,
                    f.confidence, f.analysis_engine, f.engine_version,
                    f.conflict_group, f.unresolved_reason, f.candidates, f.fact_hash
               FROM code.fact f
               JOIN code.fact_evidence fe
                 ON fe.fact_id = f.id AND fe.project_id = f.project_id
               JOIN corpus.evidence ev ON ev.id = fe.evidence_id
               JOIN code.source_file cur
                 ON cur.revision_id = $3 AND cur.project_id = $1
                AND cur.path = ev.locator->>'path'
               JOIN code.source_file prev
                 ON prev.revision_id = $2 AND prev.project_id = $1
                AND prev.path = cur.path AND prev.content_hash = cur.content_hash
              WHERE f.project_id = $1 AND f.revision_id = $2 AND f.valid_to IS NULL",
        )
        .bind(project_id)
        .bind(parent_revision_id)
        .bind(current_revision_id)
        .execute(&mut **tx)
        .await?;
        let copied = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM copied_fact_map")
            .fetch_one(&mut **tx)
            .await? as usize;
        sqlx::query(
            "INSERT INTO code.fact
               (id, project_id, revision_id, subject_kind, subject_key, relation,
                object_kind, object_key, resolution, confidence, analysis_engine,
                engine_version, conflict_group, unresolved_reason, candidates, fact_hash)
             SELECT new_id, project_id, $2, subject_kind, subject_key, relation,
                    object_kind, object_key, resolution, confidence, analysis_engine,
                    engine_version, conflict_group, unresolved_reason, candidates, fact_hash
               FROM copied_fact_map
              WHERE project_id = $1
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(project_id)
        .bind(current_revision_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO code.fact_evidence (project_id, fact_id, evidence_id, role)
             SELECT copied.project_id, copied.new_id, old.evidence_id, old.role
               FROM copied_fact_map copied
               JOIN code.fact_evidence old
                 ON old.project_id = copied.project_id AND old.fact_id = copied.old_id
             ON CONFLICT (fact_id, evidence_id) DO NOTHING",
        )
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "WITH input AS (
               SELECT DISTINCT ON (entity_type, natural_key)
                      entity_type, natural_key, resolution, fact_id
                 FROM (
                   SELECT subject_kind AS entity_type, subject_key AS natural_key,
                          resolution, new_id AS fact_id
                     FROM copied_fact_map
                   UNION ALL
                   SELECT object_kind, object_key, resolution, new_id
                     FROM copied_fact_map
                 ) AS entities
                ORDER BY entity_type, natural_key
             ), upserted AS (
               INSERT INTO graph_meta.entity
                 (id, project_id, entity_type, natural_key, name, qualified_name,
                  properties, ontology_version, revision, resolution,
                  first_seen_rev, last_seen_rev, projection_seq)
               SELECT gen_random_uuid(), $1, input.entity_type, input.natural_key,
                      regexp_replace(input.natural_key, '^.*[./:]', ''),
                      input.natural_key,
                      jsonb_build_object('source', 'code', 'fact_id', input.fact_id::text),
                      COALESCE((SELECT version FROM graph_meta.ontology_version
                                 WHERE status = 'PUBLISHED'
                                 ORDER BY published_at DESC NULLS LAST LIMIT 1), 'v1'),
                      $2, input.resolution, $2, $2, 1
                 FROM input
               ON CONFLICT (project_id, entity_type, natural_key)
               DO UPDATE SET revision = EXCLUDED.revision,
                             resolution = EXCLUDED.resolution,
                             last_seen_rev = EXCLUDED.last_seen_rev,
                             projection_seq = graph_meta.entity.projection_seq + 1,
                             deleted_at = NULL
               RETURNING id, project_id, entity_type, natural_key, name,
                         qualified_name, revision, resolution, projection_seq
             )
             INSERT INTO core.outbox_event (aggregate, aggregate_id, event_type, payload)
             SELECT 'entity', upserted.id, 'DocumentNodeUpserted',
                    jsonb_build_object(
                      'project_id', upserted.project_id,
                      'entity_id', upserted.id,
                      'uid', upserted.project_id::text || ':' || upserted.id::text,
                      'entity_type', upserted.entity_type,
                      'natural_key', upserted.natural_key,
                      'name', upserted.name,
                      'qualified_name', upserted.qualified_name,
                      'revision', upserted.revision,
                      'resolution', upserted.resolution,
                      'seq', upserted.projection_seq
                    )
               FROM upserted",
        )
        .bind(project_id)
        .bind(&current_label)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "WITH mapped AS (
               SELECT copied.new_id AS fact_id, src.id AS src_id, dst.id AS dst_id,
                      old.rel_type, copied.resolution, copied.confidence
                 FROM copied_fact_map copied
                 JOIN graph_meta.relationship old
                   ON old.project_id = $1 AND old.revision = $2 AND old.valid_to IS NULL
                 JOIN graph_meta.entity src
                   ON src.id = old.src_id AND src.project_id = $1
                 JOIN graph_meta.entity dst
                   ON dst.id = old.dst_id AND dst.project_id = $1
                WHERE src.natural_key = copied.subject_key
                  AND src.entity_type = copied.subject_kind
                  AND dst.natural_key = copied.object_key
                  AND dst.entity_type = copied.object_kind
             ), upserted AS (
               INSERT INTO graph_meta.relationship
                 (id, project_id, src_id, dst_id, rel_type, resolution, confidence,
                  fact_id, revision, projection_seq)
               SELECT gen_random_uuid(), $1, mapped.src_id, mapped.dst_id,
                      mapped.rel_type, mapped.resolution, mapped.confidence,
                      mapped.fact_id, $3, 1
                 FROM mapped
               ON CONFLICT (project_id, src_id, dst_id, rel_type, revision)
               DO UPDATE SET valid_to = NULL,
                             resolution = EXCLUDED.resolution,
                             confidence = EXCLUDED.confidence,
                             fact_id = EXCLUDED.fact_id,
                             projection_seq = graph_meta.relationship.projection_seq + 1
               RETURNING id, project_id, src_id, dst_id, rel_type, resolution,
                         confidence, revision, projection_seq
             )
             INSERT INTO core.outbox_event (aggregate, aggregate_id, event_type, payload)
             SELECT 'relationship', upserted.id, 'RelationshipUpserted',
                    jsonb_build_object(
                      'project_id', upserted.project_id,
                      'rel_id', upserted.id,
                      'src_uid', upserted.project_id::text || ':' || upserted.src_id::text,
                      'dst_uid', upserted.project_id::text || ':' || upserted.dst_id::text,
                      'rel_type', upserted.rel_type,
                      'src_type', src.entity_type,
                      'dst_type', dst.entity_type,
                      'revision', upserted.revision,
                      'resolution', upserted.resolution,
                      'confidence', upserted.confidence,
                      'seq', upserted.projection_seq
                    )
               FROM upserted
               JOIN graph_meta.entity src ON src.id = upserted.src_id
               JOIN graph_meta.entity dst ON dst.id = upserted.dst_id",
        )
        .bind(project_id)
        .bind(&parent_label)
        .bind(&current_label)
        .execute(&mut **tx)
        .await?;
        Ok((symbols, copied))
    }

    async fn project_code_facts_to_graph(
        &self,
        tx: &mut Tx<'_>,
        revision_label: &str,
        facts: &[NewCodeFact],
    ) -> Result<(), StoreError> {
        let Some(first) = facts.first() else {
            return Ok(());
        };
        let mut entities: BTreeMap<(String, String), (Uuid, String, Uuid)> = BTreeMap::new();
        for fact in facts {
            for (kind, key) in [
                (&fact.subject_kind, &fact.subject_key),
                (&fact.object_kind, &fact.object_key),
            ] {
                let entity_id = code_entity_id(fact.project_id, kind, key);
                let entry = entities
                    .entry((kind.clone(), key.clone()))
                    .or_insert_with(|| (entity_id, fact.resolution.clone(), fact.id));
                if entry.1 != "RESOLVED" && fact.resolution == "RESOLVED" {
                    entry.1 = fact.resolution.clone();
                    entry.2 = fact.id;
                }
            }
        }
        let entity_ids = entities.values().map(|(id, _, _)| *id).collect::<Vec<_>>();
        let entity_types = entities
            .keys()
            .map(|(kind, _)| kind.clone())
            .collect::<Vec<_>>();
        let natural_keys = entities
            .keys()
            .map(|(_, key)| key.clone())
            .collect::<Vec<_>>();
        let names = natural_keys
            .iter()
            .map(|key| {
                key.rsplit(['.', '/', ':'])
                    .next()
                    .unwrap_or(key)
                    .to_string()
            })
            .collect::<Vec<_>>();
        let entity_resolutions = entities
            .values()
            .map(|(_, resolution, _)| resolution.clone())
            .collect::<Vec<_>>();
        let entity_fact_ids = entities
            .values()
            .map(|(_, _, fact_id)| *fact_id)
            .collect::<Vec<_>>();
        sqlx::query(
            "WITH input AS (
               SELECT * FROM unnest(
                 $2::uuid[], $3::text[], $4::text[], $5::text[], $6::text[], $7::uuid[]
               ) AS rows(entity_id, entity_type, natural_key, name, resolution, fact_id)
             ), upserted AS (
               INSERT INTO graph_meta.entity
                 (id, project_id, entity_type, natural_key, name, qualified_name,
                  properties, ontology_version, revision, resolution,
                  first_seen_rev, last_seen_rev, projection_seq)
               SELECT input.entity_id, $1, input.entity_type, input.natural_key,
                      input.name, input.natural_key,
                      jsonb_build_object('source', 'code', 'fact_id', input.fact_id::text),
                      COALESCE((SELECT version FROM graph_meta.ontology_version
                                 WHERE status = 'PUBLISHED'
                                 ORDER BY published_at DESC NULLS LAST LIMIT 1), 'v1'),
                      $8, input.resolution, $8, $8, 1
                 FROM input
               ON CONFLICT (project_id, entity_type, natural_key)
               DO UPDATE SET name = EXCLUDED.name,
                             qualified_name = EXCLUDED.qualified_name,
                             revision = EXCLUDED.revision,
                             resolution = EXCLUDED.resolution,
                             last_seen_rev = EXCLUDED.last_seen_rev,
                             projection_seq = graph_meta.entity.projection_seq + 1,
                             deleted_at = NULL
               RETURNING id, project_id, entity_type, natural_key, name,
                         qualified_name, revision, resolution, projection_seq
             )
             INSERT INTO core.outbox_event (aggregate, aggregate_id, event_type, payload)
             SELECT 'entity', upserted.id, 'DocumentNodeUpserted',
                    jsonb_build_object(
                      'project_id', upserted.project_id,
                      'entity_id', upserted.id,
                      'uid', upserted.project_id::text || ':' || upserted.id::text,
                      'entity_type', upserted.entity_type,
                      'natural_key', upserted.natural_key,
                      'name', upserted.name,
                      'qualified_name', upserted.qualified_name,
                      'revision', upserted.revision,
                      'resolution', upserted.resolution,
                      'seq', upserted.projection_seq
                    )
               FROM upserted",
        )
        .bind(first.project_id)
        .bind(entity_ids)
        .bind(entity_types)
        .bind(natural_keys)
        .bind(names)
        .bind(entity_resolutions)
        .bind(entity_fact_ids)
        .bind(revision_label)
        .execute(&mut **tx)
        .await?;

        let relation_ids = facts
            .iter()
            .map(|fact| {
                Uuid::new_v5(
                    &fact.project_id,
                    format!(
                        "code:{}:{}:{}:{}",
                        fact.subject_key, fact.relation, fact.object_key, revision_label
                    )
                    .as_bytes(),
                )
            })
            .collect::<Vec<_>>();
        let src_ids = facts
            .iter()
            .map(|fact| code_entity_id(fact.project_id, &fact.subject_kind, &fact.subject_key))
            .collect::<Vec<_>>();
        let dst_ids = facts
            .iter()
            .map(|fact| code_entity_id(fact.project_id, &fact.object_kind, &fact.object_key))
            .collect::<Vec<_>>();
        let rel_types = facts
            .iter()
            .map(|fact| fact.relation.clone())
            .collect::<Vec<_>>();
        let resolutions = facts
            .iter()
            .map(|fact| fact.resolution.clone())
            .collect::<Vec<_>>();
        let confidences = facts.iter().map(|fact| fact.confidence).collect::<Vec<_>>();
        let fact_ids = facts.iter().map(|fact| fact.id).collect::<Vec<_>>();
        let src_types = facts
            .iter()
            .map(|fact| fact.subject_kind.clone())
            .collect::<Vec<_>>();
        let dst_types = facts
            .iter()
            .map(|fact| fact.object_kind.clone())
            .collect::<Vec<_>>();
        sqlx::query(
            "WITH input AS (
               SELECT * FROM unnest(
                 $2::uuid[], $3::uuid[], $4::uuid[], $5::text[], $6::text[],
                 $7::real[], $8::uuid[], $9::text[], $10::text[]
               ) AS rows(rel_id, src_id, dst_id, rel_type, resolution,
                         confidence, fact_id, src_type, dst_type)
             ), upserted AS (
               INSERT INTO graph_meta.relationship
                 (id, project_id, src_id, dst_id, rel_type, resolution, confidence,
                  fact_id, revision, projection_seq)
               SELECT input.rel_id, $1, input.src_id, input.dst_id, input.rel_type,
                      input.resolution, input.confidence, input.fact_id, $11, 1
                 FROM input
               ON CONFLICT (project_id, src_id, dst_id, rel_type, revision)
               DO UPDATE SET valid_to = NULL,
                             resolution = EXCLUDED.resolution,
                             confidence = EXCLUDED.confidence,
                             fact_id = EXCLUDED.fact_id,
                             projection_seq = graph_meta.relationship.projection_seq + 1
               RETURNING id, project_id, src_id, dst_id, rel_type, resolution,
                         confidence, revision, projection_seq
             )
             INSERT INTO core.outbox_event (aggregate, aggregate_id, event_type, payload)
             SELECT 'relationship', upserted.id, 'RelationshipUpserted',
                    jsonb_build_object(
                      'project_id', upserted.project_id,
                      'rel_id', upserted.id,
                      'src_uid', upserted.project_id::text || ':' || upserted.src_id::text,
                      'dst_uid', upserted.project_id::text || ':' || upserted.dst_id::text,
                      'rel_type', upserted.rel_type,
                      'src_type', input.src_type,
                      'dst_type', input.dst_type,
                      'revision', upserted.revision,
                      'resolution', upserted.resolution,
                      'confidence', upserted.confidence,
                      'seq', upserted.projection_seq
                    )
               FROM upserted
               JOIN input ON input.rel_id = upserted.id",
        )
        .bind(first.project_id)
        .bind(relation_ids)
        .bind(src_ids)
        .bind(dst_ids)
        .bind(rel_types)
        .bind(resolutions)
        .bind(confidences)
        .bind(fact_ids)
        .bind(src_types)
        .bind(dst_types)
        .bind(revision_label)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}

fn csv_text(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn csv_optional_text(value: Option<&str>) -> String {
    value.map(csv_text).unwrap_or_else(|| "\\N".into())
}

fn csv_optional_uuid(value: Option<Uuid>) -> String {
    value
        .map(|id| id.to_string())
        .unwrap_or_else(|| "\\N".into())
}

fn csv_bytea(value: &[u8]) -> String {
    let mut encoded = String::with_capacity(2 + value.len() * 2);
    encoded.push_str("\\x");
    for byte in value {
        encoded.push_str(&format!("{byte:02x}"));
    }
    csv_text(&encoded)
}

fn code_repository_row(row: &sqlx::postgres::PgRow) -> CodeRepositoryRow {
    CodeRepositoryRow {
        id: row.get("id"),
        project_id: row.get("project_id"),
        kind: row.get("kind"),
        url: row.get("url"),
        default_branch: row.get("default_branch"),
        settings: row.get("settings"),
        created_at: row.get("created_at"),
    }
}

fn code_revision_row(row: &sqlx::postgres::PgRow) -> CodeRevisionRow {
    CodeRevisionRow {
        id: row.get("id"),
        project_id: row.get("project_id"),
        repository_id: row.get("repository_id"),
        revision: row.get("revision"),
        parent_id: row.get("parent_id"),
        tree_digest: row.get("tree_digest"),
        snapshot_key: row.get("snapshot_key"),
        committed_at: row.get("committed_at"),
        is_baseline: row.get("is_baseline"),
        created_at: row.get("created_at"),
    }
}

fn code_source_file_row(row: &sqlx::postgres::PgRow) -> CodeSourceFileRow {
    CodeSourceFileRow {
        id: row.get("id"),
        revision_id: row.get("revision_id"),
        path: row.get("path"),
        language: row.get("language"),
        content_hash: row.get("content_hash"),
        size_bytes: row.get("size_bytes"),
        encoding: row.get("encoding"),
        loc: row.get("loc"),
        line_offsets: row.get("line_offsets"),
        exact_offsets: row.get("exact_offsets"),
    }
}

fn code_analysis_run_row(row: &sqlx::postgres::PgRow) -> CodeAnalysisRunRow {
    CodeAnalysisRunRow {
        id: row.get("id"),
        project_id: row.get("project_id"),
        revision_id: row.get("revision_id"),
        status: row.get("status"),
        engine_versions: row.get("engine_versions"),
        classpath_complete: row.get("classpath_complete"),
        stats: row.get("stats"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        created_at: row.get("created_at"),
        error: row.get("error"),
    }
}

fn code_entity_id(project_id: Uuid, kind: &str, key: &str) -> Uuid {
    Uuid::new_v5(&project_id, format!("code-entity:{kind}:{key}").as_bytes())
}
