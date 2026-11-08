//! ステップ DSL の保存時検証と実行ガード（`detail/09` §4.5 / §6 / §7 / §12）。
//!
//! 検証は「保存時」。実行時に静かに SKIPPED にして気付かれない状態を作らない。
//! Runner は実装していないが、DSL の安全性だけは先に固定する。

use std::collections::BTreeSet;

use contracts::{ExecutionImport, TestCaseDsl, TestEnvironment};

/// v=1 の語彙。`custom_step` は既定無効なので含めない。
pub const V1_ACTIONS: &[&str] = &[
    "goto",
    "fill",
    "select",
    "check",
    "uncheck",
    "click",
    "press",
    "upload",
    "wait_for",
    "assert_text",
    "assert_value",
    "assert_visible",
    "assert_hidden",
    "assert_count",
    "assert_url",
    "assert_table",
    "field_error",
    "screenshot",
];

pub const ASSERTION_ACTIONS: &[&str] = &[
    "assert_text",
    "assert_value",
    "assert_visible",
    "assert_hidden",
    "assert_count",
    "assert_url",
    "assert_table",
    "field_error",
];

/// `test.test_execution.failure_kind` の 8 値（DB の CHECK と一致）。
pub const FAILURE_KINDS: &[&str] = &[
    "ASSERTION",
    "SELECTOR_NOT_FOUND",
    "TIMEOUT",
    "NETWORK",
    "DATA",
    "ENV",
    "RUNNER",
    "CANCELLED",
];

const EXECUTION_STATUSES: &[&str] = &["PENDING", "PASSED", "FAILED", "BLOCKED", "SKIPPED", "ERROR"];
const EXECUTION_SOURCES: &[&str] = &["IMPORTED", "PLAYWRIGHT", "SIMULATED"];

fn host_of(url: &str) -> String {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    after_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn origin_allowed(url: &str, env: &TestEnvironment) -> bool {
    if !url.contains("://") {
        return true; // 相対 path は environment.base_url 側に寄る
    }
    let host = host_of(url);
    if host.is_empty() {
        return false;
    }
    let mut allowed = BTreeSet::new();
    allowed.insert(host_of(&env.base_url));
    for origin in &env.allowed_origins {
        allowed.insert(host_of(origin));
    }
    allowed.contains(&host)
}

/// 保存時に拒否すべき理由を全部返す。空なら通過。
pub fn validate_dsl(dsl: &TestCaseDsl, env: &TestEnvironment) -> Vec<String> {
    let mut errors = Vec::new();
    if dsl.v != 1 {
        errors.push(format!("unsupported_dsl_version:{}", dsl.v));
    }
    if dsl.steps.is_empty() {
        errors.push("no_steps".to_owned());
    }
    let actions: BTreeSet<&str> = V1_ACTIONS.iter().copied().collect();
    let assertions: BTreeSet<&str> = ASSERTION_ACTIONS.iter().copied().collect();
    let mut assertion_count = 0usize;

    for (index, step) in dsl.steps.iter().enumerate() {
        if step.action == "custom_step" {
            errors.push(format!("custom_step_disabled:step{index}"));
            continue;
        }
        if !actions.contains(step.action.as_str()) {
            errors.push(format!("unknown_action:{}:step{index}", step.action));
        }
        if assertions.contains(step.action.as_str()) {
            assertion_count += 1;
        }
        if step.action == "wait_for" && step.ms.unwrap_or(0) > 5000 {
            errors.push(format!("wait_for_ms_exceeds_5000:step{index}"));
        }
        if let Some(css) = step.target.get("css") {
            if css.contains("nth-child") || css.contains(":has(") {
                errors.push(format!("brittle_css_selector:step{index}"));
            }
        }
        for key in ["url", "path"] {
            if let Some(url) = step.target.get(key) {
                if !origin_allowed(url, env) {
                    errors.push(format!("origin_not_allowed:step{index}"));
                }
            }
        }
        if let Some(value) = &step.value {
            if value.contains("${env.") || value.contains("${secret.") {
                errors.push(format!("forbidden_binding:step{index}"));
            }
        }
    }
    if assertion_count == 0 {
        errors.push("no_assertion".to_owned());
    }
    errors.sort();
    errors.dedup();
    errors
}

/// `@destructive` を実行前に SKIPPED にする（R-07）。
///
/// **許可リスト方式**にする。`PRODUCTION かどうか` を判定すると、`prod` や
/// 空白混じりの値、将来追加された未知の環境名が「本番ではない」と解釈され、
/// 破壊的ケースが素通りする。**知らない環境では実行しない**（fail-closed）。
pub fn guard_execution(destructive: bool, env: &TestEnvironment) -> bool {
    if !destructive {
        return false;
    }
    !env.guard_kind.eq_ignore_ascii_case("LOCAL") && !env.guard_kind.eq_ignore_ascii_case("STAGING")
}

/// 再試行は TIMEOUT / NETWORK の 1 回だけ（detail/09 §6.4）。
pub fn retry_allowed(failure_kind: &str) -> bool {
    matches!(failure_kind, "TIMEOUT" | "NETWORK")
}

pub struct ImportContext {
    pub case_status: String,
    pub destructive: bool,
    pub guard_kind: String,
    /// 同一 run / case の attempt=1 の失敗種別。
    pub attempt1_failure_kind: Option<String>,
}

pub fn validate_import(item: &ExecutionImport, ctx: &ImportContext) -> Vec<String> {
    let mut errors = Vec::new();
    if !EXECUTION_STATUSES.contains(&item.status.as_str()) {
        errors.push(format!("invalid_execution_status:{}", item.status));
    }
    if !EXECUTION_SOURCES.contains(&item.execution_source.as_str()) {
        errors.push(format!(
            "invalid_execution_source:{}",
            item.execution_source
        ));
    }
    if let Some(kind) = &item.failure_kind {
        if !FAILURE_KINDS.contains(&kind.as_str()) {
            errors.push(format!("invalid_failure_kind:{kind}"));
        }
    }
    if item.status == "FAILED" && item.failure_kind.is_none() {
        errors.push("failed_without_failure_kind".to_owned());
    }
    if item.status == "PASSED" && item.failure_kind.is_some() {
        errors.push("passed_with_failure_kind".to_owned());
    }
    if !(1..=2).contains(&item.attempt) {
        errors.push(format!("invalid_attempt:{}", item.attempt));
    }
    if item.attempt == 2 {
        match &ctx.attempt1_failure_kind {
            Some(kind) if retry_allowed(kind) => {}
            Some(kind) => errors.push(format!("retry_not_allowed:{kind}")),
            None => errors.push("retry_without_first_failure".to_owned()),
        }
    }
    if ctx.case_status != "APPROVED" && item.status != "SKIPPED" {
        errors.push(format!("case_not_approved:{}", ctx.case_status));
    }
    // `guard_execution` と同じ許可リストで判定する。片方だけ「PRODUCTION か」で
    // 見ると、未知の環境名で 2 つの判定が食い違う。
    if ctx.destructive
        && item.status != "SKIPPED"
        && !ctx.guard_kind.eq_ignore_ascii_case("LOCAL")
        && !ctx.guard_kind.eq_ignore_ascii_case("STAGING")
    {
        errors.push("destructive_in_production".to_owned());
    }
    errors.sort();
    errors.dedup();
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::DslStep;
    use std::collections::BTreeMap;

    fn env() -> TestEnvironment {
        TestEnvironment {
            id: "env-stg-01".to_owned(),
            base_url: "https://stg.example.internal/app".to_owned(),
            guard_kind: "STAGING".to_owned(),
            allowed_origins: vec!["stg.example.internal".to_owned()],
            parallel: 4,
        }
    }

    fn valid_dsl() -> TestCaseDsl {
        TestCaseDsl {
            v: 1,
            case_code: "TC-1".to_owned(),
            title: "t".to_owned(),
            preconditions: Vec::new(),
            steps: vec![
                DslStep {
                    action: "goto".to_owned(),
                    target: BTreeMap::from([("path".to_owned(), "/user/edit".to_owned())]),
                    ..DslStep::default()
                },
                DslStep {
                    action: "field_error".to_owned(),
                    target: BTreeMap::from([("label".to_owned(), "住所".to_owned())]),
                    value: Some("エラー".to_owned()),
                    ..DslStep::default()
                },
            ],
            data: None,
            selector_strategy: "role_first".to_owned(),
        }
    }

    #[test]
    fn valid_dsl_passes() {
        assert!(validate_dsl(&valid_dsl(), &env()).is_empty());
    }

    #[test]
    fn r14_wait_for_over_5000_is_rejected() {
        let mut dsl = valid_dsl();
        dsl.steps.insert(
            1,
            DslStep {
                action: "wait_for".to_owned(),
                ms: Some(6000),
                ..DslStep::default()
            },
        );
        assert!(validate_dsl(&dsl, &env())
            .iter()
            .any(|e| e.starts_with("wait_for_ms_exceeds_5000")));
    }

    #[test]
    fn r15_unknown_action_is_rejected() {
        let mut dsl = valid_dsl();
        dsl.steps.push(DslStep {
            action: "teleport".to_owned(),
            ..DslStep::default()
        });
        assert!(validate_dsl(&dsl, &env())
            .iter()
            .any(|e| e.starts_with("unknown_action")));
    }

    #[test]
    fn r08_external_origin_is_rejected() {
        let mut dsl = valid_dsl();
        dsl.steps[0].target.insert(
            "url".to_owned(),
            "https://evil.example.com/login".to_owned(),
        );
        assert!(validate_dsl(&dsl, &env())
            .iter()
            .any(|e| e.starts_with("origin_not_allowed")));
    }

    #[test]
    fn no_assertion_is_rejected() {
        let mut dsl = valid_dsl();
        dsl.steps.pop();
        assert!(validate_dsl(&dsl, &env()).contains(&"no_assertion".to_owned()));
    }

    #[test]
    fn secret_binding_and_brittle_css_are_rejected() {
        let mut dsl = valid_dsl();
        dsl.steps[1].value = Some("${secret.token}".to_owned());
        dsl.steps[1]
            .target
            .insert("css".to_owned(), "li:nth-child(2)".to_owned());
        let errors = validate_dsl(&dsl, &env());
        assert!(errors.iter().any(|e| e.starts_with("forbidden_binding")));
        assert!(errors.iter().any(|e| e.starts_with("brittle_css_selector")));
    }

    #[test]
    fn r07_production_destructive_is_skipped() {
        let mut production = env();
        production.guard_kind = "PRODUCTION".to_owned();
        assert!(guard_execution(true, &production));
        assert!(!guard_execution(false, &production));
        assert!(!guard_execution(true, &env()));
    }

    #[test]
    fn r07_unknown_environment_does_not_run_destructive() {
        // 許可リスト方式。知らない環境名で破壊的ケースを走らせない。
        for unknown in ["prod", "PRODUCTION ", "PREPROD", ""] {
            let mut environment = env();
            environment.guard_kind = unknown.to_owned();
            assert!(
                guard_execution(true, &environment),
                "destructive must be skipped for guard_kind={unknown:?}"
            );
        }
        // 明示的に許可された環境だけ実行する（大文字小文字は問わない）。
        for allowed in ["LOCAL", "local", "STAGING", "staging"] {
            let mut environment = env();
            environment.guard_kind = allowed.to_owned();
            assert!(
                !guard_execution(true, &environment),
                "destructive must run for guard_kind={allowed:?}"
            );
        }
    }

    #[test]
    fn r05_r06_retry_policy() {
        assert!(!retry_allowed("ASSERTION"));
        assert!(retry_allowed("TIMEOUT"));
        assert!(retry_allowed("NETWORK"));
        assert!(!retry_allowed("SELECTOR_NOT_FOUND"));
    }
}
