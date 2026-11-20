//! Retrieval Engine のオーケストレーション（Phase 2）。
//!
//! 方針: **既存の `knowledge::search` をそのまま呼ぶ**。検索の意味論を変えないため。
//! ここが担うのは「戦略の決定」「コンテキストの組み立て」「痕跡の記録」だけ。
//! グラフ拡張と階層拡張は Phase 3 / 4 で、この関数の中に段として足す。

use contracts::{
    ContextBlock, DocType, ErrorCode, RawScore, RelationPath, RetrievalRequest, RetrievalResponse,
    RetrievalRunListResponse, RetrievalRunSummary, RetrievalStage, RetrievalStats,
    RetrievalStrategy, RetrievedChunk, SearchFilters, SearchMode, SearchRequest, SourceDocument,
};
use domain::{ChunkId, DocumentId, EvidenceId, ProjectId, RevisionId};
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::AppError;
use crate::App;
use domain::Permission;

impl App {
    /// 統一检索入口。
    pub async fn retrieval_query(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        req: RetrievalRequest,
    ) -> Result<RetrievalResponse, AppError> {
        let started = std::time::Instant::now();
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::Search.access_for(role).can_read() {
            return Err(AppError::Forbidden("knowledge:read がありません".into()));
        }
        let query = req.query.trim().to_string();
        if query.is_empty() || query.chars().count() > 2000 {
            return Err(AppError::Validation("query は 1〜2000 文字です".into()));
        }
        if req.top_k < 1 || req.top_k > 100 {
            return Err(AppError::Validation("top_k は 1〜100 です".into()));
        }

        let mut degraded: Vec<String> = Vec::new();

        // ── 戦略の決定（指定が無ければ規則）
        let (strategy, strategy_reason) = match req.strategy {
            Some(strategy) => (strategy, "呼び出し側が指定".to_string()),
            None => {
                let choice = engine_retrieval::select_strategy(&query);
                (choice.strategy, choice.reason)
            }
        };

        // Phase 2 で実装済みなのは GLOBAL / DOCUMENT_LEVEL のみ。
        // 未実装の戦略は **黙って別物を返さず**、GLOBAL で実行した上で明示する。
        let mut strategy_reason = strategy_reason;
        let effective = match strategy {
            RetrievalStrategy::Global
            | RetrievalStrategy::DocumentLevel
            | RetrievalStrategy::CrossDocument => strategy,
            other => {
                degraded.push(format!("strategy_not_implemented:{}", other.as_str()));
                // 理由と実行内容がずれると trace が読めない。必ず注記する。
                strategy_reason = format!(
                    "{strategy_reason}（ただし {} は未実装のため GLOBAL で実行）",
                    other.as_str()
                );
                RetrievalStrategy::Global
            }
        };

        // ── フィルタ（DOCUMENT_LEVEL は種別語から DocType を補う）
        let mut filters: SearchFilters = req.filters.clone();
        if filters.doc_types.is_empty() && !req.document_types.is_empty() {
            filters.doc_types = req.document_types.clone();
        }
        if effective == RetrievalStrategy::DocumentLevel && filters.doc_types.is_empty() {
            if let Some(hint) = engine_retrieval::document_type_hint(&query) {
                match DocType::parse(hint) {
                    Some(doc_type) => filters.doc_types = vec![doc_type],
                    None => degraded.push(format!("doc_type_hint_invalid:{hint}")),
                }
            } else {
                // 種別を聞かれているのに語が拾えない → 全種別のまま（理由を残す）。
                degraded.push("doc_type_unresolved".into());
            }
        }
        if let Some(domain) = req.business_function_ids.first() {
            if filters.business_domain.is_none() {
                filters.business_domain = Some(domain.clone());
            }
        }

        // ── 検索（既存実装に委譲。順序も含めて同じ意味論を保つ）
        let search_started = std::time::Instant::now();
        let search = self
            .search(
                auth,
                project,
                SearchRequest {
                    query: query.clone(),
                    mode: SearchMode::Hybrid,
                    filters,
                    top_k: req.top_k,
                    rerank: true,
                    graph_assist: false,
                    include: vec!["quote".into()],
                },
            )
            .await?;
        let search_ms = search_started.elapsed().as_millis() as i64;

        // 類似度下限は現状 `/search` が持っていないので、ここで落とす（後で一本化）。
        let threshold = req.similarity_threshold.map(f64::from).unwrap_or(0.0);
        let mut dropped_by_threshold = 0i64;
        let mut chunks: Vec<RetrievedChunk> = Vec::new();
        for result in &search.results {
            if threshold > 0.0 && result.score < threshold {
                dropped_by_threshold += 1;
                continue;
            }
            chunks.push(RetrievedChunk {
                chunk_id: result.chunk_id,
                document_id: result.document.id,
                revision_id: result.revision.id,
                quote: result.quote.clone(),
                locator: result.locator.clone(),
                // 階層は Phase 4。今は空で返し、UI 側は「未取得」と扱う。
                node_path: Vec::new(),
                score: result.score,
                // `/search` は経路ごとの生スコアを外に出していないため、
                // ここでは**融合後の値だけ**を載せる（偽の内訳を作らない）。
                raw_scores: vec![RawScore {
                    source: result.sources.join("+"),
                    score: result.score,
                }],
                recall_sources: result.sources.clone(),
                evidence_id: result.evidence_id,
                via: None,
            });
        }
        if dropped_by_threshold > 0 {
            degraded.push(format!("below_similarity_threshold:{dropped_by_threshold}"));
        }
        degraded.extend(search.degraded.iter().cloned());

        // ── グラフ拡張（CROSS_DOCUMENT）
        let mut relationships: Vec<RelationPath> = Vec::new();
        let mut graph_stage: Option<RetrievalStage> = None;

        if effective == RetrievalStrategy::CrossDocument {
            let graph_started = std::time::Instant::now();
            let (expanded, paths, dropped_limit, dropped_dup) = self
                .expand_cross_document(project, &chunks, &req.graph)
                .await?;
            let seeds: Vec<(String, f64)> = chunks
                .iter()
                .take(5)
                .map(|chunk| (chunk.chunk_id.0.to_string(), chunk.score))
                .collect();
            let candidates = expanded.len() as i64;
            let outcome = engine_retrieval::merge_expanded(
                &seeds,
                expanded,
                req.graph.max_related.max(1) as usize,
                0.7,
            );
            let graph_dropped_by_limit = dropped_limit + outcome.dropped_by_limit as i64;
            if graph_dropped_by_limit > 0 {
                degraded.push(format!("graph_truncated:{graph_dropped_by_limit}"));
            }
            if dropped_dup > 0 || outcome.dropped_duplicate > 0 {
                degraded.push(format!(
                    "graph_duplicate_skipped:{}",
                    dropped_dup + outcome.dropped_duplicate as i64
                ));
            }

            let wanted: Vec<Uuid> = outcome
                .kept
                .iter()
                .filter_map(|hit| Uuid::parse_str(&hit.chunk_id).ok())
                .collect();
            let mut tx = self.store().begin_scoped(project).await?;
            let rows = self
                .store()
                .chunks_by_ids(&mut tx, project.0, &wanted)
                .await?;
            store::commit(tx).await?;
            let mut added = 0i64;
            for row in rows {
                let Some(hit) = outcome
                    .kept
                    .iter()
                    .find(|hit| hit.chunk_id == row.chunk_id.to_string())
                else {
                    continue;
                };
                relationships.push(RelationPath {
                    from: Uuid::parse_str(&hit.from_chunk).unwrap_or_else(|_| Uuid::nil()),
                    to: row.chunk_id,
                    relation: hit.relation.clone(),
                    confidence: hit.confidence as f32,
                    evidence_id: paths
                        .iter()
                        .find(|path| path.to == row.chunk_id && path.relation == hit.relation)
                        .and_then(|path| path.evidence_id),
                });
                chunks.push(RetrievedChunk {
                    chunk_id: ChunkId(row.chunk_id),
                    document_id: DocumentId(row.document_id),
                    revision_id: RevisionId(row.revision_id),
                    quote: row.quote,
                    locator: row.locator,
                    node_path: Vec::new(),
                    score: row.score,
                    raw_scores: vec![RawScore {
                        source: "graph".into(),
                        score: hit.confidence,
                    }],
                    recall_sources: vec!["graph".into()],
                    evidence_id: row.evidence_id.map(EvidenceId),
                    via: Some(RelationPath {
                        from: Uuid::parse_str(&hit.from_chunk).unwrap_or_else(|_| Uuid::nil()),
                        to: row.chunk_id,
                        relation: hit.relation.clone(),
                        confidence: hit.confidence as f32,
                        evidence_id: None,
                    }),
                });
                added += 1;
            }
            graph_stage = Some(RetrievalStage {
                seq: 2,
                stage: "graph".into(),
                candidates,
                kept: added,
                latency_ms: graph_started.elapsed().as_millis() as i64,
            });
        }

        // ── コンテキスト（原文のみ。要約は入れない）
        let context_started = std::time::Instant::now();
        let inputs: Vec<engine_retrieval::ContextInput> = chunks
            .iter()
            .map(|chunk| engine_retrieval::ContextInput {
                chunk_id: chunk.chunk_id.0.to_string(),
                document_id: chunk.document_id.0.to_string(),
                title: search
                    .results
                    .iter()
                    .find(|result| result.chunk_id == chunk.chunk_id)
                    .map(|result| result.document.title.clone())
                    .unwrap_or_default(),
                locator: chunk.locator.clone(),
                score: chunk.score,
                quote: chunk.quote.clone(),
            })
            .collect();
        let context: ContextBlock =
            engine_retrieval::build_context(&inputs, req.context.token_limit);
        let context_ms = context_started.elapsed().as_millis() as i64;

        // ── 出典（重複除去。順序は初出）
        let mut sources: Vec<SourceDocument> = Vec::new();
        for result in &search.results {
            if sources.iter().any(|source| source.id == result.document.id) {
                continue;
            }
            sources.push(SourceDocument {
                id: result.document.id,
                title: result.document.title.clone(),
                doc_type: result.document.doc_type.as_str().to_string(),
                revision_id: result.revision.id,
            });
        }

        let latency_ms = started.elapsed().as_millis() as i64;
        let mut stages = vec![RetrievalStage {
            seq: 1,
            stage: "search".into(),
            candidates: search.total_candidates,
            kept: search.results.len() as i64,
            latency_ms: search_ms,
        }];
        if let Some(stage) = graph_stage {
            stages.push(stage);
        }
        stages.push(RetrievalStage {
            seq: stages.len() as i32 + 1,
            stage: "context".into(),
            candidates: chunks.len() as i64,
            kept: context.entries.len() as i64,
            latency_ms: context_ms,
        });

        let retrieval_id = Uuid::new_v4();
        let stats = RetrievalStats {
            candidates: search.total_candidates,
            returned: chunks.len() as i64,
            context_tokens: context.tokens,
            latency_ms,
        };

        // ── 痕跡（原文は入れない。要約だけ）
        let mut tx = self.store().begin_scoped(project).await?;
        let run = store::RetrievalRunDraft {
            id: retrieval_id,
            project_id: project.0,
            query: query.clone(),
            strategy: effective.as_str().to_string(),
            strategy_reason: strategy_reason.clone(),
            request: serde_json::json!({
                "requested_strategy": req.strategy.map(|s| s.as_str()),
                "top_k": req.top_k,
                "similarity_threshold": req.similarity_threshold,
                "document_types": req.document_types.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
                "graph": {
                    "depth": req.graph.depth,
                    "direction": req.graph.direction,
                    "relationship_types": req.graph.relationship_types,
                    "max_related": req.graph.max_related,
                    "min_confidence": req.graph.min_confidence,
                },
                "context_token_limit": req.context.token_limit,
            }),
            response_stats: serde_json::json!({
                "candidates": stats.candidates,
                "returned": stats.returned,
                "context_tokens": stats.context_tokens,
                "context_dropped": context.dropped,
            }),
            degraded: degraded.clone(),
            latency_ms,
            created_by: Some(auth.user_id.0),
        };
        let stage_drafts: Vec<store::RetrievalStageDraft> = stages
            .iter()
            .map(|stage| store::RetrievalStageDraft {
                seq: stage.seq,
                stage: stage.stage.clone(),
                input_summary: serde_json::json!({
                    "query_chars": query.chars().count(),
                    "query_hash": engine_rag::sha256_hex(query.as_bytes()),
                }),
                output_summary: serde_json::json!({ "kept": stage.kept }),
                candidates: stage.candidates,
                kept: stage.kept,
                latency_ms: stage.latency_ms,
            })
            .collect();
        self.store()
            .insert_retrieval_run(&mut tx, &run, &stage_drafts)
            .await?;
        store::commit(tx).await?;

        Ok(RetrievalResponse {
            retrieval_id,
            query,
            selected_strategy: effective,
            strategy_reason,
            chunks,
            relationships,
            sources,
            context,
            stages,
            degraded,
            stats,
        })
    }

    /// 実行記録の一覧（管理画面の入口）。
    pub async fn retrieval_runs(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        limit: i64,
    ) -> Result<RetrievalRunListResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::Search.access_for(role).can_read() {
            return Err(AppError::Forbidden("knowledge:read がありません".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .retrieval_runs(&mut tx, project.0, limit.clamp(1, 100))
            .await?;
        store::commit(tx).await?;
        Ok(RetrievalRunListResponse {
            items: rows
                .into_iter()
                .map(|row| RetrievalRunSummary {
                    id: row.id,
                    query: row.query,
                    strategy: row.strategy,
                    strategy_reason: row.strategy_reason,
                    latency_ms: row.latency_ms,
                    degraded: row.degraded,
                    created_at: row.created_at,
                })
                .collect(),
        })
    }
}

impl App {
    /// 跨文档扩展：chunk 之间的关系 + 章节级图关系。
    ///
    /// 2 経路を持つ理由: 段落レベルの引用は `chunk_relation`（精密・人手）、
    /// 章レベルの派生/実装は `graph_meta.relationship`（既存の投影）。
    /// どちらも **根拠のない自動推定はしない**。
    #[allow(clippy::type_complexity)]
    async fn expand_cross_document(
        &self,
        project: ProjectId,
        chunks: &[RetrievedChunk],
        graph: &contracts::GraphOptions,
    ) -> Result<
        (
            Vec<engine_retrieval::ExpandedHit>,
            Vec<contracts::RelationPath>,
            i64,
            i64,
        ),
        AppError,
    > {
        let seeds: Vec<Uuid> = chunks
            .iter()
            .take(5)
            .map(|chunk| chunk.chunk_id.0)
            .collect();
        if seeds.is_empty() {
            return Ok((Vec::new(), Vec::new(), 0, 0));
        }
        let direction = if graph.direction.is_empty() {
            "BOTH"
        } else {
            graph.direction.as_str()
        };
        let mut expanded = Vec::new();
        let mut paths: Vec<RelationPath> = Vec::new();
        let mut dropped_dup = 0i64;

        let mut tx = self.store().begin_scoped(project).await?;

        // ── 経路 1: chunk 間の明示関係
        let relations = self
            .store()
            .chunk_relations(
                &mut tx,
                store::ChunkRelationQuery {
                    project_id: project.0,
                    chunk_ids: &seeds,
                    rel_types: &graph.relationship_types,
                    direction,
                    min_confidence: graph.min_confidence,
                    limit: graph.max_related.max(1) * 2,
                },
            )
            .await?;
        for relation in &relations {
            let (from, to) = if seeds.contains(&relation.src_chunk_id) {
                (relation.src_chunk_id, relation.dst_chunk_id)
            } else {
                (relation.dst_chunk_id, relation.src_chunk_id)
            };
            expanded.push(engine_retrieval::ExpandedHit {
                chunk_id: to.to_string(),
                from_chunk: from.to_string(),
                relation: relation.relation.clone(),
                confidence: f64::from(relation.confidence),
                hop: 1,
            });
            paths.push(RelationPath {
                from,
                to,
                relation: relation.relation.clone(),
                confidence: relation.confidence,
                evidence_id: relation.evidence_id.map(EvidenceId),
            });
        }

        // ── 経路 2: 章レベルの図（chunk → Section 実体 → 近傍 → chunk）
        let sections = self.store().chunk_sections(&mut tx, &seeds).await?;
        let mut node_ids: Vec<Uuid> = sections.iter().map(|section| section.node_id).collect();
        node_ids.sort();
        node_ids.dedup();
        if !node_ids.is_empty() {
            let keys: Vec<String> = node_ids.iter().map(|id| id.to_string()).collect();
            let entities = self
                .store()
                .graph_entities_by_natural_keys(&mut tx, project.0, &["Section".to_string()], &keys)
                .await?;
            for entity in entities {
                let neighbors = self
                    .store()
                    .graph_neighbors(
                        &mut tx,
                        project.0,
                        entity.id,
                        &graph.relationship_types,
                        graph.max_related.max(1) * 2,
                    )
                    .await?;
                for neighbor in neighbors {
                    if neighbor.confidence < graph.min_confidence {
                        continue;
                    }
                    // 章以外（コード・テスト）は chunk を持たないので今回は辿らない。
                    if neighbor.entity.entity_type != "Section" {
                        continue;
                    }
                    let Ok(target_node) = Uuid::parse_str(&neighbor.entity.natural_key) else {
                        continue;
                    };
                    let rows = self
                        .store()
                        .chunks_for_nodes(&mut tx, project.0, &[target_node], 3)
                        .await?;
                    for row in rows {
                        if seeds.contains(&row.chunk_id) {
                            dropped_dup += 1;
                            continue;
                        }
                        expanded.push(engine_retrieval::ExpandedHit {
                            chunk_id: row.chunk_id.to_string(),
                            from_chunk: entity.natural_key.clone(),
                            relation: neighbor.rel_type.clone(),
                            confidence: f64::from(neighbor.confidence),
                            hop: 1,
                        });
                        paths.push(RelationPath {
                            from: Uuid::parse_str(&entity.natural_key)
                                .unwrap_or_else(|_| Uuid::nil()),
                            to: row.chunk_id,
                            relation: neighbor.rel_type.clone(),
                            confidence: neighbor.confidence,
                            evidence_id: None,
                        });
                    }
                }
            }
        }

        store::commit(tx).await?;
        Ok((expanded, paths, 0, dropped_dup))
    }
}
