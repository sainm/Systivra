import { useCallback, useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { ErrorNotice } from "../components/ui";
import {
  approveTestCase,
  cleanupRunArtifacts,
  createTestRun,
  deriveTestPlan,
  expireTestArtifacts,
  extractTestRules,
  getTestOverview,
  listImpactAssessments,
  recomputeTestCoverage,
} from "../lib/api";
import { useT } from "../i18n";
import type {
  CoverageView,
  Me,
  Membership,
  TestArtifact,
  TestCaseView,
  TestOverviewResponse,
} from "../lib/types";

/** 規約抽出の入力。ユーザーが自分で入れる（サンプルを勝手に送らない）。 */
type RuleSource = {
  kind: "CODE" | "SPEC";
  field_ref: string;
  text: string;
};

const EMPTY_SOURCE: RuleSource = { kind: "SPEC", field_ref: "", text: "" };

function Metric({ label, value, tone }: { label: string; value: string | number; tone?: string }) {
  return (
    <div className={`metric-card ${tone ?? ""}`}>
      <span className="faint">{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function CoverageCard({
  label,
  num,
  den,
  ratio,
  degraded,
  degradedLabel,
}: {
  label: string;
  num: number;
  den: number;
  ratio: number;
  degraded: string[];
  degradedLabel: string;
}) {
  return (
    <div className="metric-card test-coverage-card">
      <span className="faint">{label}</span>
      <strong>{(ratio * 100).toFixed(1)}%</strong>
      <span className="num faint">
        {num} / {den}
      </span>
      {degraded.length > 0 ? (
        <span className="chip chip-warn" title={degraded.join(", ")}>
          {degradedLabel}
        </span>
      ) : null}
    </div>
  );
}

function CoverageMatrix({
  coverage,
  degradedLabel,
  labels,
}: {
  coverage: CoverageView | null;
  degradedLabel: string;
  labels: {
    requirement: string;
    rule: string;
    screen: string;
    impact: string;
    regression: string;
  };
}) {
  const detail = coverage?.detail;
  const metrics = [
    ["requirement", labels.requirement, detail?.requirement],
    ["rule", labels.rule, detail?.rule],
    ["screen", labels.screen, detail?.screen],
    ["impact", labels.impact, detail?.impact],
    ["regression", labels.regression, detail?.regression],
  ] as const;
  return (
    <div className="test-coverage-grid">
      {metrics.map(([key, label, metric]) => (
        <CoverageCard
          key={key}
          label={label}
          num={metric?.num ?? 0}
          den={metric?.den ?? 0}
          ratio={metric?.ratio ?? 0}
          degraded={metric?.degraded ?? []}
          degradedLabel={degradedLabel}
        />
      ))}
    </div>
  );
}

function statusChipClass(status: string): string {
  if (status === "APPROVED" || status === "PASSED" || status === "DONE") return "chip chip-success";
  if (status === "FAILED" || status === "CONFLICT") return "chip chip-danger";
  if (status === "DRAFT" || status === "QUEUED" || status === "READY") return "chip chip-warn";
  return "chip";
}

function artifactChipClass(artifact: TestArtifact): string {
  if (artifact.status === "TRUNCATED") return "chip chip-warn";
  if (artifact.kind === "UPLOAD_FAILED") return "chip chip-danger";
  if (artifact.status === "EXPIRED") return "chip";
  return "chip chip-accent";
}

export function TestPage({
  me,
  membership,
  onLoggedOut,
}: {
  me: Me;
  membership: Membership;
  onLoggedOut: () => void;
}) {
  const { t } = useT();
  const pid = membership.project_id;
  const [searchParams] = useSearchParams();
  const planFilter = searchParams.get("plan");
  const [overview, setOverview] = useState<TestOverviewResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState("");
  const [impactId, setImpactId] = useState("");
  const [sources, setSources] = useState<RuleSource[]>([{ ...EMPTY_SOURCE }]);
  const filledSources = sources.filter(
    (source) => source.field_ref.trim() !== "" && source.text.trim() !== "",
  );

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setOverview(await getTestOverview(pid));
    } catch (cause) {
      setError(cause);
    } finally {
      setLoading(false);
    }
  }, [pid]);

  useEffect(() => {
    void load();
  }, [load]);

  async function runAction(key: string, work: () => Promise<void>, message: string) {
    setBusy(key);
    setError(null);
    setNotice("");
    try {
      await work();
      setNotice(message);
      await load();
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy("");
    }
  }

  function doExtract() {
    return runAction(
      "extract",
      async () => {
        await extractTestRules(pid, {
          sources: filledSources.map((source) => ({
            kind: source.kind,
            field_ref: source.field_ref.trim(),
            text: source.text.trim(),
            evidence_ids: [],
          })),
        });
      },
      t("test.notice.extract"),
    );
  }

  function doDerive() {
    return runAction(
      "derive",
      async () => {
        let id = impactId.trim();
        if (!id) {
          const list = await listImpactAssessments(pid);
          id = list.items[0]?.impact_id ?? "";
        }
        await deriveTestPlan(pid, {
          impact_id: id || null,
          name: "Q-01 から派生したテスト計画",
          max_drafts: 20,
        });
      },
      t("test.notice.derive"),
    );
  }

  function doRun() {
    return runAction(
      "run",
      async () => {
        await createTestRun(pid, {
          name: `run-${Date.now()}`,
          plan_id: planFilter ?? null,
          environment: {
            id: "env-stg-01",
            base_url: "http://127.0.0.1/app",
            guard_kind: "STAGING",
            allowed_origins: ["127.0.0.1"],
            parallel: 4,
          },
        });
      },
      t("test.notice.run"),
    );
  }

  function doApprove(caseId: string) {
    return runAction(
      `approve:${caseId}`,
      async () => {
        await approveTestCase(pid, caseId);
      },
      t("test.notice.approve"),
    );
  }

  function doCoverage() {
    const runId = overview?.runs[0]?.id;
    if (!runId) return;
    return runAction(
      "coverage",
      async () => {
        await recomputeTestCoverage(pid, { test_run_id: runId });
      },
      t("test.notice.coverage"),
    );
  }

  function doExpireArtifacts() {
    return runAction(
      "expire",
      async () => {
        await expireTestArtifacts(pid, {});
      },
      t("test.notice.expireArtifacts"),
    );
  }

  function doCleanupArtifacts(runId: string) {
    return runAction(
      `cleanup:${runId}`,
      async () => {
        await cleanupRunArtifacts(pid, runId);
      },
      t("test.notice.artifactsCleanup"),
    );
  }

  const cases: TestCaseView[] = overview?.cases ?? [];
  const approvedCount = cases.filter((item) => item.status === "APPROVED").length;
  const coverage = overview?.coverage ?? null;
  const degraded = overview?.degraded ?? [];

  return (
    <AppShell me={me} project={membership} active="tests" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("test.title")}</h1>
          <p className="page-subtitle">{t("test.subtitle")}</p>
        </div>
        <div className="row">
          {planFilter ? (
            <span className="chip chip-accent" title={planFilter}>
              {t("test.planScoped")}: {planFilter.slice(0, 8)}
            </span>
          ) : null}
          <span className="chip chip-warn">{t("test.draftBadge")}</span>
          <span className="chip">{t("test.runnerBadge")}</span>
          <button className="btn btn-sm" onClick={() => void load()}>
            {t("common.refresh")}
          </button>
        </div>
      </div>

      {error ? (
        <div className="page-error">
          <ErrorNotice error={error} onRetry={() => void load()} />
        </div>
      ) : null}
      {notice ? <div className="test-notice">{notice}</div> : null}

      <section className="panel panel-body section-gap stack">
        <h2 className="panel-title">{t("test.degraded.title")}</h2>
        <div className="row row-wrap">
          {degraded.length === 0 ? (
            <span className="muted">{t("test.validation.ok")}</span>
          ) : (
            degraded.map((reason) => (
              <span className="chip chip-warn" key={reason}>
                {reason}
              </span>
            ))
          )}
        </div>
      </section>

      <section className="code-metrics section-gap">
        <Metric label={t("test.metrics.rules")} value={overview?.rules.length ?? 0} />
        <Metric label={t("test.metrics.plans")} value={overview?.plans.length ?? 0} />
        <Metric label={t("test.metrics.cases")} value={cases.length} />
        <Metric label={t("test.metrics.approved")} value={approvedCount} />
        <Metric label={t("test.metrics.runs")} value={overview?.runs.length ?? 0} />
        <Metric label={t("test.metrics.defects")} value={overview?.defects.length ?? 0} />
      </section>

      <section className="panel panel-body section-gap stack">
        <h2 className="panel-title">{t("test.actions.title")}</h2>
        <div className="test-action-grid">
          <button
            className="btn btn-primary btn-sm"
            disabled={busy !== "" || filledSources.length === 0}
            onClick={() => void doExtract()}
          >
            {t("test.actions.extract")}
          </button>
          <div className="field">
            <label className="faint" htmlFor="impact-id">
              {t("test.actions.impactId")}
            </label>
            <input
              id="impact-id"
              className="input"
              value={impactId}
              placeholder={t("test.actions.impactIdHint")}
              onChange={(event) => setImpactId(event.target.value)}
            />
          </div>
          <button className="btn btn-sm" disabled={busy !== ""} onClick={() => void doDerive()}>
            {t("test.actions.derive")}
          </button>
          <button className="btn btn-sm" disabled={busy !== ""} onClick={() => void doRun()}>
            {t("test.actions.run")}
          </button>
          <button
            className="btn btn-ghost btn-sm"
            disabled={busy !== "" || !overview?.runs.length}
            onClick={() => void doCoverage()}
          >
            {t("test.actions.coverage")}
          </button>
          <button
            className="btn btn-ghost btn-sm"
            disabled={busy !== ""}
            onClick={() => void doExpireArtifacts()}
          >
            {t("test.actions.expireArtifacts")}
          </button>
        </div>
        {busy ? <span className="muted">{busy}</span> : null}
        <div className="stack section-gap">
          <span className="faint">{t("test.sources.title")}</span>
          {sources.map((source, index) => (
            <div className="row row-wrap" key={index}>
              <select
                className="select"
                value={source.kind}
                onChange={(event) =>
                  setSources((rows) =>
                    rows.map((row, i) =>
                      i === index ? { ...row, kind: event.target.value as RuleSource["kind"] } : row,
                    ),
                  )
                }
              >
                <option value="SPEC">SPEC</option>
                <option value="CODE">CODE</option>
              </select>
              <input
                className="input"
                placeholder={t("test.sources.field")}
                value={source.field_ref}
                onChange={(event) =>
                  setSources((rows) =>
                    rows.map((row, i) =>
                      i === index ? { ...row, field_ref: event.target.value } : row,
                    ),
                  )
                }
              />
              <input
                className="input"
                placeholder={t("test.sources.text")}
                value={source.text}
                onChange={(event) =>
                  setSources((rows) =>
                    rows.map((row, i) => (i === index ? { ...row, text: event.target.value } : row)),
                  )
                }
              />
              <button
                className="btn btn-ghost btn-sm"
                disabled={sources.length === 1}
                onClick={() => setSources((rows) => rows.filter((_, i) => i !== index))}
              >
                {t("test.sources.remove")}
              </button>
            </div>
          ))}
          <div className="row row-wrap">
            <button
              className="btn btn-sm"
              onClick={() => setSources((rows) => [...rows, { ...EMPTY_SOURCE }])}
            >
              {t("test.sources.add")}
            </button>
          </div>
        </div>
      </section>

      <section className="panel panel-body section-gap stack">
        <h2 className="panel-title">{t("test.coverage.title")}</h2>
        <CoverageMatrix
          coverage={coverage}
          degradedLabel={t("test.coverage.degraded")}
          labels={{
            requirement: t("test.coverage.requirement"),
            rule: t("test.coverage.rule"),
            screen: t("test.coverage.screen"),
            impact: t("test.coverage.impact"),
            regression: t("test.coverage.regression"),
          }}
        />
        {coverage?.detail.degraded?.length ? (
          <div className="row row-wrap">
            {coverage.detail.degraded?.map((reason) => (
              <span className="chip chip-warn" key={reason}>
                {reason}
              </span>
            ))}
          </div>
        ) : null}
      </section>

      <section className="panel panel-body section-gap">
        <h2 className="panel-title">{t("test.table.cases")}</h2>
        <div className="table-scroll">
          <table className="data">
            <thead>
              <tr>
                <th>{t("test.col.code")}</th>
                <th>{t("test.col.title")}</th>
                <th>{t("test.col.status")}</th>
                <th>{t("test.dsl.steps")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {cases.map((item) => (
                <tr key={item.id} className="test-case-row">
                  <td className="num">{item.code}</td>
                  <td>{item.title}</td>
                  <td>
                    <span className={statusChipClass(item.status)}>{item.status}</span>
                  </td>
                  <td className="num">{item.dsl.steps?.length ?? 0}</td>
                  <td>
                    {item.status === "DRAFT" ? (
                      <button
                        className="btn btn-sm"
                        disabled={busy !== "" || item.validation_errors.length > 0}
                        onClick={() => void doApprove(item.id)}
                      >
                        {t("test.actions.approve")}
                      </button>
                    ) : null}
                    {item.validation_errors.length > 0 ? (
                      <span className="chip chip-danger" title={item.validation_errors.join(", ")}>
                        {item.validation_errors[0]}
                      </span>
                    ) : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!loading && cases.length === 0 ? <p className="muted">{t("test.empty")}</p> : null}
      </section>

      <div className="test-layout">
        <details className="panel panel-body" open>
          <summary className="panel-title">{t("test.table.rules")}</summary>
          <div className="table-scroll">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("test.col.field")}</th>
                  <th>{t("test.col.condition")}</th>
                  <th>{t("test.col.source")}</th>
                  <th>{t("test.col.resolution")}</th>
                </tr>
              </thead>
              <tbody>
                {(overview?.rules ?? []).map((rule) => (
                  <tr key={rule.id} className="test-rule-row">
                    <td className="num">{rule.field_ref}</td>
                    <td>{`${rule.kind} ${rule.condition}`}</td>
                    <td>{rule.source}</td>
                    <td>
                      <span className={statusChipClass(rule.resolution)}>{rule.resolution}</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>

        <details className="panel panel-body">
          <summary className="panel-title">{t("test.table.data")}</summary>
          <div className="table-scroll">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("test.col.code")}</th>
                  <th>{t("test.col.field")}</th>
                  <th>{t("test.col.value")}</th>
                </tr>
              </thead>
              <tbody>
                {(overview?.data ?? []).map((row, index) => (
                  <tr key={`${row.case_id}-${row.label}-${index}`} className="test-data-row">
                    <td className="num">{row.label}</td>
                    <td>{row.field_ref}</td>
                    <td className="test-value">{row.value || "(空)"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>

        <details className="panel panel-body">
          <summary className="panel-title">{t("test.table.runs")}</summary>
          <div className="table-scroll">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("test.col.code")}</th>
                  <th>{t("test.col.status")}</th>
                  <th>{t("test.col.environment")}</th>
                  <th>{t("test.metrics.cases")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {(overview?.runs ?? []).map((run) => (
                  <tr key={run.id} className="test-run-row">
                    <td className="num">{run.name}</td>
                    <td>
                      <span className={statusChipClass(run.status)}>{run.status}</span>
                    </td>
                    <td>{`${run.environment_id} / ${run.guard_kind}`}</td>
                    <td className="num">{run.case_count}</td>
                    <td>
                      <button
                        className="btn btn-sm"
                        disabled={busy !== ""}
                        onClick={() => void doCleanupArtifacts(run.id)}
                      >
                        {t("test.actions.cleanupArtifacts")}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>

        <details className="panel panel-body">
          <summary className="panel-title">{t("test.table.executions")}</summary>
          <div className="table-scroll">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("test.col.status")}</th>
                  <th>{t("test.col.attempt")}</th>
                  <th>{t("test.col.source")}</th>
                  <th>{t("test.col.failure")}</th>
                  <th>{t("test.col.artifacts")}</th>
                </tr>
              </thead>
              <tbody>
                {(overview?.executions ?? []).map((execution) => (
                  <tr key={execution.id} className="test-execution-row">
                    <td>
                      <span className={statusChipClass(execution.status)}>{execution.status}</span>
                    </td>
                    <td className="num">{execution.attempt}</td>
                    <td>{execution.execution_source}</td>
                    <td className="num">{execution.failure_kind ?? "-"}</td>
                    <td>
                      {execution.artifacts.length === 0 ? (
                        <span className="muted">-</span>
                      ) : (
                        <span className="row row-wrap">
                          {execution.artifacts.map((artifact, index) => (
                            <span
                              key={`${artifact.kind}-${index}`}
                              className={artifactChipClass(artifact)}
                              title={artifact.uri || artifact.kind}
                            >
                              {artifact.status ?? artifact.kind}
                            </span>
                          ))}
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>

        <details className="panel panel-body">
          <summary className="panel-title">{t("test.table.defects")}</summary>
          <div className="table-scroll">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("test.col.title")}</th>
                  <th>{t("test.col.status")}</th>
                  <th>{t("test.col.recurrence")}</th>
                </tr>
              </thead>
              <tbody>
                {(overview?.defects ?? []).map((defect) => (
                  <tr key={defect.id} className="test-defect-row">
                    <td>{defect.title}</td>
                    <td>{defect.status}</td>
                    <td className="num">{defect.recurrence}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>
      </div>
    </AppShell>
  );
}
