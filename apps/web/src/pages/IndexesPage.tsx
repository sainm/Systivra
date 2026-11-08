import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice } from "../components/ui";
import { activateIndexBuild, createIndexBuild, listIndexBuilds } from "../lib/api";
import { formatDate, formatNumber } from "../lib/format";
import type { IndexBuildSummary, Me, Membership } from "../lib/types";
import { useT } from "../i18n";

const MODELS = ["local-hash-1024", "local-hash-1024-v2"];

export function IndexesPage({
  me,
  membership,
  onLoggedOut,
}: {
  me: Me;
  membership: Membership;
  onLoggedOut: () => void;
}) {
  const { t, lang } = useT();
  const { pid = "" } = useParams();
  const [builds, setBuilds] = useState<IndexBuildSummary[]>([]);
  const [model, setModel] = useState(MODELS[MODELS.length - 1]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const response = await listIndexBuilds(pid);
      setBuilds(response.items);
      setError(null);
    } catch (cause) {
      setError(cause);
    } finally {
      setLoading(false);
    }
  }, [pid]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!builds.some((build) => build.status === "BUILDING")) return;
    const timer = window.setInterval(() => void load(), 2000);
    return () => window.clearInterval(timer);
  }, [builds, load]);

  async function create() {
    setBusy(true);
    setError(null);
    try {
      await createIndexBuild(pid, model);
      setNotice(t("indexes.startedNotice"));
      await load();
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  async function activate(build: IndexBuildSummary) {
    setBusy(true);
    setError(null);
    try {
      await activateIndexBuild(pid, build.id);
      setNotice(t("indexes.activated"));
      await load();
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  return (
    <AppShell me={me} project={membership} active="indexes" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("indexes.title")}</h1>
          <p className="page-subtitle">{t("indexes.subtitle")}</p>
        </div>
        <Link className="btn btn-sm" to={`/projects/${membership.project_id}/documents`}>
          {t("nav.documents")}
        </Link>
      </div>

      <div className="panel panel-body stack" style={{ marginBottom: 16 }}>
        <div className="filters">
          <label className="label" htmlFor="index-model">
            {t("indexes.model")}
          </label>
          <select
            id="index-model"
            className="select"
            value={model}
            onChange={(event) => setModel(event.target.value)}
          >
            {MODELS.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
          <button className="btn btn-primary" onClick={() => void create()} disabled={busy}>
            {t("indexes.create")}
          </button>
        </div>
        {notice ? <span className="chip chip-accent">{notice}</span> : null}
      </div>

      {error ? <div className="page-error"><ErrorNotice error={error} onRetry={() => void load()} /></div> : null}

      <div className="panel">
        <div className="panel-head">
          <span className="panel-title">{t("indexes.list")}</span>
          <button className="btn btn-sm" onClick={() => void load()} disabled={busy}>
            {t("common.refresh")}
          </button>
        </div>
        {loading ? (
          <div className="panel-body stack">
            <div className="skeleton" style={{ width: "40%" }} />
            <div className="skeleton" style={{ width: "70%" }} />
          </div>
        ) : builds.length === 0 ? (
          <EmptyState title={t("indexes.empty")} hint={t("indexes.emptyHint")} />
        ) : (
          <div className="table-wrap">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("indexes.model")}</th>
                  <th>{t("indexes.status")}</th>
                  <th className="num">{t("indexes.chunks")}</th>
                  <th>{t("indexes.started")}</th>
                  <th>{t("indexes.active")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {builds.map((build) => (
                  <tr key={build.id}>
                    <td>
                      <span className="row-title">{build.model_name}</span>
                      <span className="faint">rev {build.revision}</span>
                    </td>
                    <td>
                      <span className={`chip ${build.status === "READY" ? "chip-accent" : build.status === "FAILED" ? "badge-danger" : "badge-warn"}`}>
                        {build.status}
                      </span>
                    </td>
                    <td className="num">{formatNumber(build.chunk_count, lang)}</td>
                    <td className="muted">
                      {build.started_at ? formatDate(build.started_at, lang) : "—"}
                    </td>
                    <td>
                      {build.is_active ? (
                        <span className="chip chip-accent">{t("indexes.activeYes")}</span>
                      ) : (
                        <span className="faint">—</span>
                      )}
                    </td>
                    <td className="num">
                      <button
                        className="btn btn-sm"
                        disabled={busy || build.status !== "READY" || build.is_active}
                        onClick={() => void activate(build)}
                      >
                        {t("indexes.activate")}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </AppShell>
  );
}
