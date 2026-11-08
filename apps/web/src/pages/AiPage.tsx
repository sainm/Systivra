import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { ErrorNotice } from "../components/ui";
import {
  createAgentRun,
  createWorkflow,
  createWorkflowRun,
  decideApproval,
  discoverMcpServer,
  getAiOverview,
  getJob,
  getMcpTools,
  registerAgent,
  registerMcpServer,
} from "../lib/api";
import { useT } from "../i18n";
import type { AiOverviewResponse, McpToolsResponse, Me, Membership } from "../lib/types";

const SAMPLE_WORKFLOW = {
  nodes: [
    { id: "start", kind: "start", name: "開始" },
    { id: "cov", kind: "capability", name: "被覆取得", capability: "coverage.get" },
    { id: "gate", kind: "approval", name: "実行承認", action: "test.run_execute" },
    { id: "end", kind: "end", name: "終了" },
  ],
  edges: [
    { from: "start", to: "cov" },
    { from: "cov", to: "gate" },
    { from: "gate", to: "end" },
  ],
};

function statusClass(status: string): string {
  if (["SUCCEEDED", "APPROVED", "UP", "configured"].includes(status)) return "chip chip-success";
  if (["FAILED", "REJECTED", "DOWN"].includes(status)) return "chip chip-danger";
  if (["WAITING_APPROVAL", "PENDING", "DEGRADED", "unavailable"].includes(status))
    return "chip chip-warn";
  return "chip";
}

/** 承認の起票元。Workflow のノードか、MCP の許可証か、Agent の実行か。 */
function approvalSource(approval: {
  workflow_run_id?: string | null;
  run_id?: string | null;
  action: string;
}): "workflow" | "mcp" | "agent" {
  if (approval.workflow_run_id) return "workflow";
  if (approval.run_id) return "agent";
  if (approval.action.startsWith("mcp.call:")) return "mcp";
  return "agent";
}

export function AiPage({
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
  const [overview, setOverview] = useState<AiOverviewResponse | null>(null);
  const [tools, setTools] = useState<McpToolsResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState("");
  const [notice, setNotice] = useState("");
  const [error, setError] = useState<unknown>(null);

  const [agentCode, setAgentCode] = useState("impact-analyst");
  const [profile, setProfile] = useState("reasoning");
  const [promptRef, setPromptRef] = useState("prompt://impact-analyst@1");
  const [promptBody, setPromptBody] = useState(
    "影響範囲を要約し、narrative と basis 付き hypotheses を JSON で返す。",
  );
  const [capabilities, setCapabilities] = useState("knowledge.search,coverage.get");
  const [question, setQuestion] = useState("UserService.save の変更影響を要約して");
  const [mcpName, setMcpName] = useState("platform-mcp");
  const [mcpTransport, setMcpTransport] = useState("http");
  const [mcpConfig, setMcpConfig] = useState(
    '{"url":"http://127.0.0.1:18100/mcp","timeout_ms":2000}',
  );

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [ai, toolList] = await Promise.all([getAiOverview(pid), getMcpTools(pid)]);
      setOverview(ai);
      setTools(toolList);
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

  function doRegisterAgent() {
    return runAction(
      "agent",
      async () => {
        await registerAgent(pid, {
          code: agentCode,
          model_profile: profile,
          prompt_ref: promptRef,
          prompt_body: promptBody,
          capabilities: capabilities
            .split(",")
            .map((item) => item.trim())
            .filter(Boolean),
          budgets: {
            max_tool_calls: 40,
            max_tokens: 120000,
            max_wall_time_sec: 300,
            max_cost_usd: 1.5,
          },
          output_schema: "schema://impact-narrative@1",
          approval_required: false,
        });
      },
      t("ai.notice.agentRegistered"),
    );
  }

  function doRunAgent() {
    return runAction(
      "run",
      async () => {
        const accepted = await createAgentRun(pid, agentCode, { question });
        for (let attempt = 0; attempt < 60; attempt += 1) {
          const job = await getJob(accepted.job_id);
          if (["SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED"].includes(job.status)) break;
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
      },
      t("ai.notice.agentRun"),
    );
  }

  function doRegisterMcp() {
    return runAction(
      "mcp",
      async () => {
        const config = JSON.parse(mcpConfig) as Record<string, unknown>;
        await registerMcpServer(pid, {
          name: mcpName,
          transport: mcpTransport,
          config,
        });
        await discoverMcpServer(pid, mcpName);
      },
      t("ai.notice.mcpRegistered"),
    );
  }

  function doCreateWorkflow() {
    return runAction(
      "workflow",
      async () => {
        await createWorkflow(pid, { code: "impact-regression", definition: SAMPLE_WORKFLOW });
      },
      t("ai.notice.workflowCreated"),
    );
  }

  function doRunWorkflow(workflowId: string) {
    return runAction(
      "workflowRun",
      async () => {
        const accepted = await createWorkflowRun(pid, workflowId);
        for (let attempt = 0; attempt < 60; attempt += 1) {
          const job = await getJob(accepted.job_id);
          if (["SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED"].includes(job.status)) break;
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
      },
      t("ai.notice.workflowRun"),
    );
  }

  function doDecide(approvalId: string, decision: "APPROVE" | "REJECT") {
    return runAction(
      `decide:${approvalId}`,
      async () => {
        await decideApproval(pid, approvalId, { decision, comment: "UI から判断" });
        for (let attempt = 0; attempt < 40; attempt += 1) {
          const ai = await getAiOverview(pid);
          const waiting = ai.workflow_runs.some(
            (run) => run.status === "WAITING_APPROVAL" || run.status === "RUNNING",
          );
          if (!waiting) break;
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
      },
      t("ai.notice.approved"),
    );
  }

  const agents = overview?.agents ?? [];
  const servers = overview?.mcp_servers ?? [];
  const workflows = overview?.workflows ?? [];
  const workflowRuns = overview?.workflow_runs ?? [];
  const approvals = overview?.approvals ?? [];

  function approvalSourceLabel(approval: (typeof approvals)[number]): string {
    const source = approvalSource(approval);
    if (source === "workflow") return t("ai.approval.sourceWorkflow");
    if (source === "mcp") return t("ai.approval.sourceMcp");
    return t("ai.approval.sourceAgent");
  }
  const pending = approvals.filter((item) => item.status === "PENDING");

  return (
    <AppShell me={me} project={membership} active="ai" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("ai.title")}</h1>
          <p className="page-subtitle">{t("ai.subtitle")}</p>
        </div>
        <div className="row">
          <span className={statusClass(overview?.ai_status ?? "unavailable")}>
            {overview?.ai_status === "configured"
              ? t("ai.status.configured")
              : t("ai.status.unavailable")}
          </span>
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
        <h2 className="panel-title">{t("ai.degraded.title")}</h2>
        <div className="row row-wrap">
          {(overview?.degraded ?? []).length === 0 ? (
            <span className="muted">{t("test.validation.ok")}</span>
          ) : (
            (overview?.degraded ?? []).map((reason) => (
              <span className="chip chip-warn" key={reason}>
                {reason}
              </span>
            ))
          )}
        </div>
      </section>

      <section className="code-metrics section-gap">
        <div className="metric-card">
          <span className="faint">{t("ai.metrics.agents")}</span>
          <strong>{agents.length}</strong>
        </div>
        <div className="metric-card">
          <span className="faint">{t("ai.metrics.mcp")}</span>
          <strong>{servers.length}</strong>
        </div>
        <div className="metric-card">
          <span className="faint">{t("ai.metrics.workflows")}</span>
          <strong>{workflows.length}</strong>
        </div>
        <div className="metric-card">
          <span className="faint">{t("ai.metrics.approvals")}</span>
          <strong>{pending.length}</strong>
        </div>
      </section>

      <section className="panel panel-body section-gap stack">
        <h2 className="panel-title">{t("ai.table.agents")}</h2>
        <div className="ai-form-grid">
          <div className="field">
            <label className="faint" htmlFor="agent-code">{t("ai.agent.code")}</label>
            <input id="agent-code" className="input" value={agentCode} onChange={(e) => setAgentCode(e.target.value)} />
          </div>
          <div className="field">
            <label className="faint" htmlFor="agent-profile">{t("ai.agent.profile")}</label>
            <select id="agent-profile" className="select" value={profile} onChange={(e) => setProfile(e.target.value)}>
              <option value="fast">fast</option>
              <option value="reasoning">reasoning</option>
              <option value="long_context">long_context</option>
            </select>
          </div>
          <div className="field">
            <label className="faint" htmlFor="agent-prompt-ref">{t("ai.agent.promptRef")}</label>
            <input id="agent-prompt-ref" className="input" value={promptRef} onChange={(e) => setPromptRef(e.target.value)} />
          </div>
          <div className="field">
            <label className="faint" htmlFor="agent-caps">{t("ai.agent.capabilities")}</label>
            <input id="agent-caps" className="input" value={capabilities} onChange={(e) => setCapabilities(e.target.value)} />
          </div>
          <div className="field ai-span-2">
            <label className="faint" htmlFor="agent-prompt">{t("ai.agent.prompt")}</label>
            <input id="agent-prompt" className="input" value={promptBody} onChange={(e) => setPromptBody(e.target.value)} />
          </div>
          <button className="btn btn-primary btn-sm" disabled={busy !== ""} onClick={() => void doRegisterAgent()}>
            {t("ai.agent.register")}
          </button>
        </div>

        <div className="table-scroll">
          <table className="data">
            <thead>
              <tr>
                <th>{t("ai.col.code")}</th>
                <th>{t("ai.col.version")}</th>
                <th>{t("ai.agent.profile")}</th>
                <th>{t("ai.agent.capabilities")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {agents.map((agent) => (
                <tr key={agent.id} className="ai-agent-row">
                  <td className="num">{agent.code}</td>
                  <td className="num">v{agent.current_version}</td>
                  <td>{agent.model_profile}</td>
                  <td className="num">{agent.capabilities.join(", ")}</td>
                  <td>
                    <button className="btn btn-sm" disabled={busy !== ""} onClick={() => void doRunAgent()}>
                      {t("ai.agent.run")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!loading && agents.length === 0 ? <p className="muted">{t("ai.empty")}</p> : null}

        <div className="row">
          <input
            className="input"
            aria-label={t("ai.agent.question")}
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
          />
        </div>

        <h3 className="panel-title">{t("ai.agent.runs")}</h3>
        <div className="table-scroll">
          <table className="data">
            <thead>
              <tr>
                <th>{t("ai.col.code")}</th>
                <th>{t("ai.col.status")}</th>
                <th>{t("ai.col.action")}</th>
              </tr>
            </thead>
            <tbody>
              {(overview?.agent_runs ?? []).map((run) => (
                <tr key={run.id} className="ai-run-row">
                  <td className="num">{run.agent_code}</td>
                  <td>
                    <span className={statusClass(run.status)}>{run.status}</span>
                  </td>
                  <td className="num">
                    {run.error ? JSON.stringify(run.error) : run.output ? "narrative" : "-"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <section className="panel panel-body section-gap stack">
        <h2 className="panel-title">{t("ai.table.mcp")}</h2>
        <div className="row">
          <input
            className="input"
            aria-label={t("ai.mcp.name")}
            value={mcpName}
            onChange={(e) => setMcpName(e.target.value)}
          />
          <select
            className="input"
            aria-label={t("ai.mcp.transport")}
            value={mcpTransport}
            onChange={(e) => {
              const transport = e.target.value;
              setMcpTransport(transport);
              setMcpConfig(
                transport === "http"
                  ? '{"url":"http://127.0.0.1:18100/mcp","timeout_ms":2000}'
                  : '{"command":"python3","args":["scripts/mock-mcp-server.py","--stdio"]}',
              );
            }}
          >
            <option value="http">http</option>
            <option value="stdio">stdio</option>
          </select>
          <textarea
            className="input"
            aria-label={t("ai.mcp.config")}
            rows={2}
            value={mcpConfig}
            onChange={(e) => setMcpConfig(e.target.value)}
          />
          <button className="btn btn-sm" disabled={busy !== ""} onClick={() => void doRegisterMcp()}>
            {t("ai.mcp.register")}
          </button>
          <span className="chip">{t("ai.mcp.tools")}: {tools?.tools.length ?? 0}</span>
        </div>
        <div className="table-scroll">
          <table className="data mcp-table">
            <thead>
              <tr>
                <th>{t("ai.mcp.name")}</th>
                <th>{t("ai.mcp.transport")}</th>
                <th>{t("ai.col.status")}</th>
                <th>{t("ai.mcp.tools")}</th>
                <th>{t("ai.mcp.detail")}</th>
                <th>schema_hash</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {servers.map((server) => (
                <tr key={server.id} className="ai-mcp-row">
                  <td className="num">{server.name}</td>
                  <td>{server.transport}</td>
                  <td>
                    <span className={statusClass(server.health_status)}>{server.health_status}</span>
                  </td>
                  <td className="num">{server.tool_count}</td>
                  <td className="faint">{server.health_detail ?? "-"}</td>
                  <td className="num">{server.schema_hash?.slice(0, 20) ?? "-"}</td>
                  <td>
                    <button
                      className="btn btn-sm"
                      disabled={busy !== ""}
                      onClick={() =>
                        void runAction(
                          `discover-${server.name}`,
                          () => discoverMcpServer(pid, server.name).then(() => undefined),
                          t("ai.notice.mcpDiscovered"),
                        )
                      }
                    >
                      {t("ai.mcp.discover")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <section className="panel panel-body section-gap stack">
        <h2 className="panel-title">{t("ai.table.workflows")}</h2>
        <div className="row">
          <button className="btn btn-primary btn-sm" disabled={busy !== ""} onClick={() => void doCreateWorkflow()}>
            {t("ai.workflow.create")}
          </button>
        </div>
        <div className="table-scroll">
          <table className="data">
            <thead>
              <tr>
                <th>{t("ai.col.code")}</th>
                <th>{t("ai.col.version")}</th>
                <th>{t("ai.col.status")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {workflows.map((workflow) => (
                <tr key={workflow.id} className="ai-workflow-row">
                  <td className="num">{workflow.code}</td>
                  <td className="num">v{workflow.version}</td>
                  <td>{workflow.status}</td>
                  <td>
                    <button className="btn btn-sm" disabled={busy !== ""} onClick={() => void doRunWorkflow(workflow.id)}>
                      {t("ai.workflow.run")}
                    </button>{" "}
                    <Link
                      className="btn btn-ghost btn-sm"
                      to={`/projects/${pid}/workflows/${workflow.id}/edit`}
                    >
                      {t("workflow.edit")}
                    </Link>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <h3 className="panel-title">{t("ai.workflow.runs")}</h3>
        <div className="table-scroll">
          <table className="data">
            <thead>
              <tr>
                <th>{t("ai.col.code")}</th>
                <th>{t("ai.col.status")}</th>
                <th>{t("ai.col.action")}</th>
              </tr>
            </thead>
            <tbody>
              {workflowRuns.map((run) => (
                <tr key={run.id} className="ai-workflow-run-row">
                  <td className="num">{run.code}</td>
                  <td>
                    <span className={statusClass(run.status)}>{run.status}</span>
                  </td>
                  <td className="num">{run.current_node ?? "-"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <section className="panel panel-body section-gap stack">
        <h2 className="panel-title">{t("ai.table.approvals")}</h2>
        <div className="table-scroll">
          <table className="data">
            <thead>
              <tr>
                <th>{t("ai.col.action")}</th>
                <th>{t("ai.col.source")}</th>
                <th>{t("ai.col.status")}</th>
                <th>{t("ai.col.created")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {approvals.map((approval) => (
                <tr key={approval.id} className="ai-approval-row">
                  <td className="num">{approval.action}</td>
                  <td className="num">{approvalSourceLabel(approval)}</td>
                  <td>
                    <span className={statusClass(approval.status)}>{approval.status}</span>
                    {approval.consumed_at ? (
                      <>
                        {" "}
                        <span className="faint">{t("ai.approval.consumed")}</span>
                      </>
                    ) : null}
                  </td>
                  <td className="num">{approval.created_at}</td>
                  <td>
                    {approval.status === "PENDING" ? (
                      <>
                        <button className="btn btn-sm" disabled={busy !== ""} onClick={() => void doDecide(approval.id, "APPROVE")}>
                          {t("ai.workflow.approve")}
                        </button>{" "}
                        <button className="btn btn-ghost btn-sm" disabled={busy !== ""} onClick={() => void doDecide(approval.id, "REJECT")}>
                          {t("ai.workflow.reject")}
                        </button>
                      </>
                    ) : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </AppShell>
  );
}
