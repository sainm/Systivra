import { useCallback, useEffect, useMemo, useState } from "react";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice } from "../components/ui";
import {
  codeFileContentUrl,
  createCodeAnalysis,
  createCodeRepository,
  getJob,
  listCodeAnalysisRuns,
  listCodeFacts,
  listCodeFiles,
  listCodeRepositories,
  listCodeRevisions,
  listCodeSymbols,
  syncCodeRevision,
} from "../lib/api";
import type {
  CodeAnalysisRunResponse,
  CodeFactSummary,
  CodeRepositorySummary,
  CodeRevisionSummary,
  CodeSourceFileSummary,
  CodeSymbolSummary,
  Me,
  Membership,
} from "../lib/types";
import { useT } from "../i18n";

export function CodePage({
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
  const [repositories, setRepositories] = useState<CodeRepositorySummary[]>([]);
  const [repositoryId, setRepositoryId] = useState("");
  const [revisions, setRevisions] = useState<CodeRevisionSummary[]>([]);
  const [revisionId, setRevisionId] = useState("");
  const [runs, setRuns] = useState<CodeAnalysisRunResponse[]>([]);
  const [symbols, setSymbols] = useState<CodeSymbolSummary[]>([]);
  const [facts, setFacts] = useState<CodeFactSummary[]>([]);
  const [files, setFiles] = useState<CodeSourceFileSummary[]>([]);
  const [selectedFact, setSelectedFact] = useState<CodeFactSummary | null>(null);
  const [path, setPath] = useState("repositories/phase4-sample");
  const [kind, setKind] = useState("DIR");
  const [defaultBranch, setDefaultBranch] = useState("main");
  const [revision, setRevision] = useState("snapshot-001");
  const [symbolQuery, setSymbolQuery] = useState("");
  const [relation, setRelation] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState<unknown>(null);

  const loadRepositories = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [repoResponse, runResponse] = await Promise.all([
        listCodeRepositories(pid),
        listCodeAnalysisRuns(pid),
      ]);
      setRepositories(repoResponse.items);
      setRuns(runResponse.items);
      setRepositoryId((current) => current || repoResponse.items[0]?.id || "");
    } catch (cause) {
      setError(cause);
    } finally {
      setLoading(false);
    }
  }, [pid]);

  const loadRevisions = useCallback(async () => {
    if (!repositoryId) {
      setRevisions([]);
      return;
    }
    try {
      const response = await listCodeRevisions(pid, repositoryId);
      setRevisions(response.items);
      setRevisionId((current) =>
        response.items.some((item) => item.id === current)
          ? current
          : response.items[0]?.id || "",
      );
    } catch (cause) {
      setError(cause);
    }
  }, [pid, repositoryId]);

  const loadDetail = useCallback(async () => {
    if (!revisionId) {
      setSymbols([]);
      setFacts([]);
      setFiles([]);
      return;
    }
    try {
      const [symbolResponse, factResponse, fileResponse, runResponse] = await Promise.all([
        listCodeSymbols(pid, revisionId, symbolQuery),
        listCodeFacts(pid, revisionId, "", relation),
        listCodeFiles(pid, revisionId),
        listCodeAnalysisRuns(pid),
      ]);
      setSymbols(symbolResponse.items);
      setFacts(factResponse.items);
      setFiles(fileResponse.items);
      setRuns(runResponse.items);
      setSelectedFact((current) =>
        current && factResponse.items.some((item) => item.id === current.id)
          ? current
          : factResponse.items[0] || null,
      );
    } catch (cause) {
      setError(cause);
    }
  }, [pid, relation, revisionId, symbolQuery]);

  useEffect(() => {
    void loadRepositories();
  }, [loadRepositories]);

  useEffect(() => {
    void loadRevisions();
  }, [loadRevisions]);

  useEffect(() => {
    void loadDetail();
  }, [loadDetail]);

  const currentRun = useMemo(
    () => runs.find((run) => run.revision_id === revisionId) ?? null,
    [revisionId, runs],
  );
  const selectedRevision = revisions.find((item) => item.id === revisionId) ?? null;
  const selectedRepository = repositories.find((item) => item.id === repositoryId) ?? null;
  const selectedFile = selectedFact
    ? files.find((file) => file.path === factPath(selectedFact))
    : undefined;

  function changeKind(nextKind: string) {
    setKind(nextKind);
    if (nextKind === "GIT") {
      setRevision(defaultBranch.trim() || "main");
    } else if (kind === "GIT") {
      setRevision("snapshot-001");
    }
  }

  function selectRepository(nextId: string) {
    const next = repositories.find((item) => item.id === nextId);
    setRepositoryId(nextId);
    if (next?.kind === "GIT") {
      setKind("GIT");
      setDefaultBranch(next.default_branch || "main");
      setRevision(next.default_branch || "main");
    } else if (next) {
      setKind(next.kind);
    }
  }

  async function registerRepository() {
    setBusy("repository");
    setError(null);
    setNotice("");
    try {
      const repository = await createCodeRepository(pid, {
        kind,
        path: path.trim(),
        default_branch: kind === "GIT" ? defaultBranch.trim() || null : null,
      });
      setRepositories((current) => [
        repository,
        ...current.filter((item) => item.id !== repository.id),
      ]);
      setRepositoryId(repository.id);
      setNotice(t("code.repositorySaved"));
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(null);
    }
  }

  async function synchronize() {
    if (!repositoryId || !revision.trim()) return;
    setBusy("sync");
    setError(null);
    setNotice("");
    try {
      const accepted = await syncCodeRevision(pid, {
        repository_id: repositoryId,
        revision: revision.trim(),
        committed_at: null,
      });
      await pollJob(accepted.job_id);
      await loadRevisions();
      setNotice(t("code.syncDone"));
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(null);
    }
  }

  async function analyze() {
    if (!revisionId) return;
    setBusy("analysis");
    setError(null);
    setNotice("");
    try {
      const accepted = await createCodeAnalysis(pid, revisionId);
      await pollJob(accepted.job_id);
      await loadDetail();
      setNotice(t("code.analysisDone"));
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(null);
    }
  }

  async function pollJob(jobId: string) {
    for (let attempt = 0; attempt < 90; attempt += 1) {
      const job = await getJob(jobId);
      if (job.status === "FAILED" || job.status === "CANCELLED" || job.status === "TIMEOUT") {
        throw new Error(job.errors[0]?.message ?? t("code.jobFailed"));
      }
      if (job.status === "SUCCEEDED" || job.status === "SUCCEEDED_WITH_ERRORS") {
        return job;
      }
      await delay(800);
    }
    throw new Error(t("code.jobTimeout"));
  }

  return (
    <AppShell me={me} project={membership} active="code" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("code.title")}</h1>
          <p className="page-subtitle">{t("code.subtitle")}</p>
        </div>
        <div className="row">
          {selectedRepository ? (
            <span className="chip chip-success" data-testid="repository-kind">
              {selectedRepository.kind}
            </span>
          ) : null}
          {selectedRevision ? (
            <span className="chip num">
              {t("code.fileCount")}: {selectedRevision.file_count}
            </span>
          ) : null}
          {currentRun ? <span className={`chip ${runClass(currentRun)}`}>{currentRun.status}</span> : null}
          <button className="btn btn-sm" onClick={() => void loadRepositories()}>
            {t("common.refresh")}
          </button>
        </div>
      </div>

      {error ? (
        <div className="page-error">
          <ErrorNotice error={error} onRetry={() => void loadRepositories()} />
        </div>
      ) : null}
      {notice ? <div className="notice section-gap">{notice}</div> : null}

      <section className="panel panel-body section-gap stack">
        <div className="row row-wrap">
          <div className="field">
            <label className="label" htmlFor="code-kind">{t("code.repositoryKind")}</label>
            <select
              id="code-kind"
              className="select"
              value={kind}
              onChange={(event) => changeKind(event.target.value)}
            >
              <option value="DIR">{t("code.kindDir")}</option>
              <option value="SNAPSHOT">{t("code.kindSnapshot")}</option>
              <option value="GIT">{t("code.kindGit")}</option>
            </select>
          </div>
          <div className="field grow">
            <label className="label" htmlFor="code-path">
              {kind === "GIT" ? t("code.gitPath") : t("code.repositoryPath")}
            </label>
            <input
              id="code-path"
              className="input"
              value={path}
              onChange={(event) => setPath(event.target.value)}
              placeholder={kind === "GIT" ? "repositories/sample.git" : "repositories/phase4-sample"}
            />
          </div>
          {kind === "GIT" ? (
            <div className="field">
              <label className="label" htmlFor="code-branch">{t("code.defaultBranch")}</label>
              <input
                id="code-branch"
                className="input"
                value={defaultBranch}
                onChange={(event) => setDefaultBranch(event.target.value)}
              />
            </div>
          ) : null}
          <button
            className="btn"
            disabled={busy !== null}
            onClick={() => void registerRepository()}
          >
            {busy === "repository" ? t("common.loading") : t("code.registerRepository")}
          </button>
          <div className="field grow">
            <label className="label" htmlFor="code-repository">{t("code.repository")}</label>
            <select
              id="code-repository"
              className="select"
              value={repositoryId}
              onChange={(event) => selectRepository(event.target.value)}
            >
              <option value="">—</option>
              {repositories.map((repository) => (
                <option key={repository.id} value={repository.id}>
                  [{repository.kind}] {repository.path}
                </option>
              ))}
            </select>
          </div>
          <div className="field grow">
            <label className="label" htmlFor="code-revision">{t("code.revisionName")}</label>
            <input
              id="code-revision"
              className="input"
              value={revision}
              onChange={(event) => setRevision(event.target.value)}
            />
          </div>
          <button
            className="btn btn-primary"
            disabled={busy !== null || !repositoryId}
            onClick={() => void synchronize()}
          >
            {busy === "sync" ? t("code.syncing") : t("code.sync")}
          </button>
          <button
            className="btn"
            disabled={busy !== null || !revisionId}
            onClick={() => void analyze()}
          >
            {busy === "analysis" ? t("code.analyzing") : t("code.analyze")}
          </button>
        </div>
        <p className="faint m-0">
          {kind === "GIT" ? t("code.gitHint") : t("code.pathHint")} {t("code.svnUnsupported")}
        </p>
      </section>

      {loading ? (
        <div className="panel panel-body stack">
          <div className="skeleton skeleton-50" />
          <div className="skeleton skeleton-80" />
        </div>
      ) : (
        <div className="code-layout">
          <aside className="panel">
            <div className="panel-head">
              <span className="panel-title">{t("code.revisions")}</span>
              <span className="chip num">{revisions.length}</span>
            </div>
            <div className="panel-body">
              {revisions.length === 0 ? (
                <EmptyState title={t("code.noRevisions")} hint={t("code.syncHint")} />
              ) : (
                <ul className="tree-list">
                  {revisions.map((item) => (
                    <li key={item.id}>
                      <button
                        className="tree-item"
                        data-selected={item.id === revisionId}
                        onClick={() => setRevisionId(item.id)}
                      >
                        <span>{item.revision}</span>
                        <span className="faint num">{item.file_count}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </aside>

          <section className="stack">
            <div className="code-metrics">
              <Metric label={t("code.filesAnalyzed")} value={currentRun?.files_analyzed ?? 0} />
              <Metric label={t("code.filesSkipped")} value={currentRun?.files_skipped ?? 0} />
              <Metric label={t("code.symbols")} value={currentRun?.symbols ?? symbols.length} />
              <Metric label={t("code.facts")} value={currentRun?.facts ?? facts.length} />
              <Metric
                label={t("code.unresolved")}
                value={`${currentRun?.unresolved ?? unresolvedCount(facts)} (${(
                  (currentRun?.unresolved_rate ?? unresolvedCount(facts)) * 100
                ).toFixed(1)}%)`}
                danger={(currentRun?.unresolved_rate ?? 0) > 0}
              />
            </div>

            <div className="panel">
              <div className="panel-head">
                <span className="panel-title">{t("code.qualityGate")}</span>
                {currentRun ? (
                  <span className="chip">
                    classpath: {currentRun.classpath_complete ? "complete" : "missing"}
                  </span>
                ) : null}
              </div>
              <div className="panel-body stack">
                {currentRun ? (
                  <>
                    <div className="row row-wrap">
                      <span className="chip num">
                        {t("code.filesFailed")}: {currentRun.files_failed}
                      </span>
                      <span className="chip num">
                        {t("code.unresolved")}: {currentRun.unresolved}
                      </span>
                      <span className="chip">
                        {t("code.started")}: {formatTime(currentRun.started_at)}
                      </span>
                      <span className="chip">
                        {t("code.finished")}: {formatTime(currentRun.finished_at)}
                      </span>
                    </div>
                    <div>
                      <div className="label">{t("code.degradedReasons")}</div>
                      <div className="stack-sm">
                        {currentRun.degraded_reasons.length ? (
                          currentRun.degraded_reasons.map((reason) => (
                            <div key={reason} className="quality-warning">{reason}</div>
                          ))
                        ) : (
                          <span className="faint">{t("common.none")}</span>
                        )}
                      </div>
                    </div>
                  </>
                ) : (
                  <p className="muted m-0">{t("code.noAnalysis")}</p>
                )}
              </div>
            </div>

            <div className="panel">
              <div className="panel-head">
                <span className="panel-title">{t("code.symbols")}</span>
                <span className="chip num">{symbols.length}</span>
              </div>
              <div className="panel-body">
                <div className="row section-gap">
                  <input
                    className="input search-query"
                    value={symbolQuery}
                    onChange={(event) => setSymbolQuery(event.target.value)}
                    placeholder={t("code.symbolFilter")}
                  />
                </div>
                <div className="table-scroll">
                  <table className="data">
                    <thead>
                      <tr>
                        <th>{t("code.kind")}</th>
                        <th>{t("code.fqn")}</th>
                        <th>{t("code.lines")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {symbols.map((symbol) => (
                        <tr key={symbol.id}>
                          <td><span className="chip">{symbol.kind}</span></td>
                          <td className="mono code-wrap">{symbol.fqn}</td>
                          <td className="num">{symbol.line_start}–{symbol.line_end}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                {!symbols.length ? (
                  <EmptyState title={t("code.noSymbols")} hint={t("code.analyzeHint")} />
                ) : null}
              </div>
            </div>
          </section>
        </div>
      )}

      <div className="code-layout section-gap">
        <section className="panel">
          <div className="panel-head">
            <span className="panel-title">{t("code.facts")}</span>
            <div className="row">
              <select
                className="select"
                value={relation}
                onChange={(event) => setRelation(event.target.value)}
                aria-label={t("code.relation")}
              >
                <option value="">{t("common.all")}</option>
                {[...new Set(facts.map((fact) => fact.relation))].sort().map((value) => (
                  <option key={value} value={value}>{value}</option>
                ))}
              </select>
              <span className="chip num">{facts.length}</span>
            </div>
          </div>
          <div className="panel-body table-scroll">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("code.subject")}</th>
                  <th>{t("code.relation")}</th>
                  <th>{t("code.object")}</th>
                  <th>{t("code.resolution")}</th>
                </tr>
              </thead>
              <tbody>
                {facts.map((fact) => (
                  <tr
                    key={fact.id}
                    className="clickable-row"
                    data-selected={selectedFact?.id === fact.id}
                    onClick={() => setSelectedFact(fact)}
                  >
                    <td className="mono code-wrap">{fact.subject_key}</td>
                    <td><span className="chip">{fact.relation}</span></td>
                    <td className="mono code-wrap">{fact.object_key}</td>
                    <td><span className={`chip ${resolutionClass(fact.resolution)}`}>{fact.resolution}</span></td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!facts.length ? (
              <EmptyState title={t("code.noFacts")} hint={t("code.analyzeHint")} />
            ) : null}
          </div>
        </section>

        <aside className="panel">
          <div className="panel-head">
            <span className="panel-title">{t("code.evidence")}</span>
            {selectedFile ? (
              <a
                className="btn btn-sm"
                href={codeFileContentUrl(selectedFile.id)}
                target="_blank"
                rel="noreferrer"
              >
                {t("code.openSource")}
              </a>
            ) : null}
          </div>
          <div className="panel-body stack">
            {selectedFact ? (
              <>
                <div className="row row-wrap">
                  <span className="chip">{selectedFact.analysis_engine}</span>
                  <span className="chip num">confidence {selectedFact.confidence.toFixed(2)}</span>
                  <span className={`chip ${resolutionClass(selectedFact.resolution)}`}>
                    {selectedFact.resolution}
                  </span>
                </div>
                {selectedFact.unresolved_reason ? (
                  <p className="quality-warning m-0">{selectedFact.unresolved_reason}</p>
                ) : null}
                {selectedFact.evidence.map((evidence) => {
                  const locator = asCodeLocator(evidence.locator);
                  return (
                    <div key={evidence.id} className="stack-sm">
                      <div className="row row-wrap">
                        <span className="chip mono">{locator.path}</span>
                        <span className="chip num">
                          L{locator.line}:{locator.column}
                        </span>
                        <span className="chip num">
                          {locator.byte_start}–{locator.byte_end}
                        </span>
                      </div>
                      <pre className="code-quote">{evidence.quote}</pre>
                    </div>
                  );
                })}
              </>
            ) : (
              <p className="muted m-0">{t("code.selectFactHint")}</p>
            )}
          </div>
        </aside>
      </div>
    </AppShell>
  );
}

function Metric({
  label,
  value,
  danger = false,
}: {
  label: string;
  value: string | number;
  danger?: boolean;
}) {
  return (
    <div className={`metric-card ${danger ? "metric-danger" : ""}`}>
      <span className="label">{label}</span>
      <strong className="num">{value}</strong>
    </div>
  );
}

function factPath(fact: CodeFactSummary): string | undefined {
  return asCodeLocator(fact.evidence[0]?.locator).path;
}

function asCodeLocator(value: unknown): {
  path?: string;
  line?: number;
  column?: number;
  byte_start?: number;
  byte_end?: number;
} {
  return typeof value === "object" && value !== null
    ? (value as {
        path?: string;
        line?: number;
        column?: number;
        byte_start?: number;
        byte_end?: number;
      })
    : {};
}

function unresolvedCount(facts: CodeFactSummary[]): number {
  return facts.filter((fact) => fact.resolution !== "RESOLVED").length;
}

function resolutionClass(resolution: string): string {
  if (resolution === "RESOLVED") return "chip-success";
  if (resolution === "UNRESOLVED" || resolution === "CONFLICT") return "chip-danger";
  return "chip-warning";
}

function runClass(run: CodeAnalysisRunResponse): string {
  if (run.status === "SUCCEEDED") return "chip-success";
  if (run.status === "FAILED") return "chip-danger";
  return "chip-warning";
}

function formatTime(value?: string | null): string {
  if (!value) return "—";
  return new Date(value).toLocaleString();
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

