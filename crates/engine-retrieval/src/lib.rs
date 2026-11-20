//! Retrieval Engine の純ロジック（IO なし）。
//!
//! ここには**判断**だけを置く: 戦略選択、コンテキストの詰め方、トークン概算。
//! DB・HTTP・モデルには一切触れないので、すべて単体テストで固定できる。
//! 実際の検索は `app::retrieval` が既存の `search` を呼んで組み立てる。

use contracts::{ContextBlock, ContextEntry, RetrievalStrategy};
use uuid::Uuid;

/// 壊れた値は nil にして、呼び出し側で気付けるようにする。
fn parse_uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap_or_else(|_| Uuid::nil())
}

/// 戦略選択の結果。`reason` は画面と trace にそのまま出す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyChoice {
    pub strategy: RetrievalStrategy,
    pub reason: String,
}

/// 問題文から戦略を決める（規則ベース。LLM は使わない）。
///
/// 上から順に評価し、最初に当たった規則を採る。順序には意味がある:
/// 「影響」系は必ず CROSS_DOCUMENT、「比較」系は CROSS_DOCUMENT + HIERARCHICAL を
/// 期待するが、Phase 2 では HIERARCHICAL 未実装のため CROSS_DOCUMENT を返し、
/// 実装状況は呼び出し側が `degraded` に書く。
pub fn select_strategy(query: &str) -> StrategyChoice {
    let text = query.to_lowercase();

    if contains_any(
        &text,
        &["影響", "波及", "修正後", "変更したら", "変更後", "impact"],
    ) {
        return StrategyChoice {
            strategy: RetrievalStrategy::CrossDocument,
            reason: "影響・波及を問う語があるため CROSS_DOCUMENT".into(),
        };
    }
    if contains_any(&text, &["比較", "一致", "差分", "違い", "compare", "diff"]) {
        return StrategyChoice {
            strategy: RetrievalStrategy::CrossDocument,
            reason: "比較・一致を問う語があるため CROSS_DOCUMENT（階層拡張と併用）".into(),
        };
    }
    if let Some(kind) = document_type_hint(&text) {
        return StrategyChoice {
            strategy: RetrievalStrategy::DocumentLevel,
            reason: format!("文書種別の語（{kind}）があるため DOCUMENT_LEVEL"),
        };
    }
    if contains_any(
        &text,
        &["章", "節", "この画面", "当該画面", "该项目", "该项目の"],
    ) {
        return StrategyChoice {
            strategy: RetrievalStrategy::Hierarchical,
            reason: "章・節・指示語があるため HIERARCHICAL".into(),
        };
    }
    StrategyChoice {
        strategy: RetrievalStrategy::Global,
        reason: "特定の手がかりが無いため GLOBAL".into(),
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

/// 文書種別を表す語。**ファイル名ではなくメタデータ（`document.doc_type`）に対応させる**
/// ため、ここでは「どの種別を指しているか」だけを返す。
pub fn document_type_hint(text: &str) -> Option<&'static str> {
    // 長い語を先に判定する（「基本設計」が「設計」に食われないように）。
    // **値は contracts::DocType の実際の列挙名**（DETAILED_SPEC は存在しない。
    // 詳細設計は DETAILED_DESIGN）。ここを間違えると DB の CHECK で落ちる。
    const PAIRS: &[(&str, &str)] = &[
        ("概要設計", "REQUIREMENT"),
        ("基本設計", "BASIC_DESIGN"),
        ("詳細設計", "DETAILED_DESIGN"),
        ("テスト設計", "TEST_SPEC"),
    ];
    PAIRS
        .iter()
        .find(|(needle, _)| text.contains(needle))
        .map(|(_, kind)| *kind)
}

/// トークン数の概算。
///
/// 正確なトークナイザはモデル依存なので**ここでは近似**する:
/// ASCII は 4 文字 1 トークン、非 ASCII（日本語・中国語）は 1 文字 1 トークン弱。
/// 予算管理が目的なので、多少多めに出るぶんには構わない（溢れるより良い）。
pub fn estimate_tokens(text: &str) -> i64 {
    let mut ascii = 0i64;
    let mut wide = 0i64;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            wide += 1;
        }
    }
    (ascii + 3) / 4 + wide
}

/// コンテキストに詰める 1 件分の入力。
#[derive(Debug, Clone)]
pub struct ContextInput {
    pub chunk_id: String,
    pub document_id: String,
    pub title: String,
    pub locator: serde_json::Value,
    pub score: f64,
    pub quote: String,
}

/// 予算内に収める。
///
/// 方針（設計 §6）:
/// - **1 件を途中で切らない**。切ると証拠が断章取義になる。
/// - 予算を超える場合は「スコアの低いものから落とす」。
/// - 落とした件数は必ず返す（黙って減らさない）。
pub fn build_context(inputs: &[ContextInput], token_limit: i64) -> ContextBlock {
    let limit = if token_limit <= 0 { 3000 } else { token_limit };
    let mut used = 0i64;
    let mut entries: Vec<ContextEntry> = Vec::new();
    let mut dropped = 0i64;

    for input in inputs {
        let tokens = estimate_tokens(&input.quote) + estimate_tokens(&input.title) + 8;
        if used + tokens > limit {
            dropped += 1;
            continue;
        }
        used += tokens;
        entries.push(ContextEntry {
            chunk_id: domain::ChunkId(parse_uuid(&input.chunk_id)),
            document_id: domain::DocumentId(parse_uuid(&input.document_id)),
            title: input.title.clone(),
            locator: input.locator.clone(),
            score: input.score,
            tokens,
            quote: input.quote.clone(),
        });
    }

    let text = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            format!(
                "[{}] {} / {} (score {:.3})\n{}",
                index + 1,
                entry.title,
                compact(&entry.locator),
                entry.score,
                entry.quote
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    ContextBlock {
        text,
        tokens: used,
        entries,
        dropped,
    }
}

fn compact(locator: &serde_json::Value) -> String {
    match locator {
        serde_json::Value::Null => "-".into(),
        serde_json::Value::Object(map) => map
            .iter()
            .filter_map(|(key, value)| match value {
                serde_json::Value::Null => None,
                serde_json::Value::String(text) => Some(format!("{key}={text}")),
                other => Some(format!("{key}={other}")),
            })
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_cross_document_for_impact_questions() {
        let choice = select_strategy("顧客検索の入力チェックを変えたら影響は？");
        assert_eq!(choice.strategy, RetrievalStrategy::CrossDocument);
        assert!(choice.reason.contains("影響"));
    }

    #[test]
    fn picks_document_level_for_design_type_words() {
        // 「詳細設計」は BASIC_DESIGN に食われてはいけない。
        let choice = select_strategy("顧客検索の詳細設計を確認したい");
        assert_eq!(choice.strategy, RetrievalStrategy::DocumentLevel);
        assert!(choice.reason.contains("DETAILED_DESIGN"));
    }

    #[test]
    fn basic_design_is_not_mistaken_for_detail() {
        assert_eq!(document_type_hint("基本設計を見たい"), Some("BASIC_DESIGN"));
        assert_eq!(
            document_type_hint("詳細設計を見たい"),
            Some("DETAILED_DESIGN")
        );
    }

    #[test]
    fn comparison_wins_over_document_type() {
        // 比較は種別より優先（両方の文書を跨ぐ必要があるため）。
        let choice = select_strategy("基本設計と詳細設計は一致していますか");
        assert_eq!(choice.strategy, RetrievalStrategy::CrossDocument);
    }

    #[test]
    fn hints_are_valid_doc_types() {
        // 綴り間違い（DETAIL_DESIGN / SUMMARY_DESIGN 等）をここで止める。
        for query in ["概要設計", "基本設計", "詳細設計", "テスト設計"] {
            let hint = document_type_hint(query).expect("hint");
            assert!(
                contracts::DocType::parse(hint).is_some(),
                "{hint} は DocType として不正"
            );
        }
    }

    #[test]
    fn falls_back_to_global() {
        let choice = select_strategy("顧客検索とは");
        assert_eq!(choice.strategy, RetrievalStrategy::Global);
    }

    #[test]
    fn estimates_tokens_for_mixed_text() {
        // 日本語 10 文字 + ASCII 8 文字 → 10 + 2 = 12
        assert_eq!(estimate_tokens("あいうえおかきくけこabcdefgh"), 12);
    }

    #[test]
    fn context_never_truncates_a_single_entry() {
        let long = "あ".repeat(100);
        let inputs = vec![ContextInput {
            chunk_id: "00000000-0000-4000-8000-000000000001".into(),
            document_id: "00000000-0000-4000-8000-000000000002".into(),
            title: "t".into(),
            locator: serde_json::json!({"page": 1}),
            score: 0.9,
            quote: long,
        }];
        // 予算 50 では 1 件（約 110 トークン）が入らない → 落として dropped=1。
        let block = build_context(&inputs, 50);
        assert!(block.entries.is_empty());
        assert_eq!(block.dropped, 1);
        assert!(block.text.is_empty());
    }

    #[test]
    fn context_drops_lowest_scores_last() {
        let make = |id: &str, score: f64| ContextInput {
            chunk_id: id.into(),
            document_id: "00000000-0000-4000-8000-000000000002".into(),
            title: "doc".into(),
            locator: serde_json::json!({"page": 1}),
            score,
            quote: "あ".repeat(50),
        };
        let inputs = vec![
            make("00000000-0000-4000-8000-00000000000a", 0.9),
            make("00000000-0000-4000-8000-00000000000b", 0.5),
        ];
        // 1 件あたり約 59 トークン。予算 100 なら 1 件だけ入る（2 件で 118）。
        let block = build_context(&inputs, 100);
        assert_eq!(block.entries.len(), 1);
        assert_eq!(block.dropped, 1);
        assert!(block.entries[0].score > 0.8);
    }
}

// ───────────────────────── グラフ拡張の統合（Phase 3）

/// 拡張で見つかった候補。
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedHit {
    pub chunk_id: String,
    /// 拡張の起点になった chunk。
    pub from_chunk: String,
    pub relation: String,
    /// 0..1。手動関係なら 1.0、規則推定なら低め。
    pub confidence: f64,
    /// 起点からのホップ数（1 以上）。
    pub hop: u32,
}

/// 統合の結果。**落とした理由を数える**（黙って減らさない）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergeOutcome {
    pub kept: Vec<ExpandedHit>,
    pub dropped_duplicate: usize,
    pub dropped_by_limit: usize,
}

/// 既に持っている chunk を除き、スコア順に上限まで採る。
///
/// スコアは `seed_score * decay^(hop-1) * confidence`。融合ではないので
/// ベクトルの類似度と直接足さない（足すと根拠のない順位になる）。
pub fn merge_expanded(
    seeds: &[(String, f64)],
    expanded: Vec<ExpandedHit>,
    max_related: usize,
    decay: f64,
) -> MergeOutcome {
    use std::collections::HashSet;

    let seed_scores: std::collections::HashMap<&str, f64> = seeds
        .iter()
        .map(|(id, score)| (id.as_str(), *score))
        .collect();
    let mut seen: HashSet<String> = seeds.iter().map(|(id, _)| id.clone()).collect();

    let mut scored: Vec<(f64, ExpandedHit)> = Vec::new();
    let mut dropped_duplicate = 0usize;
    for hit in expanded {
        if seen.contains(&hit.chunk_id) {
            dropped_duplicate += 1;
            continue;
        }
        let seed_score = seed_scores
            .get(hit.from_chunk.as_str())
            .copied()
            .unwrap_or(0.5);
        let score = seed_score * decay.powi(hit.hop.saturating_sub(1) as i32) * hit.confidence;
        seen.insert(hit.chunk_id.clone());
        scored.push((score, hit));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let total = scored.len();
    let kept: Vec<ExpandedHit> = scored
        .into_iter()
        .take(max_related)
        .map(|(_, hit)| hit)
        .collect();
    MergeOutcome {
        kept,
        dropped_duplicate,
        dropped_by_limit: total.saturating_sub(max_related),
    }
}

#[cfg(test)]
mod graph_tests {
    use super::*;

    fn hit(id: &str, from: &str, confidence: f64, hop: u32) -> ExpandedHit {
        ExpandedHit {
            chunk_id: id.into(),
            from_chunk: from.into(),
            relation: "IMPLEMENTS".into(),
            confidence,
            hop,
        }
    }

    #[test]
    fn drops_items_already_in_the_base_results() {
        let seeds = vec![("a".to_string(), 0.9)];
        let outcome = merge_expanded(&seeds, vec![hit("a", "a", 1.0, 1)], 10, 0.7);
        assert!(outcome.kept.is_empty());
        assert_eq!(outcome.dropped_duplicate, 1);
    }

    #[test]
    fn prefers_high_confidence_and_short_hops() {
        let seeds = vec![("a".to_string(), 1.0)];
        let outcome = merge_expanded(
            &seeds,
            vec![hit("far", "a", 1.0, 3), hit("near", "a", 0.6, 1)],
            10,
            0.5,
        );
        // near: 1.0*1*0.6 = 0.6 / far: 1.0*0.25*1.0 = 0.25
        assert_eq!(outcome.kept[0].chunk_id, "near");
    }

    #[test]
    fn counts_what_the_limit_dropped() {
        let seeds = vec![("a".to_string(), 1.0)];
        let outcome = merge_expanded(
            &seeds,
            vec![
                hit("x", "a", 1.0, 1),
                hit("y", "a", 0.9, 1),
                hit("z", "a", 0.8, 1),
            ],
            2,
            0.7,
        );
        assert_eq!(outcome.kept.len(), 2);
        assert_eq!(outcome.dropped_by_limit, 1);
    }

    #[test]
    fn dedupes_duplicates_within_the_expansion() {
        let seeds = vec![("a".to_string(), 1.0)];
        let outcome = merge_expanded(
            &seeds,
            vec![hit("x", "a", 1.0, 1), hit("x", "a", 0.9, 1)],
            10,
            0.7,
        );
        assert_eq!(outcome.kept.len(), 1);
        assert_eq!(outcome.dropped_duplicate, 1);
    }
}
