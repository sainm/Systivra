import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { ErrorNotice } from "../components/ui";
import {
  createWorkflow,
  getMcpTools,
  getWorkflowDetail,
} from "../lib/api";
import { useT, type MessageKey } from "../i18n";
import type { McpToolsResponse, Me, Membership, WorkflowNode } from "../lib/types";

interface EditorNode {
  id: string;
  kind: string;
  name: string;
  capability: string;
  action: string;
  x: number;
  y: number;
}

interface EditorEdge {
  from: string;
  to: string;
}

const NODE_W = 156;
const NODE_H = 58;
const KINDS = ["start", "capability", "approval", "end"] as const;

const KIND_LABEL: Record<string, MessageKey> = {
  start: "workflow.kind.start",
  capability: "workflow.kind.capability",
  approval: "workflow.kind.approval",
  end: "workflow.kind.end",
};

function kindClass(kind: string): string {
  if (kind === "start") return "wf-node wf-start";
  if (kind === "capability") return "wf-node wf-capability";
  if (kind === "approval") return "wf-node wf-approval";
  return "wf-node wf-end";
}

export function WorkflowEditorPage({
  me,
  membership,
  onLoggedOut,
}: {
  me: Me;
  membership: Membership;
  onLoggedOut: () => void;
}) {
  const { t } = useT();
  const { pid = "", wid = "" } = useParams();
  const [code, setCode] = useState("");
  const [version, setVersion] = useState(0);
  const [nodes, setNodes] = useState<EditorNode[]>([]);
  const [edges, setEdges] = useState<EditorEdge[]>([]);
  const [tools, setTools] = useState<McpToolsResponse | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [linkFrom, setLinkFrom] = useState<string | null>(null);
  const [drag, setDrag] = useState<{ id: string; startX: number; startY: number; x: number; y: number } | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState<unknown>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [detail, toolList] = await Promise.all([
        getWorkflowDetail(pid, wid),
        getMcpTools(pid),
      ]);
      setCode(detail.code);
      setVersion(detail.version);
      setTools(toolList);
      setNodes(
        detail.definition.nodes.map((node: WorkflowNode, index: number) => ({
          id: node.id,
          kind: node.kind,
          name: node.name,
          capability: node.capability ?? "",
          action: node.action ?? "",
          x: node.x ?? 40 + (index % 3) * 200,
          y: node.y ?? 40 + Math.floor(index / 3) * 100,
        })),
      );
      setEdges(
        (detail.definition.edges ?? []).map((edge: { from: string; to: string }) => ({
          from: edge.from,
          to: edge.to,
        })),
      );
    } catch (cause) {
      setError(cause);
    } finally {
      setLoading(false);
    }
  }, [pid, wid]);

  useEffect(() => {
    void load();
  }, [load]);

  const readOnlyCapabilities = useMemo(
    () => (tools?.tools ?? []).filter((tool) => tool.side_effects === "read"),
    [tools],
  );

  const problems = useMemo(() => {
    const list: string[] = [];
    if (!nodes.some((node) => node.kind === "start")) list.push(t("workflow.needStart"));
    if (!nodes.some((node) => node.kind === "end")) list.push(t("workflow.needEnd"));
    const ids = new Set(nodes.map((node) => node.id));
    for (const edge of edges) {
      if (!ids.has(edge.from) || !ids.has(edge.to)) list.push(t("workflow.danglingEdge"));
    }
    for (const node of nodes) {
      if (node.kind === "capability" && !node.capability) list.push(t("workflow.needCapability"));
    }
    return Array.from(new Set(list));
  }, [nodes, edges, t]);

  function addNode(kind: string) {
    const index = nodes.length;
    const node: EditorNode = {
      id: `${kind}-${Date.now().toString(36)}`,
      kind,
      name: t(KIND_LABEL[kind] ?? "workflow.kind.capability"),
      capability: kind === "capability" ? readOnlyCapabilities[0]?.capability ?? "" : "",
      action: kind === "approval" ? "test.run_execute" : "",
      x: 40 + (index % 3) * 200,
      y: 40 + Math.floor(index / 3) * 100,
    };
    setNodes((current) => [...current, node]);
    setSelected(node.id);
  }

  function removeNode(id: string) {
    setNodes((current) => current.filter((node) => node.id !== id));
    setEdges((current) => current.filter((edge) => edge.from !== id && edge.to !== id));
    if (selected === id) setSelected(null);
  }

  function updateNode(id: string, patch: Partial<EditorNode>) {
    setNodes((current) =>
      current.map((node) => (node.id === id ? { ...node, ...patch } : node)),
    );
  }

  function pickForLink(id: string) {
    if (!linkFrom) {
      setLinkFrom(id);
      return;
    }
    if (linkFrom !== id) {
      setEdges((current) =>
        current.some((edge) => edge.from === linkFrom && edge.to === id)
          ? current
          : [...current, { from: linkFrom, to: id }],
      );
    }
    setLinkFrom(null);
  }

  function onPointerDown(event: React.PointerEvent<HTMLDivElement>, node: EditorNode) {
    if (event.button !== 0) return;
    if (linkFrom) {
      pickForLink(node.id);
      return;
    }
    setSelected(node.id);
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    setDrag({ id: node.id, startX: event.clientX, startY: event.clientY, x: node.x, y: node.y });
  }

  function onPointerMove(event: React.PointerEvent<HTMLDivElement>) {
    if (!drag) return;
    updateNode(drag.id, {
      x: Math.max(0, drag.x + event.clientX - drag.startX),
      y: Math.max(0, drag.y + event.clientY - drag.startY),
    });
  }

  function onPointerUp() {
    setDrag(null);
  }

  async function save() {
    setSaving(true);
    setError(null);
    setNotice("");
    try {
      const response = await createWorkflow(pid, {
        code,
        definition: {
          nodes: nodes.map((node) => ({
            id: node.id,
            kind: node.kind,
            name: node.name,
            capability: node.capability || null,
            action: node.action || null,
            x: node.x,
            y: node.y,
          })),
          edges,
        },
      });
      setVersion(response.version);
      setNotice(t("workflow.saved"));
    } catch (cause) {
      setError(cause);
    } finally {
      setSaving(false);
    }
  }

  const selectedNode = nodes.find((node) => node.id === selected) ?? null;

  return (
    <AppShell me={me} project={membership} active="ai" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("workflow.title")}</h1>
          <p className="page-subtitle">{t("workflow.subtitle")}</p>
        </div>
        <div className="row">
          <span className="chip">{code} v{version}</span>
          <Link className="btn btn-ghost btn-sm" to={`/projects/${pid}/ai`}>
            {t("workflow.back")}
          </Link>
        </div>
      </div>

      {error ? (
        <div className="page-error">
          <ErrorNotice error={error} onRetry={() => void load()} />
        </div>
      ) : null}
      {notice ? <div className="test-notice">{notice}</div> : null}

      <section className="panel panel-body section-gap">
        {loading ? <p className="muted">{t("common.loading")}</p> : null}
        <div className="row row-wrap">
          {KINDS.map((kind) => (
            <button key={kind} className="btn btn-sm" onClick={() => addNode(kind)}>
              + {t(KIND_LABEL[kind] ?? "workflow.kind.capability")}
            </button>
          ))}
          <button
            className={`btn btn-sm ${linkFrom ? "btn-primary" : ""}`}
            disabled={!selected}
            onClick={() => {
              if (!selected) return;
              setLinkFrom(selected);
            }}
          >
            {linkFrom ? t("workflow.linkPick") : t("workflow.link")}
          </button>
          <button className="btn btn-primary btn-sm" disabled={saving || problems.length > 0} onClick={() => void save()}>
            {t("workflow.save")}
          </button>
          {problems.length > 0 ? (
            <span className="chip chip-danger">{problems[0]}</span>
          ) : null}
        </div>
      </section>

      <div className="wf-layout section-gap">
        <div className="panel wf-canvas">
          <svg className="wf-edges">
            {edges.map((edge, index) => {
              const from = nodes.find((node) => node.id === edge.from);
              const to = nodes.find((node) => node.id === edge.to);
              if (!from || !to) return null;
              return (
                <line
                  key={`${edge.from}-${edge.to}-${index}`}
                  x1={from.x + NODE_W / 2}
                  y1={from.y + NODE_H / 2}
                  x2={to.x + NODE_W / 2}
                  y2={to.y + NODE_H / 2}
                  className="wf-edge"
                />
              );
            })}
          </svg>
          {nodes.map((node) => (
            <div
              key={node.id}
              className={`${kindClass(node.kind)} ${selected === node.id ? "wf-selected" : ""} ${
                linkFrom === node.id ? "wf-link-source" : ""
              }`}
              style={{ left: node.x, top: node.y, width: NODE_W, height: NODE_H }}
              onPointerDown={(event) => onPointerDown(event, node)}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
            >
              <strong>{node.name}</strong>
              <span className="wf-kind">{node.kind}</span>
            </div>
          ))}
        </div>

        <aside className="panel panel-body wf-props">
          <h2 className="panel-title">{t("workflow.props")}</h2>
          {selectedNode ? (
            <div className="stack">
              <div className="field">
                <label className="faint" htmlFor="wf-name">{t("workflow.name")}</label>
                <input
                  id="wf-name"
                  className="input"
                  value={selectedNode.name}
                  onChange={(event) => updateNode(selectedNode.id, { name: event.target.value })}
                />
              </div>
              {selectedNode.kind === "capability" ? (
                <div className="field">
                  <label className="faint" htmlFor="wf-cap">{t("workflow.capability")}</label>
                  <select
                    id="wf-cap"
                    className="select"
                    value={selectedNode.capability}
                    onChange={(event) => updateNode(selectedNode.id, { capability: event.target.value })}
                  >
                    {readOnlyCapabilities.map((tool) => (
                      <option key={tool.capability} value={tool.capability}>
                        {tool.capability}
                      </option>
                    ))}
                  </select>
                </div>
              ) : null}
              {selectedNode.kind === "approval" ? (
                <div className="field">
                  <label className="faint" htmlFor="wf-action">{t("workflow.action")}</label>
                  <input
                    id="wf-action"
                    className="input"
                    value={selectedNode.action}
                    onChange={(event) => updateNode(selectedNode.id, { action: event.target.value })}
                  />
                </div>
              ) : null}
              <button className="btn btn-danger btn-sm" onClick={() => removeNode(selectedNode.id)}>
                {t("workflow.delete")}
              </button>
            </div>
          ) : (
            <p className="muted">{t("workflow.pickNode")}</p>
          )}
        </aside>
      </div>

      <section className="panel panel-body section-gap">
        <p className="faint">{t("workflow.hint")}</p>
      </section>
    </AppShell>
  );
}
