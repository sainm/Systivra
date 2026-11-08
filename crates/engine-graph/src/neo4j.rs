use std::collections::HashMap;
use std::sync::Arc;

use neo4j::address::Address;
use neo4j::driver::auth::AuthToken;
use neo4j::driver::{ConnectionConfig, Driver, DriverConfig};
use neo4j::retry::ExponentialBackoff;
use neo4j::{value_map, ValueReceive, ValueSend};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("Neo4j URI が不正です: {0}")]
    Uri(String),
    #[error("Neo4j エラー: {0}")]
    Neo4j(String),
    #[error("イベント payload が不正です: {0}")]
    Payload(String),
}

pub struct Neo4jProjector {
    driver: Driver,
    database: Arc<String>,
}

#[derive(Debug, Clone)]
pub struct EntityProjection {
    pub entity_id: Uuid,
    pub uid: String,
    pub project_id: Uuid,
    pub entity_type: String,
    pub natural_key: String,
    pub name: String,
    pub qualified_name: Option<String>,
    pub revision: String,
    pub resolution: String,
    pub seq: i64,
}

#[derive(Debug, Clone)]
pub struct RelationshipProjection {
    pub rel_id: Uuid,
    pub src_uid: String,
    pub dst_uid: String,
    pub project_id: Uuid,
    pub resolution: String,
    pub confidence: f32,
    pub revision: String,
    pub seq: i64,
}

impl Neo4jProjector {
    pub fn new(uri: &str, user: &str, password: &str, database: &str) -> Result<Self, GraphError> {
        let (host, port) = parse_uri(uri)?;
        let address = Address::from((host.as_str(), port));
        let auth = AuthToken::new_basic_auth(user, password);
        let driver = Driver::new(
            ConnectionConfig::new(address),
            DriverConfig::new().with_auth(Arc::new(auth)),
        );
        Ok(Self {
            driver,
            database: Arc::new(database.to_string()),
        })
    }

    pub async fn ensure_schema(&self) -> Result<(), GraphError> {
        for query in [
            "CREATE CONSTRAINT entity_uid IF NOT EXISTS FOR (e:Entity) REQUIRE e.uid IS UNIQUE",
            "CREATE INDEX entity_lookup IF NOT EXISTS FOR (e:Entity) ON (e.project_id, e.entity_type, e.name)",
            "CREATE INDEX entity_qname IF NOT EXISTS FOR (e:Entity) ON (e.project_id, e.qualified_name)",
        ] {
            self.run(query, value_map!()).await?;
        }
        Ok(())
    }

    pub async fn apply(
        &self,
        event_type: &str,
        payload: &Value,
        outbox_seq: i64,
        event_id: Uuid,
    ) -> Result<(), GraphError> {
        match event_type {
            "DocumentNodeUpserted" | "EntityUpserted" => {
                let uid = required_str(payload, "uid")?;
                let entity_id = required_str(payload, "entity_id")?;
                let project_id = required_str(payload, "project_id")?;
                let entity_type = required_str(payload, "entity_type")?;
                let natural_key = payload
                    .get("natural_key")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let name = required_str(payload, "name")?;
                let revision = required_str(payload, "revision")?;
                let resolution = payload
                    .get("resolution")
                    .and_then(Value::as_str)
                    .unwrap_or("RESOLVED");
                let seq = payload.get("seq").and_then(Value::as_i64).unwrap_or(0);
                let qualified_name = payload
                    .get("qualified_name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                self.run(
                    "MERGE (e:Entity {uid: $uid})
                     ON CREATE SET e.projection_seq = $seq
                     WITH e
                     WHERE e.projection_seq <= $seq
                     SET e.entity_id = $entity_id,
                         e.project_id = $project_id,
                         e.entity_type = $entity_type,
                         e.natural_key = $natural_key,
                         e.name = $name,
                         e.qualified_name = $qualified_name,
                         e.revision = $revision,
                         e.resolution = $resolution,
                         e.projection_seq = $seq,
                         e.deleted_at = null",
                    value_map!({
                        "uid": uid,
                        "entity_id": entity_id,
                        "project_id": project_id,
                        "entity_type": entity_type,
                        "natural_key": natural_key,
                        "name": name,
                        "qualified_name": qualified_name,
                        "revision": revision,
                        "resolution": resolution,
                        "seq": seq,
                    }),
                )
                .await?;
            }
            "DocumentRevisionSuperseded" => {
                let project_id = required_str(payload, "project_id")?;
                let old_revision = required_str(payload, "old_revision")?;
                self.run(
                    "MATCH (e:Entity {project_id: $project_id, revision: $old_revision})
                     DETACH DELETE e",
                    value_map!({
                        "project_id": project_id,
                        "old_revision": old_revision,
                    }),
                )
                .await?;
            }
            "RelationshipUpserted" => {
                let rel_id = required_str(payload, "rel_id")?;
                let src_uid = required_str(payload, "src_uid")?;
                let dst_uid = required_str(payload, "dst_uid")?;
                let rel_type = required_str(payload, "rel_type")?;
                if !rel_type
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch == '_')
                {
                    return Err(GraphError::Payload(format!(
                        "関係型が識別子ではありません: {rel_type}"
                    )));
                }
                let project_id = required_str(payload, "project_id")?;
                let resolution = payload
                    .get("resolution")
                    .and_then(Value::as_str)
                    .unwrap_or("RESOLVED");
                let revision = required_str(payload, "revision")?;
                let confidence = payload
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0);
                let seq = payload.get("seq").and_then(Value::as_i64).unwrap_or(0);
                let query = format!(
                    "MATCH (a:Entity {{uid: $src_uid}}), (b:Entity {{uid: $dst_uid}})
                     MERGE (a)-[r:{rel_type} {{rel_id: $rel_id}}]->(b)
                     ON CREATE SET r.projection_seq = $seq
                     WITH r
                     WHERE r.projection_seq <= $seq
                     SET r.project_id = $project_id,
                         r.resolution = $resolution,
                         r.confidence = $confidence,
                         r.revision = $revision,
                         r.projection_seq = $seq"
                );
                self.run(
                    &query,
                    value_map!({
                        "rel_id": rel_id,
                        "src_uid": src_uid,
                        "dst_uid": dst_uid,
                        "project_id": project_id,
                        "resolution": resolution,
                        "revision": revision,
                        "confidence": confidence,
                        "seq": seq,
                    }),
                )
                .await?;
            }
            "ProjectionTombstone" => {
                let uid = required_str(payload, "uid")?;
                self.run(
                    "MATCH (e:Entity {uid: $uid}) DETACH DELETE e",
                    value_map!({"uid": uid}),
                )
                .await?;
            }
            other => {
                return Err(GraphError::Payload(format!(
                    "未対応のイベント型です: {other}"
                )));
            }
        }

        let project_id = payload
            .get("project_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !project_id.is_empty() {
            self.run(
                "MERGE (s:ProjectionState {project_id: $project_id})
                 SET s.last_event_seq = $seq, s.last_event_id = $event_id,
                     s.updated_at = datetime()",
                value_map!({
                    "project_id": project_id,
                    "seq": outbox_seq,
                    "event_id": event_id.to_string(),
                }),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn apply_entities(&self, entities: &[EntityProjection]) -> Result<(), GraphError> {
        self.apply_entities_as("Entity", entities).await
    }

    pub async fn apply_entities_as(
        &self,
        label: &str,
        entities: &[EntityProjection],
    ) -> Result<(), GraphError> {
        label_ident(label)?;
        if entities.is_empty() {
            return Ok(());
        }
        let rows = entities
            .iter()
            .map(|entity| {
                HashMap::<String, ValueSend>::from([
                    ("entity_id".into(), entity.entity_id.to_string().into()),
                    ("uid".into(), entity.uid.clone().into()),
                    ("project_id".into(), entity.project_id.to_string().into()),
                    ("entity_type".into(), entity.entity_type.clone().into()),
                    ("natural_key".into(), entity.natural_key.clone().into()),
                    ("name".into(), entity.name.clone().into()),
                    (
                        "qualified_name".into(),
                        entity.qualified_name.clone().into(),
                    ),
                    ("revision".into(), entity.revision.clone().into()),
                    ("resolution".into(), entity.resolution.clone().into()),
                    ("seq".into(), entity.seq.into()),
                ])
            })
            .collect::<Vec<_>>();
        self.run(
            &format!(
                "UNWIND $rows AS row
             MERGE (e:{label} {{uid: row.uid}})
             ON CREATE SET e.projection_seq = row.seq
             WITH e, row
             WHERE e.projection_seq <= row.seq
             SET e.entity_id = row.entity_id,
                 e.project_id = row.project_id,
                 e.entity_type = row.entity_type,
                 e.natural_key = row.natural_key,
                 e.name = row.name,
                 e.qualified_name = row.qualified_name,
                 e.revision = row.revision,
                 e.resolution = row.resolution,
                 e.projection_seq = row.seq,
                 e.deleted_at = null"
            ),
            value_map!({"rows": rows}),
        )
        .await
    }

    pub async fn apply_relationships(
        &self,
        rel_type: &str,
        relationships: &[RelationshipProjection],
    ) -> Result<(), GraphError> {
        self.apply_relationships_as("Entity", rel_type, relationships)
            .await
    }

    pub async fn apply_relationships_as(
        &self,
        label: &str,
        rel_type: &str,
        relationships: &[RelationshipProjection],
    ) -> Result<(), GraphError> {
        label_ident(label)?;
        if relationships.is_empty() {
            return Ok(());
        }
        if !rel_type
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch == '_')
        {
            return Err(GraphError::Payload(format!(
                "関係型が識別子ではありません: {rel_type}"
            )));
        }
        let rows = relationships
            .iter()
            .map(|relationship| {
                HashMap::<String, ValueSend>::from([
                    ("rel_id".into(), relationship.rel_id.to_string().into()),
                    ("src_uid".into(), relationship.src_uid.clone().into()),
                    ("dst_uid".into(), relationship.dst_uid.clone().into()),
                    (
                        "project_id".into(),
                        relationship.project_id.to_string().into(),
                    ),
                    ("resolution".into(), relationship.resolution.clone().into()),
                    ("confidence".into(), relationship.confidence.into()),
                    ("revision".into(), relationship.revision.clone().into()),
                    ("seq".into(), relationship.seq.into()),
                ])
            })
            .collect::<Vec<_>>();
        let query = format!(
            "UNWIND $rows AS row
             MATCH (a:{label} {{uid: row.src_uid}}), (b:{label} {{uid: row.dst_uid}})
             MERGE (a)-[r:{rel_type} {{rel_id: row.rel_id}}]->(b)
             ON CREATE SET r.projection_seq = row.seq
             WITH r, row
             WHERE r.projection_seq <= row.seq
             SET r.project_id = row.project_id,
                 r.resolution = row.resolution,
                 r.confidence = row.confidence,
                 r.revision = row.revision,
                 r.projection_seq = row.seq"
        );
        self.run(&query, value_map!({"rows": rows})).await
    }

    pub async fn count_label(
        &self,
        project_id: &str,
        label: &str,
    ) -> Result<(i64, i64), GraphError> {
        label_ident(label)?;
        let result = self
            .driver
            .execute_query(&format!(
                "MATCH (e:{label} {{project_id: $project_id}})
                 OPTIONAL MATCH (e)-[r]->(:{label} {{project_id: $project_id}})
                 RETURN count(DISTINCT e) AS nodes, count(DISTINCT r) AS rels"
            ))
            .with_database(self.database.clone())
            .with_parameters(value_map!({"project_id": project_id}))
            .run_with_retry(ExponentialBackoff::default())
            .map_err(|error| GraphError::Neo4j(error.to_string()))?;
        let mut record = result
            .records
            .into_iter()
            .next()
            .ok_or_else(|| GraphError::Neo4j("集計結果が空です".into()))?;
        let nodes = match record.take_value("nodes") {
            Some(ValueReceive::Integer(value)) => value,
            _ => 0,
        };
        let rels = match record.take_value("rels") {
            Some(ValueReceive::Integer(value)) => value,
            _ => 0,
        };
        Ok((nodes, rels))
    }

    pub async fn switch_label(
        &self,
        project_id: &str,
        shadow_label: &str,
        old_label: &str,
    ) -> Result<(), GraphError> {
        label_ident(shadow_label)?;
        label_ident(old_label)?;
        self.run(
            &format!(
                "MATCH (o:Entity {{project_id: $project_id}})
                 SET o:{old_label}
                 REMOVE o:Entity
                 WITH count(o) AS old_nodes
                 MATCH (n:{shadow_label} {{project_id: $project_id}})
                 SET n:Entity
                 REMOVE n:{shadow_label}
                 RETURN old_nodes, count(n) AS new_nodes"
            ),
            value_map!({"project_id": project_id}),
        )
        .await
    }

    pub async fn delete_label(&self, project_id: &str, label: &str) -> Result<(), GraphError> {
        label_ident(label)?;
        self.run(
            &format!("MATCH (n:{label} {{project_id: $project_id}}) DETACH DELETE n"),
            value_map!({"project_id": project_id}),
        )
        .await
    }

    pub async fn apply_projection_state(
        &self,
        project_id: Uuid,
        outbox_seq: i64,
        event_id: Uuid,
    ) -> Result<(), GraphError> {
        self.run(
            "MERGE (s:ProjectionState {project_id: $project_id})
             SET s.last_event_seq = $seq, s.last_event_id = $event_id,
                 s.updated_at = datetime()",
            value_map!({
                "project_id": project_id.to_string(),
                "seq": outbox_seq,
                "event_id": event_id.to_string(),
            }),
        )
        .await
    }

    pub async fn count_project(&self, project_id: &str) -> Result<(i64, i64), GraphError> {
        let result = self
            .driver
            .execute_query(
                "MATCH (e:Entity {project_id: $project_id})
                 OPTIONAL MATCH (e)-[r]->(:Entity {project_id: $project_id})
                 RETURN count(DISTINCT e) AS nodes, count(DISTINCT r) AS rels",
            )
            .with_database(self.database.clone())
            .with_parameters(value_map!({"project_id": project_id}))
            .run_with_retry(ExponentialBackoff::default())
            .map_err(|error| GraphError::Neo4j(error.to_string()))?;
        let mut record = result
            .records
            .into_iter()
            .next()
            .ok_or_else(|| GraphError::Neo4j("集計結果が空です".into()))?;
        let nodes = match record.take_value("nodes") {
            Some(ValueReceive::Integer(value)) => value,
            _ => 0,
        };
        let rels = match record.take_value("rels") {
            Some(ValueReceive::Integer(value)) => value,
            _ => 0,
        };
        Ok((nodes, rels))
    }

    pub async fn clear_project(&self, project_id: &str) -> Result<(), GraphError> {
        self.run(
            "MATCH (e:Entity {project_id: $project_id}) DETACH DELETE e",
            value_map!({"project_id": project_id}),
        )
        .await?;
        self.run(
            "MATCH (s:ProjectionState {project_id: $project_id}) DETACH DELETE s",
            value_map!({"project_id": project_id}),
        )
        .await?;
        Ok(())
    }

    pub async fn health(&self) -> Result<(), GraphError> {
        self.run("RETURN 1", value_map!()).await?;
        Ok(())
    }

    async fn run(
        &self,
        query: &str,
        parameters: HashMap<String, neo4j::ValueSend>,
    ) -> Result<(), GraphError> {
        self.driver
            .execute_query(query)
            .with_database(self.database.clone())
            .with_parameters(parameters)
            .run_with_retry(ExponentialBackoff::default())
            .map(|_| ())
            .map_err(|error| GraphError::Neo4j(error.to_string()))
    }
}

fn required_str<'a>(payload: &'a Value, key: &str) -> Result<&'a str, GraphError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| GraphError::Payload(format!("{key} がありません")))
}

fn parse_uri(uri: &str) -> Result<(String, u16), GraphError> {
    let rest = uri
        .strip_prefix("bolt://")
        .or_else(|| uri.strip_prefix("neo4j://"))
        .unwrap_or(uri);
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>()
                .map_err(|_| GraphError::Uri(uri.to_string()))?,
        ),
        None => (rest, 7687),
    };
    if host.is_empty() {
        return Err(GraphError::Uri(uri.to_string()));
    }
    Ok((host.to_string(), port))
}

fn label_ident(label: &str) -> Result<&str, GraphError> {
    if !label.is_empty()
        && label
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        Ok(label)
    } else {
        Err(GraphError::Payload(format!(
            "ラベルが識別子ではありません: {label}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bolt_and_plain_uris() {
        assert_eq!(
            parse_uri("bolt://localhost:7687").unwrap(),
            ("localhost".into(), 7687)
        );
        assert_eq!(parse_uri("neo4j://db:7688").unwrap(), ("db".into(), 7688));
        assert_eq!(parse_uri("localhost").unwrap(), ("localhost".into(), 7687));
    }
}
