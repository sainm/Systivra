use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphEntity {
    pub id: Uuid,
    pub entity_type: String,
    pub name: String,
    pub qualified_name: Option<String>,
    pub revision: String,
    pub resolution: String,
    pub properties: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphEdge {
    pub src_id: Uuid,
    pub dst_id: Uuid,
    pub rel_type: String,
    pub resolution: String,
    pub confidence: f32,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphEntitySearchResponse {
    pub items: Vec<GraphEntity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphNeighbor {
    pub entity: GraphEntity,
    pub relation: String,
    pub direction: String,
    pub confidence: f32,
    pub revision: String,
    pub depth: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphNeighborsResponse {
    pub items: Vec<GraphNeighbor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphSubgraphResponse {
    pub nodes: Vec<GraphEntity>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphPathResponse {
    pub nodes: Vec<GraphEntity>,
    pub relations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TraceabilityResponse {
    pub entity: GraphEntity,
    pub ancestors: Vec<GraphEntity>,
    pub descendants: Vec<GraphEntity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OntologyEntityType {
    pub name: String,
    pub display_name_ja: String,
    pub parent_type: Option<String>,
    pub is_abstract: bool,
    pub properties_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OntologyRelationType {
    pub name: String,
    pub display_name_ja: String,
    pub source_types: Vec<String>,
    pub target_types: Vec<String>,
    pub cardinality: String,
    pub is_transitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OntologyResponse {
    pub version: String,
    pub status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub entity_types: Vec<OntologyEntityType>,
    pub relation_types: Vec<OntologyRelationType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphStatsResponse {
    pub pg_nodes: i64,
    pub pg_relationships: i64,
    pub neo4j_nodes: i64,
    pub neo4j_relationships: i64,
    pub drift_rate: f64,
    pub last_event_seq: i64,
    pub ontology_version: String,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct PublishOntologyRequest {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PublishOntologyResponse {
    pub version: String,
    pub status: String,
    pub entity_types: i64,
    pub relation_types: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphRebuildResponse {
    pub entities: i64,
    pub relationships: i64,
}

#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct GraphEntityQuery {
    pub q: Option<String>,
    pub entity_type: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct GraphNeighborsQuery {
    pub rel_types: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct GraphTraceabilityQuery {
    pub depth: Option<i32>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct GraphSubgraphQuery {
    pub root: Uuid,
    pub depth: Option<i32>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct GraphPathQuery {
    pub src: Uuid,
    pub dst: Uuid,
    pub max_depth: Option<i32>,
}
