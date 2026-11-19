import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { ErrorNotice } from "../components/ui";
import {
  createOntologyDraft,
  deleteOntologyEntityType,
  getOntology,
  publishOntology,
  upsertOntologyEntityType,
} from "../lib/api";
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
  const [draft, setDraft] = useState<OntologyResponse | null>(null);
  const [typeName, setTypeName] = useState("");
  const [typeDisplay, setTypeDisplay] = useState("");
  const [typeParent, setTypeParent] = useState("");

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

  async function openDraft() {
    setBusy(true);
    setError(null);
    try {
      setDraft(await createOntologyDraft(pid));
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  async function addType() {
    if (!typeName.trim() || !typeDisplay.trim()) return;
    setBusy(true);
    setError(null);
    try {
      // name は大文字始まりの英数字。サーバーでも検証するが、入力時に落とす。
      const updated = await upsertOntologyEntityType(pid, {
        name: typeName.trim(),
        display_name_ja: typeDisplay.trim(),
        parent_type: typeParent.trim() ? typeParent.trim() : null,
        is_abstract: false,
        properties_schema: {},
      });
      setDraft(updated);
      setTypeName("");
      setTypeDisplay("");
      setNotice(t("ontology.addType"));
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  async function removeType(name: string) {
    setBusy(true);
    setError(null);
    try {
      setDraft(await deleteOntologyEntityType(pid, name));
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  async function publishDraft() {
    setBusy(true);
    setError(null);
    try {
      const published = await publishOntology(pid, "", true);
      setNotice(t("ontology.published", { version: published.version }));
      setDraft(null);
      await load();
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

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
          <button className="btn btn-sm" onClick={() => void openDraft()} disabled={busy}>
            {t("ontology.editDraft")}
          </button>
          {notice ? <span className="chip">{notice}</span> : null}
        </div>
        {draft ? (
          <div className="stack-sm">
            <div className="row row-wrap">
              <span className="chip chip-warn">
                {t("ontology.draft")}: {draft.version}
              </span>
              <button className="btn btn-primary btn-sm" onClick={() => void publishDraft()} disabled={busy}>
                {t("ontology.publishDraft")}
              </button>
              <button className="btn btn-ghost btn-sm" onClick={() => setDraft(null)}>
                {t("ontology.closeDraft")}
              </button>
            </div>
            <p className="muted m-0">{t("ontology.draftHint")}</p>
            <div className="filters">
              <input
                className="input"
                placeholder={t("ontology.name")}
                value={typeName}
                onChange={(event) => setTypeName(event.target.value)}
              />
              <input
                className="input"
                placeholder={t("ontology.displayName")}
                value={typeDisplay}
                onChange={(event) => setTypeDisplay(event.target.value)}
              />
              <input
                className="input"
                placeholder={t("ontology.parent")}
                value={typeParent}
                onChange={(event) => setTypeParent(event.target.value)}
              />
              <button className="btn btn-primary" onClick={() => void addType()} disabled={busy}>
                {t("ontology.addType")}
              </button>
            </div>
          </div>
        ) : null}
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
                    <th>{t("ontology.parent")}</th>
                    {draft ? <th /> : null}
                  </tr>
                </thead>
                <tbody>
                  {(draft ?? ontology).entity_types.map((type) => (
                    <tr key={type.name}>
                      <td className="mono">{type.name}</td>
                      <td>{type.display_name_ja}</td>
                      <td>{type.parent_type ?? "-"}</td>
                      {draft ? (
                        <td>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() => void removeType(type.name)}
                            disabled={busy}
                          >
                            {t("ontology.deleteType")}
                          </button>
                        </td>
                      ) : null}
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
