import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { ErrorNotice } from "../components/ui";
import {
  createOntologyDraft,
  listOntologyVersions,
  deleteOntologyEntityType,
  deleteOntologyRelationType,
  getOntology,
  publishOntology,
  upsertOntologyEntityType,
  upsertOntologyRelationType,
} from "../lib/api";
import type {
  Me,
  Membership,
  OntologyResponse,
  OntologyVersionListResponse,
} from "../lib/types";
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
  const [relationName, setRelationName] = useState("");
  const [relationDisplay, setRelationDisplay] = useState("");
  const [relationSource, setRelationSource] = useState("");
  const [relationTarget, setRelationTarget] = useState("");
  const [relationCardinality, setRelationCardinality] = useState("N:M");
  // 編集中の行。名前はキーなので変更させない（変えたいときは追加＋削除）。
  const [editingType, setEditingType] = useState<string | null>(null);
  const [editingRelation, setEditingRelation] = useState<string | null>(null);
  const [versions, setVersions] = useState<OntologyVersionListResponse["items"]>([]);
  const [baseVersion, setBaseVersion] = useState("");

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
    // 基底にできる版を読む。DRAFT は除く（編集中の版を基底にはしない）。
    void listOntologyVersions(pid)
      .then((response) =>
        setVersions(response.items.filter((item) => item.status !== "DRAFT")),
      )
      .catch(() => setVersions([]));
  }, [pid]);

  async function openDraft() {
    setBusy(true);
    setError(null);
    try {
      setDraft(await createOntologyDraft(pid, baseVersion || null));
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
      setTypeParent("");
      setEditingType(null);
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

  function editType(type: OntologyResponse["entity_types"][number]) {
    setEditingType(type.name);
    setTypeName(type.name);
    setTypeDisplay(type.display_name_ja);
    setTypeParent(type.parent_type ?? "");
  }

  function editRelation(type: OntologyResponse["relation_types"][number]) {
    setEditingRelation(type.name);
    setRelationName(type.name);
    setRelationDisplay(type.display_name_ja);
    setRelationSource(type.source_types.join(", "));
    setRelationTarget(type.target_types.join(", "));
    setRelationCardinality(type.cardinality);
  }

  function cancelEdit() {
    setEditingType(null);
    setEditingRelation(null);
    setTypeName("");
    setTypeDisplay("");
    setTypeParent("");
    setRelationName("");
    setRelationDisplay("");
    setRelationSource("");
    setRelationTarget("");
    setRelationCardinality("N:M");
  }

  async function addRelation() {
    if (!relationName.trim() || !relationDisplay.trim()) return;
    // カンマ（と読点）区切り。空なら `*`（任意の型）として送る。
    const split = (value: string) =>
      value
        .split(/[,、]/)
        .map((item) => item.trim())
        .filter(Boolean);
    const source = split(relationSource);
    const target = split(relationTarget);
    setBusy(true);
    setError(null);
    try {
      setDraft(
        await upsertOntologyRelationType(pid, {
          name: relationName.trim(),
          display_name_ja: relationDisplay.trim(),
          source_types: source.length > 0 ? source : ["*"],
          target_types: target.length > 0 ? target : ["*"],
          cardinality: relationCardinality,
          is_transitive: false,
        }),
      );
      setRelationName("");
      setRelationDisplay("");
      setRelationSource("");
      setRelationTarget("");
      setRelationCardinality("N:M");
      setEditingRelation(null);
      setNotice(t("ontology.addRelation"));
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  }

  async function removeRelation(name: string) {
    setBusy(true);
    setError(null);
    try {
      setDraft(await deleteOntologyRelationType(pid, name));
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
          <select
            className="input"
            value={baseVersion}
            onChange={(event) => setBaseVersion(event.target.value)}
            title={t("ontology.base")}
          >
            <option value="">{t("ontology.base")}: {t("ontology.version")}</option>
            {versions.map((item) => (
              <option key={item.id} value={item.id}>
                {item.version} ({item.status})
              </option>
            ))}
          </select>
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
              {draft.based_on ? (
                <span className="chip">
                  {t("ontology.basedOn")}:{" "}
                  {versions.find((item) => item.id === draft.based_on)?.version ??
                    draft.based_on.slice(0, 8)}
                </span>
              ) : null}
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
                // 名前はキー。変えると「別の型を作って古いのを消す」になるので、
                // 編集中は固定する。
                disabled={editingType !== null}
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
                {editingType ? t("ontology.update") : t("ontology.addType")}
              </button>
              {editingType ? (
                <button className="btn btn-ghost" onClick={cancelEdit}>
                  {t("ontology.cancel")}
                </button>
              ) : null}
            </div>
            <div className="filters">
              <input
                className="input"
                placeholder={t("ontology.name")}
                value={relationName}
                disabled={editingRelation !== null}
                onChange={(event) => setRelationName(event.target.value)}
              />
              <input
                className="input"
                placeholder={t("ontology.displayName")}
                value={relationDisplay}
                onChange={(event) => setRelationDisplay(event.target.value)}
              />
              <input
                className="input"
                placeholder={t("ontology.sourceTypes")}
                value={relationSource}
                onChange={(event) => setRelationSource(event.target.value)}
              />
              <input
                className="input"
                placeholder={t("ontology.targetTypes")}
                value={relationTarget}
                onChange={(event) => setRelationTarget(event.target.value)}
              />
              <select
                className="input"
                value={relationCardinality}
                onChange={(event) => setRelationCardinality(event.target.value)}
              >
                <option value="N:M">N:M</option>
                <option value="1:N">1:N</option>
                <option value="N:1">N:1</option>
                <option value="1:1">1:1</option>
              </select>
              <button className="btn" onClick={() => void addRelation()} disabled={busy}>
                {editingRelation ? t("ontology.update") : t("ontology.addRelation")}
              </button>
              {editingRelation ? (
                <button className="btn btn-ghost" onClick={cancelEdit}>
                  {t("ontology.cancel")}
                </button>
              ) : null}
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
                            onClick={() => editType(type)}
                            disabled={busy}
                          >
                            {t("ontology.edit")}
                          </button>
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
                    <th>{t("ontology.sourceTypes")}</th>
                    {draft ? <th /> : null}
                  </tr>
                </thead>
                <tbody>
                  {(draft ?? ontology).relation_types.map((type) => (
                    <tr key={type.name}>
                      <td className="mono">{type.name}</td>
                      <td>{type.display_name_ja}</td>
                      <td>{type.cardinality}</td>
                      <td className="mono">
                        {type.source_types.join(",")} → {type.target_types.join(",")}
                      </td>
                      {draft ? (
                        <td>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() => editRelation(type)}
                            disabled={busy}
                          >
                            {t("ontology.edit")}
                          </button>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() => void removeRelation(type.name)}
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
        </div>
      ) : null}
    </AppShell>
  );
}
