use std::collections::BTreeMap;

use contracts::{
    ErrorCode, GraphEdge, GraphEntity, GraphEntitySearchResponse, GraphNeighbor,
    GraphNeighborsResponse, GraphPathResponse, GraphStatsResponse, GraphSubgraphResponse,
    OntologyEntityType, OntologyRelationType, OntologyResponse, TraceabilityResponse,
};
use domain::{Permission, ProjectId};
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::AppError;
use crate::App;

impl App {
    pub async fn graph_ontology(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<OntologyResponse, AppError> {
        self.require_graph_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let row = self
            .store()
            .published_ontology(&mut tx)
            .await?
            .ok_or_else(|| AppError::NotFound(ErrorCode::NotFound))?;
        store::commit(tx).await?;
        let entity_types: Vec<OntologyEntityType> = serde_json::from_value(row.entity_types)
            .map_err(|e| AppError::Validation(e.to_string()))?;
        let relation_types: Vec<OntologyRelationType> = serde_json::from_value(row.relation_types)
            .map_err(|e| AppError::Validation(e.to_string()))?;
        Ok(OntologyResponse {
            version: row.version,
            status: row.status,
            published_at: row.published_at,
            entity_types,
            relation_types,
        })
    }

    pub async fn graph_search(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        query: &str,
        entity_type: Option<&str>,
        limit: i64,
    ) -> Result<GraphEntitySearchResponse, AppError> {
        self.require_graph_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .graph_search_entities(&mut tx, project.0, query, entity_type, limit.clamp(1, 100))
            .await?;
        store::commit(tx).await?;
        Ok(GraphEntitySearchResponse {
            items: rows.into_iter().map(entity).collect(),
        })
    }

    pub async fn graph_neighbors(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        entity_id: Uuid,
        rel_types: Vec<String>,
        limit: i64,
    ) -> Result<GraphNeighborsResponse, AppError> {
        self.require_graph_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .graph_neighbors(
                &mut tx,
                project.0,
                entity_id,
                &rel_types,
                limit.clamp(1, 200),
            )
            .await?;
        store::commit(tx).await?;
        Ok(GraphNeighborsResponse {
            items: rows
                .into_iter()
                .map(|row| GraphNeighbor {
                    entity: entity(row.entity),
                    relation: row.rel_type,
                    direction: row.direction,
                    confidence: row.confidence,
                    revision: row.edge_revision,
                    depth: row.depth,
                })
                .collect(),
        })
    }

    pub async fn graph_subgraph(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        root: Uuid,
        depth: i32,
        limit: i64,
    ) -> Result<GraphSubgraphResponse, AppError> {
        self.require_graph_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let graph = self
            .store()
            .graph_subgraph(
                &mut tx,
                project.0,
                root,
                depth.clamp(1, 3),
                limit.clamp(1, 500),
            )
            .await?;
        store::commit(tx).await?;
        Ok(GraphSubgraphResponse {
            nodes: graph.nodes.into_iter().map(entity).collect(),
            edges: graph.edges.into_iter().map(edge).collect(),
        })
    }

    pub async fn graph_path(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        src: Uuid,
        dst: Uuid,
        max_depth: i32,
    ) -> Result<Option<GraphPathResponse>, AppError> {
        self.require_graph_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let path = self
            .store()
            .graph_path(&mut tx, project.0, src, dst, max_depth.clamp(1, 3))
            .await?;
        store::commit(tx).await?;
        Ok(path.map(|path| GraphPathResponse {
            nodes: path.nodes.into_iter().map(entity).collect(),
            relations: path.rel_types,
        }))
    }

    pub async fn graph_traceability(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        entity_id: Uuid,
        depth: i32,
        limit: i64,
    ) -> Result<TraceabilityResponse, AppError> {
        self.require_graph_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let target = self
            .store()
            .graph_entity(&mut tx, project.0, entity_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        let (ancestors, descendants) = self
            .store()
            .graph_traceability(
                &mut tx,
                project.0,
                entity_id,
                depth.clamp(1, 3),
                limit.clamp(1, 200),
            )
            .await?;
        store::commit(tx).await?;
        Ok(TraceabilityResponse {
            entity: entity(target),
            ancestors: ancestors.into_iter().map(entity).collect(),
            descendants: descendants.into_iter().map(entity).collect(),
        })
    }

    pub async fn graph_stats(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<GraphStatsResponse, AppError> {
        self.require_graph_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let (pg_nodes, pg_relationships) = self
            .store()
            .graph_project_counts(&mut tx, project.0)
            .await?;
        let last_event_seq = self
            .store()
            .graph_last_event_seq(&mut tx, project.0)
            .await?;
        let pending_events = self.store().pending_event_count(&mut tx, project.0).await?;
        let drift = self
            .store()
            .latest_projection_drift(&mut tx, project.0)
            .await?;
        let build = self
            .store()
            .latest_projection_build(&mut tx, project.0)
            .await?;
        let ontology_version = self.store().published_ontology_version(&mut tx).await?;
        store::commit(tx).await?;
        let (neo4j_nodes, neo4j_relationships) = neo4j_counts(project.0).await.unwrap_or((0, 0));
        let drift_rate = if pg_nodes > 0 {
            (pg_nodes - neo4j_nodes).unsigned_abs() as f64 / pg_nodes as f64
        } else {
            0.0
        };
        Ok(GraphStatsResponse {
            pg_nodes,
            pg_relationships,
            neo4j_nodes,
            neo4j_relationships,
            drift_rate,
            last_event_seq,
            ontology_version,
            pending_events,
            last_drift_checked_at: drift.as_ref().map(|row| row.0),
            last_drift_rate: drift.as_ref().map(|row| row.1),
            drift_degraded: drift.as_ref().map(|row| row.2).unwrap_or(false),
            last_build: build.map(|row| contracts::GraphBuildView {
                build_id: row.0,
                status: row.1,
                shadow_label: row.2,
                drift_rate: row.3,
                entities_written: row.4,
                relationships_written: row.5,
                finished_at: row.6,
            }),
        })
    }

    pub async fn graph_publish_ontology(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        version: &str,
    ) -> Result<contracts::PublishOntologyResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::OntologyPublish.access_for(role).can_write() {
            return Err(AppError::Forbidden("ontology:publish がありません".into()));
        }
        let version = version.trim();
        if version.is_empty() || version.chars().count() > 40 {
            return Err(AppError::Validation("version は 1〜40 文字です".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let (entity_types, relation_types) = self
            .store()
            .publish_ontology(&mut tx, version, auth.user_id.0)
            .await?;
        store::commit(tx).await?;
        Ok(contracts::PublishOntologyResponse {
            version: version.to_string(),
            status: "PUBLISHED".into(),
            entity_types,
            relation_types,
        })
    }

    pub async fn graph_rebuild(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<contracts::GraphRebuildResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::ProviderManage.access_for(role).can_write() {
            return Err(AppError::Forbidden("provider:manage がありません".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let base_seq = self.store().graph_outbox_head_seq(&mut tx).await?;
        let (pg_nodes, pg_relationships) = self
            .store()
            .graph_project_counts(&mut tx, project.0)
            .await?;
        let build_id = Uuid::new_v4();
        let short = build_id.simple().to_string()[..8].to_owned();
        let shadow_label = format!("Entity__{short}");
        self.store()
            .create_projection_build(&mut tx, build_id, project.0, &shadow_label, base_seq)
            .await?;
        store::commit(tx).await?;

        let projector = neo4j_projector().ok_or_else(|| {
            AppError::CapabilityUnsupported("Neo4j projector が設定されていません".into())
        })?;
        projector
            .ensure_schema()
            .await
            .map_err(|e| AppError::Validation(e.to_string()))?;
        let batch = std::env::var("GRAPH_REBUILD_BATCH")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(1_000)
            .clamp(1, 5_000);
        let delay_ms = std::env::var("GRAPH_REBUILD_STEP_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);

        let mut entity_count = 0i64;
        let mut after_id = Uuid::nil();
        loop {
            if self.rebuild_aborted(project, build_id).await? {
                let _ = projector
                    .delete_label(&project.0.to_string(), &shadow_label)
                    .await;
                return Ok(contracts::GraphRebuildResponse {
                    entities: entity_count,
                    relationships: 0,
                    build_id,
                    status: "ABORTED".into(),
                    drift_rate: 0.0,
                    previous_entities: pg_nodes,
                    previous_relationships: pg_relationships,
                });
            }
            let mut tx = self.store().begin_scoped(project).await?;
            let page = self
                .store()
                .graph_entities_page(&mut tx, project.0, after_id, batch)
                .await?;
            store::commit(tx).await?;
            if page.is_empty() {
                break;
            }
            let rows = page
                .iter()
                .map(|entity| engine_graph::EntityProjection {
                    entity_id: entity.id,
                    uid: format!("{}:{}", project.0, entity.id),
                    project_id: project.0,
                    entity_type: entity.entity_type.clone(),
                    natural_key: entity.natural_key.clone(),
                    name: entity.name.clone(),
                    qualified_name: entity.qualified_name.clone(),
                    revision: entity.revision.clone(),
                    resolution: entity.resolution.clone(),
                    seq: entity.projection_seq,
                })
                .collect::<Vec<_>>();
            projector
                .apply_entities_as(&shadow_label, &rows)
                .await
                .map_err(|e| AppError::Validation(e.to_string()))?;
            entity_count += page.len() as i64;
            after_id = page.last().map(|entity| entity.id).unwrap_or(after_id);
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }

        let mut relationship_count = 0i64;
        after_id = Uuid::nil();
        loop {
            if self.rebuild_aborted(project, build_id).await? {
                let _ = projector
                    .delete_label(&project.0.to_string(), &shadow_label)
                    .await;
                return Ok(contracts::GraphRebuildResponse {
                    entities: entity_count,
                    relationships: relationship_count,
                    build_id,
                    status: "ABORTED".into(),
                    drift_rate: 0.0,
                    previous_entities: pg_nodes,
                    previous_relationships: pg_relationships,
                });
            }
            let mut tx = self.store().begin_scoped(project).await?;
            let page = self
                .store()
                .graph_relationships_page(&mut tx, project.0, after_id, batch)
                .await?;
            store::commit(tx).await?;
            if page.is_empty() {
                break;
            }
            let mut by_type: BTreeMap<String, Vec<engine_graph::RelationshipProjection>> =
                BTreeMap::new();
            for relationship in &page {
                by_type
                    .entry(relationship.rel_type.clone())
                    .or_default()
                    .push(engine_graph::RelationshipProjection {
                        rel_id: relationship.id,
                        src_uid: format!("{}:{}", project.0, relationship.src_id),
                        dst_uid: format!("{}:{}", project.0, relationship.dst_id),
                        project_id: project.0,
                        resolution: relationship.resolution.clone(),
                        confidence: relationship.confidence,
                        revision: relationship.revision.clone(),
                        seq: relationship.projection_seq,
                    });
            }
            for (rel_type, rows) in by_type {
                projector
                    .apply_relationships_as(&shadow_label, &rel_type, &rows)
                    .await
                    .map_err(|e| AppError::Validation(e.to_string()))?;
            }
            relationship_count += page.len() as i64;
            after_id = page
                .last()
                .map(|relationship| relationship.id)
                .unwrap_or(after_id);
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }

        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .update_projection_build(
                &mut tx,
                build_id,
                "VERIFYING",
                entity_count,
                relationship_count,
                None,
            )
            .await?;
        store::commit(tx).await?;

        let (shadow_nodes, shadow_relationships) = projector
            .count_label(&project.0.to_string(), &shadow_label)
            .await
            .map_err(|e| AppError::Validation(e.to_string()))?;
        let diff =
            (pg_nodes - shadow_nodes).abs() + (pg_relationships - shadow_relationships).abs();
        let drift_rate = diff as f64 / 1_000.0;
        if diff > 0 || drift_rate > 0.001 {
            let _ = projector
                .delete_label(&project.0.to_string(), &shadow_label)
                .await;
            let mut tx = self.store().begin_scoped(project).await?;
            self.store()
                .update_projection_build(
                    &mut tx,
                    build_id,
                    "FAILED",
                    entity_count,
                    relationship_count,
                    Some(drift_rate),
                )
                .await?;
            store::commit(tx).await?;
            return Err(AppError::Validation(format!(
                "検証で差分 {diff} 件（drift {drift_rate:.5}）。切替を中止しました"
            )));
        }

        let old_label = format!("Entity__old_{short}");
        projector
            .switch_label(&project.0.to_string(), &shadow_label, &old_label)
            .await
            .map_err(|e| AppError::Validation(e.to_string()))?;
        let _ = projector
            .delete_label(&project.0.to_string(), &old_label)
            .await;

        let mut replay_seq = base_seq;
        loop {
            let mut tx = self.store().begin_scoped(project).await?;
            let events = self
                .store()
                .graph_events_after(&mut tx, project.0, replay_seq, 500)
                .await?;
            store::commit(tx).await?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                projector
                    .apply(&event.event_type, &event.payload, event.seq, event.event_id)
                    .await
                    .map_err(|e| AppError::Validation(e.to_string()))?;
                replay_seq = event.seq;
            }
        }

        let mut tx = self.store().begin_scoped(project).await?;
        let head_seq = self.store().graph_outbox_head_seq(&mut tx).await?;
        self.store()
            .update_projection_build(
                &mut tx,
                build_id,
                "SWITCHED",
                entity_count,
                relationship_count,
                Some(drift_rate),
            )
            .await?;
        store::commit(tx).await?;
        projector
            .apply_projection_state(project.0, head_seq, build_id)
            .await
            .map_err(|e| AppError::Validation(e.to_string()))?;

        Ok(contracts::GraphRebuildResponse {
            entities: entity_count,
            relationships: relationship_count,
            build_id,
            status: "SWITCHED".into(),
            drift_rate,
            previous_entities: pg_nodes,
            previous_relationships: pg_relationships,
        })
    }

    async fn rebuild_aborted(&self, project: ProjectId, build_id: Uuid) -> Result<bool, AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        let status = self
            .store()
            .projection_build_status(&mut tx, build_id)
            .await?;
        store::commit(tx).await?;
        Ok(!matches!(status.as_deref(), Some("BUILDING" | "VERIFYING")))
    }

    pub async fn graph_abort_rebuild(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<contracts::GraphRebuildAbortResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::ProviderManage.access_for(role).can_write() {
            return Err(AppError::Forbidden("provider:manage がありません".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let aborted = self
            .store()
            .abort_projection_build(&mut tx, project.0)
            .await?;
        store::commit(tx).await?;
        let build_id = aborted.as_ref().map(|(id, _)| *id);
        let mut shadow_deleted = false;
        if let Some((_, label)) = &aborted {
            if let Some(projector) = neo4j_projector() {
                projector
                    .delete_label(&project.0.to_string(), label)
                    .await
                    .map_err(|e| AppError::Validation(e.to_string()))?;
                shadow_deleted = true;
            }
        }
        Ok(contracts::GraphRebuildAbortResponse {
            build_id,
            status: if aborted.is_some() { "ABORTED" } else { "NOOP" }.into(),
            shadow_deleted,
        })
    }

    pub async fn detect_graph_drift(
        &self,
        project: ProjectId,
    ) -> Result<contracts::GraphDriftView, AppError> {
        let projector = neo4j_projector().ok_or_else(|| {
            AppError::CapabilityUnsupported("Neo4j projector が設定されていません".into())
        })?;
        let mut tx = self.store().begin_scoped(project).await?;
        let (pg_nodes, pg_relationships) = self
            .store()
            .graph_project_counts(&mut tx, project.0)
            .await?;
        store::commit(tx).await?;
        let (neo4j_nodes, neo4j_relationships) = projector
            .count_project(&project.0.to_string())
            .await
            .map_err(|e| AppError::Validation(e.to_string()))?;
        let diff = (pg_nodes - neo4j_nodes).abs() + (pg_relationships - neo4j_relationships).abs();
        let drift_rate = diff as f64 / 1_000.0;
        let degraded = diff > 0 || drift_rate > 0.001;
        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .insert_projection_drift(
                &mut tx,
                &store::NewProjectionDrift {
                    project_id: project.0,
                    pg_nodes,
                    pg_relationships,
                    neo4j_nodes,
                    neo4j_relationships,
                    drift_rate,
                    degraded,
                },
            )
            .await?;
        if degraded {
            self.store()
                .append_audit(
                    &mut tx,
                    &store::AuditDraft {
                        project_id: Some(project.0),
                        actor_type: "SYSTEM".into(),
                        actor_id: "graph-drift".into(),
                        action: "graph.drift.detected".into(),
                        target_type: Some("graph_meta.projection_state".into()),
                        target_id: Some(project.0.to_string()),
                        input_digest: None,
                        result: "DEGRADED".into(),
                        detail: serde_json::json!({
                            "pg_nodes": pg_nodes,
                            "pg_relationships": pg_relationships,
                            "neo4j_nodes": neo4j_nodes,
                            "neo4j_relationships": neo4j_relationships,
                            "drift_rate": drift_rate,
                        }),
                        trace_id: None,
                    },
                )
                .await?;
        }
        store::commit(tx).await?;
        Ok(contracts::GraphDriftView {
            checked_at: chrono::Utc::now(),
            pg_nodes,
            pg_relationships,
            neo4j_nodes,
            neo4j_relationships,
            drift_rate,
            degraded,
        })
    }

    async fn require_graph_read(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::Search.access_for(role).can_read() {
            return Err(AppError::Forbidden("knowledge:read がありません".into()));
        }
        Ok(())
    }
}

fn entity(row: store::GraphEntityRow) -> GraphEntity {
    GraphEntity {
        id: row.id,
        entity_type: row.entity_type,
        name: row.name,
        qualified_name: row.qualified_name,
        revision: row.revision,
        resolution: row.resolution,
        properties: row.properties,
    }
}

fn edge(row: store::GraphEdgeRow) -> GraphEdge {
    GraphEdge {
        src_id: row.src_id,
        dst_id: row.dst_id,
        rel_type: row.rel_type,
        resolution: row.resolution,
        confidence: row.confidence,
        revision: row.revision,
    }
}

async fn neo4j_counts(project_id: Uuid) -> Option<(i64, i64)> {
    let projector = neo4j_projector()?;
    projector.count_project(&project_id.to_string()).await.ok()
}

fn neo4j_projector() -> Option<engine_graph::Neo4jProjector> {
    if std::env::var("NEO4J_DISABLED").ok().as_deref() == Some("1") {
        return None;
    }
    let uri = std::env::var("NEO4J_URI").unwrap_or_else(|_| "bolt://localhost:7687".to_string());
    let user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let password = std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j".to_string());
    let database = std::env::var("NEO4J_DATABASE").unwrap_or_else(|_| "neo4j".to_string());
    engine_graph::Neo4jProjector::new(&uri, &user, &password, &database).ok()
}
