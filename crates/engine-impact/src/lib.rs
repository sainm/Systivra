//! 変更影響の決定論的走査。
//!
//! この crate は IO を持たない。呼び出し側が PG / Neo4j から取得したノードと
//! 辺を渡し、同じ入力から常に同じ順序・同じ経路・同じ評点を返す。
//! 動的挙動や未解析コードを「影響なし」に丸めない。経路上の最も弱い解決状態を
//! 引き継ぎ、打切り理由を明示する。

#![forbid(unsafe_code)]

pub mod diff;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const ENGINE_VERSION: &str = concat!("impact-", env!("CARGO_PKG_VERSION"));

pub use diff::{line_hunks, parse_unified_diff, DiffParseError, LineHunk, UnifiedHunk};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactNode {
    pub id: Uuid,
    pub kind: String,
    pub natural_key: String,
    pub name: String,
    pub resolution: String,
    pub confidence: f32,
    pub properties: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactEdge {
    pub id: Uuid,
    pub src: Uuid,
    pub dst: Uuid,
    pub relation: String,
    pub resolution: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Direction {
    Changed,
    Upstream,
    Downstream,
    Dispatch,
    SharedResource,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Changed => "changed",
            Self::Upstream => "upstream",
            Self::Downstream => "downstream",
            Self::Dispatch => "dispatch",
            Self::SharedResource => "shared_resource",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Resolution {
    Resolved,
    Partial,
    Inferred,
    Unresolved,
    Conflict,
}

impl Resolution {
    pub fn parse(value: &str) -> Self {
        match value {
            "RESOLVED" => Self::Resolved,
            "PARTIAL" => Self::Partial,
            "INFERRED" => Self::Inferred,
            "CONFLICT" => Self::Conflict,
            _ => Self::Unresolved,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resolved => "RESOLVED",
            Self::Partial => "PARTIAL",
            Self::Inferred => "INFERRED",
            Self::Unresolved => "UNRESOLVED",
            Self::Conflict => "CONFLICT",
        }
    }

    pub fn strength(self) -> u8 {
        match self {
            Self::Resolved => 4,
            Self::Partial => 3,
            Self::Inferred => 2,
            Self::Conflict => 1,
            Self::Unresolved => 0,
        }
    }

    pub fn weakest(self, other: Self) -> Self {
        if self.strength() <= other.strength() {
            self
        } else {
            other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImpactCategory {
    Changed,
    Code,
    Screen,
    Document,
    Data,
    Other,
}

impl ImpactCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Changed => "CHANGED",
            Self::Code => "CODE",
            Self::Screen => "SCREEN",
            Self::Document => "DOCUMENT",
            Self::Data => "DATA",
            Self::Other => "OTHER",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathEdge {
    pub edge_id: Uuid,
    pub src: Uuid,
    pub dst: Uuid,
    pub relation: String,
    pub resolution: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactItem {
    pub entity_id: Uuid,
    pub natural_key: String,
    pub label: String,
    pub entity_kind: String,
    pub category: String,
    pub level: String,
    pub depth: i32,
    pub direction: String,
    pub resolution: String,
    pub confidence: f32,
    pub why: String,
    pub path: Vec<Uuid>,
    pub path_edges: Vec<PathEdge>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImpactStats {
    pub total: i64,
    pub code: i64,
    pub screens: i64,
    pub documents: i64,
    pub data: i64,
    pub resolved: i64,
    pub partial: i64,
    pub inferred: i64,
    pub unresolved: i64,
    pub conflict: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactTruncation {
    pub hubs: i64,
    pub depth: bool,
    pub budget: bool,
    pub no_logical_change: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskFactor {
    pub name: String,
    pub weight: f32,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactRisk {
    pub score: f32,
    pub level: String,
    pub factors: Vec<RiskFactor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactTraversal {
    pub items: Vec<ImpactItem>,
    pub stats: ImpactStats,
    pub truncation: ImpactTruncation,
    pub risk: ImpactRisk,
    pub unresolved_edges: Vec<PathEdge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactOptions {
    pub max_depth: i32,
    pub hub_threshold: i32,
    pub max_items: i32,
}

impl Default for ImpactOptions {
    fn default() -> Self {
        Self {
            max_depth: 6,
            hub_threshold: 200,
            max_items: 500,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct QueueState {
    node: Uuid,
    depth: i32,
    path: Vec<Uuid>,
    edges: Vec<PathEdge>,
    reached_from: Direction,
    resolution: Resolution,
    confidence: f32,
}

pub fn analyze(
    nodes: &[ImpactNode],
    edges: &[ImpactEdge],
    seeds: &[Uuid],
    options: &ImpactOptions,
) -> ImpactTraversal {
    let options = ImpactOptions {
        max_depth: options.max_depth.clamp(1, 10),
        hub_threshold: options.hub_threshold.clamp(1, 100_000),
        max_items: options.max_items.clamp(1, 10_000),
    };
    let node_by_id: HashMap<Uuid, &ImpactNode> = nodes.iter().map(|node| (node.id, node)).collect();
    let mut adjacency: HashMap<Uuid, Vec<&ImpactEdge>> = HashMap::new();
    for edge in edges {
        adjacency.entry(edge.src).or_default().push(edge);
        adjacency.entry(edge.dst).or_default().push(edge);
    }
    for values in adjacency.values_mut() {
        values.sort_by_key(|edge| {
            (
                edge.relation.clone(),
                edge.src.min(edge.dst),
                edge.src.max(edge.dst),
                edge.id,
            )
        });
    }

    let mut traversal = ImpactTraversal {
        items: Vec::new(),
        stats: ImpactStats::default(),
        truncation: ImpactTruncation::default(),
        risk: ImpactRisk {
            score: 0.0,
            level: "LOW".into(),
            factors: Vec::new(),
        },
        unresolved_edges: Vec::new(),
    };
    let mut valid_seeds = BTreeSet::new();
    let mut queue = VecDeque::new();
    for seed in seeds {
        let Some(node) = node_by_id.get(seed) else {
            continue;
        };
        if !valid_seeds.insert(*seed) {
            continue;
        }
        let resolution = Resolution::parse(&node.resolution);
        queue.push_back(QueueState {
            node: *seed,
            depth: 0,
            path: vec![*seed],
            edges: Vec::new(),
            reached_from: Direction::Changed,
            resolution,
            confidence: node.confidence,
        });
    }
    if valid_seeds.is_empty() {
        traversal.truncation.no_logical_change = true;
        traversal.risk = risk(&traversal, &options);
        return traversal;
    }

    let mut best_depth: BTreeMap<Uuid, i32> = BTreeMap::new();
    let mut emitted = BTreeSet::new();
    while let Some(state) = queue.pop_front() {
        if traversal.items.len() >= options.max_items as usize && !valid_seeds.contains(&state.node)
        {
            traversal.truncation.budget = true;
            break;
        }
        if let Some(previous) = best_depth.get(&state.node) {
            if *previous < state.depth {
                continue;
            }
            if *previous == state.depth && emitted.contains(&state.node) {
                continue;
            }
        }
        best_depth.insert(state.node, state.depth);
        let Some(node) = node_by_id.get(&state.node) else {
            continue;
        };
        if emitted.insert(state.node) {
            let item = make_item(node, &state, &node_by_id);
            if item.resolution != Resolution::Resolved.as_str() {
                if let Some(edge) = state.edges.last() {
                    traversal.unresolved_edges.push(edge.clone());
                }
            }
            traversal.items.push(item);
        }
        if state.depth >= options.max_depth {
            if adjacency
                .get(&state.node)
                .is_some_and(|neighbors| !neighbors.is_empty())
            {
                traversal.truncation.depth = true;
            }
            continue;
        }

        let degree = adjacency
            .get(&state.node)
            .map(|values| values.len())
            .unwrap_or(0);
        if !valid_seeds.contains(&state.node) && degree > options.hub_threshold as usize {
            traversal.truncation.hubs += 1;
            continue;
        }
        let Some(neighbors) = adjacency.get(&state.node) else {
            continue;
        };
        for edge in neighbors {
            let (next, direction) = if edge.src == state.node {
                (edge.dst, traverse_direction(edge, false))
            } else {
                (edge.src, traverse_direction(edge, true))
            };
            if state.path.contains(&next) {
                continue;
            }
            let mut next_path = state.path.clone();
            next_path.push(next);
            let mut next_edges = state.edges.clone();
            next_edges.push(PathEdge {
                edge_id: edge.id,
                src: edge.src,
                dst: edge.dst,
                relation: edge.relation.clone(),
                resolution: edge.resolution.clone(),
                confidence: edge.confidence,
            });
            queue.push_back(QueueState {
                node: next,
                depth: state.depth + 1,
                path: next_path,
                edges: next_edges,
                reached_from: direction,
                resolution: state
                    .resolution
                    .weakest(Resolution::parse(&edge.resolution)),
                confidence: state.confidence.min(edge.confidence),
            });
        }
    }

    traversal.items.sort_by(|left, right| {
        (
            left.depth,
            category_order(&left.category),
            left.entity_kind.as_str(),
            left.natural_key.as_str(),
            left.entity_id,
        )
            .cmp(&(
                right.depth,
                category_order(&right.category),
                right.entity_kind.as_str(),
                right.natural_key.as_str(),
                right.entity_id,
            ))
    });
    traversal.items.dedup_by_key(|item| item.entity_id);
    traversal.unresolved_edges.sort_by_key(|edge| edge.edge_id);
    traversal.unresolved_edges.dedup_by_key(|edge| edge.edge_id);
    traversal.stats = statistics(&traversal.items);
    traversal.risk = risk(&traversal, &options);
    traversal
}

fn make_item(
    node: &ImpactNode,
    state: &QueueState,
    node_by_id: &HashMap<Uuid, &ImpactNode>,
) -> ImpactItem {
    let category = category_for(node);
    let last_edge = state.edges.last();
    let why = explain(node, last_edge, state.reached_from, node_by_id);
    ImpactItem {
        entity_id: node.id,
        natural_key: node.natural_key.clone(),
        label: node.name.clone(),
        entity_kind: node.kind.clone(),
        category: category.as_str().into(),
        level: if state.depth == 0 {
            "L1".into()
        } else if state.depth == 1 {
            "L2".into()
        } else {
            "L3".into()
        },
        depth: state.depth,
        direction: state.reached_from.as_str().into(),
        resolution: state.resolution.as_str().into(),
        confidence: state.confidence,
        why,
        path: state.path.clone(),
        path_edges: state.edges.clone(),
    }
}

fn traverse_direction(edge: &ImpactEdge, from_dst: bool) -> Direction {
    if edge.relation == "DISPATCHES" || edge.relation == "IMPLEMENTS" || edge.relation == "EXTENDS"
    {
        return Direction::Dispatch;
    }
    if matches!(edge.relation.as_str(), "READS" | "WRITES" | "QUERIES") {
        return Direction::SharedResource;
    }
    if from_dst {
        Direction::Upstream
    } else {
        Direction::Downstream
    }
}

fn explain(
    node: &ImpactNode,
    edge: Option<&PathEdge>,
    direction: Direction,
    node_by_id: &HashMap<Uuid, &ImpactNode>,
) -> String {
    let Some(edge) = edge else {
        return match Resolution::parse(&node.resolution) {
            Resolution::Resolved => "変更のシードとして直接指定された".into(),
            resolution => format!(
                "変更のシードとして直接指定された（{}）",
                resolution.as_str()
            ),
        };
    };
    let peer_id = if edge.src == node.id {
        edge.dst
    } else {
        edge.src
    };
    let peer = node_by_id
        .get(&peer_id)
        .map(|value| value.name.as_str())
        .unwrap_or("関連実体");
    let base = match (edge.relation.as_str(), direction) {
        ("CALLS", Direction::Upstream) => format!("{peer} から呼び出される"),
        ("CALLS", Direction::Downstream) => format!("{peer} を呼び出す"),
        ("DECLARES", Direction::Downstream) => format!("{peer} が宣言する"),
        ("DECLARES", Direction::Upstream) => format!("{peer} を宣言する"),
        ("HANDLES", _) => format!("{peer} の画面処理に関係する"),
        ("SUBMITS_TO", _) => format!("{peer} へ送信する画面に関係する"),
        ("QUERIES", _) | ("READS", _) | ("WRITES", _) => {
            format!("{peer} と共有データ資源でつながる")
        }
        ("DEPENDS_ON", _) => format!("{peer} に依存する"),
        ("EXTENDS", _) | ("IMPLEMENTS", _) => {
            format!("{peer} の継承・実装関係にある")
        }
        _ => format!("{peer} との {} 関係から到達した", edge.relation),
    };
    let resolution = Resolution::parse(&edge.resolution);
    if resolution == Resolution::Resolved {
        base
    } else {
        format!("{base}（{}）", resolution.as_str())
    }
}

fn category_for(node: &ImpactNode) -> ImpactCategory {
    match node.kind.as_str() {
        "Screen" | "Form" => ImpactCategory::Screen,
        "Document" | "Section" | "Sheet" | "Region" => ImpactCategory::Document,
        "Table" | "Database" | "Schema" | "Column" | "View" => ImpactCategory::Data,
        "Module" | "Package" | "Application" | "Class" | "Method" | "Field" | "Interface"
        | "Action" | "Controller" | "Service" | "Component" | "Configuration" | "SqlStatement"
        | "API" | "Job" | "Batch" => ImpactCategory::Code,
        _ => ImpactCategory::Other,
    }
}

fn category_order(category: &str) -> u8 {
    match category {
        "CHANGED" => 0,
        "CODE" => 1,
        "SCREEN" => 2,
        "DOCUMENT" => 3,
        "DATA" => 4,
        _ => 5,
    }
}

fn statistics(items: &[ImpactItem]) -> ImpactStats {
    let mut stats = ImpactStats {
        total: items.len() as i64,
        ..ImpactStats::default()
    };
    for item in items {
        match item.category.as_str() {
            "CODE" | "CHANGED" => stats.code += 1,
            "SCREEN" => stats.screens += 1,
            "DOCUMENT" => stats.documents += 1,
            "DATA" => stats.data += 1,
            _ => {}
        }
        match item.resolution.as_str() {
            "RESOLVED" => stats.resolved += 1,
            "PARTIAL" => stats.partial += 1,
            "INFERRED" => stats.inferred += 1,
            "CONFLICT" => stats.conflict += 1,
            _ => stats.unresolved += 1,
        }
    }
    stats
}

fn risk(traversal: &ImpactTraversal, options: &ImpactOptions) -> ImpactRisk {
    let breadth = (traversal.stats.total as f32 / 50.0).min(1.0);
    let uncertain = if traversal.stats.total == 0 {
        1.0
    } else {
        (traversal.stats.inferred + traversal.stats.unresolved + traversal.stats.conflict) as f32
            / traversal.stats.total as f32
    };
    let max_reached = traversal
        .items
        .iter()
        .map(|item| item.depth)
        .max()
        .unwrap_or(0) as f32;
    let depth = (max_reached / options.max_depth as f32).min(1.0);
    let truncation = if traversal.truncation.depth
        || traversal.truncation.budget
        || traversal.truncation.hubs > 0
    {
        1.0
    } else {
        0.0
    };
    let factors = vec![
        RiskFactor {
            name: "impact_breadth".into(),
            weight: 0.35,
            value: breadth,
        },
        RiskFactor {
            name: "uncertainty_ratio".into(),
            weight: 0.30,
            value: uncertain,
        },
        RiskFactor {
            name: "depth_reach".into(),
            weight: 0.20,
            value: depth,
        },
        RiskFactor {
            name: "truncation".into(),
            weight: 0.15,
            value: truncation,
        },
    ];
    let score = factors
        .iter()
        .map(|factor| factor.weight * factor.value)
        .sum::<f32>()
        .clamp(0.0, 1.0);
    let level = if score >= 0.70 {
        "HIGH"
    } else if score >= 0.40 {
        "MEDIUM"
    } else {
        "LOW"
    };
    ImpactRisk {
        score: (score * 1000.0).round() / 1000.0,
        level: level.into(),
        factors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: u128, kind: &str, key: &str, resolution: &str) -> ImpactNode {
        ImpactNode {
            id: Uuid::from_u128(id),
            kind: kind.into(),
            natural_key: key.into(),
            name: key.into(),
            resolution: resolution.into(),
            confidence: 1.0,
            properties: json!({}),
        }
    }

    fn edge(id: u128, src: u128, dst: u128, relation: &str, resolution: &str) -> ImpactEdge {
        ImpactEdge {
            id: Uuid::from_u128(id),
            src: Uuid::from_u128(src),
            dst: Uuid::from_u128(dst),
            relation: relation.into(),
            resolution: resolution.into(),
            confidence: 0.8,
        }
    }

    #[test]
    fn traverses_both_directions_and_keeps_weakest_resolution() {
        let nodes = vec![
            node(1, "Method", "demo.Service.update", "RESOLVED"),
            node(2, "Method", "demo.Action.save", "RESOLVED"),
            node(3, "Method", "demo.Batch.run", "INFERRED"),
        ];
        let edges = vec![
            edge(10, 2, 1, "CALLS", "RESOLVED"),
            edge(11, 3, 2, "CALLS", "INFERRED"),
        ];
        let result = analyze(
            &nodes,
            &edges,
            &[Uuid::from_u128(1)],
            &ImpactOptions::default(),
        );
        assert_eq!(result.items.len(), 3);
        let batch = result
            .items
            .iter()
            .find(|item| item.entity_id == Uuid::from_u128(3))
            .unwrap();
        assert_eq!(batch.resolution, "INFERRED");
        assert_eq!(batch.depth, 2);
        assert_eq!(
            batch.path,
            vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)]
        );
    }

    #[test]
    fn output_is_stable_for_the_same_input() {
        let nodes = vec![
            node(1, "Method", "demo.B", "RESOLVED"),
            node(2, "Method", "demo.A", "RESOLVED"),
        ];
        let edges = vec![
            edge(10, 1, 2, "CALLS", "RESOLVED"),
            edge(9, 2, 1, "CALLS", "RESOLVED"),
        ];
        let first = analyze(
            &nodes,
            &edges,
            &[Uuid::from_u128(1)],
            &ImpactOptions::default(),
        );
        let second = analyze(
            &nodes,
            &edges,
            &[Uuid::from_u128(1)],
            &ImpactOptions::default(),
        );
        assert_eq!(first, second);
    }

    #[test]
    fn hub_is_included_but_not_expanded() {
        let mut nodes = vec![
            node(1, "Method", "seed", "RESOLVED"),
            node(2, "Table", "HUGE", "RESOLVED"),
        ];
        for index in 3..13 {
            nodes.push(node(index, "Method", &format!("m{index}"), "RESOLVED"));
        }
        let mut edges = vec![edge(100, 1, 2, "QUERIES", "RESOLVED")];
        for index in 3..13 {
            edges.push(edge(100 + index, index, 2, "QUERIES", "RESOLVED"));
        }
        let result = analyze(
            &nodes,
            &edges,
            &[Uuid::from_u128(1)],
            &ImpactOptions {
                max_depth: 6,
                hub_threshold: 4,
                max_items: 50,
            },
        );
        assert!(result
            .items
            .iter()
            .any(|item| item.entity_id == Uuid::from_u128(2)));
        assert!(result
            .items
            .iter()
            .all(|item| item.entity_id != Uuid::from_u128(3)));
        assert_eq!(result.truncation.hubs, 1);
    }

    #[test]
    fn missing_seed_is_reported_not_silently_empty() {
        let result = analyze(&[], &[], &[Uuid::from_u128(1)], &ImpactOptions::default());
        assert!(result.items.is_empty());
        assert!(result.truncation.no_logical_change);
    }
}
