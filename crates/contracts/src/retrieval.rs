//! Retrieval Engine の契約（Phase 2 の最小形）。
//!
//! 既存の `SearchRequest` / `SearchResponse` は GLOBAL 戦略の 1 実装として残し、
//! こちらは**戦略・トレース・コンテキスト**まで含む統一形。
//! 段階的に増やす前提で、未知の値は `#[serde(default)]` で受ける。

use chrono::{DateTime, Utc};
use domain::{ChunkId, DocumentId, EvidenceId, RevisionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::doc_type::DocType;
use crate::knowledge::SearchFilters;

/// 检索策略。Phase 2 では GLOBAL と DOCUMENT_LEVEL のみ実装。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetrievalStrategy {
    Global,
    DocumentLevel,
    Hierarchical,
    CrossDocument,
    Hybrid,
}

impl RetrievalStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "GLOBAL",
            Self::DocumentLevel => "DOCUMENT_LEVEL",
            Self::Hierarchical => "HIERARCHICAL",
            Self::CrossDocument => "CROSS_DOCUMENT",
            Self::Hybrid => "HYBRID",
        }
    }
}

/// 图的扩展条件。Phase 2 では受け取るだけで未使用（trace に残す）。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GraphOptions {
    /// 1〜3。範囲外は丸めて `degraded` に記録する。
    #[serde(default = "default_depth")]
    pub depth: i32,
    /// `OUT` / `IN` / `BOTH`
    #[serde(default)]
    pub direction: String,
    #[serde(default)]
    pub relationship_types: Vec<String>,
    #[serde(default = "default_related")]
    pub max_related: i64,
    #[serde(default)]
    pub min_confidence: f32,
}

fn default_depth() -> i32 {
    1
}

fn default_related() -> i64 {
    20
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            depth: default_depth(),
            direction: String::new(),
            relationship_types: Vec::new(),
            max_related: default_related(),
            min_confidence: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ContextOptions {
    /// 0 を渡すと既定（3000）に戻す。
    #[serde(default = "default_token_limit")]
    pub token_limit: i64,
    #[serde(default)]
    pub include_hierarchy: bool,
}

fn default_token_limit() -> i64 {
    3000
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            token_limit: default_token_limit(),
            include_hierarchy: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct RetrievalRequest {
    pub query: String,
    /// 省略すると規則で自動選択し、`strategy_reason` に理由を残す。
    #[serde(default)]
    pub strategy: Option<RetrievalStrategy>,
    #[serde(default)]
    pub document_ids: Vec<DocumentId>,
    #[serde(default)]
    pub document_types: Vec<DocType>,
    /// 現状は `filters.business_domain` に写す（業務機能の実体は未実装）。
    #[serde(default)]
    pub business_function_ids: Vec<String>,
    #[serde(default = "default_top_k")]
    pub top_k: i64,
    #[serde(default)]
    pub similarity_threshold: Option<f32>,
    #[serde(default)]
    pub graph: GraphOptions,
    #[serde(default)]
    pub context: ContextOptions,
    #[serde(default)]
    pub filters: SearchFilters,
}

fn default_top_k() -> i64 {
    12
}

/// 1 本の召回経路の生スコア。融合後も**消さない**（説明可能性のため）。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RawScore {
    pub source: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RelationPath {
    pub from: Uuid,
    pub to: Uuid,
    pub relation: String,
    pub confidence: f32,
    pub evidence_id: Option<EvidenceId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetrievedChunk {
    pub chunk_id: ChunkId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    /// 原文。**書き換えない**。
    pub quote: String,
    pub locator: Value,
    #[serde(default)]
    pub node_path: Vec<String>,
    /// 融合後のスコア（順序付けに使った値）。
    pub score: f64,
    #[serde(default)]
    pub raw_scores: Vec<RawScore>,
    /// `vector` / `lexical` / `graph` / `hierarchy`
    #[serde(default)]
    pub recall_sources: Vec<String>,
    #[serde(default)]
    pub evidence_id: Option<EvidenceId>,
    /// グラフ拡張で得た場合の経路。
    #[serde(default)]
    pub via: Option<RelationPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SourceDocument {
    pub id: DocumentId,
    pub title: String,
    pub doc_type: String,
    pub revision_id: RevisionId,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ContextEntry {
    pub chunk_id: ChunkId,
    pub document_id: DocumentId,
    pub title: String,
    pub locator: Value,
    pub score: f64,
    pub tokens: i64,
    pub quote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ContextBlock {
    /// 原文を連結したもの。要約や生成文は**入れない**。
    pub text: String,
    pub tokens: i64,
    #[serde(default)]
    pub entries: Vec<ContextEntry>,
    /// 予算の都合で落とした件数（黙って減らさない）。
    pub dropped: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetrievalStage {
    pub seq: i32,
    pub stage: String,
    pub candidates: i64,
    pub kept: i64,
    pub latency_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetrievalStats {
    pub candidates: i64,
    pub returned: i64,
    pub context_tokens: i64,
    pub latency_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetrievalResponse {
    pub retrieval_id: Uuid,
    pub query: String,
    pub selected_strategy: RetrievalStrategy,
    /// なぜその戦略になったか（自動選択の根拠）。
    pub strategy_reason: String,
    pub chunks: Vec<RetrievedChunk>,
    #[serde(default)]
    pub relationships: Vec<RelationPath>,
    pub sources: Vec<SourceDocument>,
    pub context: ContextBlock,
    pub stages: Vec<RetrievalStage>,
    #[serde(default)]
    pub degraded: Vec<String>,
    pub stats: RetrievalStats,
}

/// 実行記録の 1 行（管理画面用）。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetrievalRunSummary {
    pub id: Uuid,
    pub query: String,
    pub strategy: String,
    pub strategy_reason: String,
    pub latency_ms: i64,
    pub degraded: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetrievalRunListResponse {
    pub items: Vec<RetrievalRunSummary>,
}
