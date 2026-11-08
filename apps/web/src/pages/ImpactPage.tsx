import { useCallback, useEffect, useMemo, useState } from "react";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice } from "../components/ui";
import {
  createImpactAssessment,
  getImpactAssessment,
  getJob,
  narrateImpact,
  listCodeRevisions,
  searchGraphEntities,
} from "../lib/api";
import type {
  CodeRevisionSummary,
  GraphEntity,
  ImpactAssessmentDraft,
  ImpactItem,
  Me,
  Membership,
} from "../lib/types";
import { useT, type MessageKey } from "../i18n";

type Mode = "ENTITY" | "DIFF";

export function ImpactPage({
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
  const [mode, setMode] = useState<Mode>("ENTITY");
  const [entities, setEntities] = useState<GraphEntity[]>([]);
  const [entityQuery, setEntityQuery] = useState("");
  const [selectedEntityIds, setSelectedEntityIds] = useState<string[]>([]);
  const [revisions, setRevisions] = useState<CodeRevisionSummary[]>([]);
  const [baseRevisionId, setBaseRevisionId] = useState("");
  const [headRevisionId, setHeadRevisionId] = useState("");
  const [diffUnified, setDiffUnified] = useState("");
  const [maxDepth, setMaxDepth] = useState(6);
  const [hubThreshold, setHubThreshold] = useState(200);
  const [includeDocuments, setIncludeDocuments] = useState(true);
  const [draft, setDraft] = useState<ImpactAssessmentDraft | null>(null);
  const [impactId, setImpactId] = useState<string | null>(null);
  const [impactNarrative, setImpactNarrative] = useState<string | null>(null);
  const [aiDegraded, setAiDegraded] = useState<string[]>([]);
  const [impactStatus, setImpactStatus] = useState("");
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [graph, revisionResponse] = await Promise.all([
        searchGraphEntities(pid, { limit: 80 }),
        listCodeRevisions(pid),
      ]);
      setEntities(graph.items);
      setSelectedEntityIds((current) => {
        if (current.some((id) => graph.items.some((item) => item.id === id))) {
          return current;
        }
        return graph.items[0] ? [graph.items[0].id] : [];
      });
      setRevisions(revisionResponse.items);
      const byId = new Map(revisionResponse.items.map((item) => [item.id, item]));
      const pairHead = revisionResponse.items.find((item) => {
        const parent = item.parent_id ? byId.get(item.parent_id) : undefined;
        return item.file_count > 0 && parent && parent.repository_id === item.repository_id;
      });
      const pairBase = pairHead?.parent_id ? byId.get(pairHead.parent_id) : undefined;
      setHeadRevisionId((current) => current || pairHead?.id || "");
      setBaseRevisionId((current) => current || pairBase?.id || "");
    } catch (cause) {
      setError(cause);
    } finally {
      setLoading(false);
    }
  }, [pid]);

  useEffect(() => {
    void load();
  }, [load]);

  const filteredEntities = useMemo(() => {
    const query = entityQuery.trim().toLowerCase();
    if (!query) return entities;
    return entities.filter((entity) =>
      `${entity.name} ${entity.qualified_name ?? ""} ${entity.entity_type}`
        .toLowerCase()
        .includes(query),
    );
  }, [entities, entityQuery]);

  async function askAi() {
    if (!impactId) return;
    setRunning(true);
    setError(null);
    try {
      const response = await narrateImpact(impactId);
      setImpactNarrative(response.narrative ?? null);
      setAiDegraded(response.degraded ?? []);
    } catch (cause) {
      setError(cause);
    } finally {
      setRunning(false);
    }
  }

  async function runAssessment() {
    setRunning(true);
    setError(null);
    setDraft(null);
    setImpactStatus("QUEUED");
    try {
      const accepted = await createImpactAssessment(pid, {
        seed:
          mode === "ENTITY"
            ? { kind: "ENTITY", entity_ids: selectedEntityIds }
            : {
                kind: "DIFF",
                base_revision_id: baseRevisionId,
                head_revision_id: headRevisionId,
                paths: [],
                diff_unified: diffUnified.trim() || null,
              },
        options: {
          max_depth: maxDepth,
          hub_threshold: hubThreshold,
          max_items: 500,
          include_documents: includeDocuments,
        },
      });
      for (let attempt = 0; attempt < 120; attempt += 1) {
        setImpactId(accepted.impact_id);
        const job = await getJob(accepted.job_id);
        setImpactStatus(job.status);
        if (job.status === "FAILED" || job.status === "CANCELLED" || job.status === "TIMEOUT") {
          throw new Error(job.errors[0]?.message ?? t("impact.runFailed"));
        }
        if (job.status === "SUCCEEDED" || job.status === "SUCCEEDED_WITH_ERRORS") {
          const response = await getImpactAssessment(accepted.impact_id);
          setImpactStatus(response.status);
          setDraft(response.report ?? null);
          return;
        }
        await delay(700);
      }
      throw new Error(t("impact.runTimeout"));
    } catch (cause) {
      setError(cause);
      setImpactStatus("FAILED");
    } finally {
      setRunning(false);
    }
  }

  function toggleEntity(entityId: string) {
    setSelectedEntityIds((current) =>
      current.includes(entityId)
        ? current.filter((id) => id !== entityId)
        : [...current, entityId],
    );
  }

  const grouped = draft ? groupItems(draft.items) : new Map<string, ImpactItem[]>();
  const canRun =
    !running &&
    (mode === "ENTITY"
      ? selectedEntityIds.length > 0
      : Boolean(headRevisionId && (baseRevisionId || diffUnified.trim())));

  return (
    <AppShell me={me} project={membership} active="impact" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("impact.title")}</h1>
          <p className="page-subtitle">{t("impact.subtitle")}</p>
        </div>
        <div className="row">
          <span className="chip chip-warn">{t("impact.draftBadge")}</span>
          {impactStatus ? <span className={`chip ${statusClass(impactStatus)}`}>{impactStatus}</span> : null}
          {impactId && draft ? (
            <>
              <button className="btn btn-sm" disabled={running} onClick={() => void askAi()}>
                {t("impact.narrate")}
              </button>
              <a
                className="btn btn-sm"
                href={`/api/v1/impacts/${impactId}/report.md`}
                download
              >
                {t("impact.download")}
              </a>
              <a
                className="btn btn-sm"
                href={`/api/v1/impacts/${impactId}/report.pdf`}
                download
              >
                {t("impact.downloadPdf")}
              </a>
            </>
          ) : null}
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

      {impactNarrative || aiDegraded.length > 0 ? (
        <section className="panel panel-body section-gap stack">
          <div className="row row-wrap">
            {aiDegraded.map((reason) => (
              <span className="chip chip-warn" key={reason}>
                {reason}
              </span>
            ))}
          </div>
          {impactNarrative ? (
            <details className="panel panel-body" open>
              <summary className="panel-title">{t("impact.narrative")}</summary>
              <p>{impactNarrative}</p>
              <p className="faint">{t("impact.narrativeHint")}</p>
            </details>
          ) : null}
        </section>
      ) : null}

      <section className="panel panel-body section-gap stack">
        <div className="row">
          <button
            className={`btn btn-sm ${mode === "ENTITY" ? "btn-primary" : ""}`}
            onClick={() => setMode("ENTITY")}
          >
            {t("impact.mode.entity")}
          </button>
          <button
            className={`btn btn-sm ${mode === "DIFF" ? "btn-primary" : ""}`}
            onClick={() => setMode("DIFF")}
          >
            {t("impact.mode.diff")}
          </button>
        </div>

        {mode === "ENTITY" ? (
          <div className="impact-seed-grid">
            <div className="stack-sm">
              <div className="row">
                <strong>{t("impact.seed.entities")}</strong>
                <span className="chip num">{selectedEntityIds.length}</span>
              </div>
              <input
                className="input"
                value={entityQuery}
                onChange={(event) => setEntityQuery(event.target.value)}
                placeholder={t("impact.seed.filter")}
              />
              <div className="impact-entity-list">
                {filteredEntities.map((entity) => (
                  <label
                    key={entity.id}
                    className="impact-entity-option"
                    data-selected={selectedEntityIds.includes(entity.id)}
                  >
                    <input
                      type="checkbox"
                      checked={selectedEntityIds.includes(entity.id)}
                      onChange={() => toggleEntity(entity.id)}
                    />
                    <span>
                      <strong>{entity.name}</strong>
                      <span className="faint mono">{entity.entity_type}</span>
                    </span>
                  </label>
                ))}
                {!filteredEntities.length ? (
                  <EmptyState title={t("impact.seed.none")} hint={t("graph.empty")} />
                ) : null}
              </div>
            </div>
          </div>
        ) : (
          <div className="impact-seed-grid">
            <div className="field grow">
              <label className="label" htmlFor="impact-base">{t("impact.seed.base")}</label>
              <select
                id="impact-base"
                className="select"
                value={baseRevisionId}
                onChange={(event) => setBaseRevisionId(event.target.value)}
              >
                {revisions.map((revision) => (
                  <option key={revision.id} value={revision.id}>
                    {revision.revision} · {revision.file_count} files
                  </option>
                ))}
              </select>
            </div>
            <div className="field grow">
              <label className="label" htmlFor="impact-head">{t("impact.seed.head")}</label>
              <select
                id="impact-head"
                className="select"
                value={headRevisionId}
                onChange={(event) => setHeadRevisionId(event.target.value)}
              >
                {revisions.map((revision) => (
                  <option key={revision.id} value={revision.id}>
                    {revision.revision} · {revision.file_count} files
                  </option>
                ))}
              </select>
            </div>
            {!revisions.length ? (
              <p className="muted m-0">{t("impact.seed.noRevisions")}</p>
            ) : null}
            <label className="field grow impact-diff-input">
              <span className="label">{t("impact.seed.unifiedDiff")}</span>
              <textarea
                className="input"
                value={diffUnified}
                onChange={(event) => setDiffUnified(event.target.value)}
                placeholder={t("impact.seed.unifiedDiffHint")}
                rows={5}
              />
            </label>
          </div>
        )}

        <div className="row row-wrap">
          <label className="field compact-field">
            <span>{t("impact.options.depth")}</span>
            <input
              className="input compact-input"
              type="number"
              min={1}
              max={10}
              value={maxDepth}
              onChange={(event) => setMaxDepth(Number(event.target.value))}
            />
          </label>
          <label className="field compact-field">
            <span>{t("impact.options.hub")}</span>
            <input
              className="input compact-input"
              type="number"
              min={1}
              value={hubThreshold}
              onChange={(event) => setHubThreshold(Number(event.target.value))}
            />
          </label>
          <label className="row">
            <input
              type="checkbox"
              checked={includeDocuments}
              onChange={(event) => setIncludeDocuments(event.target.checked)}
            />
            <span>{t("impact.options.documents")}</span>
          </label>
          <button className="btn btn-primary" disabled={!canRun} onClick={() => void runAssessment()}>
            {running ? t("impact.running") : t("impact.run")}
          </button>
        </div>
        <p className="faint m-0">{t("impact.safety")}</p>
      </section>

      {loading ? (
        <div className="panel panel-body stack section-gap">
          <div className="skeleton skeleton-60" />
          <div className="skeleton skeleton-80" />
        </div>
      ) : draft ? (
        <>
          <section className="code-metrics section-gap">
            <Metric label={t("impact.metric.total")} value={draft.stats.total} />
            <Metric label={t("impact.scope.code")} value={draft.stats.code} />
            <Metric label={t("impact.scope.screens")} value={draft.stats.screens} />
            <Metric label={t("impact.scope.documents")} value={draft.stats.documents} />
            <Metric
              label={t("impact.metric.uncertain")}
              value={draft.stats.inferred + draft.stats.unresolved + draft.stats.conflict}
            />
          </section>

          <section className="panel section-gap">
            <div className="panel-head">
              <span className="panel-title">{t("impact.summary.title")}</span>
              <div className="row">
                <span className={`chip ${riskClass(draft.risk.level)}`}>
                  {draft.risk.level} {draft.risk.score.toFixed(3)}
                </span>
                <span className="chip">{draft.approval_state}</span>
              </div>
            </div>
            <div className="panel-body stack">
              <div className="row row-wrap">
                <span className="chip chip-success">
                  {t("impact.safety.codes")}: {draft.code_modified ? "MODIFIED" : "NOT_MODIFIED"}
                </span>
                <span className="chip">
                  {t("impact.safety.tests")}: {draft.tests_executed ? "EXECUTED" : "NOT_EXECUTED"}
                </span>
                <span className="chip num">{draft.engine_version}</span>
                <span className="chip">{draft.ontology_version}</span>
              </div>
              <div>
                <div className="label">{t("impact.seed.list")}</div>
                <div className="row row-wrap">
                  {draft.seeds.map((seed, index) => (
                    <span key={`${seed.natural_key}-${index}`} className="chip">
                      {seed.kind}: {seed.label}
                    </span>
                  ))}
                </div>
              </div>
              {(draft.changed_hunks ?? []).length ? (
                <div>
                  <div className="label">{t("impact.changedLines")}</div>
                  <div className="row row-wrap">
                    {(draft.changed_hunks ?? []).map((hunk, index) => (
                      <span
                        className="chip mono"
                        key={`${hunk.path}-${hunk.new_start}-${hunk.new_end}-${index}`}
                      >
                        {hunk.path}:{hunk.new_start}–{hunk.new_end}
                      </span>
                    ))}
                  </div>
                </div>
              ) : null}
              <div>
                <div className="label">{t("impact.degraded.title")}</div>
                <div className="stack-sm">
                  {draft.degraded.length ? (
                    draft.degraded.map((reason) => (
                      <div className="quality-warning" key={reason}>{reason}</div>
                    ))
                  ) : (
                    <span className="faint">{t("common.none")}</span>
                  )}
                </div>
              </div>
              <div className="row row-wrap">
                <span className="chip num">hubs: {draft.truncated.hubs}</span>
                <span className="chip num">depth: {String(draft.truncated.depth)}</span>
                <span className="chip num">budget: {String(draft.truncated.budget)}</span>
                <span className="chip num">unverified: {draft.unverified.length}</span>
              </div>
            </div>
          </section>

          {["CHANGED", "CODE", "SCREEN", "DOCUMENT", "DATA"].map((category) => {
            const items = grouped.get(category) ?? [];
            if (!items.length) return null;
            return (
              <section className="panel block section-gap" key={category}>
                <div className="panel-head">
                  <span className="panel-title">{categoryLabel(category, t)}</span>
                  <span className="chip num">{items.length}</span>
                </div>
                <div className="panel-body stack-sm">
                  {items.map((item) => (
                    <ImpactItemCard key={item.item_id} item={item} />
                  ))}
                </div>
              </section>
            );
          })}

          {draft.unverified.length ? (
            <section className="panel section-gap">
              <div className="panel-head">
                <span className="panel-title">{t("impact.unverified.title")}</span>
                <span className="chip chip-warn">{draft.unverified.length}</span>
              </div>
              <div className="panel-body stack-sm">
                {draft.unverified.map((item, index) => (
                  <div className="impact-item" key={`${item.natural_key}-${index}`}>
                    <div className="impact-item-main">
                      <span className="badge badge-warn">{item.category}</span>
                      <strong>{item.label}</strong>
                      <span className="faint mono">{item.natural_key}</span>
                    </div>
                    <div className="impact-item-side">
                      <span>{item.why}</span>
                      <span className="faint">{item.reason}</span>
                    </div>
                  </div>
                ))}
              </div>
            </section>
          ) : null}
        </>
      ) : null}
    </AppShell>
  );
}

function ImpactItemCard({ item }: { item: ImpactItem }) {
  return (
    <article className="impact-item">
      <div className="impact-item-main">
        <span className={`badge ${resolutionBadge(item.resolution)}`}>{item.resolution}</span>
        <span className="badge">{item.level}</span>
        <span className="badge">{item.direction}</span>
        <strong>{item.label}</strong>
        <span className="faint mono">{item.natural_key}</span>
      </div>
      <div className="impact-item-side">
        <span>{item.why}</span>
        <span className="chip num">{Math.round(item.confidence * 100)}%</span>
      </div>
      {item.evidence.map((evidence) => (
        <div className="impact-evidence" key={evidence.evidence_id ?? `${item.item_id}-${evidence.quote}`}>
          <div className="row row-wrap">
            <span className="chip mono">{evidence.source_type}</span>
            <span className="chip">{evidence.resolution}</span>
          </div>
          <pre className="code-quote">{evidence.quote}</pre>
          <pre className="impact-locator">{JSON.stringify(evidence.locator, null, 2)}</pre>
        </div>
      ))}
      {item.path.length > 1 ? (
        <div className="impact-path">
          {item.path.map((node, index) => (
            <span key={`${node}-${index}`}>
              {index > 0 ? " → " : ""}
              <span className="mono">{node.slice(0, 8)}</span>
            </span>
          ))}
        </div>
      ) : null}
    </article>
  );
}

function Metric({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="metric-card">
      <span className="label">{label}</span>
      <strong className="num">{value}</strong>
    </div>
  );
}

function groupItems(items: ImpactItem[]): Map<string, ImpactItem[]> {
  const grouped = new Map<string, ImpactItem[]>();
  for (const item of items) {
    const values = grouped.get(item.category) ?? [];
    values.push(item);
    grouped.set(item.category, values);
  }
  return grouped;
}

function resolutionBadge(resolution: string): string {
  if (resolution === "RESOLVED") return "badge-ok";
  if (resolution === "INFERRED" || resolution === "PARTIAL") return "badge-warn";
  return "badge-danger";
}

function riskClass(level: string): string {
  if (level === "HIGH") return "chip-danger";
  if (level === "MEDIUM") return "chip-warning";
  return "chip-success";
}

function statusClass(status: string): string {
  if (status === "SUCCEEDED") return "chip-success";
  if (status === "FAILED") return "chip-danger";
  return "chip-warning";
}

function categoryLabel(category: string, t: (key: MessageKey) => string): string {
  if (category === "CHANGED") return t("impact.metric.changed");
  if (category === "CODE") return t("impact.block.code");
  if (category === "SCREEN") return t("impact.block.screens");
  if (category === "DOCUMENT") return t("impact.block.documents");
  return t("impact.metric.data");
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}
