import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice } from "../components/ui";
import {
  getGraphStats,
  getGraphSubgraph,
  getOntology,
  getTraceability,
  rebuildGraph,
  searchGraphEntities,
} from "../lib/api";
import type {
  GraphEntity,
  GraphStatsResponse,
  GraphSubgraphResponse,
  Me,
  Membership,
  OntologyResponse,
  TraceabilityResponse,
} from "../lib/types";
import { useT } from "../i18n";

const WIDTH = 900;
const HEIGHT = 520;

export function GraphPage({
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
  const [stats, setStats] = useState<GraphStatsResponse | null>(null);
  const [ontology, setOntology] = useState<OntologyResponse | null>(null);
  const [entities, setEntities] = useState<GraphEntity[]>([]);
  const [query, setQuery] = useState("");
  const [entityType, setEntityType] = useState("");
  const [selected, setSelected] = useState<GraphEntity | null>(null);
  const [graph, setGraph] = useState<GraphSubgraphResponse | null>(null);
  const [trace, setTrace] = useState<TraceabilityResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [graphStats, published, results] = await Promise.all([
        getGraphStats(pid),
        getOntology(pid),
        searchGraphEntities(pid, { limit: 50 }),
      ]);
      setStats(graphStats);
      setOntology(published);
      setEntities(results.items);
      if (results.items[0]) {
        await selectEntity(results.items[0]);
      }
    } catch (cause) {
      setError(cause);
    } finally {
      setLoading(false);
    }
  }, [pid]);

  useEffect(() => {
    void load();
  }, [load]);

  async function search() {
    setError(null);
    try {
      const results = await searchGraphEntities(pid, {
        q: query.trim(),
        entityType: entityType || undefined,
        limit: 50,
      });
      setEntities(results.items);
      if (results.items[0]) await selectEntity(results.items[0]);
    } catch (cause) {
      setError(cause);
    }
  }

  async function rebuild() {
    setError(null);
    try {
      await rebuildGraph(pid);
      await load();
    } catch (cause) {
      setError(cause);
    }
  }

  async function selectEntity(entity: GraphEntity) {
    setSelected(entity);
    setDetailLoading(true);
    try {
      const [subgraph, traceability] = await Promise.all([
        getGraphSubgraph(pid, entity.id, 2),
        getTraceability(pid, entity.id),
      ]);
      setGraph(subgraph);
      setTrace(traceability);
    } catch (cause) {
      setError(cause);
    } finally {
      setDetailLoading(false);
    }
  }

  const positions = layout(graph?.nodes ?? []);

  return (
    <AppShell me={me} project={membership} active="graph" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("graph.title")}</h1>
          <p className="page-subtitle">{t("graph.subtitle")}</p>
        </div>
        <div className="row">
          {stats ? (
            <>
              <span className="chip num">{t("graph.nodes")}: {stats.pg_nodes}</span>
              <span className="chip num">{t("graph.relationships")}: {stats.pg_relationships}</span>
              <span className="chip num">Neo4j: {stats.neo4j_nodes}/{stats.neo4j_relationships}</span>
              <span className="chip num">{t("graph.drift")}: {stats.drift_rate.toFixed(3)}</span>
              <span className="chip">{t("graph.ontologyVersion")}: {stats.ontology_version}</span>
            </>
          ) : null}
          <Link className="btn btn-sm" to={`/projects/${membership.project_id}/ontology`}>
            {t("graph.openOntology")}
          </Link>
          <button className="btn btn-sm" onClick={() => void rebuild()}>
            {t("graph.rebuild")}
          </button>
        </div>
      </div>

      {error ? <div className="page-error"><ErrorNotice error={error} onRetry={() => void load()} /></div> : null}

      <form
        className="panel panel-body filters section-gap"
        onSubmit={(event) => {
          event.preventDefault();
          void search();
        }}
      >
        <input
          className="input search-query"
          placeholder={t("graph.searchPlaceholder")}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <select
          className="select"
          value={entityType}
          onChange={(event) => setEntityType(event.target.value)}
          aria-label={t("graph.entityType")}
        >
          <option value="">{t("common.all")}</option>
          {ontology?.entity_types.map((type) => (
            <option key={type.name} value={type.name}>
              {type.display_name_ja}
            </option>
          ))}
        </select>
        <button className="btn btn-primary" type="submit">{t("search.submit")}</button>
      </form>

      {loading ? (
        <div className="panel panel-body stack">
          <div className="skeleton skeleton-50" />
          <div className="skeleton skeleton-80" />
        </div>
      ) : (
        <div className="graph-layout">
          <aside className="panel">
            <div className="panel-head">
              <span className="panel-title">{t("graph.entities")}</span>
              <span className="chip num">{entities.length}</span>
            </div>
            <div className="panel-body">
              {entities.length === 0 ? (
                <EmptyState title={t("graph.empty")} hint={t("graph.searchPlaceholder")} />
              ) : (
                <ul className="tree-list">
                  {entities.map((entity) => (
                    <li key={entity.id}>
                      <button
                        className="tree-item"
                        data-selected={selected?.id === entity.id}
                        onClick={() => void selectEntity(entity)}
                      >
                        <span>{entity.name}</span>
                        <span className="faint">{entity.entity_type}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </aside>

          <section className="stack">
            <div className="panel">
              <div className="panel-head">
                <span className="panel-title">{t("graph.subgraph")}</span>
                <span className="chip num">{graph?.edges.length ?? 0}</span>
              </div>
              <div className="panel-body">
                {graph && graph.nodes.length > 0 ? (
                  <svg
                    className="graph-canvas"
                    viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
                    role="img"
                    aria-label={t("graph.subgraph")}
                  >
                    {graph.edges.map((edge) => {
                      const from = positions.get(edge.src_id);
                      const to = positions.get(edge.dst_id);
                      if (!from || !to) return null;
                      return (
                        <g key={`${edge.src_id}-${edge.dst_id}-${edge.rel_type}`}>
                          <line
                            x1={from.x}
                            y1={from.y}
                            x2={to.x}
                            y2={to.y}
                            className="graph-edge"
                          />
                          <text
                            x={(from.x + to.x) / 2}
                            y={(from.y + to.y) / 2 - 4}
                            className="graph-edge-label"
                          >
                            {edge.rel_type}
                          </text>
                        </g>
                      );
                    })}
                    {graph.nodes.map((node) => {
                      const point = positions.get(node.id);
                      if (!point) return null;
                      return (
                        <g
                          key={node.id}
                          className="graph-node"
                          data-selected={selected?.id === node.id}
                          onClick={() => void selectEntity(node)}
                        >
                          <circle cx={point.x} cy={point.y} r={point.root ? 26 : 18} />
                          <text x={point.x} y={point.y + 34} textAnchor="middle">
                            {node.name.slice(0, 18)}
                          </text>
                        </g>
                      );
                    })}
                  </svg>
                ) : (
                  <p className="muted m-0">{t("graph.selectHint")}</p>
                )}
              </div>
            </div>

            <div className="panel">
              <div className="panel-head">
                <span className="panel-title">{t("graph.detail")}</span>
                {selected ? <span className="chip">{selected.entity_type}</span> : null}
              </div>
              <div className="panel-body stack">
                {detailLoading ? (
                  <div className="skeleton skeleton-60" />
                ) : selected ? (
                  <>
                    <div className="row">
                      <span className="chip chip-accent">{selected.name}</span>
                      <span className="chip num">{t("graph.revision")}: {selected.revision.slice(0, 8)}</span>
                      <span className="chip">{selected.resolution}</span>
                    </div>
                    <div>
                      <div className="label">{t("graph.ancestors")}</div>
                      <div className="row row-wrap">
                        {(trace?.ancestors ?? []).map((item) => (
                          <button
                            key={item.id}
                            className="chip"
                            onClick={() => void selectEntity(item)}
                          >
                            {item.name} · {item.entity_type}
                          </button>
                        ))}
                        {!trace?.ancestors.length ? <span className="faint">—</span> : null}
                      </div>
                    </div>
                    <div>
                      <div className="label">{t("graph.descendants")}</div>
                      <div className="row row-wrap">
                        {(trace?.descendants ?? []).map((item) => (
                          <button
                            key={item.id}
                            className="chip"
                            onClick={() => void selectEntity(item)}
                          >
                            {item.name} · {item.entity_type}
                          </button>
                        ))}
                        {!trace?.descendants.length ? <span className="faint">—</span> : null}
                      </div>
                    </div>
                  </>
                ) : (
                  <p className="muted m-0">{t("graph.selectHint")}</p>
                )}
              </div>
            </div>
          </section>
        </div>
      )}
    </AppShell>
  );
}

function layout(nodes: GraphEntity[]): Map<string, { x: number; y: number; root: boolean }> {
  const positions = new Map<string, { x: number; y: number; root: boolean }>();
  if (nodes.length === 0) return positions;
  const root = nodes[0];
  positions.set(root.id, { x: WIDTH / 2, y: HEIGHT / 2, root: true });
  const others = nodes.slice(1);
  others.forEach((node, index) => {
    const angle = (Math.PI * 2 * index) / Math.max(others.length, 1) - Math.PI / 2;
    const radius = index < 6 ? 150 : 220;
    positions.set(node.id, {
      x: WIDTH / 2 + Math.cos(angle) * radius,
      y: HEIGHT / 2 + Math.sin(angle) * radius,
      root: false,
    });
  });
  return positions;
}
