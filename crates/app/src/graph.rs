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
        store::commit(tx).await?;

        let projector = neo4j_projector().ok_or_else(|| {
            AppError::CapabilityUnsupported("Neo4j projector が設定されていません".into())
        })?;
        projector
            .ensure_schema()
            .await
            .map_err(|e| AppError::Validation(e.to_string()))?;
        projector
            .clear_project(&project.0.to_string())
            .await
            .map_err(|e| AppError::Validation(e.to_string()))?;
        let rebuild_event =
            uuid::Uuid::new_v5(&project.0, format!("graph-rebuild:{base_seq}").as_bytes());
        let mut entity_count = 0i64;
        let mut after_id = Uuid::nil();
        loop {
            let mut tx = self.store().begin_scoped(project).await?;
            let page = self
                .store()
                .graph_entities_page(&mut tx, project.0, after_id, 1_000)
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
                .apply_entities(&rows)
                .await
                .map_err(|e| AppError::Validation(e.to_string()))?;
            entity_count += page.len() as i64;
            after_id = page.last().map(|entity| entity.id).unwrap_or(after_id);
        }

        let mut relationship_count = 0i64;
        after_id = Uuid::nil();
        loop {
            let mut tx = self.store().begin_scoped(project).await?;
            let page = self
                .store()
                .graph_relationships_page(&mut tx, project.0, after_id, 1_000)
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
                    .apply_relationships(&rel_type, &rows)
                    .await
                    .map_err(|e| AppError::Validation(e.to_string()))?;
            }
            relationship_count += page.len() as i64;
            after_id = page
                .last()
                .map(|relationship| relationship.id)
                .unwrap_or(after_id);
        }
        projector
            .apply_projection_state(project.0, base_seq, rebuild_event)
            .await
            .map_err(|e| AppError::Validation(e.to_string()))?;
        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .record_projection_build(&mut tx, project.0, entity_count, relationship_count)
            .await?;
        store::commit(tx).await?;
        Ok(contracts::GraphRebuildResponse {
            entities: entity_count,
            relationships: relationship_count,
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
