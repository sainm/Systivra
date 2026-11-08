use std::collections::HashMap;
use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::knowledge::NodeDraft;

#[derive(Debug, Clone)]
pub struct NewProjectionDrift {
    pub project_id: Uuid,
    pub pg_nodes: i64,
    pub pg_relationships: i64,
    pub neo4j_nodes: i64,
    pub neo4j_relationships: i64,
    pub drift_rate: f64,
    pub degraded: bool,
}
use crate::pool::{Store, StoreError, Tx};

#[derive(Debug, Clone)]
pub struct OutboxEvent {
    pub event_id: Uuid,
    pub seq: i64,
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct GraphEntityRow {
    pub id: Uuid,
    pub entity_type: String,
    pub natural_key: String,
    pub name: String,
    pub qualified_name: Option<String>,
    pub revision: String,
    pub resolution: String,
    pub properties: Value,
    pub projection_seq: i64,
}

#[derive(Debug, Clone)]
pub struct GraphNeighborRow {
    pub entity: GraphEntityRow,
    pub direction: String,
    pub rel_type: String,
    pub confidence: f32,
    pub edge_revision: String,
    pub depth: i32,
}

#[derive(Debug, Clone)]
pub struct GraphEdgeRow {
    pub id: Uuid,
    pub src_id: Uuid,
    pub dst_id: Uuid,
    pub rel_type: String,
    pub resolution: String,
    pub confidence: f32,
    pub revision: String,
    pub projection_seq: i64,
}

#[derive(Debug, Clone, Default)]
pub struct GraphSubgraph {
    pub nodes: Vec<GraphEntityRow>,
    pub edges: Vec<GraphEdgeRow>,
}

#[derive(Debug, Clone)]
pub struct GraphPathRow {
    pub nodes: Vec<GraphEntityRow>,
    pub rel_types: Vec<String>,
}

impl Store {
    pub async fn graph_entities_by_ids(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        entity_ids: &[Uuid],
    ) -> Result<Vec<GraphEntityRow>, StoreError> {
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT id, entity_type, natural_key, name, qualified_name, revision, resolution, properties,
                    projection_seq
               FROM graph_meta.entity
              WHERE project_id = $1 AND id = ANY($2) AND deleted_at IS NULL
              ORDER BY id",
        )
        .bind(project_id)
        .bind(entity_ids)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(entity_row).collect())
    }

    pub async fn graph_entities_by_natural_keys(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        kinds: &[String],
        natural_keys: &[String],
    ) -> Result<Vec<GraphEntityRow>, StoreError> {
        if natural_keys.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT e.id, e.entity_type, e.natural_key, e.name, e.qualified_name,
                    e.revision, e.resolution, e.properties, e.projection_seq
               FROM graph_meta.entity e
               JOIN unnest($2::text[], $3::text[]) AS input(entity_type, natural_key)
                 ON input.entity_type = e.entity_type
                AND input.natural_key = e.natural_key
              WHERE e.project_id = $1 AND e.deleted_at IS NULL
              ORDER BY e.entity_type, e.natural_key, e.id",
        )
        .bind(project_id)
        .bind(kinds)
        .bind(natural_keys)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(entity_row).collect())
    }

    pub async fn graph_incident_edges(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        entity_ids: &[Uuid],
        limit: i64,
    ) -> Result<Vec<GraphEdgeRow>, StoreError> {
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT id, src_id, dst_id, rel_type, resolution, confidence, revision,
                    projection_seq
               FROM graph_meta.relationship
              WHERE project_id = $1 AND valid_to IS NULL
                AND (src_id = ANY($2) OR dst_id = ANY($2))
              ORDER BY rel_type, src_id, dst_id, id
              LIMIT $3",
        )
        .bind(project_id)
        .bind(entity_ids)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| GraphEdgeRow {
                id: row.get("id"),
                src_id: row.get("src_id"),
                dst_id: row.get("dst_id"),
                rel_type: row.get("rel_type"),
                resolution: row.get("resolution"),
                confidence: row.get::<f32, _>("confidence"),
                revision: row.get("revision"),
                projection_seq: row.get("projection_seq"),
            })
            .collect())
    }

    pub async fn graph_degrees(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        entity_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, i64>, StoreError> {
        if entity_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query(
            "SELECT node_id, count(*) AS degree
               FROM (
                 SELECT CASE WHEN r.src_id = input.entity_id THEN r.dst_id ELSE r.src_id END
                        AS node_id
                   FROM unnest($2::uuid[]) AS input(entity_id)
                   JOIN graph_meta.relationship r
                     ON r.project_id = $1 AND r.valid_to IS NULL
                    AND (r.src_id = input.entity_id OR r.dst_id = input.entity_id)
               ) AS neighbors
              GROUP BY node_id",
        )
        .bind(project_id)
        .bind(entity_ids)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| (row.get("node_id"), row.get("degree")))
            .collect())
    }

    pub async fn claim_graph_events(
        &self,
        tx: &mut Tx<'_>,
        worker_id: &str,
        limit: i64,
    ) -> Result<Vec<OutboxEvent>, StoreError> {
        let rows = sqlx::query(
            "UPDATE core.outbox_event o
                SET locked_by = $1,
                    locked_until = now() + interval '5 minutes'
              WHERE o.event_id IN (
                    SELECT event_id FROM core.outbox_event
                     WHERE published_at IS NULL AND NOT dead_letter
                       AND (locked_until IS NULL OR locked_until < now())
                     ORDER BY seq
                     LIMIT $2
                     FOR UPDATE SKIP LOCKED)
              RETURNING o.event_id, o.seq, o.event_type, o.payload",
        )
        .bind(worker_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| OutboxEvent {
                event_id: row.get("event_id"),
                seq: row.get("seq"),
                event_type: row.get("event_type"),
                payload: row.get("payload"),
            })
            .collect())
    }

    pub async fn mark_event_published(
        &self,
        tx: &mut Tx<'_>,
        event_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE core.outbox_event
                SET published_at = now(), locked_by = NULL, locked_until = NULL
              WHERE event_id = $1 AND published_at IS NULL",
        )
        .bind(event_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn fail_event(
        &self,
        tx: &mut Tx<'_>,
        event_id: Uuid,
        error: &Value,
    ) -> Result<bool, StoreError> {
        let row = sqlx::query(
            "UPDATE core.outbox_event
                SET attempts = attempts + 1,
                    last_error = $2,
                    dead_letter = (attempts + 1 >= 3),
                    locked_by = NULL,
                    locked_until = NULL
              WHERE event_id = $1
              RETURNING attempts, dead_letter",
        )
        .bind(event_id)
        .bind(error)
        .fetch_one(&mut **tx)
        .await?;
        let dead: bool = row.get("dead_letter");
        if dead {
            let attempts: i16 = row.get("attempts");
            sqlx::query(
                "INSERT INTO core.outbox_dead_letter (event_id, attempts, error)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (event_id) DO NOTHING",
            )
            .bind(event_id)
            .bind(attempts)
            .bind(error)
            .execute(&mut **tx)
            .await?;
        }
        Ok(dead)
    }

    pub async fn update_projection_state(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        seq: i64,
        event_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO graph_meta.projection_state
               (project_id, last_event_seq, last_event_id, updated_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT (project_id)
             DO UPDATE SET last_event_seq = EXCLUDED.last_event_seq,
                           last_event_id = EXCLUDED.last_event_id,
                           updated_at = now()",
        )
        .bind(project_id)
        .bind(seq)
        .bind(event_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn graph_counts(&self, tx: &mut Tx<'_>) -> Result<(i64, i64), StoreError> {
        let row = sqlx::query(
            "SELECT
               (SELECT count(*) FROM graph_meta.entity WHERE deleted_at IS NULL) AS nodes,
               (SELECT count(*) FROM graph_meta.relationship WHERE valid_to IS NULL) AS rels",
        )
        .fetch_one(&mut **tx)
        .await?;
        Ok((row.get("nodes"), row.get("rels")))
    }
}

impl Store {
    pub async fn sync_document_nodes(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        document_id: Uuid,
        revision_id: Uuid,
        nodes: &[NodeDraft],
    ) -> Result<usize, StoreError> {
        let mut known: HashSet<Uuid> = HashSet::new();
        let mut synced = 0usize;
        let ontology_version = self.published_ontology_version(tx).await?;

        for node in nodes {
            let Some(entity_type) = entity_type_for(&node.node_type) else {
                continue;
            };
            let natural_key = node.id.to_string();
            let name = node_name(node);
            let properties = json!({
                "document_id": document_id,
                "revision_id": revision_id,
                "node_type": node.node_type,
                "locator": node.locator,
            });
            let revision = revision_id.to_string();
            let seq: i64 = sqlx::query(
                "INSERT INTO graph_meta.entity
                   (id, project_id, entity_type, natural_key, name, qualified_name,
                    properties, ontology_version, revision, resolution,
                    first_seen_rev, last_seen_rev, projection_seq)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'RESOLVED', $9, $9, 1)
                 ON CONFLICT (project_id, entity_type, natural_key)
                 DO UPDATE SET
                   name = EXCLUDED.name,
                   qualified_name = EXCLUDED.qualified_name,
                   properties = EXCLUDED.properties,
                   revision = EXCLUDED.revision,
                   last_seen_rev = EXCLUDED.last_seen_rev,
                   projection_seq = graph_meta.entity.projection_seq + 1,
                   deleted_at = NULL
                 RETURNING projection_seq",
            )
            .bind(node.id)
            .bind(project_id)
            .bind(entity_type)
            .bind(&natural_key)
            .bind(&name)
            .bind(Option::<String>::None)
            .bind(&properties)
            .bind(&ontology_version)
            .bind(&revision)
            .fetch_one(&mut **tx)
            .await?
            .get("projection_seq");
            known.insert(node.id);

            let payload = json!({
                "project_id": project_id,
                "entity_id": node.id,
                "uid": format!("{project_id}:{}", node.id),
                "entity_type": entity_type,
                "natural_key": natural_key,
                "name": name,
                "qualified_name": Value::Null,
                "revision": revision,
                "resolution": "RESOLVED",
                "seq": seq,
            });
            self.append_graph_event(tx, "entity", node.id, "DocumentNodeUpserted", &payload)
                .await?;
            synced += 1;
        }

        for node in nodes {
            let Some(child_type) = entity_type_for(&node.node_type) else {
                continue;
            };
            let Some(parent_id) = node.parent_id else {
                continue;
            };
            if !known.contains(&parent_id) {
                let exists = sqlx::query("SELECT 1 FROM graph_meta.entity WHERE id = $1")
                    .bind(parent_id)
                    .fetch_optional(&mut **tx)
                    .await?
                    .is_some();
                if !exists {
                    continue;
                }
            }
            let parent_type: Option<String> =
                sqlx::query("SELECT entity_type FROM graph_meta.entity WHERE id = $1")
                    .bind(parent_id)
                    .fetch_optional(&mut **tx)
                    .await?
                    .map(|row| row.get("entity_type"));
            let Some(parent_type) = parent_type else {
                continue;
            };
            let revision = revision_id.to_string();
            let rel_id = Uuid::new_v5(
                &project_id,
                format!("CONTAINS:{parent_id}:{}:{revision}", node.id).as_bytes(),
            );
            let inserted = sqlx::query(
                "INSERT INTO graph_meta.relationship
                   (id, project_id, src_id, dst_id, rel_type, resolution, confidence, revision,
                    projection_seq)
                 VALUES ($1, $2, $3, $4, 'CONTAINS', 'RESOLVED', 1.0, $5, 1)
                 ON CONFLICT (project_id, src_id, dst_id, rel_type, revision)
                 DO UPDATE SET
                   valid_to = NULL,
                   confidence = 1.0,
                   projection_seq = graph_meta.relationship.projection_seq + 1
                 RETURNING id, projection_seq",
            )
            .bind(rel_id)
            .bind(project_id)
            .bind(parent_id)
            .bind(node.id)
            .bind(&revision)
            .fetch_optional(&mut **tx)
            .await?;
            if let Some(row) = inserted {
                let edge_seq: i64 = row.get("projection_seq");
                let payload = json!({
                    "project_id": project_id,
                    "rel_id": rel_id,
                    "src_uid": format!("{project_id}:{parent_id}"),
                    "dst_uid": format!("{project_id}:{}", node.id),
                    "rel_type": "CONTAINS",
                    "src_type": parent_type,
                    "dst_type": child_type,
                    "revision": revision,
                    "resolution": "RESOLVED",
                    "confidence": 1.0,
                    "seq": edge_seq,
                });
                self.append_graph_event(
                    tx,
                    "relationship",
                    rel_id,
                    "RelationshipUpserted",
                    &payload,
                )
                .await?;
            }
        }

        Ok(synced)
    }

    pub async fn tombstone_document(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        document_id: Uuid,
    ) -> Result<i64, StoreError> {
        let rows = sqlx::query(
            "UPDATE graph_meta.entity
                SET deleted_at = now(), projection_seq = projection_seq + 1
              WHERE project_id = $1
                AND deleted_at IS NULL
                AND properties->>'document_id' = $2
              RETURNING id, projection_seq",
        )
        .bind(project_id)
        .bind(document_id.to_string())
        .fetch_all(&mut **tx)
        .await?;
        if rows.is_empty() {
            return Ok(0);
        }
        sqlx::query(
            "UPDATE graph_meta.relationship
                SET valid_to = now()
              WHERE project_id = $1 AND valid_to IS NULL
                AND (src_id = ANY($2) OR dst_id = ANY($2))",
        )
        .bind(project_id)
        .bind(
            rows.iter()
                .map(|row| row.get::<Uuid, _>("id"))
                .collect::<Vec<_>>(),
        )
        .execute(&mut **tx)
        .await?;
        for row in &rows {
            let entity_id: Uuid = row.get("id");
            let seq: i64 = row.get("projection_seq");
            let payload = json!({
                "project_id": project_id,
                "uid": format!("{project_id}:{entity_id}"),
                "entity_id": entity_id,
                "deleted_at": chrono::Utc::now().to_rfc3339(),
                "seq": seq,
            });
            self.append_graph_event(tx, "entity", entity_id, "ProjectionTombstone", &payload)
                .await?;
        }
        Ok(rows.len() as i64)
    }

    async fn append_graph_event(
        &self,
        tx: &mut Tx<'_>,
        aggregate: &str,
        aggregate_id: Uuid,
        event_type: &str,
        payload: &Value,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO core.outbox_event (aggregate, aggregate_id, event_type, payload)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(aggregate)
        .bind(aggregate_id)
        .bind(event_type)
        .bind(payload)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}

impl Store {
    pub async fn graph_search_entities(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        query: &str,
        entity_type: Option<&str>,
        limit: i64,
    ) -> Result<Vec<GraphEntityRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, entity_type, natural_key, name, qualified_name, revision, resolution, properties,
                    projection_seq
               FROM graph_meta.entity
              WHERE project_id = $1 AND deleted_at IS NULL
                AND ($2 = '' OR name ILIKE '%' || $2 || '%'
                     OR qualified_name ILIKE '%' || $2 || '%')
                AND ($3::text IS NULL OR entity_type = $3)
              ORDER BY similarity(name, $2) DESC, name, id
              LIMIT $4",
        )
        .bind(project_id)
        .bind(query)
        .bind(entity_type)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(entity_row).collect())
    }

    pub async fn graph_entities_mentioned_in(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        text: &str,
        limit: i64,
    ) -> Result<Vec<GraphEntityRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, entity_type, natural_key, name, qualified_name, revision, resolution, properties,
                    projection_seq
               FROM graph_meta.entity
              WHERE project_id = $1 AND deleted_at IS NULL
                AND (
                  (length(name) >= 2 AND position(lower(name) in lower($2)) > 0)
                  OR (
                    qualified_name IS NOT NULL
                    AND length(qualified_name) >= 2
                    AND position(lower(qualified_name) in lower($2)) > 0
                  )
                )
              ORDER BY length(name) DESC, name, id
              LIMIT $3",
        )
        .bind(project_id)
        .bind(text)
        .bind(limit.clamp(1, 500))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(entity_row).collect())
    }

    pub async fn graph_entity(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        entity_id: Uuid,
    ) -> Result<Option<GraphEntityRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, entity_type, natural_key, name, qualified_name, revision, resolution, properties,
                    projection_seq
               FROM graph_meta.entity
              WHERE project_id = $1 AND id = $2 AND deleted_at IS NULL",
        )
        .bind(project_id)
        .bind(entity_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(entity_row))
    }

    pub async fn graph_neighbors(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        entity_id: Uuid,
        rel_types: &[String],
        limit: i64,
    ) -> Result<Vec<GraphNeighborRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT e.id, e.entity_type, e.natural_key, e.name, e.qualified_name, e.revision, e.resolution,
                    e.properties, e.projection_seq, 1 AS depth,
                    CASE WHEN r.src_id = $2 THEN 'OUT' ELSE 'IN' END AS direction,
                    r.rel_type, r.confidence, r.revision AS edge_revision
               FROM graph_meta.relationship r
               JOIN graph_meta.entity e
                 ON e.id = CASE WHEN r.src_id = $2 THEN r.dst_id ELSE r.src_id END
              WHERE r.project_id = $1 AND r.valid_to IS NULL
                AND (r.src_id = $2 OR r.dst_id = $2)
                AND ($3::text[] = '{}' OR r.rel_type = ANY($3))
                AND e.project_id = $1 AND e.deleted_at IS NULL
              ORDER BY e.entity_type, e.name, e.id
              LIMIT $4",
        )
        .bind(project_id)
        .bind(entity_id)
        .bind(rel_types)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| GraphNeighborRow {
                entity: entity_row(row),
                direction: row.get("direction"),
                rel_type: row.get("rel_type"),
                confidence: row.get::<f32, _>("confidence"),
                edge_revision: row.get("edge_revision"),
                depth: row.get("depth"),
            })
            .collect())
    }

    pub async fn graph_subgraph(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        root: Uuid,
        depth: i32,
        limit: i64,
    ) -> Result<GraphSubgraph, StoreError> {
        let rows = sqlx::query(
            "WITH RECURSIVE walk(entity_id, depth, path) AS (
               SELECT $2::uuid, 0, ARRAY[$2::uuid]
               UNION ALL
               SELECT CASE WHEN r.src_id = w.entity_id THEN r.dst_id ELSE r.src_id END,
                      w.depth + 1,
                      w.path || CASE WHEN r.src_id = w.entity_id THEN r.dst_id ELSE r.src_id END
                 FROM walk w
                 JOIN graph_meta.relationship r
                   ON (r.src_id = w.entity_id OR r.dst_id = w.entity_id)
                WHERE r.project_id = $1 AND r.valid_to IS NULL AND w.depth < $3
                  AND NOT (CASE WHEN r.src_id = w.entity_id THEN r.dst_id ELSE r.src_id END) = ANY(w.path)
             )
             SELECT DISTINCT ON (e.id)
                    e.id, e.entity_type, e.natural_key, e.name, e.qualified_name, e.revision, e.resolution,
                    e.properties, e.projection_seq, min(w.depth) OVER (PARTITION BY e.id) AS depth
               FROM walk w
               JOIN graph_meta.entity e
                 ON e.id = w.entity_id AND e.project_id = $1 AND e.deleted_at IS NULL
              ORDER BY e.id, depth
              LIMIT $4",
        )
        .bind(project_id)
        .bind(root)
        .bind(depth)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        let nodes: Vec<GraphEntityRow> = rows.iter().map(entity_row).collect();
        let ids: Vec<Uuid> = nodes.iter().map(|node| node.id).collect();
        let edges = if ids.is_empty() {
            Vec::new()
        } else {
            let rows = sqlx::query(
                "SELECT id, src_id, dst_id, rel_type, resolution, confidence, revision,
                        projection_seq
                   FROM graph_meta.relationship
                  WHERE project_id = $1 AND valid_to IS NULL
                    AND src_id = ANY($2) AND dst_id = ANY($2)",
            )
            .bind(project_id)
            .bind(&ids)
            .fetch_all(&mut **tx)
            .await?;
            rows.iter()
                .map(|row| GraphEdgeRow {
                    id: row.get("id"),
                    src_id: row.get("src_id"),
                    dst_id: row.get("dst_id"),
                    rel_type: row.get("rel_type"),
                    resolution: row.get("resolution"),
                    confidence: row.get::<f32, _>("confidence"),
                    revision: row.get("revision"),
                    projection_seq: row.get("projection_seq"),
                })
                .collect()
        };
        Ok(GraphSubgraph { nodes, edges })
    }

    pub async fn graph_path(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        src: Uuid,
        dst: Uuid,
        max_depth: i32,
    ) -> Result<Option<GraphPathRow>, StoreError> {
        let row = sqlx::query(
            "WITH RECURSIVE search(node_id, path, rels, depth) AS (
               SELECT $2::uuid, ARRAY[$2::uuid], ARRAY[]::text[], 0
               UNION ALL
               SELECT CASE WHEN r.src_id = s.node_id THEN r.dst_id ELSE r.src_id END,
                      s.path || CASE WHEN r.src_id = s.node_id THEN r.dst_id ELSE r.src_id END,
                      s.rels || r.rel_type,
                      s.depth + 1
                 FROM search s
                 JOIN graph_meta.relationship r
                   ON (r.src_id = s.node_id OR r.dst_id = s.node_id)
                WHERE r.project_id = $1 AND r.valid_to IS NULL AND s.depth < $4
                  AND NOT (CASE WHEN r.src_id = s.node_id THEN r.dst_id ELSE r.src_id END) = ANY(s.path)
             )
             SELECT path, rels FROM search
              WHERE node_id = $3
              ORDER BY depth
              LIMIT 1",
        )
        .bind(project_id)
        .bind(src)
        .bind(dst)
        .bind(max_depth)
        .fetch_optional(&mut **tx)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let path: Vec<Uuid> = row.get("path");
        let rels: Vec<String> = row.get("rels");
        let rows = sqlx::query(
            "SELECT id, entity_type, natural_key, name, qualified_name, revision, resolution, properties,
                    projection_seq
               FROM graph_meta.entity
              WHERE project_id = $1 AND id = ANY($2) AND deleted_at IS NULL",
        )
        .bind(project_id)
        .bind(&path)
        .fetch_all(&mut **tx)
        .await?;
        let by_id: std::collections::HashMap<Uuid, GraphEntityRow> = rows
            .iter()
            .map(|row| (row.get("id"), entity_row(row)))
            .collect();
        let nodes = path
            .iter()
            .filter_map(|id| by_id.get(id).cloned())
            .collect();
        Ok(Some(GraphPathRow {
            nodes,
            rel_types: rels,
        }))
    }

    pub async fn graph_traceability(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        entity_id: Uuid,
        depth: i32,
        limit: i64,
    ) -> Result<(Vec<GraphEntityRow>, Vec<GraphEntityRow>), StoreError> {
        let ancestors = sqlx::query(
            "WITH RECURSIVE up(id, depth, path) AS (
               SELECT $2::uuid, 0, ARRAY[$2::uuid]
               UNION ALL
               SELECT r.src_id, u.depth + 1, u.path || r.src_id
                 FROM up u
                 JOIN graph_meta.relationship r ON r.dst_id = u.id
                WHERE r.project_id = $1 AND r.valid_to IS NULL AND u.depth < $3
                  AND NOT r.src_id = ANY(u.path)
             )
             SELECT DISTINCT ON (e.id)
                    e.id, e.entity_type, e.natural_key, e.name, e.qualified_name, e.revision, e.resolution,
                    e.properties, e.projection_seq, u.depth
               FROM up u
               JOIN graph_meta.entity e ON e.id = u.id AND e.project_id = $1 AND e.deleted_at IS NULL
              WHERE u.depth > 0
              ORDER BY e.id, u.depth
              LIMIT $4",
        )
        .bind(project_id)
        .bind(entity_id)
        .bind(depth)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        let descendants = sqlx::query(
            "WITH RECURSIVE down(id, depth, path) AS (
               SELECT $2::uuid, 0, ARRAY[$2::uuid]
               UNION ALL
               SELECT r.dst_id, d.depth + 1, d.path || r.dst_id
                 FROM down d
                 JOIN graph_meta.relationship r ON r.src_id = d.id
                WHERE r.project_id = $1 AND r.valid_to IS NULL AND d.depth < $3
                  AND NOT r.dst_id = ANY(d.path)
             )
             SELECT DISTINCT ON (e.id)
                    e.id, e.entity_type, e.natural_key, e.name, e.qualified_name, e.revision, e.resolution,
                    e.properties, e.projection_seq, d.depth
               FROM down d
               JOIN graph_meta.entity e ON e.id = d.id AND e.project_id = $1 AND e.deleted_at IS NULL
              WHERE d.depth > 0
              ORDER BY e.id, d.depth
              LIMIT $4",
        )
        .bind(project_id)
        .bind(entity_id)
        .bind(depth)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok((
            ancestors.iter().map(entity_row).collect(),
            descendants.iter().map(entity_row).collect(),
        ))
    }

    pub async fn all_graph_entities(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<GraphEntityRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, entity_type, natural_key, name, qualified_name, revision, resolution, properties,
                    projection_seq
               FROM graph_meta.entity
              WHERE project_id = $1 AND deleted_at IS NULL
              ORDER BY id",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(entity_row).collect())
    }

    pub async fn graph_entities_page(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        after_id: Uuid,
        limit: i64,
    ) -> Result<Vec<GraphEntityRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, entity_type, natural_key, name, qualified_name, revision, resolution, properties,
                    projection_seq
               FROM graph_meta.entity
              WHERE project_id = $1 AND deleted_at IS NULL AND id > $2
              ORDER BY id
              LIMIT $3",
        )
        .bind(project_id)
        .bind(after_id)
        .bind(limit.clamp(1, 10_000))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(entity_row).collect())
    }

    pub async fn all_graph_relationships(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<GraphEdgeRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, src_id, dst_id, rel_type, resolution, confidence, revision,
                    projection_seq
               FROM graph_meta.relationship
              WHERE project_id = $1 AND valid_to IS NULL
              ORDER BY src_id, dst_id, rel_type",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| GraphEdgeRow {
                id: row.get("id"),
                src_id: row.get("src_id"),
                dst_id: row.get("dst_id"),
                rel_type: row.get("rel_type"),
                resolution: row.get("resolution"),
                confidence: row.get::<f32, _>("confidence"),
                revision: row.get("revision"),
                projection_seq: row.get("projection_seq"),
            })
            .collect())
    }

    pub async fn graph_relationships_page(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        after_id: Uuid,
        limit: i64,
    ) -> Result<Vec<GraphEdgeRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, src_id, dst_id, rel_type, resolution, confidence, revision,
                    projection_seq
               FROM graph_meta.relationship
              WHERE project_id = $1 AND valid_to IS NULL AND id > $2
              ORDER BY id
              LIMIT $3",
        )
        .bind(project_id)
        .bind(after_id)
        .bind(limit.clamp(1, 10_000))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| GraphEdgeRow {
                id: row.get("id"),
                src_id: row.get("src_id"),
                dst_id: row.get("dst_id"),
                rel_type: row.get("rel_type"),
                resolution: row.get("resolution"),
                confidence: row.get::<f32, _>("confidence"),
                revision: row.get("revision"),
                projection_seq: row.get("projection_seq"),
            })
            .collect())
    }

    pub async fn graph_outbox_head_seq(&self, tx: &mut Tx<'_>) -> Result<i64, StoreError> {
        Ok(
            sqlx::query("SELECT COALESCE(max(seq), 0) AS seq FROM core.outbox_event")
                .fetch_one(&mut **tx)
                .await?
                .get("seq"),
        )
    }

    pub async fn record_projection_build(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        entity_count: i64,
        relationship_count: i64,
    ) -> Result<(), StoreError> {
        let seq: i64 = sqlx::query("SELECT COALESCE(max(seq), 0) AS seq FROM core.outbox_event")
            .fetch_one(&mut **tx)
            .await?
            .get("seq");
        sqlx::query(
            "INSERT INTO graph_meta.projection_build
               (project_id, status, shadow_label, base_event_seq,
                entities_written, relationships_written, drift_rate, finished_at)
             VALUES ($1, 'READY', 'in-place', $2, $3, $4, 0, now())",
        )
        .bind(project_id)
        .bind(seq)
        .bind(entity_count)
        .bind(relationship_count)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn graph_project_counts(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<(i64, i64), StoreError> {
        let row = sqlx::query(
            "SELECT
               (SELECT count(*) FROM graph_meta.entity
                 WHERE project_id = $1 AND deleted_at IS NULL) AS nodes,
               (SELECT count(*) FROM graph_meta.relationship
                 WHERE project_id = $1 AND valid_to IS NULL) AS rels",
        )
        .bind(project_id)
        .fetch_one(&mut **tx)
        .await?;
        Ok((row.get("nodes"), row.get("rels")))
    }

    pub async fn list_project_ids(&self, tx: &mut Tx<'_>) -> Result<Vec<Uuid>, StoreError> {
        let rows = sqlx::query("SELECT id FROM core.project ORDER BY id")
            .fetch_all(&mut **tx)
            .await?;
        Ok(rows.iter().map(|row| row.get("id")).collect())
    }

    pub async fn pending_event_count(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<i64, StoreError> {
        let row = sqlx::query(
            "SELECT count(*) AS n FROM core.outbox_event
              WHERE published_at IS NULL AND NOT dead_letter
                AND payload->>'project_id' = $1",
        )
        .bind(project_id.to_string())
        .fetch_one(&mut **tx)
        .await?;
        Ok(row.get("n"))
    }

    pub async fn insert_projection_drift(
        &self,
        tx: &mut Tx<'_>,
        drift: &NewProjectionDrift,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO graph_meta.projection_drift
               (project_id, pg_nodes, pg_relationships, neo4j_nodes,
                neo4j_relationships, drift_rate, degraded)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(drift.project_id)
        .bind(drift.pg_nodes)
        .bind(drift.pg_relationships)
        .bind(drift.neo4j_nodes)
        .bind(drift.neo4j_relationships)
        .bind(drift.drift_rate)
        .bind(drift.degraded)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn latest_projection_drift(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Option<(DateTime<Utc>, f64, bool, i64, i64, i64, i64)>, StoreError> {
        let row = sqlx::query(
            "SELECT checked_at, drift_rate::float8 AS drift_rate, degraded, pg_nodes,
                    pg_relationships, neo4j_nodes, neo4j_relationships
               FROM graph_meta.projection_drift
              WHERE project_id = $1
              ORDER BY checked_at DESC, id DESC
              LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(|row| {
            (
                row.get("checked_at"),
                row.get("drift_rate"),
                row.get("degraded"),
                row.get("pg_nodes"),
                row.get("pg_relationships"),
                row.get("neo4j_nodes"),
                row.get("neo4j_relationships"),
            )
        }))
    }

    pub async fn create_projection_build(
        &self,
        tx: &mut Tx<'_>,
        build_id: Uuid,
        project_id: Uuid,
        shadow_label: &str,
        base_event_seq: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO graph_meta.projection_build
               (build_id, project_id, status, shadow_label, base_event_seq)
             VALUES ($1, $2, 'BUILDING', $3, $4)",
        )
        .bind(build_id)
        .bind(project_id)
        .bind(shadow_label)
        .bind(base_event_seq)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn update_projection_build(
        &self,
        tx: &mut Tx<'_>,
        build_id: Uuid,
        status: &str,
        entities_written: i64,
        relationships_written: i64,
        drift_rate: Option<f64>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE graph_meta.projection_build
                SET status = $2,
                    entities_written = $3,
                    relationships_written = $4,
                    drift_rate = $5,
                    finished_at = CASE
                        WHEN $2 IN ('SWITCHED','FAILED','ABORTED','READY') THEN now()
                        ELSE finished_at
                    END
              WHERE build_id = $1",
        )
        .bind(build_id)
        .bind(status)
        .bind(entities_written)
        .bind(relationships_written)
        .bind(drift_rate)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn projection_build_status(
        &self,
        tx: &mut Tx<'_>,
        build_id: Uuid,
    ) -> Result<Option<String>, StoreError> {
        let row = sqlx::query("SELECT status FROM graph_meta.projection_build WHERE build_id = $1")
            .bind(build_id)
            .fetch_optional(&mut **tx)
            .await?;
        Ok(row.map(|row| row.get("status")))
    }

    pub async fn latest_projection_build(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<
        Option<(
            Uuid,
            String,
            String,
            Option<f64>,
            i64,
            i64,
            Option<DateTime<Utc>>,
        )>,
        StoreError,
    > {
        let row = sqlx::query(
            "SELECT build_id, status, shadow_label, drift_rate::float8 AS drift_rate,
                    entities_written, relationships_written, finished_at
               FROM graph_meta.projection_build
              WHERE project_id = $1
              ORDER BY started_at DESC
              LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(|row| {
            (
                row.get("build_id"),
                row.get("status"),
                row.get("shadow_label"),
                row.get::<Option<f64>, _>("drift_rate"),
                row.get("entities_written"),
                row.get("relationships_written"),
                row.get("finished_at"),
            )
        }))
    }

    pub async fn abort_projection_build(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Option<(Uuid, String)>, StoreError> {
        let row = sqlx::query(
            "UPDATE graph_meta.projection_build
                SET status = 'ABORTED', finished_at = now()
              WHERE project_id = $1 AND status IN ('BUILDING','VERIFYING','READY')
              RETURNING build_id, shadow_label",
        )
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|row| (row.get("build_id"), row.get("shadow_label"))))
    }

    pub async fn graph_events_after(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<OutboxEvent>, StoreError> {
        let rows = sqlx::query(
            "SELECT event_id, seq, event_type, payload
               FROM core.outbox_event
              WHERE seq > $2 AND payload->>'project_id' = $1
              ORDER BY seq
              LIMIT $3",
        )
        .bind(project_id.to_string())
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| OutboxEvent {
                event_id: row.get("event_id"),
                seq: row.get("seq"),
                event_type: row.get("event_type"),
                payload: row.get("payload"),
            })
            .collect())
    }

    pub async fn published_ontology_version(&self, tx: &mut Tx<'_>) -> Result<String, StoreError> {
        let row = sqlx::query(
            "SELECT version FROM graph_meta.ontology_version
              WHERE status = 'PUBLISHED'
              ORDER BY published_at DESC NULLS LAST
              LIMIT 1",
        )
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row
            .map(|row| row.get("version"))
            .unwrap_or_else(|| "v1".to_string()))
    }

    pub async fn graph_last_event_seq(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<i64, StoreError> {
        let row = sqlx::query(
            "SELECT last_event_seq FROM graph_meta.projection_state WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|row| row.get("last_event_seq")).unwrap_or(0))
    }
}

#[derive(Debug, Clone)]
pub struct OntologyRow {
    pub version: String,
    pub status: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub entity_types: Value,
    pub relation_types: Value,
}

impl Store {
    pub async fn published_ontology(
        &self,
        tx: &mut Tx<'_>,
    ) -> Result<Option<OntologyRow>, StoreError> {
        let row = sqlx::query(
            "SELECT v.version, v.status, v.published_at,
                    COALESCE((SELECT json_agg(json_build_object(
                       'name', t.name, 'display_name_ja', t.display_name_ja,
                       'parent_type', t.parent_type, 'is_abstract', t.is_abstract,
                       'properties_schema', t.properties_schema) ORDER BY t.name)
                       FROM graph_meta.entity_type t WHERE t.ontology_version_id = v.id), '[]') AS entity_types,
                    COALESCE((SELECT json_agg(json_build_object(
                       'name', r.name, 'display_name_ja', r.display_name_ja,
                       'source_types', r.source_types, 'target_types', r.target_types,
                       'cardinality', r.cardinality, 'is_transitive', r.is_transitive) ORDER BY r.name)
                       FROM graph_meta.relation_type r WHERE r.ontology_version_id = v.id), '[]') AS relation_types
               FROM graph_meta.ontology_version v
              WHERE v.status = 'PUBLISHED'
              ORDER BY v.published_at DESC NULLS LAST
              LIMIT 1",
        )
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|row| OntologyRow {
            version: row.get("version"),
            status: row.get("status"),
            published_at: row.get("published_at"),
            entity_types: row.get("entity_types"),
            relation_types: row.get("relation_types"),
        }))
    }

    pub async fn publish_ontology(
        &self,
        tx: &mut Tx<'_>,
        version: &str,
        user_id: Uuid,
    ) -> Result<(i64, i64), StoreError> {
        let row = sqlx::query(
            "SELECT id FROM graph_meta.ontology_version
              WHERE status = 'PUBLISHED'
              ORDER BY published_at DESC NULLS LAST LIMIT 1",
        )
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| StoreError::Pool("published ontology not found".into()))?;
        let source: Uuid = row.get("id");
        sqlx::query(
            "UPDATE graph_meta.ontology_version SET status = 'DEPRECATED'
              WHERE status = 'PUBLISHED'",
        )
        .execute(&mut **tx)
        .await?;
        let row = sqlx::query(
            "INSERT INTO graph_meta.ontology_version
               (version, status, published_at, published_by)
             VALUES ($1, 'PUBLISHED', now(), $2)
             RETURNING id",
        )
        .bind(version)
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
        let target: Uuid = row.get("id");
        sqlx::query(
            "INSERT INTO graph_meta.entity_type
               (ontology_version_id, name, display_name_ja, parent_type, is_abstract, properties_schema)
             SELECT $2, name, display_name_ja, parent_type, is_abstract, properties_schema
               FROM graph_meta.entity_type WHERE ontology_version_id = $1",
        )
        .bind(source)
        .bind(target)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO graph_meta.relation_type
               (ontology_version_id, name, display_name_ja, source_types, target_types,
                cardinality, is_transitive, properties_schema)
             SELECT $2, name, display_name_ja, source_types, target_types,
                    cardinality, is_transitive, properties_schema
               FROM graph_meta.relation_type WHERE ontology_version_id = $1",
        )
        .bind(source)
        .bind(target)
        .execute(&mut **tx)
        .await?;
        let counts = sqlx::query(
            "SELECT
               (SELECT count(*) FROM graph_meta.entity_type WHERE ontology_version_id = $1) AS entities,
               (SELECT count(*) FROM graph_meta.relation_type WHERE ontology_version_id = $1) AS relations",
        )
        .bind(target)
        .fetch_one(&mut **tx)
        .await?;
        Ok((counts.get("entities"), counts.get("relations")))
    }
}

fn entity_row(row: &sqlx::postgres::PgRow) -> GraphEntityRow {
    GraphEntityRow {
        id: row.get("id"),
        entity_type: row.get("entity_type"),
        natural_key: row.get("natural_key"),
        name: row.get("name"),
        qualified_name: row.get("qualified_name"),
        revision: row.get("revision"),
        resolution: row.get("resolution"),
        properties: row.get("properties"),
        projection_seq: row.get("projection_seq"),
    }
}

fn entity_type_for(node_type: &str) -> Option<&'static str> {
    match node_type {
        "DOCUMENT" => Some("Document"),
        "SHEET" => Some("Sheet"),
        "SECTION" => Some("Section"),
        "REGION" => Some("Region"),
        "TABLE" => Some("Table"),
        _ => None,
    }
}

fn node_name(node: &NodeDraft) -> String {
    if let Some(text) = node.text.as_deref().filter(|text| !text.trim().is_empty()) {
        return text.trim().to_string();
    }
    for key in ["title", "sheet", "range", "anchor_cell", "name"] {
        if let Some(value) = node.locator.get(key).and_then(Value::as_str) {
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    if let Some(path) = node.locator.get("section_path").and_then(Value::as_array) {
        let parts: Vec<&str> = path.iter().filter_map(Value::as_str).collect();
        if !parts.is_empty() {
            return parts.join(" / ");
        }
    }
    if let Some(page) = node.locator.get("page").and_then(Value::as_i64) {
        return format!("{} p.{page}", node.node_type);
    }
    format!("{} #{}", node.node_type, node.ordinal + 1)
}

impl Store {
    /// 改版切替: 旧 revision のグラフ実体を tombstone し、
    /// projector 向けに `DocumentRevisionSuperseded` を積む。
    pub async fn supersede_document_revision(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        document_id: Uuid,
        old_revision: Uuid,
        new_revision: Uuid,
    ) -> Result<u64, StoreError> {
        let result = sqlx::query(
            "UPDATE graph_meta.entity
                SET deleted_at = now()
              WHERE project_id = $1 AND revision = $2
                AND properties->>'document_id' = $3",
        )
        .bind(project_id)
        .bind(old_revision.to_string())
        .bind(document_id.to_string())
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE graph_meta.relationship r
                SET valid_to = now()
              WHERE r.project_id = $1 AND r.valid_to IS NULL
                AND (r.src_id IN (SELECT id FROM graph_meta.entity
                                   WHERE project_id = $1 AND revision = $2
                                     AND properties->>'document_id' = $3)
                  OR r.dst_id IN (SELECT id FROM graph_meta.entity
                                   WHERE project_id = $1 AND revision = $2
                                     AND properties->>'document_id' = $3))",
        )
        .bind(project_id)
        .bind(old_revision.to_string())
        .bind(document_id.to_string())
        .execute(&mut **tx)
        .await?;
        let payload = json!({
            "project_id": project_id,
            "document_id": document_id,
            "old_revision": old_revision.to_string(),
            "new_revision": new_revision.to_string(),
            "seq": 0,
        });
        self.append_graph_event(
            tx,
            "revision",
            document_id,
            "DocumentRevisionSuperseded",
            &payload,
        )
        .await?;
        Ok(result.rows_affected())
    }
}
