//! 被覆 5 指標（`04-コアエンジン` §4.6 / `detail/09` §9）。
//!
//! 算入しない: DRAFT / 未実行 / BLOCKED・SKIPPED / covers が空。
//! 「生成したが未実行」は被覆 0（R-03 / R-04）。
//! Requirement だけはグラフの `VERIFIES` 辺が要る。辺が無い縦断では
//! 近似値を出さず、num=0 + degraded で明示する。

use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageCase {
    pub case_id: Uuid,
    /// DRAFT / APPROVED / DEPRECATED
    pub status: String,
    pub covers_rules: Vec<Uuid>,
    pub covers_impacts: Vec<Uuid>,
    pub covers_screens: Vec<String>,
    pub executed: bool,
    pub passed: bool,
    pub skipped: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageTotals {
    pub rule_ids: Vec<Uuid>,
    pub screen_ids: Vec<String>,
    pub impact_ids: Vec<Uuid>,
    pub requirement_ids: Vec<Uuid>,
    pub regression_case_ids: Vec<Uuid>,
    /// グラフに `TestCase-[:VERIFIES]->Requirement` があるか。
    pub graph_verifies_available: bool,
    /// グラフから得た「当該 run で PASSED の TestCase が検証する要件」。
    pub requirement_covered: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MetricResult {
    pub num: i64,
    pub den: i64,
    pub ratio: f64,
    pub degraded: Vec<String>,
}

impl MetricResult {
    fn new(num: usize, den: usize, extra_degraded: &[&str]) -> Self {
        let mut degraded: Vec<String> = extra_degraded.iter().map(|s| (*s).to_owned()).collect();
        if den == 0 {
            degraded.push("denominator_zero".to_owned());
        }
        let ratio = if den == 0 {
            0.0
        } else {
            (num as f64 / den as f64 * 10_000.0).round() / 10_000.0
        };
        Self {
            num: num as i64,
            den: den as i64,
            ratio,
            degraded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExcludedResult {
    pub draft_cases: i64,
    pub never_executed: i64,
    pub skipped: i64,
    pub covers_empty: i64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CoverageResult {
    pub requirement: MetricResult,
    pub rule: MetricResult,
    pub screen: MetricResult,
    pub impact: MetricResult,
    pub regression: MetricResult,
    pub excluded: ExcludedResult,
    pub degraded: Vec<String>,
}

fn intersection_size<T: Ord + Clone>(eligible: &BTreeSet<T>, universe: &[T]) -> usize {
    let set: BTreeSet<T> = universe.iter().cloned().collect();
    eligible.intersection(&set).count()
}

pub fn compute_coverage(totals: &CoverageTotals, cases: &[CoverageCase]) -> CoverageResult {
    let excluded = ExcludedResult {
        draft_cases: cases.iter().filter(|c| c.status != "APPROVED").count() as i64,
        never_executed: cases.iter().filter(|c| !c.executed).count() as i64,
        skipped: cases.iter().filter(|c| c.skipped).count() as i64,
        covers_empty: cases
            .iter()
            .filter(|c| {
                c.covers_rules.is_empty()
                    && c.covers_impacts.is_empty()
                    && c.covers_screens.is_empty()
            })
            .count() as i64,
    };

    // 「承認済み + 実行済み + 合格 + スキップでない」だけが分子の根拠。
    let eligible: Vec<&CoverageCase> = cases
        .iter()
        .filter(|c| c.status == "APPROVED" && c.executed && c.passed && !c.skipped)
        .collect();

    let mut rules = BTreeSet::new();
    let mut screens = BTreeSet::new();
    let mut impacts = BTreeSet::new();
    let mut passed_case_ids = BTreeSet::new();
    for case in &eligible {
        rules.extend(case.covers_rules.iter().copied());
        screens.extend(case.covers_screens.iter().cloned());
        impacts.extend(case.covers_impacts.iter().copied());
        passed_case_ids.insert(case.case_id);
    }

    let requirement_degraded: Vec<&str> = if totals.graph_verifies_available {
        vec![]
    } else {
        vec!["REQUIREMENT_VIA_RULE", "GRAPH_VERIFIES_UNAVAILABLE"]
    };
    let requirement_num = if totals.graph_verifies_available {
        let covered: BTreeSet<Uuid> = totals.requirement_covered.iter().copied().collect();
        intersection_size(&covered, &totals.requirement_ids)
    } else {
        0
    };

    let requirement = MetricResult::new(
        requirement_num,
        totals.requirement_ids.len(),
        &requirement_degraded,
    );
    let rule = MetricResult::new(
        intersection_size(&rules, &totals.rule_ids),
        totals.rule_ids.len(),
        &[],
    );
    let screen = MetricResult::new(
        intersection_size(&screens, &totals.screen_ids),
        totals.screen_ids.len(),
        &[],
    );
    let impact = MetricResult::new(
        intersection_size(&impacts, &totals.impact_ids),
        totals.impact_ids.len(),
        &[],
    );
    let regression = MetricResult::new(
        intersection_size(&passed_case_ids, &totals.regression_case_ids),
        totals.regression_case_ids.len(),
        &[],
    );

    let mut degraded = Vec::new();
    if !totals.graph_verifies_available {
        degraded.push("REQUIREMENT_VIA_RULE".to_owned());
        degraded.push("GRAPH_VERIFIES_UNAVAILABLE".to_owned());
    }
    if totals.impact_ids.is_empty() {
        degraded.push("tests_not_available".to_owned());
    }
    degraded.sort();
    degraded.dedup();

    CoverageResult {
        requirement,
        rule,
        screen,
        impact,
        regression,
        excluded,
        degraded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn totals() -> CoverageTotals {
        CoverageTotals {
            rule_ids: vec![id(1), id(2), id(3), id(4)],
            screen_ids: vec!["S1".to_owned(), "S2".to_owned()],
            impact_ids: vec![id(10), id(11)],
            requirement_ids: vec![id(20), id(21), id(22)],
            regression_case_ids: vec![id(100), id(101), id(102)],
            graph_verifies_available: false,
            requirement_covered: Vec::new(),
        }
    }

    fn case(n: u128, status: &str, executed: bool, passed: bool) -> CoverageCase {
        CoverageCase {
            case_id: id(n),
            status: status.to_owned(),
            covers_rules: vec![id(1), id(2)],
            covers_impacts: vec![id(10)],
            covers_screens: vec!["S1".to_owned()],
            executed,
            passed,
            skipped: false,
        }
    }

    #[test]
    fn r02_same_input_yields_identical_numbers() {
        let cases = vec![
            case(100, "APPROVED", true, true),
            case(101, "APPROVED", false, false),
        ];
        assert_eq!(
            compute_coverage(&totals(), &cases),
            compute_coverage(&totals(), &cases)
        );
    }

    #[test]
    fn r03_r04_draft_and_unexecuted_are_zero() {
        let cases = vec![
            case(100, "DRAFT", true, true),
            case(101, "APPROVED", false, false),
        ];
        let out = compute_coverage(&totals(), &cases);
        assert_eq!(out.rule.num, 0);
        assert_eq!(out.regression.num, 0);
        assert_eq!(out.excluded.draft_cases, 1);
        assert_eq!(out.excluded.never_executed, 1);
    }

    #[test]
    fn only_passed_approved_cases_count() {
        let cases = vec![
            case(100, "APPROVED", true, true),
            case(101, "APPROVED", true, false),
            case(102, "APPROVED", true, true),
        ];
        let out = compute_coverage(&totals(), &cases);
        assert_eq!(out.rule.num, 2, "covers_rules の和集合は 2 つ");
        assert_eq!(out.impact.num, 1);
        assert_eq!(out.screen.num, 1);
        assert_eq!(out.regression.num, 2);
    }

    #[test]
    fn requirement_is_zero_and_flagged_without_graph() {
        let cases = vec![case(100, "APPROVED", true, true)];
        let out = compute_coverage(&totals(), &cases);
        assert_eq!(out.requirement.num, 0);
        assert!(out
            .requirement
            .degraded
            .contains(&"REQUIREMENT_VIA_RULE".to_owned()));
        assert!(out
            .degraded
            .contains(&"GRAPH_VERIFIES_UNAVAILABLE".to_owned()));
    }

    #[test]
    fn denominator_zero_is_explicit() {
        let out = compute_coverage(&CoverageTotals::default(), &[]);
        assert_eq!(out.rule.ratio, 0.0);
        assert!(out.rule.degraded.contains(&"denominator_zero".to_owned()));
    }

    #[test]
    fn skipped_cases_are_excluded() {
        let mut skipped = case(100, "APPROVED", true, true);
        skipped.skipped = true;
        let out = compute_coverage(&totals(), &[skipped]);
        assert_eq!(out.rule.num, 0);
        assert_eq!(out.excluded.skipped, 1);
    }
}
