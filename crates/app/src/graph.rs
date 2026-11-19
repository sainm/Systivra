use std::collections::BTreeMap;

use chrono::Utc;
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
            id: None,
            based_on: None,
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
        // **未接続と 0 件を区別する。** `unwrap_or((0,0))` だと Neo4j を止めただけで
        // drift_rate が 1.0 になり「100% 漂流」の誤報になる。
        let counts = neo4j_counts(project.0).await;
        let (neo4j_nodes, neo4j_relationships) = match counts {
            Some((nodes, relationships)) => (Some(nodes), Some(relationships)),
            None => (None, None),
        };
        let drift_rate = match counts {
            Some((nodes, _)) if pg_nodes > 0 => {
                Some((pg_nodes - nodes).unsigned_abs() as f64 / pg_nodes as f64)
            }
            Some(_) => Some(0.0),
            None => None,
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

    /// 編集中の DRAFT を返す（無ければ作る）。
    ///
    /// 型を足すたびに公開版を書き換えると、途中の状態が全員に見えてしまう。
    /// 編集は DRAFT に閉じ込め、公開は明示的な操作にする。
    pub async fn ensure_ontology_draft(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        base_version_id: Option<Uuid>,
    ) -> Result<contracts::OntologyResponse, AppError> {
        self.require_ontology_write(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        if let Some(base) = base_version_id {
            if !self.store().ontology_version_exists(&mut tx, base).await? {
                return Err(AppError::Validation(format!(
                    "複製元の版がありません: {base}"
                )));
            }
        }
        let version = format!("draft-{}", Utc::now().format("%Y%m%d%H%M%S"));
        let (id, label) = self
            .store()
            .create_ontology_draft(&mut tx, &version, base_version_id)
            .await?;
        // 複製元は store が決める（base 指定が無ければ公開版）ので、読み戻して返す。
        let based_on = self
            .store()
            .ontology_draft(&mut tx)
            .await?
            .and_then(|(_, _, based_on)| based_on);
        let (entity_types, relation_types) = self.store().ontology_types(&mut tx, id).await?;
        store::commit(tx).await?;
        Ok(contracts::OntologyResponse {
            id: Some(id),
            based_on,
            version: label,
            status: "DRAFT".into(),
            published_at: None,
            entity_types: serde_json::from_value(entity_types)
                .map_err(|error| AppError::Validation(error.to_string()))?,
            relation_types: serde_json::from_value(relation_types)
                .map_err(|error| AppError::Validation(error.to_string()))?,
        })
    }

    /// 版の一覧（基底を選ばせるため）。
    pub async fn ontology_versions(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<contracts::OntologyVersionListResponse, AppError> {
        self.require_graph_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self.store().ontology_versions(&mut tx).await?;
        store::commit(tx).await?;
        Ok(contracts::OntologyVersionListResponse {
            items: rows
                .into_iter()
                .map(|row| contracts::OntologyVersionSummary {
                    id: row.id,
                    version: row.version,
                    status: row.status,
                    published_at: row.published_at,
                })
                .collect(),
        })
    }

    /// 編集中の DRAFT を読む（編集用。公開版とは別）。
    pub async fn ontology_draft(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<contracts::OntologyResponse, AppError> {
        self.require_graph_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let Some((id, label, based_on)) = self.store().ontology_draft(&mut tx).await? else {
            return Err(AppError::NotFound(ErrorCode::NotFound));
        };
        let (entity_types, relation_types) = self.store().ontology_types(&mut tx, id).await?;
        store::commit(tx).await?;
        Ok(contracts::OntologyResponse {
            id: Some(id),
            based_on,
            version: label,
            status: "DRAFT".into(),
            published_at: None,
            entity_types: serde_json::from_value(entity_types)
                .map_err(|error| AppError::Validation(error.to_string()))?,
            relation_types: serde_json::from_value(relation_types)
                .map_err(|error| AppError::Validation(error.to_string()))?,
        })
    }

    /// DRAFT に型を追加 / 更新する。
    pub async fn upsert_ontology_entity_type(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        entity_type: contracts::OntologyEntityType,
    ) -> Result<contracts::OntologyResponse, AppError> {
        self.require_ontology_write(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let Some((version_id, _label, _based_on)) = self.store().ontology_draft(&mut tx).await?
        else {
            return Err(AppError::Validation(
                "編集中の版がありません。先に DRAFT を作ってください".into(),
            ));
        };
        let (entity_json, relation_json) = self.store().ontology_types(&mut tx, version_id).await?;
        let existing: Vec<contracts::OntologyEntityType> = serde_json::from_value(entity_json)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        let relations: Vec<contracts::OntologyRelationType> = serde_json::from_value(relation_json)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        validate_entity_type(&existing, &entity_type)?;
        self.store()
            .upsert_ontology_entity_type(
                &mut tx,
                version_id,
                &store::OntologyEntityTypeDraft {
                    name: entity_type.name.clone(),
                    display_name_ja: entity_type.display_name_ja.clone(),
                    parent_type: entity_type.parent_type.clone(),
                    is_abstract: entity_type.is_abstract,
                    properties_schema: entity_type.properties_schema.clone(),
                },
            )
            .await?;
        let (entity_json, relation_json) = self.store().ontology_types(&mut tx, version_id).await?;
        store::commit(tx).await?;
        let _ = relations;
        Ok(contracts::OntologyResponse {
            id: Some(version_id),
            based_on: _based_on,
            version: _label,
            status: "DRAFT".into(),
            published_at: None,
            entity_types: serde_json::from_value(entity_json)
                .map_err(|error| AppError::Validation(error.to_string()))?,
            relation_types: serde_json::from_value(relation_json)
                .map_err(|error| AppError::Validation(error.to_string()))?,
        })
    }

    /// DRAFT から型を削除する。**参照されている型は消さない**（壊れた定義を残さない）。
    pub async fn delete_ontology_entity_type(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        name: &str,
    ) -> Result<contracts::OntologyResponse, AppError> {
        self.require_ontology_write(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let Some((version_id, label, _based_on)) = self.store().ontology_draft(&mut tx).await?
        else {
            return Err(AppError::Validation(
                "編集中の版がありません。先に DRAFT を作ってください".into(),
            ));
        };
        let (entity_json, relation_json) = self.store().ontology_types(&mut tx, version_id).await?;
        let existing: Vec<contracts::OntologyEntityType> = serde_json::from_value(entity_json)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        let relations: Vec<contracts::OntologyRelationType> = serde_json::from_value(relation_json)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        if !existing.iter().any(|item| item.name == name) {
            return Err(AppError::NotFound(ErrorCode::NotFound));
        }
        let child: Vec<&str> = existing
            .iter()
            .filter(|item| item.parent_type.as_deref() == Some(name))
            .map(|item| item.name.as_str())
            .collect();
        if !child.is_empty() {
            return Err(AppError::Validation(format!(
                "子型があるため削除できません: {}",
                child.join(", ")
            )));
        }
        let used: Vec<&str> = relations
            .iter()
            .filter(|relation| {
                relation.source_types.iter().any(|item| item == name)
                    || relation.target_types.iter().any(|item| item == name)
            })
            .map(|relation| relation.name.as_str())
            .collect();
        if !used.is_empty() {
            return Err(AppError::Validation(format!(
                "関係型が参照しているため削除できません: {}",
                used.join(", ")
            )));
        }
        self.store()
            .delete_ontology_entity_type(&mut tx, version_id, name)
            .await?;
        let (entity_json, relation_json) = self.store().ontology_types(&mut tx, version_id).await?;
        store::commit(tx).await?;
        Ok(contracts::OntologyResponse {
            id: Some(version_id),
            based_on: _based_on,
            version: label,
            status: "DRAFT".into(),
            published_at: None,
            entity_types: serde_json::from_value(entity_json)
                .map_err(|error| AppError::Validation(error.to_string()))?,
            relation_types: serde_json::from_value(relation_json)
                .map_err(|error| AppError::Validation(error.to_string()))?,
        })
    }

    /// DRAFT に関係型を追加 / 更新する。
    pub async fn upsert_ontology_relation_type(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        relation: contracts::OntologyRelationType,
    ) -> Result<contracts::OntologyResponse, AppError> {
        self.require_ontology_write(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let Some((version_id, label, _based_on)) = self.store().ontology_draft(&mut tx).await?
        else {
            return Err(AppError::Validation(
                "編集中の版がありません。先に DRAFT を作ってください".into(),
            ));
        };
        let (entity_json, _relation_json) =
            self.store().ontology_types(&mut tx, version_id).await?;
        let entities: Vec<contracts::OntologyEntityType> = serde_json::from_value(entity_json)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        validate_relation_type(&entities, &relation)?;
        self.store()
            .upsert_ontology_relation_type(
                &mut tx,
                version_id,
                &store::OntologyRelationTypeDraft {
                    name: relation.name.clone(),
                    display_name_ja: relation.display_name_ja.clone(),
                    source_types: relation.source_types.clone(),
                    target_types: relation.target_types.clone(),
                    cardinality: relation.cardinality.clone(),
                    is_transitive: relation.is_transitive,
                    properties_schema: serde_json::json!({}),
                },
            )
            .await?;
        let (entity_json, relation_json) = self.store().ontology_types(&mut tx, version_id).await?;
        store::commit(tx).await?;
        Ok(contracts::OntologyResponse {
            id: Some(version_id),
            based_on: _based_on,
            version: label,
            status: "DRAFT".into(),
            published_at: None,
            entity_types: serde_json::from_value(entity_json)
                .map_err(|error| AppError::Validation(error.to_string()))?,
            relation_types: serde_json::from_value(relation_json)
                .map_err(|error| AppError::Validation(error.to_string()))?,
        })
    }

    /// DRAFT から関係型を削除する。
    pub async fn delete_ontology_relation_type(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        name: &str,
    ) -> Result<contracts::OntologyResponse, AppError> {
        self.require_ontology_write(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let Some((version_id, label, _based_on)) = self.store().ontology_draft(&mut tx).await?
        else {
            return Err(AppError::Validation(
                "編集中の版がありません。先に DRAFT を作ってください".into(),
            ));
        };
        let (_, relation_json) = self.store().ontology_types(&mut tx, version_id).await?;
        let relations: Vec<contracts::OntologyRelationType> = serde_json::from_value(relation_json)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        if !relations.iter().any(|item| item.name == name) {
            return Err(AppError::NotFound(ErrorCode::NotFound));
        }
        self.store()
            .delete_ontology_relation_type(&mut tx, version_id, name)
            .await?;
        let (entity_json, relation_json) = self.store().ontology_types(&mut tx, version_id).await?;
        store::commit(tx).await?;
        Ok(contracts::OntologyResponse {
            id: Some(version_id),
            based_on: _based_on,
            version: label,
            status: "DRAFT".into(),
            published_at: None,
            entity_types: serde_json::from_value(entity_json)
                .map_err(|error| AppError::Validation(error.to_string()))?,
            relation_types: serde_json::from_value(relation_json)
                .map_err(|error| AppError::Validation(error.to_string()))?,
        })
    }

    async fn require_ontology_write(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::OntologyPublish.access_for(role).can_write() {
            return Err(AppError::Forbidden("ontology:publish がありません".into()));
        }
        Ok(())
    }

    pub async fn graph_publish_ontology(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        version: &str,
        draft: bool,
    ) -> Result<contracts::PublishOntologyResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::OntologyPublish.access_for(role).can_write() {
            return Err(AppError::Forbidden("ontology:publish がありません".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        if draft {
            // 編集した DRAFT をそのまま公開する（型編集の成果を反映）。
            let Some((version_id, label, _based_on)) = self.store().ontology_draft(&mut tx).await?
            else {
                return Err(AppError::Validation("編集中の版がありません".into()));
            };
            let (entity_types, relation_types) =
                self.store().ontology_types(&mut tx, version_id).await?;
            self.store()
                .promote_ontology_draft(&mut tx, version_id, auth.user_id.0)
                .await?;
            store::commit(tx).await?;
            let entity_count = entity_types.as_array().map_or(0, Vec::len) as i64;
            let relation_count = relation_types.as_array().map_or(0, Vec::len) as i64;
            return Ok(contracts::PublishOntologyResponse {
                version: label,
                status: "PUBLISHED".into(),
                entity_types: entity_count,
                relation_types: relation_count,
            });
        }
        let version = version.trim();
        if version.is_empty() || version.chars().count() > 40 {
            return Err(AppError::Validation("version は 1〜40 文字です".into()));
        }
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

/// 型 1 件の妥当性。**壊れた型を公開するとグラフ全体が読めなくなる**ので、
/// 名前・親の存在・親子の循環・スキーマの形をここで止める。
fn validate_entity_type(
    existing: &[contracts::OntologyEntityType],
    entity_type: &contracts::OntologyEntityType,
) -> Result<(), AppError> {
    let name = entity_type.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::Validation("name は 1〜64 文字です".into()));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or('a');
    if !first.is_ascii_uppercase() {
        return Err(AppError::Validation(
            "name は大文字で始めてください（例: Screen）".into(),
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::Validation(
            "name は英数字と _ だけにしてください".into(),
        ));
    }
    let display = entity_type.display_name_ja.trim();
    if display.is_empty() || display.chars().count() > 80 {
        return Err(AppError::Validation(
            "display_name_ja は 1〜80 文字です".into(),
        ));
    }
    if !entity_type.properties_schema.is_object() {
        return Err(AppError::Validation(
            "properties_schema はオブジェクトで指定してください".into(),
        ));
    }
    if let Some(parent) = &entity_type.parent_type {
        if parent == name {
            return Err(AppError::Validation("自分自身を親にはできません".into()));
        }
        if !existing.iter().any(|item| &item.name == parent) {
            return Err(AppError::Validation(format!("親型がありません: {parent}")));
        }
        // 親をたどって循環していないか（既存 + 今回の変更を反映した鎖で見る）。
        let mut current = parent.clone();
        let mut guard = 0;
        while let Some(next) = existing
            .iter()
            .find(|item| item.name == current && item.name != name)
            .and_then(|item| item.parent_type.clone())
        {
            if next == name {
                return Err(AppError::Validation(format!(
                    "親子が循環します: {name} → {parent} → … → {name}"
                )));
            }
            current = next;
            guard += 1;
            if guard > 64 {
                return Err(AppError::Validation("親型の階層が深すぎます".into()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod ontology_tests {
    use super::*;
    use contracts::OntologyEntityType;

    fn ty(name: &str, parent: Option<&str>) -> OntologyEntityType {
        OntologyEntityType {
            name: name.into(),
            display_name_ja: format!("{name} の表示名"),
            parent_type: parent.map(str::to_owned),
            is_abstract: false,
            properties_schema: serde_json::json!({}),
        }
    }

    #[test]
    fn accepts_new_type_with_existing_parent() {
        let existing = vec![ty("Artifact", None), ty("Screen", Some("Artifact"))];
        assert!(validate_entity_type(&existing, &ty("Dialog", Some("Screen"))).is_ok());
    }

    #[test]
    fn rejects_missing_parent() {
        // 親が無いと階層が宙に浮く。公開後に気付くのが一番高くつく。
        let existing = vec![ty("Artifact", None)];
        let error = validate_entity_type(&existing, &ty("Dialog", Some("Ghost")))
            .expect_err("未知の親は弾く");
        assert!(format!("{error:?}").contains("親型がありません"));
    }

    #[test]
    fn rejects_parent_cycle() {
        let existing = vec![ty("A", None), ty("B", Some("A")), ty("C", Some("B"))];
        // C の親を A にすると A → … の鎖が C に戻る。
        let error = validate_entity_type(&existing, &ty("A", Some("C"))).expect_err("循環は弾く");
        assert!(format!("{error:?}").contains("循環"));
    }

    #[test]
    fn rejects_bad_names_and_schema() {
        let existing = vec![ty("Artifact", None)];
        assert!(validate_entity_type(&existing, &ty("screen", None)).is_err());
        assert!(validate_entity_type(&existing, &ty("Screen-Type", None)).is_err());
        let mut broken = ty("Screen", None);
        broken.properties_schema = serde_json::json!([]);
        assert!(validate_entity_type(&existing, &broken).is_err());
        let mut empty = ty("Screen", None);
        empty.display_name_ja = "  ".into();
        assert!(validate_entity_type(&existing, &empty).is_err());
    }

    #[test]
    fn rejects_self_parent() {
        let existing = vec![ty("Artifact", None)];
        assert!(validate_entity_type(&existing, &ty("Artifact", Some("Artifact"))).is_err());
    }
}

/// 関係型 1 件の妥当性。
///
/// 端点の型が無い関係を許すと、グラフの辺が存在しない型を指す。
/// `*` は「任意の型」を表す既存データの書き方なので許可する。
fn validate_relation_type(
    entities: &[contracts::OntologyEntityType],
    relation: &contracts::OntologyRelationType,
) -> Result<(), AppError> {
    let name = relation.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::Validation("name は 1〜64 文字です".into()));
    }
    let mut chars = name.chars();
    if !chars.next().unwrap_or('a').is_ascii_uppercase() {
        return Err(AppError::Validation(
            "name は大文字で始めてください（例: REFERENCES）".into(),
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::Validation(
            "name は英数字と _ だけにしてください".into(),
        ));
    }
    let display = relation.display_name_ja.trim();
    if display.is_empty() || display.chars().count() > 80 {
        return Err(AppError::Validation(
            "display_name_ja は 1〜80 文字です".into(),
        ));
    }
    if !matches!(relation.cardinality.as_str(), "1:1" | "1:N" | "N:1" | "N:M") {
        return Err(AppError::Validation(format!(
            "cardinality は 1:1 / 1:N / N:1 / N:M のいずれかです: {}",
            relation.cardinality
        )));
    }
    for (label, types) in [
        ("source_types", &relation.source_types),
        ("target_types", &relation.target_types),
    ] {
        if types.is_empty() {
            return Err(AppError::Validation(format!("{label} が空です")));
        }
        for item in types {
            if item == "*" {
                continue;
            }
            if !entities.iter().any(|entity| &entity.name == item) {
                return Err(AppError::Validation(format!(
                    "{label} に存在しない型があります: {item}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod relation_tests {
    use super::*;
    use contracts::{OntologyEntityType, OntologyRelationType};

    fn entity(name: &str) -> OntologyEntityType {
        OntologyEntityType {
            name: name.into(),
            display_name_ja: format!("{name} の表示名"),
            parent_type: None,
            is_abstract: false,
            properties_schema: serde_json::json!({}),
        }
    }

    fn relation(name: &str, source: &[&str], target: &[&str]) -> OntologyRelationType {
        OntologyRelationType {
            name: name.into(),
            display_name_ja: format!("{name} の表示名"),
            source_types: source.iter().map(|s| s.to_string()).collect(),
            target_types: target.iter().map(|s| s.to_string()).collect(),
            cardinality: "N:M".into(),
            is_transitive: false,
        }
    }

    #[test]
    fn accepts_relation_between_existing_types() {
        let entities = vec![entity("Screen"), entity("Field")];
        assert!(
            validate_relation_type(&entities, &relation("HAS_FIELD", &["Screen"], &["Field"]))
                .is_ok()
        );
        // `*` は既存データの「任意」表記。
        assert!(validate_relation_type(&entities, &relation("CONTAINS", &["*"], &["*"])).is_ok());
    }

    #[test]
    fn rejects_unknown_endpoint_type() {
        let entities = vec![entity("Screen")];
        let error =
            validate_relation_type(&entities, &relation("HAS_FIELD", &["Screen"], &["Ghost"]))
                .expect_err("未知の端点は弾く");
        assert!(format!("{error:?}").contains("存在しない型"));
    }

    #[test]
    fn rejects_bad_cardinality_and_empty_endpoints() {
        let entities = vec![entity("Screen")];
        let mut bad = relation("HAS_FIELD", &["Screen"], &["Screen"]);
        bad.cardinality = "many".into();
        assert!(validate_relation_type(&entities, &bad).is_err());

        let empty = relation("HAS_FIELD", &[], &["Screen"]);
        assert!(validate_relation_type(&entities, &empty).is_err());
    }

    #[test]
    fn rejects_bad_relation_names() {
        let entities = vec![entity("Screen")];
        assert!(validate_relation_type(
            &entities,
            &relation("has_field", &["Screen"], &["Screen"])
        )
        .is_err());
        assert!(validate_relation_type(
            &entities,
            &relation("HAS-FIELD", &["Screen"], &["Screen"])
        )
        .is_err());
    }
}
