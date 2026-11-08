//! 検証ルールの抽出（`04-コアエンジン` §4.6「ルール抽出」）。
//!
//! 正規表現エンジンは使わず、決定的な文字走査で「読めた記述」だけをルールにする。
//! 読めなかった記述は `unparsed` に数え、推測で `REQUIRED` などに落とさない。

use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use crate::{stable_id, ENGINE_VERSION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleInput {
    /// CODE / SPEC / INFERRED
    pub kind: String,
    pub field_ref: String,
    pub text: String,
    pub evidence_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExtractedRule {
    pub id: Uuid,
    pub field_ref: String,
    /// REQUIRED / LENGTH / RANGE / PATTERN / CROSS_FIELD / CUSTOM
    pub kind: String,
    pub condition: String,
    /// CODE / SPEC / CONFLICT / INFERRED
    pub source: String,
    pub resolution: String,
    pub evidence_ids: Vec<Uuid>,
    /// 競合判定用の正規化キー。
    pub condition_key: String,
    pub engine_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleExtraction {
    pub rules: Vec<ExtractedRule>,
    pub conflicts: usize,
    pub unparsed: usize,
}

impl RuleExtraction {
    pub fn degraded(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.unparsed > 0 {
            out.push("rules_not_parsed".to_owned());
        }
        if self.conflicts > 0 {
            out.push("rules_conflict".to_owned());
        }
        if self.rules.iter().any(|r| r.evidence_ids.is_empty()) {
            out.push("rules_without_evidence".to_owned());
        }
        out
    }
}

/// 全角数字を ASCII へ寄せ、連続した数字列を取り出す。
fn numbers(text: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        let digit = ch.to_digit(10).or_else(|| {
            // 全角 0〜9
            let c = ch as u32;
            if (0xFF10..=0xFF19).contains(&c) {
                Some(c - 0xFF10)
            } else {
                None
            }
        });
        if let Some(d) = digit {
            current.push(char::from_digit(d, 10).unwrap_or('0'));
        } else if !current.is_empty() {
            if let Ok(n) = current.parse::<u32>() {
                out.push(n);
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        if let Ok(n) = current.parse::<u32>() {
            out.push(n);
        }
    }
    out
}

fn has_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

/// 記述片 → (kind, condition)。読めなければ None。
fn parse_condition(raw: &str) -> Option<(&'static str, String)> {
    let text = raw.trim();
    let lower = text.to_ascii_lowercase();
    let nums = numbers(text);

    // 項目間検証。数値より先に判定する（「開始日 < 終了日」に数字が無いため）。
    if has_any(&lower, &["start<end", "開始<終了", "開始日<終了日"])
        || (has_any(text, &["開始", "start"])
            && has_any(text, &["終了", "end"])
            && has_any(text, &["<", "未満", "以前"]))
    {
        return Some(("CROSS_FIELD", "cross=start<end".to_owned()));
    }

    // HTML / JSP 属性の maxlength。
    if lower.contains("maxlength") {
        if let Some(n) = nums.first() {
            return Some(("LENGTH", format!("max={n}")));
        }
    }

    // Java の検証コード: `s.length() > 20` / `size() >= 20`。
    if lower.contains("length()") || lower.contains("size()") {
        if let Some(n) = nums.first() {
            if has_any(text, &[">=", ">", "超過", "超える"]) {
                return Some(("LENGTH", format!("max={n}")));
            }
            if has_any(text, &["<=", "<", "未満"]) {
                return Some(("LENGTH", format!("min={n}")));
            }
        }
    }

    // 設計書の日本語記述: 「全角20文字以内」「20桁以下」。
    if has_any(
        text,
        &["文字以内", "文字以下", "桁以内", "桁以下", "文字まで"],
    ) {
        if let Some(n) = nums.first() {
            return Some(("LENGTH", format!("max={n}")));
        }
    }

    // 範囲。
    if lower.contains("min=") && lower.contains("max=") && nums.len() >= 2 {
        return Some(("RANGE", format!("min={};max={}", nums[0], nums[1])));
    }
    if has_any(text, &["〜", "～", ".."]) && nums.len() >= 2 {
        return Some(("RANGE", format!("min={};max={}", nums[0], nums[1])));
    }

    // 形式。
    if let Some(pos) = lower.find("pattern=") {
        let rest = &text[pos + "pattern=".len()..];
        let value = rest
            .trim_start_matches(['"', '\'', ' '])
            .split(['"', '\'', ' ', '>', '/'])
            .next()
            .unwrap_or("")
            .trim()
            .to_owned();
        if !value.is_empty() {
            return Some(("PATTERN", format!("pattern={value}")));
        }
    }
    if has_any(text, &["正規表現", "形式:", "パターン:"]) && !nums.is_empty() {
        return Some(("PATTERN", format!("pattern={}", nums[0])));
    }

    // 必須。
    if has_any(&lower, &["required", "notempty", "notblank"])
        || has_any(text, &["必須", "入力してください", "空不可"])
    {
        return Some(("REQUIRED", "required=true".to_owned()));
    }

    None
}

fn normalize_source(kind: &str) -> String {
    match kind.to_ascii_uppercase().as_str() {
        "CODE" => "CODE".to_owned(),
        "SPEC" => "SPEC".to_owned(),
        _ => "INFERRED".to_owned(),
    }
}

pub fn extract_rules(project: Uuid, inputs: &[RuleInput]) -> RuleExtraction {
    let mut rules: Vec<ExtractedRule> = Vec::new();
    let mut unparsed = 0usize;

    for input in inputs {
        let Some((kind, condition)) = parse_condition(&input.text) else {
            unparsed += 1;
            continue;
        };
        let source = normalize_source(&input.kind);
        let resolution = if source == "INFERRED" {
            "INFERRED"
        } else if input.evidence_ids.is_empty() {
            "PARTIAL"
        } else {
            "RESOLVED"
        };
        let evidence_ids = {
            let mut set = BTreeSet::new();
            set.extend(input.evidence_ids.iter().copied());
            set.into_iter().collect::<Vec<_>>()
        };
        let condition_key = format!("{kind}:{condition}");
        let id = stable_id(
            project,
            &[
                &input.field_ref,
                kind,
                &condition,
                &source,
                &evidence_ids
                    .iter()
                    .map(Uuid::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            ],
        );
        rules.push(ExtractedRule {
            id,
            field_ref: input.field_ref.trim().to_owned(),
            kind: kind.to_owned(),
            condition,
            source,
            resolution: resolution.to_owned(),
            evidence_ids,
            condition_key,
            engine_version: ENGINE_VERSION.to_owned(),
        });
    }

    // 同じ (field_ref, kind) に異なる条件があれば「両方残して CONFLICT」。
    // 片方を勝たせると、コードと設計書の不一致という発見そのものが消える。
    let mut grouped: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for rule in &rules {
        grouped
            .entry((rule.field_ref.clone(), rule.kind.clone()))
            .or_default()
            .insert(rule.condition_key.clone());
    }
    let conflict_keys: BTreeSet<(String, String)> = grouped
        .iter()
        .filter(|(_, conditions)| conditions.len() > 1)
        .map(|(key, _)| key.clone())
        .collect();
    for rule in &mut rules {
        if conflict_keys.contains(&(rule.field_ref.clone(), rule.kind.clone())) {
            rule.resolution = "CONFLICT".to_owned();
        }
    }

    rules.sort_by(|a, b| {
        (&a.field_ref, &a.kind, &a.condition, &a.source, a.id).cmp(&(
            &b.field_ref,
            &b.kind,
            &b.condition,
            &b.source,
            b.id,
        ))
    });
    rules.dedup_by_key(|r| r.id);

    RuleExtraction {
        rules,
        conflicts: conflict_keys.len(),
        unparsed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Uuid {
        Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0)
    }

    fn input(kind: &str, field: &str, text: &str) -> RuleInput {
        RuleInput {
            kind: kind.to_owned(),
            field_ref: field.to_owned(),
            text: text.to_owned(),
            evidence_ids: vec![Uuid::from_u128(7)],
        }
    }

    #[test]
    fn reads_code_and_spec_length_rules() {
        let inputs = vec![
            input("CODE", "user.addr", "if (s.length() > 20) errors.add(...)"),
            input("SPEC", "user.addr", "住所は全角20文字以内"),
            input("SPEC", "user.name", "必須項目"),
            input("CODE", "user.age", "min=1 max=99"),
        ];
        let out = extract_rules(project(), &inputs);
        assert_eq!(out.unparsed, 0);
        assert_eq!(out.conflicts, 0, "同じ上限は競合ではない");
        assert_eq!(out.rules.len(), 4);
        assert!(out.rules.iter().any(|r| r.kind == "REQUIRED"));
        assert!(out.rules.iter().any(|r| r.kind == "RANGE"));
    }

    #[test]
    fn conflicting_code_and_spec_are_both_kept() {
        let inputs = vec![
            input("CODE", "user.addr", "if (s.length() > 20) errors.add(...)"),
            input("SPEC", "user.addr", "住所は全角30文字以内"),
        ];
        let out = extract_rules(project(), &inputs);
        assert_eq!(out.conflicts, 1);
        assert_eq!(out.rules.len(), 2);
        assert!(out.rules.iter().all(|r| r.resolution == "CONFLICT"));
        assert!(out.degraded().contains(&"rules_conflict".to_owned()));
    }

    #[test]
    fn unparseable_text_is_counted_not_invented() {
        let inputs = vec![input("SPEC", "x.y", "特記事項なし")];
        let out = extract_rules(project(), &inputs);
        assert!(out.rules.is_empty());
        assert_eq!(out.unparsed, 1);
        assert!(out.degraded().contains(&"rules_not_parsed".to_owned()));
    }

    #[test]
    fn same_input_yields_same_ids() {
        let inputs = vec![
            input("CODE", "a.b", "<input maxlength=\"20\">"),
            input("SPEC", "a.c", "開始日 < 終了日"),
        ];
        let a = extract_rules(project(), &inputs);
        let b = extract_rules(project(), &inputs);
        assert_eq!(a, b);
    }

    #[test]
    fn japanese_specific_rules_are_recognised() {
        let inputs = vec![
            input("SPEC", "user.name", "全角のみ、外字は不可"),
            input("SPEC", "user.addr", "２０文字以内"),
        ];
        let out = extract_rules(project(), &inputs);
        assert!(out.rules.iter().any(|r| r.condition == "max=20"));
    }
}
