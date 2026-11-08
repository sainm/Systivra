//! `test` schema へのアクセス。SQL と行→構造体の変換だけを持つ。

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

// ── 書き込みパラメータ ──────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NewValidationRule {
    pub id: Uuid,
    pub project_id: Uuid,
    pub field_ref: String,
    pub kind: String,
    pub condition: String,
    pub source: String,
    pub evidence_ids: Vec<Uuid>,
    pub resolution: String,
    pub condition_key: String,
}

#[derive(Debug, Clone)]
pub struct NewTestPlan {
    pub id: Uuid,
    pub project_id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub scope: Value,
    pub impact_id: Option<Uuid>,
    pub origin: String,
}

#[derive(Debug, Clone)]
pub struct NewTestScenario {
    pub id: Uuid,
    pub project_id: Uuid,
    pub plan_id: Uuid,
    pub code: String,
    pub title: String,
    pub kind: String,
    pub evidence_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct NewTestCase {
    pub id: Uuid,
    pub project_id: Uuid,
    pub scenario_id: Uuid,
    pub code: String,
    pub title: String,
    pub status: String,
    pub dsl: Value,
    pub preconditions: Value,
    pub expected: Value,
    pub covers_rules: Vec<Uuid>,
    pub covers_impacts: Vec<Uuid>,
    pub covers_screens: Vec<String>,
    pub automation: Value,
    pub destructive: bool,
    pub generated_by: Option<Uuid>,
    pub evidence_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct NewTestStep {
    pub id: Uuid,
    pub project_id: Uuid,
    pub case_id: Uuid,
    pub ordinal: i32,
    pub action: String,
    pub target: Value,
    pub value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewTestDataSet {
    pub id: Uuid,
    pub project_id: Uuid,
    pub case_id: Uuid,
    pub label: String,
    pub kind: String,
    pub rule_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct NewTestDataRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub data_set_id: Uuid,
    pub field_ref: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct NewTestRun {
    pub id: Uuid,
    pub project_id: Uuid,
    pub plan_id: Option<Uuid>,
    pub name: String,
    pub environment: Value,
    pub environment_id: String,
    pub status: String,
    pub lock_acquired: bool,
}

#[derive(Debug, Clone)]
pub struct NewTestExecution {
    pub id: Uuid,
    pub project_id: Uuid,
    pub test_run_id: Uuid,
    pub test_case_id: Uuid,
    pub attempt: i16,
    pub status: String,
    pub execution_source: String,
    pub failure_kind: Option<String>,
    pub failure_step: Option<i32>,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub duration_ms: Option<i32>,
    pub artifacts: Value,
    pub fingerprint: Option<String>,
}

// ── 行 ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ValidationRuleRow {
    pub id: Uuid,
    pub field_ref: String,
    pub kind: String,
    pub condition: String,
    pub source: String,
    pub resolution: String,
    pub evidence_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TestPlanRow {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub origin: String,
    pub impact_id: Option<Uuid>,
    pub scope: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TestCaseRow {
    pub id: Uuid,
    pub scenario_id: Uuid,
    pub plan_id: Uuid,
    pub code: String,
    pub title: String,
    pub status: String,
    pub dsl: Value,
    pub covers_rules: Vec<Uuid>,
    pub covers_impacts: Vec<Uuid>,
    pub covers_screens: Vec<String>,
    pub evidence_ids: Vec<Uuid>,
    pub destructive: bool,
}

#[derive(Debug, Clone)]
pub struct TestDataRow {
    pub case_id: Uuid,
    pub label: String,
    pub kind: String,
    pub field_ref: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct TestRunRow {
    pub id: Uuid,
    pub plan_id: Option<Uuid>,
    pub name: String,
    pub status: String,
    pub environment: Value,
    pub environment_id: String,
    pub guard_kind: String,
    pub lock_acquired: bool,
    pub case_count: i32,
    pub job_id: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TestExecutionRow {
    pub id: Uuid,
    pub test_run_id: Uuid,
    pub test_case_id: Uuid,
    pub attempt: i16,
    pub status: String,
    pub execution_source: String,
    pub failure_kind: Option<String>,
    pub failure_step: Option<i32>,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub fingerprint: Option<String>,
    pub artifacts: Value,
}

#[derive(Debug, Clone)]
pub struct DefectRow {
    pub id: Uuid,
    pub fingerprint: String,
    pub title: String,
    pub severity: String,
    pub status: String,
    pub recurrence: i32,
    pub execution_id: Option<Uuid>,
    pub impact_item_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CoverageSnapshotRow {
    pub id: Uuid,
    pub test_run_id: Uuid,
    pub scope: Value,
    pub detail: Value,
    pub computed_at: DateTime<Utc>,
}

impl Store {
    // ── rules ───────────────────────────────────────────────

    pub async fn insert_validation_rules(
        &self,
        tx: &mut Tx<'_>,
        rules: &[NewValidationRule],
    ) -> Result<usize, StoreError> {
        let mut written = 0usize;
        for rule in rules {
            let result = sqlx::query(
                "INSERT INTO test.validation_rule
                   (id, project_id, field_ref, kind, condition, source,
                    evidence_ids, resolution, condition_key)
                 VALUES ($1,$2,$3,$4,$5,$6,$7::uuid[],$8,$9)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(rule.id)
            .bind(rule.project_id)
            .bind(&rule.field_ref)
            .bind(&rule.kind)
            .bind(&rule.condition)
            .bind(&rule.source)
            .bind(&rule.evidence_ids)
            .bind(&rule.resolution)
            .bind(&rule.condition_key)
            .execute(&mut **tx)
            .await?;
            written += result.rows_affected() as usize;
        }
        Ok(written)
    }

    pub async fn list_validation_rules(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<ValidationRuleRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, field_ref, kind, condition, source, resolution,
                    evidence_ids, created_at
               FROM test.validation_rule
              WHERE project_id = $1
              ORDER BY field_ref, kind, condition, id",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(validation_rule_row).collect())
    }

    // ── plan / scenario / case ──────────────────────────────

    pub async fn insert_test_plan(
        &self,
        tx: &mut Tx<'_>,
        plan: &NewTestPlan,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO test.test_plan
               (id, project_id, code, name, status, scope, impact_id, origin)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(plan.id)
        .bind(plan.project_id)
        .bind(&plan.code)
        .bind(&plan.name)
        .bind(&plan.status)
        .bind(&plan.scope)
        .bind(plan.impact_id)
        .bind(&plan.origin)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_test_plans(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<TestPlanRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, code, name, status, origin, impact_id, scope, created_at
               FROM test.test_plan
              WHERE project_id = $1
              ORDER BY created_at DESC, code",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(test_plan_row).collect())
    }

    pub async fn test_plan(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        plan_id: Uuid,
    ) -> Result<Option<TestPlanRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, code, name, status, origin, impact_id, scope, created_at
               FROM test.test_plan
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(plan_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(test_plan_row))
    }

    /// 既存計画へ追記するとき、scope を差し替える。
    /// 計画そのもの（code / name / status）は変えない。
    pub async fn update_test_plan_scope(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        plan_id: Uuid,
        scope: &Value,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE test.test_plan SET scope = $3
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(plan_id)
        .bind(scope)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn insert_test_scenarios(
        &self,
        tx: &mut Tx<'_>,
        scenarios: &[NewTestScenario],
    ) -> Result<(), StoreError> {
        for scenario in scenarios {
            sqlx::query(
                "INSERT INTO test.test_scenario
                   (id, project_id, plan_id, code, title, kind, evidence_ids)
                 VALUES ($1,$2,$3,$4,$5,$6,$7::uuid[])
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(scenario.id)
            .bind(scenario.project_id)
            .bind(scenario.plan_id)
            .bind(&scenario.code)
            .bind(&scenario.title)
            .bind(&scenario.kind)
            .bind(&scenario.evidence_ids)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    pub async fn insert_test_cases(
        &self,
        tx: &mut Tx<'_>,
        cases: &[NewTestCase],
    ) -> Result<(), StoreError> {
        for case in cases {
            sqlx::query(
                "INSERT INTO test.test_case
                   (id, project_id, scenario_id, code, title, status, dsl,
                    preconditions, expected, covers_rules, covers_impacts,
                    covers_screens, automation, destructive, generated_by)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::uuid[],$11::uuid[],
                         $12::text[],$13,$14,$15)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(case.id)
            .bind(case.project_id)
            .bind(case.scenario_id)
            .bind(&case.code)
            .bind(&case.title)
            .bind(&case.status)
            .bind(&case.dsl)
            .bind(&case.preconditions)
            .bind(&case.expected)
            .bind(&case.covers_rules)
            .bind(&case.covers_impacts)
            .bind(&case.covers_screens)
            .bind(&case.automation)
            .bind(case.destructive)
            .bind(case.generated_by)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    pub async fn list_test_cases(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<TestCaseRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT c.id, c.scenario_id, s.plan_id, c.code, c.title, c.status, c.dsl,
                    c.covers_rules, c.covers_impacts, c.covers_screens,
                    c.destructive, s.evidence_ids
               FROM test.test_case c
               JOIN test.test_scenario s ON s.id = c.scenario_id
              WHERE c.project_id = $1
              ORDER BY c.code",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(test_case_row).collect())
    }

    pub async fn test_case(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        case_id: Uuid,
    ) -> Result<Option<TestCaseRow>, StoreError> {
        let row = sqlx::query(
            "SELECT c.id, c.scenario_id, s.plan_id, c.code, c.title, c.status, c.dsl,
                    c.covers_rules, c.covers_impacts, c.covers_screens,
                    c.destructive, s.evidence_ids
               FROM test.test_case c
               JOIN test.test_scenario s ON s.id = c.scenario_id
              WHERE c.project_id = $1 AND c.id = $2",
        )
        .bind(project_id)
        .bind(case_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(test_case_row))
    }

    pub async fn approve_test_case(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        case_id: Uuid,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE test.test_case SET status = 'APPROVED'
              WHERE project_id = $1 AND id = $2 AND status = 'DRAFT'",
        )
        .bind(project_id)
        .bind(case_id)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn insert_test_steps(
        &self,
        tx: &mut Tx<'_>,
        steps: &[NewTestStep],
    ) -> Result<(), StoreError> {
        for step in steps {
            sqlx::query(
                "INSERT INTO test.test_step
                   (id, project_id, case_id, ordinal, action, target, value)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)
                 ON CONFLICT (case_id, ordinal) DO NOTHING",
            )
            .bind(step.id)
            .bind(step.project_id)
            .bind(step.case_id)
            .bind(step.ordinal)
            .bind(&step.action)
            .bind(&step.target)
            .bind(&step.value)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    pub async fn insert_test_data(
        &self,
        tx: &mut Tx<'_>,
        sets: &[NewTestDataSet],
        rows: &[NewTestDataRow],
    ) -> Result<(), StoreError> {
        for set in sets {
            sqlx::query(
                "INSERT INTO test.test_data_set
                   (id, project_id, case_id, label, kind, rule_id)
                 VALUES ($1,$2,$3,$4,$5,$6)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(set.id)
            .bind(set.project_id)
            .bind(set.case_id)
            .bind(&set.label)
            .bind(&set.kind)
            .bind(set.rule_id)
            .execute(&mut **tx)
            .await?;
        }
        for row in rows {
            sqlx::query(
                "INSERT INTO test.test_data_row
                   (id, project_id, data_set_id, field_ref, value)
                 VALUES ($1,$2,$3,$4,$5)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(row.project_id)
            .bind(row.data_set_id)
            .bind(&row.field_ref)
            .bind(&row.value)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    pub async fn list_test_data(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<TestDataRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT s.case_id, s.label, s.kind, r.field_ref, r.value
               FROM test.test_data_row r
               JOIN test.test_data_set s ON s.id = r.data_set_id
              WHERE r.project_id = $1
              ORDER BY s.case_id, s.label, r.field_ref",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| TestDataRow {
                case_id: row.get("case_id"),
                label: row.get("label"),
                kind: row.get("kind"),
                field_ref: row.get("field_ref"),
                value: row.get("value"),
            })
            .collect())
    }

    // ── run ─────────────────────────────────────────────────

    pub async fn env_lock_holder(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        environment_id: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        let row = sqlx::query(
            "SELECT id FROM test.test_run
              WHERE project_id = $1 AND environment_id = $2 AND lock_acquired
              LIMIT 1",
        )
        .bind(project_id)
        .bind(environment_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(|r| r.get("id")))
    }

    pub async fn insert_test_run(
        &self,
        tx: &mut Tx<'_>,
        run: &NewTestRun,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO test.test_run
               (id, project_id, plan_id, name, environment, environment_id,
                status, lock_acquired)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(run.id)
        .bind(run.project_id)
        .bind(run.plan_id)
        .bind(&run.name)
        .bind(&run.environment)
        .bind(&run.environment_id)
        .bind(&run.status)
        .bind(run.lock_acquired)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_test_runs(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<TestRunRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, plan_id, name, status, environment, environment_id,
                    lock_acquired, case_count, job_id, started_at, finished_at, created_at
               FROM test.test_run
              WHERE project_id = $1
              ORDER BY created_at DESC, name",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(test_run_row).collect())
    }

    pub async fn test_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
    ) -> Result<Option<TestRunRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, plan_id, name, status, environment, environment_id,
                    lock_acquired, case_count, job_id, started_at, finished_at, created_at
               FROM test.test_run
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(test_run_row))
    }

    pub async fn set_test_run_job(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE test.test_run SET job_id = $3 WHERE project_id = $1 AND id = $2")
            .bind(project_id)
            .bind(run_id)
            .bind(job_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn finish_test_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
        status: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE test.test_run
                SET status = $3, lock_acquired = false, finished_at = now()
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(run_id)
        .bind(status)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// ロックを解放した後に呼ぶ。同環境の最古 QUEUED run を 1 件だけ起動可能にする。
    pub async fn activate_next_queued_run(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        environment_id: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        let row = sqlx::query(
            "UPDATE test.test_run SET lock_acquired = true
              WHERE id = (
                SELECT id FROM test.test_run
                 WHERE project_id = $1 AND environment_id = $2
                   AND status = 'QUEUED' AND NOT lock_acquired
                 ORDER BY created_at, id
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED
              )
              RETURNING id",
        )
        .bind(project_id)
        .bind(environment_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(|r| r.get("id")))
    }

    pub async fn set_test_run_case_count(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
        count: i32,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE test.test_run SET case_count = $3, status = 'READY'
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(run_id)
        .bind(count)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    // ── execution ───────────────────────────────────────────

    pub async fn insert_test_execution(
        &self,
        tx: &mut Tx<'_>,
        execution: &NewTestExecution,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO test.test_execution
               (id, project_id, test_run_id, test_case_id, attempt, status,
                execution_source, failure_kind, failure_step, expected, actual,
                duration_ms, artifacts, fingerprint, started_at, finished_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,now(),now())
             ON CONFLICT (test_run_id, test_case_id, attempt) DO NOTHING",
        )
        .bind(execution.id)
        .bind(execution.project_id)
        .bind(execution.test_run_id)
        .bind(execution.test_case_id)
        .bind(execution.attempt)
        .bind(&execution.status)
        .bind(&execution.execution_source)
        .bind(&execution.failure_kind)
        .bind(execution.failure_step)
        .bind(&execution.expected)
        .bind(&execution.actual)
        .bind(execution.duration_ms)
        .bind(&execution.artifacts)
        .bind(&execution.fingerprint)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_test_executions(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<TestExecutionRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, test_run_id, test_case_id, attempt, status,
                    execution_source, failure_kind, failure_step, expected,
                    actual, fingerprint, artifacts
               FROM test.test_execution
              WHERE project_id = $1
              ORDER BY test_run_id, test_case_id, attempt",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(test_execution_row).collect())
    }

    pub async fn test_execution(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        execution_id: Uuid,
    ) -> Result<Option<TestExecutionRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, test_run_id, test_case_id, attempt, status,
                    execution_source, failure_kind, failure_step, expected,
                    actual, fingerprint, artifacts
               FROM test.test_execution
              WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(execution_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(test_execution_row))
    }

    pub async fn executions_with_artifacts(
        &self,
        tx: &mut Tx<'_>,
        project: Option<Uuid>,
    ) -> Result<Vec<(Uuid, Value, DateTime<Utc>)>, StoreError> {
        let rows = match project {
            Some(pid) => {
                sqlx::query(
                    "SELECT id, artifacts, started_at FROM test.test_execution
                      WHERE project_id = $1 AND jsonb_array_length(artifacts) > 0",
                )
                .bind(pid)
                .fetch_all(&mut **tx)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT id, artifacts, started_at FROM test.test_execution
                      WHERE jsonb_array_length(artifacts) > 0",
                )
                .fetch_all(&mut **tx)
                .await?
            }
        };
        Ok(rows
            .iter()
            .map(|row| (row.get("id"), row.get("artifacts"), row.get("started_at")))
            .collect())
    }

    pub async fn update_execution_artifacts(
        &self,
        tx: &mut Tx<'_>,
        execution_id: Uuid,
        artifacts: &Value,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE test.test_execution SET artifacts = $2 WHERE id = $1")
            .bind(execution_id)
            .bind(artifacts)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn attempt1_failure_kind(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
        case_id: Uuid,
    ) -> Result<Option<String>, StoreError> {
        let row = sqlx::query(
            "SELECT failure_kind FROM test.test_execution
              WHERE project_id = $1 AND test_run_id = $2
                AND test_case_id = $3 AND attempt = 1",
        )
        .bind(project_id)
        .bind(run_id)
        .bind(case_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.and_then(|r| r.get::<Option<String>, _>("failure_kind")))
    }

    // ── defect ──────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_defect(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        id: Uuid,
        fingerprint: &str,
        title: &str,
        severity: &str,
        execution_id: Uuid,
        impact_item_id: Option<Uuid>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO test.defect
               (id, project_id, execution_id, fingerprint, title, severity,
                status, impact_item_id, recurrence)
             VALUES ($1,$2,$3,$4,$5,$6,'CANDIDATE',$7,1)
             ON CONFLICT (project_id, fingerprint)
                WHERE status IN ('CANDIDATE','OPEN')
             DO UPDATE SET recurrence = test.defect.recurrence + 1,
                           detail = jsonb_set(test.defect.detail, '{recurred}', 'true'::jsonb)",
        )
        .bind(id)
        .bind(project_id)
        .bind(execution_id)
        .bind(fingerprint)
        .bind(title)
        .bind(severity)
        .bind(impact_item_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_defects(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<DefectRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, fingerprint, title, severity, status, recurrence,
                    execution_id, impact_item_id, created_at
               FROM test.defect
              WHERE project_id = $1
              ORDER BY created_at DESC, fingerprint",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|row| DefectRow {
                id: row.get("id"),
                fingerprint: row.get("fingerprint"),
                title: row.get("title"),
                severity: row.get("severity"),
                status: row.get("status"),
                recurrence: row.get::<i32, _>("recurrence"),
                execution_id: row.get("execution_id"),
                impact_item_id: row.get("impact_item_id"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    // ── coverage ────────────────────────────────────────────

    pub async fn upsert_coverage_snapshot(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        id: Uuid,
        run_id: Uuid,
        scope: &Value,
        detail: &Value,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO test.coverage_snapshot
               (id, project_id, test_run_id, scope, requirement, rule, screen,
                impact, regression, detail)
             VALUES ($1,$2,$3,$4,
                     ($5->'requirement'->>'ratio')::numeric,
                     ($5->'rule'->>'ratio')::numeric,
                     ($5->'screen'->>'ratio')::numeric,
                     ($5->'impact'->>'ratio')::numeric,
                     ($5->'regression'->>'ratio')::numeric,
                     $5)
             ON CONFLICT (project_id, test_run_id) DO UPDATE
               SET scope = EXCLUDED.scope,
                   requirement = EXCLUDED.requirement,
                   rule = EXCLUDED.rule,
                   screen = EXCLUDED.screen,
                   impact = EXCLUDED.impact,
                   regression = EXCLUDED.regression,
                   detail = EXCLUDED.detail",
        )
        .bind(id)
        .bind(project_id)
        .bind(run_id)
        .bind(scope)
        .bind(detail)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn latest_coverage_snapshot(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Option<CoverageSnapshotRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, test_run_id, scope, detail, computed_at
               FROM test.coverage_snapshot
              WHERE project_id = $1
              ORDER BY computed_at DESC
              LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(|r| CoverageSnapshotRow {
            id: r.get("id"),
            test_run_id: r.get("test_run_id"),
            scope: r.get("scope"),
            detail: r.get("detail"),
            computed_at: r.get("computed_at"),
        }))
    }

    pub async fn coverage_snapshot(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        run_id: Uuid,
    ) -> Result<Option<CoverageSnapshotRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, test_run_id, scope, detail, computed_at
               FROM test.coverage_snapshot
              WHERE project_id = $1 AND test_run_id = $2",
        )
        .bind(project_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(|r| CoverageSnapshotRow {
            id: r.get("id"),
            test_run_id: r.get("test_run_id"),
            scope: r.get("scope"),
            detail: r.get("detail"),
            computed_at: r.get("computed_at"),
        }))
    }
}

// ── テスト資産のグラフ投影（PG 正本 + outbox） ──────────────
//
// PG の graph_meta を正本とし、同じ transaction で outbox へ
// `EntityUpserted` / `RelationshipUpserted` を積む。worker の projector が
// Neo4j へ MERGE する。被覆計算は PG を読み、Neo4j の有無に依存しない。

#[derive(Debug, Clone)]
pub struct TestGraphEntity {
    pub id: Uuid,
    pub natural_key: String,
    pub name: String,
    pub resolution: String,
}

impl Store {
    pub async fn project_test_graph(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        requirements: &[TestGraphEntity],
        cases: &[TestGraphEntity],
        verifies: &[(Uuid, Uuid)],
    ) -> Result<(), StoreError> {
        let ontology_version = self.published_ontology_version(tx).await?;
        for (entity_type, items) in [("Requirement", requirements), ("TestCase", cases)] {
            for item in items {
                sqlx::query(
                    "WITH upserted AS (
                       INSERT INTO graph_meta.entity
                         (id, project_id, entity_type, natural_key, name, properties,
                          ontology_version, revision, resolution,
                          first_seen_rev, last_seen_rev, projection_seq)
                       VALUES ($1,$2,$3,$4,$5,'{}',$6,'test',$7,'test','test',1)
                       ON CONFLICT (project_id, entity_type, natural_key)
                       DO UPDATE SET name = EXCLUDED.name,
                                     resolution = EXCLUDED.resolution,
                                     last_seen_rev = EXCLUDED.last_seen_rev,
                                     projection_seq = graph_meta.entity.projection_seq + 1,
                                     deleted_at = NULL
                       RETURNING id, project_id, entity_type, natural_key, name,
                                 revision, resolution, projection_seq
                     )
                     INSERT INTO core.outbox_event (aggregate, aggregate_id, event_type, payload)
                     SELECT 'entity', id, 'EntityUpserted',
                            jsonb_build_object(
                              'project_id', project_id,
                              'entity_id', id,
                              'uid', project_id::text || ':' || id::text,
                              'entity_type', entity_type,
                              'natural_key', natural_key,
                              'name', name,
                              'revision', revision,
                              'resolution', resolution,
                              'seq', projection_seq
                            )
                       FROM upserted",
                )
                .bind(item.id)
                .bind(project_id)
                .bind(entity_type)
                .bind(&item.natural_key)
                .bind(&item.name)
                .bind(&ontology_version)
                .bind(&item.resolution)
                .execute(&mut **tx)
                .await?;
            }
        }
        for (src, dst) in verifies {
            let rel_id = Uuid::new_v5(&project_id, format!("VERIFIES:{src}:{dst}").as_bytes());
            sqlx::query(
                "WITH upserted AS (
                   INSERT INTO graph_meta.relationship
                     (id, project_id, src_id, dst_id, rel_type, resolution, confidence,
                      revision, projection_seq)
                   VALUES ($1,$2,$3,$4,'VERIFIES','INFERRED',0.7,'test',1)
                   ON CONFLICT (project_id, src_id, dst_id, rel_type, revision)
                   DO UPDATE SET valid_to = NULL,
                                 confidence = 0.7,
                                 projection_seq = graph_meta.relationship.projection_seq + 1
                   RETURNING id, project_id, src_id, dst_id, rel_type, resolution,
                             confidence, revision, projection_seq
                 )
                 INSERT INTO core.outbox_event (aggregate, aggregate_id, event_type, payload)
                 SELECT 'relationship', id, 'RelationshipUpserted',
                        jsonb_build_object(
                          'project_id', project_id,
                          'rel_id', id,
                          'src_uid', project_id::text || ':' || src_id::text,
                          'dst_uid', project_id::text || ':' || dst_id::text,
                          'rel_type', rel_type,
                          'revision', revision,
                          'resolution', resolution,
                          'confidence', confidence,
                          'seq', projection_seq
                        )
                   FROM upserted",
            )
            .bind(rel_id)
            .bind(project_id)
            .bind(src)
            .bind(dst)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    pub async fn test_requirement_ids(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<Uuid>, StoreError> {
        let rows = sqlx::query(
            "SELECT id FROM graph_meta.entity
              WHERE project_id = $1 AND entity_type = 'Requirement'
                AND deleted_at IS NULL
              ORDER BY id",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(|row| row.get("id")).collect())
    }

    /// `(test_case_id, requirement_entity_id)` の VERIFIES 辺。
    pub async fn test_verifies_edges(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
    ) -> Result<Vec<(Uuid, Uuid)>, StoreError> {
        let rows = sqlx::query(
            "SELECT s.natural_key AS case_key, d.id AS requirement_id
               FROM graph_meta.relationship r
               JOIN graph_meta.entity s ON s.id = r.src_id
               JOIN graph_meta.entity d ON d.id = r.dst_id
              WHERE r.project_id = $1 AND r.rel_type = 'VERIFIES'
                AND r.valid_to IS NULL
                AND s.entity_type = 'TestCase' AND d.entity_type = 'Requirement'
              ORDER BY s.natural_key, d.id",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            let key: String = row.get("case_key");
            if let Ok(case_id) = key.parse::<Uuid>() {
                out.push((case_id, row.get::<Uuid, _>("requirement_id")));
            }
        }
        Ok(out)
    }
}

// ── 行変換 ──────────────────────────────────────────────────

fn validation_rule_row(row: &sqlx::postgres::PgRow) -> ValidationRuleRow {
    ValidationRuleRow {
        id: row.get("id"),
        field_ref: row.get("field_ref"),
        kind: row.get("kind"),
        condition: row.get("condition"),
        source: row.get("source"),
        resolution: row.get("resolution"),
        evidence_ids: row.get("evidence_ids"),
        created_at: row.get("created_at"),
    }
}

fn test_plan_row(row: &sqlx::postgres::PgRow) -> TestPlanRow {
    TestPlanRow {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        status: row.get("status"),
        origin: row.get("origin"),
        impact_id: row.get("impact_id"),
        scope: row.get("scope"),
        created_at: row.get("created_at"),
    }
}

fn test_case_row(row: &sqlx::postgres::PgRow) -> TestCaseRow {
    TestCaseRow {
        id: row.get("id"),
        scenario_id: row.get("scenario_id"),
        plan_id: row.get("plan_id"),
        code: row.get("code"),
        title: row.get("title"),
        status: row.get("status"),
        dsl: row.get("dsl"),
        covers_rules: row.get("covers_rules"),
        covers_impacts: row.get("covers_impacts"),
        covers_screens: row.get("covers_screens"),
        evidence_ids: row.get("evidence_ids"),
        destructive: row.get("destructive"),
    }
}

fn test_run_row(row: &sqlx::postgres::PgRow) -> TestRunRow {
    let environment: Value = row.get("environment");
    let guard_kind = environment
        .get("guard_kind")
        .and_then(Value::as_str)
        .unwrap_or("STAGING")
        .to_owned();
    TestRunRow {
        id: row.get("id"),
        plan_id: row.get("plan_id"),
        name: row.get("name"),
        status: row.get("status"),
        environment,
        environment_id: row.get("environment_id"),
        guard_kind,
        lock_acquired: row.get("lock_acquired"),
        case_count: row.get("case_count"),
        job_id: row.get("job_id"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        created_at: row.get("created_at"),
    }
}

fn test_execution_row(row: &sqlx::postgres::PgRow) -> TestExecutionRow {
    TestExecutionRow {
        id: row.get("id"),
        test_run_id: row.get("test_run_id"),
        test_case_id: row.get("test_case_id"),
        attempt: row.get("attempt"),
        status: row.get("status"),
        execution_source: row.get("execution_source"),
        failure_kind: row.get("failure_kind"),
        failure_step: row.get("failure_step"),
        expected: row.get("expected"),
        actual: row.get("actual"),
        fingerprint: row.get("fingerprint"),
        artifacts: row.get("artifacts"),
    }
}
