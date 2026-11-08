//! ルール × 影響項目から Scenario / Case / Data を決定的に派生する。
//!
//! AI に「いくつか考えて」とは言わない（`04-コアエンジン` §4.6「シナリオ派生」）。
//! 派生規則は固定で、同じ入力なら同じ ID・同じ件数・同じ順序になる。

use std::collections::BTreeMap;
use uuid::Uuid;

use crate::rule::ExtractedRule;
use crate::stable_id;
use contracts::{DslStep, TestCaseDsl};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactItemSeed {
    pub id: Uuid,
    pub label: String,
    pub kind: String,
    pub screen: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeriveInput {
    pub rules: Vec<ExtractedRule>,
    pub impact_items: Vec<ImpactItemSeed>,
    pub max_drafts: usize,
    pub base_path: String,
    /// ケース ID の衝突を避けるため、計画ごとに接頭辞を付ける。
    /// Q-01 由来の複数計画を同じプロジェクトに並べるときに必要。
    pub plan_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DerivedScenario {
    pub id: Uuid,
    pub code: String,
    pub title: String,
    /// NORMAL / ERROR / BOUNDARY / PERMISSION / STATE / DOUBLE_SUBMIT / CONCURRENT / ROLLBACK
    pub kind: String,
    pub evidence_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DerivedCase {
    pub id: Uuid,
    pub scenario_id: Uuid,
    pub code: String,
    pub title: String,
    /// 常に DRAFT。承認は人間だけ（detail/08 §6.2）。
    pub status: String,
    pub dsl: TestCaseDsl,
    pub covers_rules: Vec<Uuid>,
    pub covers_impacts: Vec<Uuid>,
    pub covers_screens: Vec<String>,
    pub evidence_ids: Vec<Uuid>,
    pub destructive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DerivedDataSet {
    pub id: Uuid,
    pub case_id: Uuid,
    pub label: String,
    /// VALID / INVALID / BOUNDARY
    pub kind: String,
    pub field_ref: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derivation {
    pub scenarios: Vec<DerivedScenario>,
    pub cases: Vec<DerivedCase>,
    pub data_sets: Vec<DerivedDataSet>,
    pub truncated: usize,
    pub not_covered: Vec<String>,
    pub degraded: Vec<String>,
}

/// 日本語特有の入力（`04-コアエンジン` §4.6「テストデータ」）。
fn japanese_samples() -> Vec<(&'static str, &'static str)> {
    vec![
        ("全角のみ", "東京都中央区"),
        ("半角混在", "Tokyo中央区"),
        ("外字1字", "\u{E000}区"),
        ("サロゲートペア", "𠮷野区"),
        ("結合文字", "か\u{3099}き区"),
        ("全角空格", "東京都\u{3000}中央区"),
    ]
}

fn repeat_fullwidth(n: u32) -> String {
    "あ".repeat(n.min(200) as usize)
}

/// ルール 1 件 → (ラベル, 種別, 値) の組。
fn data_for(rule: &ExtractedRule) -> Vec<(&'static str, &'static str, String)> {
    if let Some(rest) = rule.condition.strip_prefix("max=") {
        if let Ok(max) = rest.parse::<u32>() {
            return vec![
                (
                    "上限-1",
                    "BOUNDARY",
                    repeat_fullwidth(max.saturating_sub(1)),
                ),
                ("上限", "BOUNDARY", repeat_fullwidth(max)),
                ("上限+1", "INVALID", repeat_fullwidth(max + 1)),
            ];
        }
    }
    if let Some(rest) = rule.condition.strip_prefix("min=") {
        if let Ok(min) = rest.parse::<u32>() {
            return vec![
                ("下限-1", "INVALID", repeat_fullwidth(min.saturating_sub(1))),
                ("下限", "BOUNDARY", repeat_fullwidth(min)),
                ("下限+1", "VALID", repeat_fullwidth(min + 1)),
            ];
        }
    }
    if let Some(rest) = rule.condition.strip_prefix("min=") {
        let parts: Vec<&str> = rest.split(";max=").collect();
        if parts.len() == 2 {
            if let (Ok(min), Ok(max)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
                return vec![
                    ("下限-1", "INVALID", (min - 1).to_string()),
                    ("下限", "BOUNDARY", min.to_string()),
                    ("上限", "BOUNDARY", max.to_string()),
                    ("上限+1", "INVALID", (max + 1).to_string()),
                ];
            }
        }
    }
    match rule.kind.as_str() {
        "REQUIRED" => vec![
            ("未入力", "INVALID", String::new()),
            ("空白のみ", "INVALID", "\u{3000}".to_owned()),
            ("値あり", "VALID", "東京都".to_owned()),
        ],
        "PATTERN" => japanese_samples()
            .into_iter()
            .map(|(label, value)| {
                let kind = if label == "全角のみ" {
                    "VALID"
                } else {
                    "INVALID"
                };
                (label, kind, value.to_owned())
            })
            .collect(),
        "CROSS_FIELD" => vec![
            ("開始<終了", "VALID", "2026/01/01|2026/12/31".to_owned()),
            ("開始>終了", "INVALID", "2026/12/31|2026/01/01".to_owned()),
            ("同日", "BOUNDARY", "2026/01/01|2026/01/01".to_owned()),
            ("うるう日", "BOUNDARY", "2028/02/29|2028/03/01".to_owned()),
        ],
        _ => Vec::new(),
    }
}

fn target(label: &str) -> std::collections::BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    map.insert("label".to_owned(), label.to_owned());
    map
}

fn rule_dsl(base_path: &str, field: &str, value: &str, invalid: bool) -> TestCaseDsl {
    let mut click_target = BTreeMap::new();
    click_target.insert("role".to_owned(), "button".to_owned());
    click_target.insert("name".to_owned(), "登録".to_owned());
    let mut steps = vec![
        DslStep {
            action: "goto".to_owned(),
            target: {
                let mut m = BTreeMap::new();
                m.insert("path".to_owned(), base_path.to_owned());
                m
            },
            ..DslStep::default()
        },
        DslStep {
            action: "fill".to_owned(),
            target: target(field),
            value: Some(value.to_owned()),
            ..DslStep::default()
        },
        DslStep {
            action: "click".to_owned(),
            target: click_target,
            ..DslStep::default()
        },
    ];
    steps.push(if invalid {
        DslStep {
            action: "field_error".to_owned(),
            target: target(field),
            value: Some("入力値を確認してください".to_owned()),
            ..DslStep::default()
        }
    } else {
        DslStep {
            action: "assert_visible".to_owned(),
            target: target(field),
            ..DslStep::default()
        }
    });
    TestCaseDsl {
        v: 1,
        case_code: String::new(),
        title: String::new(),
        preconditions: vec!["ログイン済み".to_owned(), "対象画面を開いている".to_owned()],
        steps,
        data: None,
        selector_strategy: "role_first".to_owned(),
    }
}

fn impact_dsl(base_path: &str, label: &str) -> TestCaseDsl {
    TestCaseDsl {
        v: 1,
        case_code: String::new(),
        title: String::new(),
        preconditions: vec!["ログイン済み".to_owned()],
        steps: vec![
            DslStep {
                action: "goto".to_owned(),
                target: {
                    let mut m = BTreeMap::new();
                    m.insert("path".to_owned(), base_path.to_owned());
                    m
                },
                ..DslStep::default()
            },
            DslStep {
                action: "assert_text".to_owned(),
                value: Some(label.to_owned()),
                match_mode: Some("contains".to_owned()),
                ..DslStep::default()
            },
        ],
        data: None,
        selector_strategy: "role_first".to_owned(),
    }
}

fn id(project: Uuid, code: &str) -> Uuid {
    stable_id(project, &["derive", code])
}

pub fn derive_assets(project: Uuid, input: &DeriveInput) -> Derivation {
    let max = input.max_drafts.max(1);
    let prefix = if input.plan_code.is_empty() {
        "TP".to_owned()
    } else {
        input.plan_code.clone()
    };
    let mut scenarios = Vec::new();
    let mut cases = Vec::new();
    let mut data_sets = Vec::new();
    let mut not_covered = Vec::new();
    let mut truncated = 0usize;
    let mut seq = 0usize;

    // 1. ルール由来の境界ケース。
    for rule in &input.rules {
        let samples = data_for(rule);
        if samples.is_empty() {
            not_covered.push(format!(
                "{}: {} からデータを派生できない（CUSTOM / 未対応条件）",
                rule.field_ref, rule.condition
            ));
            continue;
        }
        seq += 1;
        let scenario_code = format!("{prefix}-TS-{seq:04}");
        let scenario_id = id(project, &scenario_code);
        scenarios.push(DerivedScenario {
            id: scenario_id,
            code: scenario_code.clone(),
            title: format!("{} の境界・異常入力", rule.field_ref),
            kind: "BOUNDARY".to_owned(),
            evidence_ids: rule.evidence_ids.clone(),
        });
        for (label, kind, value) in samples {
            if cases.len() >= max {
                truncated += 1;
                continue;
            }
            let case_code = format!("{prefix}-TC-{seq:04}-{}", cases.len() + 1);
            let mut dsl = rule_dsl(&input.base_path, &rule.field_ref, &value, kind == "INVALID");
            dsl.case_code = case_code.clone();
            dsl.title = format!("{} に「{}」を入力した場合", rule.field_ref, label);
            let case_id = id(project, &case_code);
            cases.push(DerivedCase {
                id: case_id,
                scenario_id,
                code: case_code.clone(),
                title: dsl.title.clone(),
                status: "DRAFT".to_owned(),
                dsl,
                covers_rules: vec![rule.id],
                covers_impacts: Vec::new(),
                covers_screens: Vec::new(),
                evidence_ids: rule.evidence_ids.clone(),
                destructive: false,
            });
            data_sets.push(DerivedDataSet {
                id: id(project, &format!("{case_code}:{label}")),
                case_id,
                label: label.to_owned(),
                kind: kind.to_owned(),
                field_ref: rule.field_ref.clone(),
                value,
            });
        }
    }

    // 2. 影響項目由来の確認ケース。Q-01 から渡った項目だけを根拠にする。
    for item in &input.impact_items {
        seq += 1;
        let scenario_code = format!("{prefix}-TS-I-{seq:04}");
        let scenario_id = id(project, &scenario_code);
        scenarios.push(DerivedScenario {
            id: scenario_id,
            code: scenario_code,
            title: format!("{} の変更確認", item.label),
            kind: "NORMAL".to_owned(),
            evidence_ids: Vec::new(),
        });
        if cases.len() >= max {
            truncated += 1;
            continue;
        }
        let case_code = format!("{prefix}-TC-I-{seq:04}");
        let mut dsl = impact_dsl(&input.base_path, &item.label);
        dsl.case_code = case_code.clone();
        dsl.title = format!("{} が画面に反映されていること", item.label);
        let case_id = id(project, &case_code);
        cases.push(DerivedCase {
            id: case_id,
            scenario_id,
            code: case_code,
            title: dsl.title.clone(),
            status: "DRAFT".to_owned(),
            dsl,
            covers_rules: Vec::new(),
            covers_impacts: vec![item.id],
            covers_screens: item.screen.clone().into_iter().collect(),
            evidence_ids: Vec::new(),
            destructive: false,
        });
    }

    if input.impact_items.is_empty() {
        not_covered.push("影響項目が渡されていないため、影響由来のケースは未生成".to_owned());
    }
    if input.rules.is_empty() {
        not_covered.push("検証ルールが無いため、境界値ケースは未生成".to_owned());
    }

    let mut degraded = Vec::new();
    if truncated > 0 {
        degraded.push("drafts_truncated".to_owned());
    }
    if cases.iter().any(|c| c.evidence_ids.is_empty()) {
        degraded.push("drafts_without_evidence".to_owned());
    }
    if !not_covered.is_empty() {
        degraded.push("tests_not_available".to_owned());
    }

    Derivation {
        scenarios,
        cases,
        data_sets,
        truncated,
        not_covered,
        degraded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{extract_rules, RuleInput};

    fn project() -> Uuid {
        Uuid::from_u128(0xaaaa_bbbb_cccc_dddd_eeee_ffff_0000_1111)
    }

    #[test]
    fn derives_boundary_cases_with_assertion_steps() {
        let rules = extract_rules(
            project(),
            &[RuleInput {
                kind: "CODE".to_owned(),
                field_ref: "user.addr".to_owned(),
                text: "if (s.length() > 20) errors.add(...)".to_owned(),
                evidence_ids: vec![Uuid::from_u128(1)],
            }],
        )
        .rules;
        let out = derive_assets(
            project(),
            &DeriveInput {
                rules,
                impact_items: Vec::new(),
                max_drafts: 20,
                base_path: "/user/edit".to_owned(),
                plan_code: "TP-TEST".to_owned(),
            },
        );
        assert_eq!(out.cases.len(), 3);
        assert!(out.cases.iter().all(|c| c.status == "DRAFT"));
        assert!(out.cases.iter().all(|c| c
            .dsl
            .steps
            .iter()
            .any(|s| s.action == "field_error" || s.action == "assert_visible")));
        assert_eq!(out.data_sets.len(), 3);
    }

    #[test]
    fn max_drafts_is_enforced_and_reported() {
        let rules = extract_rules(
            project(),
            &[RuleInput {
                kind: "SPEC".to_owned(),
                field_ref: "user.name".to_owned(),
                text: "必須".to_owned(),
                evidence_ids: Vec::new(),
            }],
        )
        .rules;
        let out = derive_assets(
            project(),
            &DeriveInput {
                rules,
                impact_items: Vec::new(),
                max_drafts: 2,
                base_path: "/user/edit".to_owned(),
                plan_code: "TP-TEST".to_owned(),
            },
        );
        assert_eq!(out.cases.len(), 2);
        assert_eq!(out.truncated, 1);
        assert!(out.degraded.contains(&"drafts_truncated".to_owned()));
    }

    #[test]
    fn japanese_specific_data_is_present() {
        let rules = extract_rules(
            project(),
            &[RuleInput {
                kind: "SPEC".to_owned(),
                field_ref: "user.name".to_owned(),
                text: "pattern=\"^[ぁ-ん]+$\" 正規表現".to_owned(),
                evidence_ids: Vec::new(),
            }],
        )
        .rules;
        let out = derive_assets(
            project(),
            &DeriveInput {
                rules,
                impact_items: Vec::new(),
                max_drafts: 20,
                base_path: "/user/edit".to_owned(),
                plan_code: "TP-TEST".to_owned(),
            },
        );
        let labels: Vec<&str> = out.data_sets.iter().map(|d| d.label.as_str()).collect();
        assert!(labels.contains(&"外字1字"));
        assert!(labels.contains(&"サロゲートペア"));
        assert!(labels.contains(&"結合文字"));
    }

    #[test]
    fn derivation_is_deterministic() {
        let rules = extract_rules(
            project(),
            &[RuleInput {
                kind: "CODE".to_owned(),
                field_ref: "a".to_owned(),
                text: "<input maxlength=\"8\">".to_owned(),
                evidence_ids: Vec::new(),
            }],
        )
        .rules;
        let make = || DeriveInput {
            rules: rules.clone(),
            impact_items: vec![ImpactItemSeed {
                id: Uuid::from_u128(9),
                label: "UserService".to_owned(),
                kind: "CODE".to_owned(),
                screen: Some("USER-EDIT".to_owned()),
            }],
            max_drafts: 20,
            base_path: "/user/edit".to_owned(),
            plan_code: "TP-TEST".to_owned(),
        };
        assert_eq!(
            derive_assets(project(), &make()),
            derive_assets(project(), &make())
        );
    }
}
