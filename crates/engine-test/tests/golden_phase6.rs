//! Phase 6 の golden set。実案件データに依存しない合成入力で、
//! `08-実装計画.md` Phase 6 の「300 ルール → 900 組以上のデータ」を毎回検査する。

use std::collections::BTreeSet;
use std::path::PathBuf;

use contracts::TestEnvironment;
use engine_test::{
    compute_coverage, coverage::CoverageCase, coverage::CoverageTotals, derive_assets,
    extract_rules, fingerprint, validate_dsl, DeriveInput, RuleInput,
};
use serde_json::Value;
use uuid::Uuid;

fn project() -> Uuid {
    Uuid::from_u128(0x6000_0000_0000_0000_0000_0000_0000_0006)
}

fn env() -> TestEnvironment {
    TestEnvironment {
        id: "env-golden".to_owned(),
        base_url: "https://stg.example.internal/app".to_owned(),
        guard_kind: "STAGING".to_owned(),
        allowed_origins: vec!["stg.example.internal".to_owned()],
        parallel: 4,
    }
}

fn thresholds() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/test/golden-phase6.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden fixture")).expect("json")
}

fn sample_inputs(count: usize) -> Vec<RuleInput> {
    (0..count)
        .map(|index| {
            let (field, text) = match index % 4 {
                0 => (
                    format!("user.field{index}"),
                    format!("if (s.length() > {}) errors.add(...)", 8 + index % 40),
                ),
                1 => (format!("user.field{index}"), "必須項目".to_owned()),
                2 => (
                    format!("user.field{index}"),
                    format!("min={} max={}", 1 + index % 5, 90 + index % 9),
                ),
                _ => (
                    format!("user.field{index}"),
                    "pattern=\"^[ぁ-ん]+$\" 正規表現".to_owned(),
                ),
            };
            RuleInput {
                kind: if index % 2 == 0 { "CODE" } else { "SPEC" }.to_owned(),
                field_ref: field,
                text,
                evidence_ids: vec![Uuid::from_u128(1000 + index as u128)],
            }
        })
        .collect()
}

#[test]
fn phase6_golden_meets_scale_and_dsl_gate() {
    let threshold = thresholds();
    let count = threshold["source_rules"].as_u64().unwrap() as usize;
    let extraction = extract_rules(project(), &sample_inputs(count));
    assert!(extraction.rules.len() >= threshold["min_rules"].as_u64().unwrap() as usize);
    assert_eq!(extraction.unparsed, 0, "合成入力はすべて読めるべき");

    let derivation = derive_assets(
        project(),
        &DeriveInput {
            rules: extraction.rules.clone(),
            impact_items: Vec::new(),
            max_drafts: 5_000,
            base_path: "/sample/user/edit".to_owned(),
            plan_code: "TP-GOLDEN".to_owned(),
        },
    );
    assert!(
        derivation.data_sets.len() >= threshold["min_data_rows"].as_u64().unwrap() as usize,
        "300 ルールから 900 組以上のデータが必要: {}",
        derivation.data_sets.len()
    );
    assert_eq!(derivation.truncated, 0);

    let valid = derivation
        .cases
        .iter()
        .filter(|case| validate_dsl(&case.dsl, &env()).is_empty())
        .count();
    let ratio = valid as f64 / derivation.cases.len() as f64;
    assert!(ratio >= threshold["min_valid_dsl_ratio"].as_f64().unwrap());
    assert!(derivation.cases.iter().all(|case| case.status == "DRAFT"));

    let labels: BTreeSet<&str> = derivation
        .data_sets
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    for label in threshold["japanese_labels"].as_array().unwrap() {
        assert!(
            labels.contains(label.as_str().unwrap()),
            "日本語特有データ {} が無い",
            label
        );
    }

    // 決定論性: 同じ入力なら同じ派生・同じ fingerprint・同じ被覆。
    let again = derive_assets(
        project(),
        &DeriveInput {
            rules: extraction.rules.clone(),
            impact_items: Vec::new(),
            max_drafts: 5_000,
            base_path: "/sample/user/edit".to_owned(),
            plan_code: "TP-GOLDEN".to_owned(),
        },
    );
    assert_eq!(derivation, again);

    let case_id = derivation.cases[0].id;
    assert_eq!(
        fingerprint(case_id, "ASSERTION", 3, Some("３ 件"), Some("4 件"), false),
        fingerprint(case_id, "ASSERTION", 3, Some("3 件"), Some("4 件"), false),
    );

    let sample_cases: Vec<CoverageCase> = derivation
        .cases
        .iter()
        .take(100)
        .map(|case| CoverageCase {
            case_id: case.id,
            status: "APPROVED".to_owned(),
            covers_rules: case.covers_rules.clone(),
            covers_impacts: case.covers_impacts.clone(),
            covers_screens: case.covers_screens.clone(),
            executed: true,
            passed: true,
            skipped: false,
        })
        .collect();
    let totals = CoverageTotals {
        rule_ids: extraction.rules.iter().map(|rule| rule.id).collect(),
        screen_ids: vec!["SAMPLE-01".to_owned()],
        impact_ids: Vec::new(),
        requirement_ids: Vec::new(),
        regression_case_ids: sample_cases.iter().map(|case| case.case_id).collect(),
        graph_verifies_available: false,
        requirement_covered: Vec::new(),
    };
    let first = compute_coverage(&totals, &sample_cases);
    let second = compute_coverage(&totals, &sample_cases);
    assert_eq!(first, second);
    assert!(first.rule.num > 0);
    assert_eq!(
        first.regression.num, 100,
        "合格ケースは回帰の分子に算入される"
    );
    assert_eq!(first.requirement.num, 0, "グラフ未接続なら近似値を出さない");
    assert!(first
        .requirement
        .degraded
        .contains(&"REQUIREMENT_VIA_RULE".to_owned()));
}
