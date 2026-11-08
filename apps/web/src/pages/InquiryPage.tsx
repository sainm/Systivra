import { useState } from "react";
import { Link } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { ErrorNotice } from "../components/ui";
import {
  createInquiry,
  deriveInquiryPlan,
  getInquiry,
  getInquiryEvents,
  narrateInquiry,
} from "../lib/api";
import { useT } from "../i18n";
import type {
  DeriveTestPlanResponse,
  InquiryHypothesis,
  InquiryItem,
  InquiryResponse,
  InquirySection,
  Me,
  Membership,
} from "../lib/types";

function statusClass(status: string): string {
  if (status === "SUCCEEDED") return "chip chip-success";
  if (status === "FAILED") return "chip chip-danger";
  if (status === "NEEDS_CLARIFICATION" || status === "RUNNING" || status === "QUEUED")
    return "chip chip-warn";
  return "chip";
}

function SectionTable({
  section,
  empty,
}: {
  section: InquirySection;
  empty: string;
}) {
  const items = section.items ?? [];
  if (items.length === 0) {
    return <p className="muted">{empty}</p>;
  }
  return (
    <div className="table-scroll">
      <table className="data">
        <tbody>
          {items.map((item: InquiryItem) => (
            <tr key={`${item.entity_id}-${item.kind}`} className="inquiry-item">
              <td className="num">
                {item.label}
                {(item.path ?? []).length > 0 ? (
                  <div className="faint mono inquiry-path">
                    {(item.path ?? [])
                      .map((node) => node.slice(0, 8))
                      .join(" → ")}
                  </div>
                ) : null}
              </td>
              <td>{item.kind}</td>
              <td>
                <span className={item.resolution === "RESOLVED" ? "chip chip-success" : "chip chip-warn"}>
                  {item.resolution}
                </span>
              </td>
              <td>{item.why}</td>
              <td className="inquiry-quote">{item.quote}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function InquiryPage({
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
  const [question, setQuestion] = useState("UserEditAction を変更したらどこに影響する？");
  const [entityId, setEntityId] = useState("");
  const [busy, setBusy] = useState(false);
  const [view, setView] = useState<InquiryResponse | null>(null);
  const [events, setEvents] = useState<string[]>([]);
  const [plan, setPlan] = useState<DeriveTestPlanResponse | null>(null);
  const [aiResult, setAiResult] = useState<{
    narrative: string | null;
    hypotheses: InquiryHypothesis[];
  } | null>(null);
  const [aiDegraded, setAiDegraded] = useState<string[]>([]);
  const [error, setError] = useState<unknown>(null);

  async function poll(inquiryId: string): Promise<InquiryResponse> {
    for (let attempt = 0; attempt < 120; attempt += 1) {
      const current = await getInquiry(pid, inquiryId);
      if (["SUCCEEDED", "FAILED", "NEEDS_CLARIFICATION"].includes(current.status)) {
        return current;
      }
      await new Promise((resolve) => setTimeout(resolve, 400));
    }
    throw new Error(t("inquiry.timeout"));
  }

  async function ask(body: { question: string; kind: string; entityIds?: string[] }) {
    setBusy(true);
    setError(null);
    setPlan(null);
    setEvents([]);
    setAiResult(null);
    setAiDegraded([]);
    try {
      const accepted = await createInquiry(pid, {
        question: body.question,
        input: { kind: body.kind, entity_ids: body.entityIds ?? [], doc_types: [] },
        mode: "FULL",
      });
      const current = await poll(accepted.inquiry_id);
      setView(current);
      if (current.status === "SUCCEEDED") {
        const list = await getInquiryEvents(pid, accepted.inquiry_id);
        setEvents(list.map(eventLabel));
      }
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  function askNatural() {
    if (!question.trim()) return;
    return ask({ question: question.trim(), kind: "NATURAL_LANGUAGE" });
  }

  function pickCandidate(entityIdValue: string, label: string) {
    setQuestion(label);
    return ask({ question: label, kind: "ENTITY", entityIds: [entityIdValue] });
  }

  function askEntity() {
    if (!entityId.trim()) return;
    return ask({ question: question.trim(), kind: "ENTITY", entityIds: [entityId.trim()] });
  }

  async function askAi() {
    if (!view) return;
    setBusy(true);
    setError(null);
    try {
      const response = await narrateInquiry(pid, view.inquiry_id);
      setAiResult(
        response.narrative
          ? {
              narrative: response.narrative,
              hypotheses: response.hypotheses ?? [],
            }
          : null,
      );
      setAiDegraded(response.degraded ?? []);
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  async function addToPlan() {
    if (!view) return;
    setBusy(true);
    setError(null);
    try {
      setPlan(
        await deriveInquiryPlan(pid, view.inquiry_id, {
          name: `${view.question.slice(0, 40)} の回帰`,
          max_drafts: 20,
        }),
      );
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  const summary = view?.summary ?? null;
  const counts = summary?.counts;
  const tests = view?.tests ?? null;
  const narrative = aiResult
    ? aiResult.narrative
    : summary?.narrative ?? null;
  const hypotheses = aiResult
    ? aiResult.hypotheses
    : summary?.hypotheses ?? [];

  return (
    <AppShell me={me} project={membership} active="inquiry" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("inquiry.title")}</h1>
          <p className="page-subtitle">{t("inquiry.subtitle")}</p>
        </div>
        <div className="row">
          {view?.status === "SUCCEEDED" ? (
            <>
              <a
                className="btn btn-sm"
                href={`/api/v1/projects/${pid}/change-inquiries/${view.inquiry_id}/report.md`}
                download
              >
                {t("inquiry.download")}
              </a>
              <a
                className="btn btn-sm"
                href={`/api/v1/projects/${pid}/change-inquiries/${view.inquiry_id}/report.pdf`}
                download
              >
                {t("inquiry.downloadPdf")}
              </a>
            </>
          ) : null}
          {view ? <span className={statusClass(view.status)}>{view.status}</span> : null}
        </div>
      </div>

      {error ? (
        <div className="page-error">
          <ErrorNotice error={error} onRetry={() => void askNatural()} />
        </div>
      ) : null}

      <section className="panel panel-body section-gap stack">
        <label className="faint" htmlFor="inquiry-question">
          {t("inquiry.question")}
        </label>
        <textarea
          id="inquiry-question"
          className="input inquiry-input"
          rows={2}
          value={question}
          onChange={(event) => setQuestion(event.target.value)}
        />
        <div className="row row-wrap">
          <button
            className="btn btn-primary btn-sm"
            disabled={busy}
            onClick={() => void askNatural()}
          >
            {t("inquiry.ask")}
          </button>
          <input
            className="input inquiry-entity"
            aria-label={t("inquiry.entityId")}
            placeholder={t("inquiry.entityIdHint")}
            value={entityId}
            onChange={(event) => setEntityId(event.target.value)}
          />
          <button className="btn btn-sm" disabled={busy || !entityId.trim()} onClick={() => void askEntity()}>
            {t("inquiry.askEntity")}
          </button>
          {busy ? <span className="muted">{t("inquiry.running")}</span> : null}
        </div>
      </section>

      {view?.status === "NEEDS_CLARIFICATION" ? (
        <section className="panel panel-body section-gap stack">
          <h2 className="panel-title">{t("inquiry.clarifyTitle")}</h2>
          <p className="muted">{t("inquiry.clarifyHint")}</p>
          {(view.candidates ?? []).map((candidate) => (
            <div className="inquiry-candidate" key={candidate.entity_id}>
              <div>
                <strong>{candidate.label}</strong>{" "}
                <span className="num faint">{candidate.natural_key}</span>
                <div className="faint">{candidate.reason}</div>
              </div>
              <div className="row">
                <span className="num">{candidate.score.toFixed(2)}</span>
                <button
                  className="btn btn-sm"
                  disabled={busy}
                  onClick={() => void pickCandidate(candidate.entity_id, candidate.label)}
                >
                  {t("inquiry.pick")}
                </button>
              </div>
            </div>
          ))}
        </section>
      ) : null}

      {view?.status === "FAILED" ? (
        <section className="panel panel-body section-gap">
          <h2 className="panel-title">{t("inquiry.failedTitle")}</h2>
          <pre className="num">{JSON.stringify(view.error, null, 2)}</pre>
        </section>
      ) : null}

      {view && summary ? (
        <>
          <section className="panel panel-body section-gap stack">
            <h2 className="panel-title">{t("inquiry.summary")}</h2>
            <p className="inquiry-headline">{summary.headline}</p>
            <div className="inquiry-counts">
              <div className="metric-card">
                <span className="faint">{t("inquiry.screens")}</span>
                <strong>{counts?.screens ?? 0}</strong>
              </div>
              <div className="metric-card">
                <span className="faint">{t("inquiry.documents")}</span>
                <strong>{counts?.documents ?? 0}</strong>
              </div>
              <div className="metric-card">
                <span className="faint">{t("inquiry.code")}</span>
                <strong>{counts?.code ?? 0}</strong>
              </div>
              <div className="metric-card">
                <span className="faint">{t("inquiry.tests")}</span>
                <strong>{counts?.tests ?? 0}</strong>
              </div>
              <div className="metric-card">
                <span className="faint">{t("inquiry.other")}</span>
                <strong>{counts?.other ?? 0}</strong>
              </div>
            </div>
            <div className="row row-wrap">
              <span
                className={
                  summary.code_modified ? "chip chip-danger" : "chip chip-success"
                }
              >
                code: {summary.code_modified ? "MODIFIED" : "NOT_MODIFIED"}
              </span>
              <span
                className={
                  summary.tests_executed ? "chip chip-danger" : "chip chip-success"
                }
              >
                tests: {summary.tests_executed ? "EXECUTED" : "NOT_EXECUTED"}
              </span>
              <span className="chip">{summary.approval_state}</span>
            </div>
            <div className="row row-wrap">
              {(summary.degraded ?? []).map((reason) => (
                <span className="chip chip-warn" key={reason}>
                  {reason}
                </span>
              ))}
            </div>
            <div className="row row-wrap">
              <button className="btn btn-sm" disabled={busy} onClick={() => void askAi()}>
                {busy ? t("inquiry.narrating") : t("inquiry.narrate")}
              </button>
              {aiDegraded.map((reason) => (
                <span className="chip chip-warn" key={reason}>
                  {reason}
                </span>
              ))}
            </div>
            {narrative || hypotheses.length > 0 ? (
              <details className="panel panel-body inquiry-ai">
                <summary className="panel-title">{t("inquiry.narrative")}</summary>
                {narrative ? <p>{narrative}</p> : null}
                {hypotheses.length > 0 ? (
                  <div className="inquiry-hypotheses">
                    <h3 className="faint">{t("inquiry.hypotheses")}</h3>
                    {hypotheses.map((hypothesis, index) => (
                      <article
                        className="inquiry-hypothesis"
                        key={`${hypothesis.claim}-${index}`}
                      >
                        <div className="row row-wrap">
                          <span className="chip chip-warn">
                            {hypothesis.resolution}
                          </span>
                          <strong>{hypothesis.claim}</strong>
                          <span className="num faint">
                            {Math.round(hypothesis.confidence * 100)}%
                          </span>
                        </div>
                        <h4 className="faint">{t("inquiry.basis")}</h4>
                        <ul className="inquiry-basis">
                          {(hypothesis.basis ?? []).map((basis) => (
                            <li key={`${basis.section}-${basis.entity_id}`}>
                              <span className="chip">{basis.section}</span>{" "}
                              <strong>{basis.label}</strong>
                              <div className="inquiry-quote">{basis.quote}</div>
                              <pre className="impact-locator">
                                {JSON.stringify(basis.locator, null, 2)}
                              </pre>
                            </li>
                          ))}
                        </ul>
                      </article>
                    ))}
                  </div>
                ) : null}
                <p className="faint">{t("inquiry.narrativeHint")}</p>
              </details>
            ) : null}
            {events.length > 0 ? (
              <div className="row row-wrap">
                <span className="faint">{t("inquiry.eventOrder")}</span>
                {events.map((name, index) => (
                  <span className="chip" key={`${name}-${index}`}>
                    {name}
                  </span>
                ))}
              </div>
            ) : null}
          </section>

          <section className="panel panel-body section-gap stack">
            <h2 className="panel-title">{t("inquiry.screens")}</h2>
            <SectionTable section={view.screens ?? { total: 0 }} empty={t("inquiry.noScreens")} />
          </section>

          <section className="panel panel-body section-gap stack">
            <h2 className="panel-title">{t("inquiry.documents")}</h2>
            <SectionTable
              section={view.documents ?? { total: 0 }}
              empty={t("inquiry.noDocuments")}
            />
          </section>

          <section className="panel panel-body section-gap stack">
            <h2 className="panel-title">{t("inquiry.code")}</h2>
            <SectionTable section={view.code ?? { total: 0 }} empty={t("inquiry.noCode")} />
          </section>

          <section className="panel panel-body section-gap stack">
            <h2 className="panel-title">{t("inquiry.tests")}</h2>
            <h3 className="faint">{t("inquiry.existing")}</h3>
            {(tests?.existing ?? []).length === 0 ? (
              <p className="muted">{t("inquiry.noExisting")}</p>
            ) : (
              <div className="table-scroll">
                <table className="data">
                  <tbody>
                    {(tests?.existing ?? []).map((item) => (
                      <tr key={item.case_id} className="inquiry-existing">
                        <td className="num">{item.code}</td>
                        <td>{item.title}</td>
                        <td>{item.status}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}

            <h3 className="faint">{t("inquiry.drafts")}</h3>
            {(tests?.drafts ?? []).length === 0 ? (
              <p className="muted">{t("inquiry.noDrafts")}</p>
            ) : (
              <div className="table-scroll">
                <table className="data">
                  <tbody>
                    {(tests?.drafts ?? []).map((item) => (
                      <tr key={item.code} className="inquiry-draft">
                        <td className="num">{item.code}</td>
                        <td>{item.title}</td>
                        <td>{item.status}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}

            {(tests?.steps ?? []).length > 0 ? (
              <>
                <h3 className="faint">{t("inquiry.steps")}</h3>
                <ol className="inquiry-steps">
                  {(tests?.steps ?? []).map((step, index) => (
                    <li key={index}>{step}</li>
                  ))}
                </ol>
              </>
            ) : null}

            {(tests?.data_boundaries ?? []).length > 0 ? (
              <>
                <h3 className="faint">{t("inquiry.data")}</h3>
                <div className="row row-wrap">
                  {(tests?.data_boundaries ?? []).map((value, index) => (
                    <span className="chip" key={index}>
                      {value}
                    </span>
                  ))}
                </div>
              </>
            ) : null}

            {(tests?.not_covered ?? []).length > 0 ? (
              <>
                <h3 className="faint">{t("inquiry.notCovered")}</h3>
                <div className="row row-wrap">
                  {(tests?.not_covered ?? []).map((value, index) => (
                    <span className="chip chip-warn" key={index}>
                      {value}
                    </span>
                  ))}
                </div>
              </>
            ) : null}

            <div className="row">
              <button className="btn btn-primary btn-sm" disabled={busy} onClick={() => void addToPlan()}>
                {t("inquiry.addToPlan")}
              </button>
              {plan ? (
                <span className="chip chip-success">
                  {t("inquiry.planCreated")}: {plan.plan_id.slice(0, 8)} / DRAFT{" "}
                  {plan.draft_case_count}
                </span>
              ) : null}
            </div>
            {plan ? (
              <>
                <p className="faint">{t("inquiry.noRunCreated")}</p>
                <Link className="btn btn-sm" to={`/projects/${pid}/tests?plan=${plan.plan_id}`}>
                  {t("inquiry.openTests")}
                </Link>
              </>
            ) : null}
          </section>
        </>
      ) : null}
    </AppShell>
  );
}

function eventLabel(item: { event: string; data: unknown }): string {
  if (item.event === "partial" && typeof item.data === "object" && item.data !== null) {
    const section = (item.data as { section?: unknown }).section;
    if (typeof section === "string") return section;
  }
  return item.event;
}
