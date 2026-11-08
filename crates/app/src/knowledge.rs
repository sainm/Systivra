use std::collections::HashSet;
use std::time::Instant;

use contracts::{
    AuditEntry, AuditListResponse, ChunkNeighbors, ChunkResponse, DocType, ErrorCode,
    EvidenceResponse, ParseRunRef, SearchDocumentRef, SearchFilters, SearchRequest, SearchResponse,
    SearchResult, SearchRevisionRef,
};
use domain::{ChunkId, DocumentId, EvidenceId, Permission, ProjectId, RevisionId};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::AppError;
use crate::App;

impl App {
    /// 既存 chunk の `tokenized` を現在の tokenizer で作り直す。
    ///
    /// 日本語形態素（Lindera / IPADIC）は実装済みだが、**既に保存された
    /// `tokenized` は古いまま**で、`fts`（生成列）も追随していない。
    /// そのため `住所` のようなクエリが本文に在るのに 0 件になる。
    /// 再取込を待たずに直すための保守操作。
    pub async fn retokenize_chunks(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        trace: Option<String>,
    ) -> Result<contracts::RetokenizeResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::DocumentIngest.access_for(role).can_write() {
            return Err(AppError::Forbidden(
                "knowledge:retokenize の権限がありません".into(),
            ));
        }
        const BATCH: i64 = 5000;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .chunks_for_retokenize(&mut tx, project.0, BATCH)
            .await?;
        let stale = self.store().count_stale_chunks(&mut tx, project.0).await?;
        let mut updated = 0usize;
        for (chunk_id, text, old) in &rows {
            let fresh = engine_rag::tokenized_text(text);
            if &fresh != old
                && self
                    .store()
                    .update_chunk_tokenized(&mut tx, project.0, *chunk_id, &fresh)
                    .await?
            {
                updated += 1;
            }
        }
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "knowledge.retokenize".into(),
                    target_type: Some("corpus.chunk".into()),
                    target_id: Some(project.0.to_string()),
                    input_digest: None,
                    result: "OK".into(),
                    detail: json!({
                        "examined": rows.len(),
                        "updated": updated,
                        "stale": stale,
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(contracts::RetokenizeResponse {
            updated,
            examined: rows.len(),
            remaining: (stale - rows.len() as i64).max(0) as usize,
        })
    }

    pub async fn search(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        req: SearchRequest,
    ) -> Result<SearchResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::Search.access_for(role).can_read() {
            return Err(AppError::Forbidden("knowledge:read がありません".into()));
        }
        let normalized_query = engine_rag::normalize_nfkc(req.query.trim());
        let query = normalized_query.as_str();
        if query.is_empty() || query.chars().count() > 2000 {
            return Err(AppError::Validation("query は 1〜2000 文字です".into()));
        }
        if req.top_k < 1 || req.top_k > 100 {
            return Err(AppError::Validation("top_k は 1〜100 です".into()));
        }
        if req.rerank && req.mode == contracts::SearchMode::Lexical {
            return Err(AppError::Validation(
                "lexical モードでは rerank を指定できません".into(),
            ));
        }
        let tsquery = engine_rag::tsquery(query);
        if tsquery.is_empty() {
            return Err(AppError::Validation(
                "検索語に有効な文字がありません".into(),
            ));
        }
        let strict_tsquery = engine_rag::strict_tsquery(query);

        let filter = store::SearchFilter {
            doc_types: req
                .filters
                .doc_types
                .iter()
                .map(|d| d.as_str().to_string())
                .collect(),
            business_domain: req.filters.business_domain.clone(),
            revision_id: req.filters.revision_id.map(|r| r.0),
        };
        let started = Instant::now();
        let limit = req.top_k * 4;
        let mode = req.mode;

        let mut tx = self.store().begin_scoped(project).await?;
        self.store().configure_vector_search(&mut tx).await?;
        let (model_id, _model_name, revision) = self
            .store()
            .active_embedding_model(&mut tx, project.0)
            .await?;
        let embedding = engine_rag::embed_for_revision(query, &revision);
        let mut lexical = if matches!(
            mode,
            contracts::SearchMode::Lexical | contracts::SearchMode::Hybrid
        ) {
            let lexical_limit = if strict_tsquery.is_empty() {
                limit
            } else {
                (req.top_k * 20).max(200)
            };
            let mut hits = Vec::new();
            if !strict_tsquery.is_empty() {
                hits = self
                    .store()
                    .search_lexical(&mut tx, project.0, &strict_tsquery, &filter, lexical_limit)
                    .await?;
            }
            if hits.is_empty() {
                hits = self
                    .store()
                    .search_lexical(&mut tx, project.0, &tsquery, &filter, lexical_limit)
                    .await?;
            }
            hits
        } else {
            Vec::new()
        };
        exact_term_rank(query, &mut lexical);
        let vector = if matches!(
            mode,
            contracts::SearchMode::Vector | contracts::SearchMode::Hybrid
        ) {
            self.store()
                .search_vector(&mut tx, project.0, model_id, &embedding, &filter, limit)
                .await?
        } else {
            Vec::new()
        };

        let mut unique = HashSet::new();
        for hit in lexical.iter().chain(vector.iter()) {
            unique.insert(hit.chunk_id);
        }
        let total_candidates = unique.len() as i64;
        let candidate_limit = if req.rerank {
            limit as usize
        } else {
            req.top_k as usize
        };
        let mut fused = engine_rag::fuse(mode, lexical, vector, candidate_limit);
        if req.rerank {
            engine_rag::rerank(query, &mut fused, req.top_k as usize);
        }
        let include_quote = req.include.is_empty() || req.include.iter().any(|v| v == "quote");
        let results: Vec<SearchResult> = fused
            .into_iter()
            .map(|hit| SearchResult {
                chunk_id: ChunkId(hit.chunk_id),
                document: SearchDocumentRef {
                    id: DocumentId(hit.document_id),
                    title: hit.document_title,
                    doc_type: parse_doc_type(&hit.doc_type),
                },
                revision: SearchRevisionRef {
                    id: RevisionId(hit.revision_id),
                    source_revision: hit.source_revision,
                },
                locator: hit.locator,
                quote: if include_quote {
                    snippet(&hit.quote)
                } else {
                    String::new()
                },
                score: round_score(hit.score),
                rerank_score: hit.rerank_score.map(round_score),
                sources: hit.sources,
                evidence_id: hit.evidence_id.map(EvidenceId),
            })
            .collect();
        let latency_ms = started.elapsed().as_millis().max(1) as i64;
        let search_id = format!("SRH-{}", Uuid::new_v4());

        let selected: Vec<Uuid> = results.iter().map(|r| r.chunk_id.0).collect();
        self.store()
            .insert_retrieval_log(
                &mut tx,
                &store::RetrievalLogDraft {
                    project_id: project.0,
                    query: query.to_string(),
                    query_tokens: engine_rag::tokenize(query)
                        .into_iter()
                        .map(|t| t.text)
                        .collect(),
                    filters: json!({
                        "doc_types": filter.doc_types,
                        "business_domain": filter.business_domain,
                        "revision_id": filter.revision_id,
                    }),
                    lexical_mode: "BOTH".into(),
                    candidates: json!({
                        "lexical": unique.len(),
                        "returned": selected.len(),
                    }),
                    selected,
                    rerank_used: req.rerank,
                    latency_ms: latency_ms as i32,
                },
            )
            .await?;
        store::commit(tx).await?;

        let mut degraded = vec!["embedding_local_fallback".to_string()];
        if req.rerank {
            degraded.push("rerank_local_heuristic".into());
        }
        if req.graph_assist {
            degraded.push("graph_assist_off".into());
        }
        degraded.push("bbox_unavailable".into());

        Ok(SearchResponse {
            search_id,
            lexical_mode: "BOTH".into(),
            degraded,
            results,
            total_candidates,
            latency_ms,
        })
    }

    pub async fn chunk_for_user(
        &self,
        auth: &Authenticated,
        chunk_id: ChunkId,
    ) -> Result<ChunkResponse, AppError> {
        let (project, role) = self
            .scope_for_resource(auth.user_id, "chunk", chunk_id.0)
            .await?;
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::NotFound(ErrorCode::NotFoundChunk));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let chunk = self
            .store()
            .find_chunk(&mut tx, project.0, chunk_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundChunk))?;
        let (prev, next) = self
            .store()
            .neighbor_chunks(&mut tx, chunk.revision_id, chunk.ordinal)
            .await?;
        store::commit(tx).await?;

        Ok(ChunkResponse {
            chunk_id: ChunkId(chunk.id),
            chunk_type: chunk.chunk_type,
            text: chunk.text,
            locator: chunk.locator,
            token_count: chunk.token_count as i64,
            char_count: chunk.char_count as i64,
            neighbors: ChunkNeighbors {
                prev: prev.into_iter().map(ChunkId).collect(),
                next: next.into_iter().map(ChunkId).collect(),
                parent: chunk.node_path.last().cloned(),
            },
            parse_run: ParseRunRef {
                parser: chunk.parser,
                parser_version: chunk.parser_version,
            },
        })
    }

    pub async fn evidence_for_user(
        &self,
        auth: &Authenticated,
        evidence_id: EvidenceId,
    ) -> Result<EvidenceResponse, AppError> {
        let (project, role) = self
            .scope_for_resource(auth.user_id, "evidence", evidence_id.0)
            .await?;
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::NotFound(ErrorCode::NotFoundEvidence));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let evidence = self
            .store()
            .find_evidence(&mut tx, project.0, evidence_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundEvidence))?;
        let (context_before, context_after) =
            match (evidence.revision_id, evidence.locator.get("page")) {
                (Some(revision), Some(page)) if page.as_i64().is_some() => {
                    let page = page.as_i64().unwrap_or(1) as i32;
                    match self.store().page_text(&mut tx, revision, page).await? {
                        Some(text) => context_around(&text, &evidence.quote),
                        None => (String::new(), String::new()),
                    }
                }
                _ => (String::new(), String::new()),
            };
        store::commit(tx).await?;

        Ok(EvidenceResponse {
            evidence_id: EvidenceId(evidence.id),
            source_type: evidence.source_type,
            document_id: evidence.document_id.map(DocumentId),
            revision_id: evidence.revision_id.map(RevisionId),
            locator: evidence.locator,
            quote: evidence.quote,
            quote_hash: engine_rag::hash_to_sha256(&evidence.quote_hash),
            context_before,
            context_after,
            resolution: evidence.resolution,
            confidence: evidence.confidence,
            document_title: evidence.document_title,
            mime_type: evidence.mime_type,
            page_count: evidence.page_count,
        })
    }

    pub async fn audit_logs(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        limit: i64,
    ) -> Result<AuditListResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::AuditQuery.access_for(role).can_read() {
            return Err(AppError::Forbidden("audit:read がありません".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .list_audit(&mut tx, project.0, limit.clamp(1, 200))
            .await?;
        store::commit(tx).await?;
        Ok(AuditListResponse {
            items: rows
                .into_iter()
                .map(|r| AuditEntry {
                    id: r.id,
                    actor_type: r.actor_type,
                    actor_id: r.actor_id,
                    action: r.action,
                    target_type: r.target_type,
                    target_id: r.target_id,
                    result: r.result,
                    trace_id: r.trace_id,
                    detail: r.detail,
                    occurred_at: r.occurred_at,
                })
                .collect(),
        })
    }

    pub async fn filters_for_project(
        &self,
        _auth: &Authenticated,
        _project: ProjectId,
    ) -> Result<SearchFilters, AppError> {
        Ok(SearchFilters::default())
    }
}

fn parse_doc_type(value: &str) -> DocType {
    DocType::ALL
        .into_iter()
        .find(|d| d.as_str() == value)
        .unwrap_or(DocType::Other)
}

fn snippet(text: &str) -> String {
    const LIMIT: usize = 600;
    let mut out: String = text.chars().take(LIMIT).collect();
    if text.chars().count() > LIMIT {
        out.push('…');
    }
    out
}

fn round_score(score: f64) -> f64 {
    (score * 10000.0).round() / 10000.0
}

fn exact_term_rank(query: &str, hits: &mut [store::SearchHit]) {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.to_lowercase())
        .filter(|term| term.chars().count() >= 2)
        .collect();
    if terms.len() < 2 {
        return;
    }
    hits.sort_by(|a, b| {
        let hits_in = |hit: &store::SearchHit| {
            let text = hit.quote.to_lowercase();
            terms.iter().filter(|term| text.contains(*term)).count()
        };
        hits_in(b).cmp(&hits_in(a))
    });
}

fn context_around(page_text: &str, quote: &str) -> (String, String) {
    let Some(pos) = page_text.find(quote) else {
        return (String::new(), String::new());
    };
    let before: String = page_text[..pos]
        .chars()
        .rev()
        .take(60)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let after_start = pos + quote.len();
    let after: String = page_text[after_start..].chars().take(60).collect();
    (before, after)
}

pub fn filter_json(filter: &store::SearchFilter) -> Value {
    json!({
        "doc_types": filter.doc_types,
        "business_domain": filter.business_domain,
        "revision_id": filter.revision_id,
    })
}
