use std::collections::{HashMap, HashSet};

use contracts::SearchMode;
use sha2::{Digest, Sha256};
use store::{SearchFilter, SearchHit};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

mod japanese;

pub const CHUNK_TOKENS: usize = 800;
pub const CHUNK_OVERLAP: usize = 80;
pub const TOKENIZER_VERSION: &str = "6.2.0";

const CHUNK_NAMESPACE: Uuid = Uuid::from_u128(0x6f9619ff_8b86_d011_b42d_00c04fc964ff);

#[derive(Debug, Clone)]
pub struct Token {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct ChunkPlan {
    pub id: Uuid,
    pub page: i32,
    pub page_end: i32,
    pub ordinal: i32,
    pub text: String,
    pub tokenized: String,
    pub token_count: i32,
    pub char_range: [usize; 2],
}

pub fn normalize_nfc(input: &str) -> String {
    input.nfc().collect()
}

pub fn normalize_nfkc(input: &str) -> String {
    input.nfkc().collect()
}

pub fn normalize_search(input: &str) -> String {
    normalize_nfkc(input)
        .chars()
        .map(|ch| match ch {
            '髙' => '高',
            '﨑' => '崎',
            '邊' => '辺',
            '郞' => '郎',
            '廣' => '広',
            '鷗' => '鴎',
            '齋' => '斎',
            '澤' => '沢',
            '國' => '国',
            '濵' | '濱' => '浜',
            '龍' => '竜',
            '嶋' | '嶌' => '島',
            other => other,
        })
        .collect()
}

fn is_katakana(ch: char) -> bool {
    matches!(
        ch as u32,
        0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9D | 0x32D0..=0x32FE
    )
}

fn push_cjk_run(tokens: &mut Vec<Token>, run: &[(usize, usize, char)]) {
    let mut cursor = 0;
    while cursor < run.len() {
        let katakana = is_katakana(run[cursor].2);
        let mut end = cursor + 1;
        while end < run.len() && is_katakana(run[end].2) == katakana {
            end += 1;
        }
        let segment = &run[cursor..end];
        if katakana {
            tokens.push(Token {
                text: segment.iter().map(|(_, _, c)| *c).collect(),
                start: segment[0].0,
                end: segment[segment.len() - 1].1,
            });
        } else {
            for i in 0..segment.len() {
                let (start, end, ch) = segment[i];
                tokens.push(Token {
                    text: ch.to_string(),
                    start,
                    end,
                });
                if let Some(next) = segment.get(i + 1) {
                    tokens.push(Token {
                        text: [ch, next.2].iter().collect(),
                        start,
                        end: next.1,
                    });
                }
            }
        }
        cursor = end;
    }
}

pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut ascii = String::new();
    let mut ascii_start = 0usize;
    let mut ascii_end = 0usize;
    let mut cjk: Vec<(usize, usize, char)> = Vec::new();

    let flush_ascii =
        |tokens: &mut Vec<Token>, buf: &mut String, start: &mut usize, end: &mut usize| {
            if !buf.is_empty() {
                tokens.push(Token {
                    text: buf.clone(),
                    start: *start,
                    end: *end,
                });
                buf.clear();
            }
        };

    let flush_cjk = |tokens: &mut Vec<Token>, run: &mut Vec<(usize, usize, char)>| {
        if !run.is_empty() {
            push_cjk_run(tokens, run);
            run.clear();
        }
    };

    for (idx, ch) in input.char_indices() {
        let end = idx + ch.len_utf8();
        if ch.is_ascii_alphanumeric() {
            if ascii.is_empty() {
                flush_cjk(&mut tokens, &mut cjk);
                ascii_start = idx;
            }
            ascii.push(ch.to_ascii_lowercase());
            ascii_end = end;
        } else {
            flush_ascii(&mut tokens, &mut ascii, &mut ascii_start, &mut ascii_end);
            if ch.is_alphanumeric() && !ch.is_ascii() {
                cjk.push((idx, end, ch));
            } else {
                flush_cjk(&mut tokens, &mut cjk);
            }
        }
    }
    flush_ascii(&mut tokens, &mut ascii, &mut ascii_start, &mut ascii_end);
    flush_cjk(&mut tokens, &mut cjk);
    tokens
}

fn search_tokens(input: &str) -> Vec<String> {
    let normalized = normalize_search(input);
    let mut tokens: Vec<String> = tokenize(&normalized)
        .into_iter()
        .map(|token| token.text)
        .collect();
    for word in japanese::words(&normalized) {
        if !tokens.contains(&word) {
            tokens.push(word);
        }
    }
    tokens
}

pub fn tokenized_text(input: &str) -> String {
    search_tokens(input).join(" ")
}

pub fn tsquery(input: &str) -> String {
    let mut tokens: Vec<String> = search_tokens(input)
        .into_iter()
        .filter(|t| t.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens.truncate(96);
    if tokens.is_empty() {
        return String::new();
    }
    tokens.join(" | ")
}

pub fn strict_tsquery(input: &str) -> String {
    let mut tokens: Vec<String> = search_tokens(input)
        .into_iter()
        .filter(|t| t.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .filter(|t| t.chars().count() > 1)
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens.truncate(32);
    if tokens.is_empty() {
        return String::new();
    }
    tokens.join(" & ")
}

pub fn chunk_page(revision_id: Uuid, page: i32, text: &str, base_ordinal: i32) -> Vec<ChunkPlan> {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut ordinal = base_ordinal;
    loop {
        let end = (start + CHUNK_TOKENS).min(tokens.len());
        let byte_start = tokens[start].start;
        let byte_end = tokens[end - 1].end;
        let slice = &text[byte_start..byte_end];
        let id = Uuid::new_v5(
            &CHUNK_NAMESPACE,
            format!("{revision_id}:{page}:{ordinal}").as_bytes(),
        );
        chunks.push(ChunkPlan {
            id,
            page,
            page_end: page,
            ordinal,
            text: slice.to_string(),
            tokenized: tokens[start..end]
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            token_count: (end - start) as i32,
            char_range: [byte_start, byte_end],
        });
        ordinal += 1;
        if end == tokens.len() {
            break;
        }
        let next = end.saturating_sub(CHUNK_OVERLAP);
        start = if next <= start { start + 1 } else { next };
    }
    chunks
}

pub fn embed(text: &str) -> Vec<f32> {
    embed_salted(text, 0)
}

pub fn embed_for_revision(text: &str, revision: &str) -> Vec<f32> {
    let salt = if revision == "1" {
        0
    } else {
        fnv1a(revision.as_bytes())
    };
    embed_salted(text, salt)
}

fn embed_salted(text: &str, salt: u64) -> Vec<f32> {
    let normalized = normalize_search(text);
    let mut values = vec![0.0f32; store::knowledge::EMBEDDING_DIM];
    let mut counts: HashMap<u64, f32> = HashMap::new();
    for token in tokenize(&normalized) {
        *counts
            .entry(fnv1a(token.text.as_bytes()) ^ salt)
            .or_insert(0.0) += 1.0;
    }
    for (hash, count) in counts {
        let idx = (hash % store::knowledge::EMBEDDING_DIM as u64) as usize;
        let sign = if hash & (1 << 63) == 0 { 1.0 } else { -1.0 };
        values[idx] += sign * (1.0 + count.ln());
    }
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut values {
            *value /= norm;
        }
    }
    values
}

pub fn fuse(
    mode: SearchMode,
    lexical: Vec<SearchHit>,
    vector: Vec<SearchHit>,
    top_k: usize,
) -> Vec<SearchHit> {
    const RRF_K: f64 = 60.0;

    struct Entry {
        hit: SearchHit,
        score: f64,
        sources: Vec<String>,
    }

    let mut merged: HashMap<Uuid, Entry> = HashMap::new();

    if matches!(mode, SearchMode::Lexical | SearchMode::Hybrid) {
        for (rank, hit) in lexical.into_iter().enumerate() {
            let score = 1.0 / (RRF_K + (rank + 1) as f64);
            merged
                .entry(hit.chunk_id)
                .and_modify(|entry| {
                    entry.score += score;
                    if !entry.sources.contains(&"fulltext".to_string()) {
                        entry.sources.push("fulltext".into());
                    }
                })
                .or_insert(Entry {
                    hit,
                    score,
                    sources: vec!["fulltext".into()],
                });
        }
    }

    if matches!(mode, SearchMode::Vector | SearchMode::Hybrid) {
        for (rank, hit) in vector.into_iter().enumerate() {
            let score = 1.0 / (RRF_K + (rank + 1) as f64);
            merged
                .entry(hit.chunk_id)
                .and_modify(|entry| {
                    entry.score += score;
                    if !entry.sources.contains(&"vector".to_string()) {
                        entry.sources.push("vector".into());
                    }
                    if entry.hit.evidence_id.is_none() {
                        entry.hit.evidence_id = hit.evidence_id;
                    }
                })
                .or_insert(Entry {
                    hit,
                    score,
                    sources: vec!["vector".into()],
                });
        }
    }

    let mut entries: Vec<Entry> = merged.into_values().collect();
    for entry in &mut entries {
        entry.hit.score = entry.score;
        entry.hit.sources = entry.sources.clone();
    }
    entries.sort_by(|a, b| {
        b.hit
            .score
            .partial_cmp(&a.hit.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.hit.chunk_id.cmp(&b.hit.chunk_id))
    });
    entries.truncate(top_k);
    entries.into_iter().map(|entry| entry.hit).collect()
}

pub fn rerank(query: &str, hits: &mut Vec<SearchHit>, top_k: usize) {
    let query_tokens: HashSet<String> = tokenize(&normalize_search(query))
        .into_iter()
        .map(|token| token.text)
        .collect();
    for hit in hits.iter_mut() {
        let hit_tokens: HashSet<String> = tokenize(&normalize_search(&hit.quote))
            .into_iter()
            .map(|token| token.text)
            .collect();
        let overlap = if query_tokens.is_empty() {
            0.0
        } else {
            query_tokens.intersection(&hit_tokens).count() as f64 / query_tokens.len() as f64
        };
        hit.rerank_score = Some(overlap);
    }
    hits.sort_by(|a, b| {
        b.rerank_score
            .partial_cmp(&a.rerank_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.chunk_id.cmp(&b.chunk_id))
    });
    hits.truncate(top_k);
}

pub fn sha256(input: &[u8]) -> Vec<u8> {
    Sha256::digest(input).to_vec()
}

pub fn sha256_hex(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub fn quote_hash(quote: &str) -> Vec<u8> {
    Sha256::digest(normalize_nfc(quote).as_bytes()).to_vec()
}

pub fn hash_to_sha256(value: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(value))
}

pub fn filter_doc_types(filter: &SearchFilter) -> Vec<String> {
    filter.doc_types.clone()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: u128, score: f64) -> SearchHit {
        hit_with_quote(id, "quote", score)
    }

    fn hit_with_quote(id: u128, quote: &str, score: f64) -> SearchHit {
        SearchHit {
            chunk_id: Uuid::from_u128(id),
            document_id: Uuid::from_u128(100),
            document_title: "doc".into(),
            doc_type: "DETAILED_DESIGN".into(),
            revision_id: Uuid::from_u128(200),
            source_revision: None,
            locator: Default::default(),
            quote: quote.into(),
            score,
            rerank_score: None,
            sources: Vec::new(),
            evidence_id: None,
        }
    }

    #[test]
    fn rrf_sums_reciprocal_ranks_across_lists() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        let fused = fuse(
            SearchMode::Hybrid,
            vec![hit(1, 0.9), hit(2, 0.8)],
            vec![hit(2, 0.7), hit(3, 0.6)],
            3,
        );
        assert_eq!(fused[0].chunk_id, b);
        assert!((fused[0].score - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-9);
        assert!(fused[0].sources.contains(&"fulltext".into()));
        assert!(fused[0].sources.contains(&"vector".into()));
        assert_eq!(fused[1].chunk_id, a);
        assert_eq!(fused[2].chunk_id, c);
    }

    #[test]
    fn mode_limits_the_lists_used_for_fusion() {
        let lexical = fuse(SearchMode::Lexical, vec![hit(1, 0.9)], vec![hit(2, 0.8)], 2);
        assert_eq!(lexical.len(), 1);
        assert_eq!(lexical[0].chunk_id, Uuid::from_u128(1));

        let vector = fuse(SearchMode::Vector, vec![hit(1, 0.9)], vec![hit(2, 0.8)], 2);
        assert_eq!(vector.len(), 1);
        assert_eq!(vector[0].chunk_id, Uuid::from_u128(2));
    }

    #[test]
    fn top_k_truncates_after_rrf_sort() {
        let fused = fuse(
            SearchMode::Hybrid,
            vec![hit(1, 0.9), hit(2, 0.8), hit(3, 0.7)],
            Vec::new(),
            2,
        );
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].chunk_id, Uuid::from_u128(1));
        assert_eq!(fused[1].chunk_id, Uuid::from_u128(2));
    }

    #[test]
    fn rerank_prefers_query_overlap() {
        let mut hits = vec![
            hit_with_quote(1, "関係のない本文", 0.9),
            hit_with_quote(2, "住所は全角20文字以内", 0.4),
        ];
        rerank("住所 全角", &mut hits, 2);
        assert_eq!(hits[0].chunk_id, Uuid::from_u128(2));
        assert!(hits[0].rerank_score.unwrap() > hits[1].rerank_score.unwrap());
    }

    #[test]
    fn rerank_truncates_after_sorting() {
        let mut hits = vec![
            hit_with_quote(1, "住所", 0.1),
            hit_with_quote(2, "無関係", 0.9),
            hit_with_quote(3, "住所 全角", 0.2),
        ];
        rerank("住所 全角", &mut hits, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk_id, Uuid::from_u128(3));
    }

    #[test]
    fn tokenizes_japanese_into_unigrams_and_bigrams() {
        let tokens = tokenize("住所チェック");
        let texts: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
        assert!(texts.contains(&"住所"));
        assert!(texts.contains(&"チェック"));
        assert!(texts.contains(&"住"));
    }

    #[test]
    fn mixed_ascii_and_cjk_keep_source_order() {
        let tokens = tokenize("第37章");
        let texts: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["第", "37", "章"]);
        assert_eq!(tokens[0].start, 0);
    }

    #[test]
    fn chunk_respects_token_window_and_overlap() {
        let text = "あ".repeat(2000);
        let chunks = chunk_page(Uuid::nil(), 3, &text, 0);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.token_count <= CHUNK_TOKENS as i32));
        assert_eq!(chunks[0].page, 3);
        assert!(chunks[1].char_range[0] < chunks[0].char_range[1]);
    }

    #[test]
    fn embedding_changes_with_model_revision() {
        let base = embed_for_revision("住所チェック", "1");
        assert_eq!(base, embed("住所チェック"));
        let next = embed_for_revision("住所チェック", "2");
        assert_eq!(next.len(), base.len());
        assert_ne!(next, base);
        assert_eq!(next, embed_for_revision("住所チェック", "2"));
    }

    #[test]
    fn embedding_is_normalized_and_stable() {
        let a = embed("住所チェックの桁数");
        let b = embed("住所チェックの桁数");
        assert_eq!(a, b);
        let norm = a.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
    }

    #[test]
    fn tokenized_text_normalizes_compatibility_ideographs() {
        let text = tokenized_text("髙橋");
        assert!(text.contains("高"), "{text}");
        assert!(text.contains("橋"), "{text}");
    }

    #[test]
    fn tokenized_text_adds_lindera_words_to_bigrams() {
        let text = tokenized_text("住所チェック");
        assert!(text.contains("住所"));
        assert!(text.contains("チェック"), "{text}");
    }

    #[test]
    fn tsquery_is_never_empty_for_japanese() {
        assert!(tsquery("住所").contains('|') || tsquery("住所").contains("住所"));
        assert!(tsquery("!!!").is_empty());
    }
}
