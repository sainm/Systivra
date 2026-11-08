import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { ErrorNotice } from "../components/ui";
import { getOntology, publishOntology } from "../lib/api";
import type { Me, Membership, OntologyResponse } from "../lib/types";
import { useT } from "../i18n";

export function OntologyPage({
  me,
  membership,
  onLoggedOut,
}: {
  me: Me;
  membership: Membership;
  onLoggedOut: () => void;
}) {
  const { t } = useT();
  const { pid = "" } = useParams();
  const [ontology, setOntology] = useState<OntologyResponse | null>(null);
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState<string | null>(null);

  async function load() {
    try {
      setOntology(await getOntology(pid));
      setError(null);
    } catch (cause) {
      setError(cause);
    }
  }

  useEffect(() => {
    void load();
  }, [pid]);

  async function publish() {
    if (!version.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const published = await publishOntology(pid, version.trim());
      setNotice(t("ontology.published", { version: published.version }));
      setVersion("");
      await load();
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  return (
    <AppShell me={me} project={membership} active="graph" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("ontology.title")}</h1>
          <p className="page-subtitle">{t("ontology.subtitle")}</p>
        </div>
        <Link className="btn btn-sm" to={`/projects/${membership.project_id}/graph`}>
          {t("graph.title")}
        </Link>
      </div>

      {error ? <div className="page-error"><ErrorNotice error={error} onRetry={() => void load()} /></div> : null}

      <div className="panel panel-body stack section-gap">
        <div className="row row-wrap">
          {ontology ? (
            <>
              <span className="chip chip-accent">{t("ontology.version")}: {ontology.version}</span>
              <span className="chip">{ontology.status}</span>
              <span className="chip num">
                {t("ontology.entityTypes")}: {ontology.entity_types.length}
              </span>
              <span className="chip num">
                {t("ontology.relationTypes")}: {ontology.relation_types.length}
              </span>
            </>
          ) : null}
        </div>
        <div className="filters">
          <input
            className="input"
            placeholder={t("ontology.publishVersion")}
            value={version}
            onChange={(event) => setVersion(event.target.value)}
          />
          <button className="btn btn-primary" onClick={() => void publish()} disabled={busy}>
            {t("ontology.publish")}
          </button>
          {notice ? <span className="chip">{notice}</span> : null}
        </div>
      </div>

      {ontology ? (
        <div className="graph-layout">
          <div className="panel">
            <div className="panel-head">
              <span className="panel-title">{t("ontology.entityTypes")}</span>
            </div>
            <div className="table-wrap">
              <table className="data">
                <thead>
                  <tr>
                    <th>{t("ontology.name")}</th>
                    <th>{t("ontology.displayName")}</th>
                  </tr>
                </thead>
                <tbody>
                  {ontology.entity_types.map((type) => (
                    <tr key={type.name}>
                      <td className="mono">{type.name}</td>
                      <td>{type.display_name_ja}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>

          <div className="panel">
            <div className="panel-head">
              <span className="panel-title">{t("ontology.relationTypes")}</span>
            </div>
            <div className="table-wrap">
              <table className="data">
                <thead>
                  <tr>
                    <th>{t("ontology.name")}</th>
                    <th>{t("ontology.displayName")}</th>
                    <th>{t("ontology.cardinality")}</th>
                  </tr>
                </thead>
                <tbody>
                  {ontology.relation_types.map((type) => (
                    <tr key={type.name}>
                      <td className="mono">{type.name}</td>
                      <td>{type.display_name_ja}</td>
                      <td>{type.cardinality}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      ) : null}
    </AppShell>
  );
}
