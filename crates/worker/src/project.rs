use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use app::App;
use domain::ProjectId;
use engine_graph::{EntityProjection, Neo4jProjector, RelationshipProjection};
use serde_json::json;
use uuid::Uuid;

static PROJECTOR: OnceLock<Option<Neo4jProjector>> = OnceLock::new();
static SCHEMA_READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
static WORKER_ID: OnceLock<String> = OnceLock::new();

fn worker_id() -> &'static str {
    WORKER_ID.get_or_init(|| {
        std::env::var("WORKER_ID")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string())
    })
}

fn projector() -> Option<&'static Neo4jProjector> {
    PROJECTOR
        .get_or_init(|| {
            if std::env::var("NEO4J_DISABLED").ok().as_deref() == Some("1") {
                return None;
            }
            let uri =
                std::env::var("NEO4J_URI").unwrap_or_else(|_| "bolt://localhost:7687".to_string());
            let user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
            let password = std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j".to_string());
            let database = std::env::var("NEO4J_DATABASE").unwrap_or_else(|_| "neo4j".to_string());
            match Neo4jProjector::new(&uri, &user, &password, &database) {
                Ok(projector) => Some(projector),
                Err(error) => {
                    tracing::warn!(%error, "Neo4j projector disabled");
                    None
                }
            }
        })
        .as_ref()
}

pub async fn drain(app: &App) -> Result<(), app::AppError> {
    let Some(projector) = projector() else {
        return Ok(());
    };
    if SCHEMA_READY
        .get_or_try_init(|| async { projector.ensure_schema().await.map_err(|e| e.to_string()) })
        .await
        .is_err()
    {
        return Ok(());
    }

    let scope = ProjectId(Uuid::nil());
    let claim_limit = std::env::var("GRAPH_CLAIM_LIMIT")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(5_000)
        .clamp(1, 5_000);
    let mut tx = app.store().begin_scoped(scope).await?;
    let events = app
        .store()
        .claim_graph_events(&mut tx, worker_id(), claim_limit)
        .await?;
    app::commit_tx(tx).await?;
    if events.is_empty() {
        return Ok(());
    }

    let mut published = Vec::<Uuid>::new();
    let mut failed = Vec::new();
    let mut entities = Vec::<(Uuid, EntityProjection)>::new();
    let mut relationships: BTreeMap<String, Vec<(Uuid, RelationshipProjection)>> = BTreeMap::new();
    for event in &events {
        match event.event_type.as_str() {
            "DocumentNodeUpserted" | "EntityUpserted" => match entity_projection(&event.payload) {
                Ok(entity) => entities.push((event.event_id, entity)),
                Err(error) => failed.push((event.event_id, error)),
            },
            "RelationshipUpserted" => match relationship_projection(&event.payload) {
                Ok((rel_type, relationship)) => relationships
                    .entry(rel_type)
                    .or_default()
                    .push((event.event_id, relationship)),
                Err(error) => failed.push((event.event_id, error)),
            },
            _ => {}
        }
    }

    for chunk in entities.chunks(1_000) {
        let rows = chunk
            .iter()
            .map(|(_, entity)| entity.clone())
            .collect::<Vec<_>>();
        match projector.apply_entities(&rows).await {
            Ok(()) => published.extend(chunk.iter().map(|(event_id, _)| *event_id)),
            Err(error) => failed.extend(
                chunk
                    .iter()
                    .map(|(event_id, _)| (*event_id, error.to_string())),
            ),
        }
    }
    for (rel_type, rows) in &relationships {
        for chunk in rows.chunks(1_000) {
            let projections = chunk
                .iter()
                .map(|(_, relationship)| relationship.clone())
                .collect::<Vec<_>>();
            match projector.apply_relationships(rel_type, &projections).await {
                Ok(()) => published.extend(chunk.iter().map(|(event_id, _)| *event_id)),
                Err(error) => failed.extend(
                    chunk
                        .iter()
                        .map(|(event_id, _)| (*event_id, error.to_string())),
                ),
            }
        }
    }
    for event in &events {
        if matches!(
            event.event_type.as_str(),
            "DocumentNodeUpserted"
                | "EntityUpserted"
                | "RelationshipUpserted"
                | "ProjectionTombstone"
                | "DocumentRevisionSuperseded"
        ) {
            continue;
        }
        failed.push((
            event.event_id,
            format!("未対応のイベント型です: {}", event.event_type),
        ));
    }
    for event in &events {
        if event.event_type != "ProjectionTombstone"
            && event.event_type != "DocumentRevisionSuperseded"
        {
            continue;
        }
        match projector
            .apply(&event.event_type, &event.payload, event.seq, event.event_id)
            .await
        {
            Ok(()) => published.push(event.event_id),
            Err(error) => failed.push((event.event_id, error.to_string())),
        }
    }

    let failed_ids = failed
        .iter()
        .map(|(event_id, _)| *event_id)
        .collect::<BTreeSet<_>>();
    let failed_projects = events
        .iter()
        .filter(|event| failed_ids.contains(&event.event_id))
        .filter_map(|event| project_id_from_payload(&event.payload))
        .collect::<BTreeSet<_>>();
    let mut states = BTreeMap::<Uuid, (i64, Uuid)>::new();
    for event in &events {
        if failed_ids.contains(&event.event_id) {
            continue;
        }
        let Some(project_id) = event
            .payload
            .get("project_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|raw| raw.parse::<Uuid>().ok())
        else {
            continue;
        };
        if failed_projects.contains(&project_id) {
            continue;
        }
        let state = states
            .entry(project_id)
            .or_insert((event.seq, event.event_id));
        if event.seq > state.0 {
            *state = (event.seq, event.event_id);
        }
    }
    for (&project_id, &(seq, event_id)) in &states {
        if let Err(error) = projector
            .apply_projection_state(project_id, seq, event_id)
            .await
        {
            tracing::warn!(%project_id, %error, "projection state mirror failed");
        }
    }

    let mut tx = app.store().begin_scoped(scope).await?;
    for event_id in &published {
        app.store().mark_event_published(&mut tx, *event_id).await?;
    }
    for (event_id, error) in &failed {
        let dead = app
            .store()
            .fail_event(&mut tx, *event_id, &json!({"message": error}))
            .await?;
        if dead {
            tracing::error!(%event_id, error, "outbox event moved to dead letter");
        }
    }
    app::commit_tx(tx).await?;

    // projection_state は RLS のテナント方針で守られるため、
    // プロジェクトごとにスコープを設定した tx で書く。
    for (project_id, (seq, event_id)) in states {
        let mut tx = app.store().begin_scoped(ProjectId(project_id)).await?;
        app.store()
            .update_projection_state(&mut tx, project_id, seq, event_id)
            .await?;
        app::commit_tx(tx).await?;
    }

    tracing::info!(
        published = published.len(),
        failed = failed.len(),
        "graph outbox projected"
    );
    Ok(())
}

fn project_id_from_payload(payload: &serde_json::Value) -> Option<Uuid> {
    payload
        .get("project_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse().ok())
}

fn entity_projection(payload: &serde_json::Value) -> Result<EntityProjection, String> {
    Ok(EntityProjection {
        entity_id: payload_uuid(payload, "entity_id")?,
        uid: payload_str(payload, "uid")?.to_string(),
        project_id: payload_uuid(payload, "project_id")?,
        entity_type: payload_str(payload, "entity_type")?.to_string(),
        natural_key: payload
            .get("natural_key")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name: payload_str(payload, "name")?.to_string(),
        qualified_name: payload
            .get("qualified_name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        revision: payload_str(payload, "revision")?.to_string(),
        resolution: payload
            .get("resolution")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("RESOLVED")
            .to_string(),
        seq: payload
            .get("seq")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
    })
}

fn relationship_projection(
    payload: &serde_json::Value,
) -> Result<(String, RelationshipProjection), String> {
    let rel_type = payload_str(payload, "rel_type")?.to_string();
    Ok((
        rel_type,
        RelationshipProjection {
            rel_id: payload_uuid(payload, "rel_id")?,
            src_uid: payload_str(payload, "src_uid")?.to_string(),
            dst_uid: payload_str(payload, "dst_uid")?.to_string(),
            project_id: payload_uuid(payload, "project_id")?,
            resolution: payload
                .get("resolution")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("RESOLVED")
                .to_string(),
            confidence: payload
                .get("confidence")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(1.0) as f32,
            revision: payload_str(payload, "revision")?.to_string(),
            seq: payload
                .get("seq")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
        },
    ))
}

fn payload_str<'a>(payload: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} がありません"))
}

fn payload_uuid(payload: &serde_json::Value, key: &str) -> Result<Uuid, String> {
    payload_str(payload, key)?
        .parse()
        .map_err(|_| format!("{key} が UUID ではありません"))
}
