import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice } from "../components/ui";
import { getDocument, getDocumentChunks, getDocumentTree } from "../lib/api";
import { formatNumber } from "../lib/format";
import type {
  DocumentChunkSummary,
  DocumentDetail,
  Me,
  Membership,
  TreeNode,
} from "../lib/types";
import { useT } from "../i18n";

function locatorLabel(locator: Record<string, unknown>): string {
  if (locator.kind === "xlsx") {
    return `${String(locator.sheet ?? "")} ${String(locator.range ?? "")}`.trim();
  }
  if (locator.kind === "docx") {
    return ((locator.section_path as string[] | undefined) ?? []).join(" / ");
  }
  if (locator.kind === "pptx") {
    return `Slide ${String(locator.slide ?? "")}`;
  }
  if (locator.page) {
    return `p.${String(locator.page)}`;
  }
  return String(locator.kind ?? "");
}

export function ChunkExplorerPage({
  me,
  membership,
  onLoggedOut,
}: {
  me: Me;
  membership: Membership;
  onLoggedOut: () => void;
}) {
  const { t, lang } = useT();
  const { did = "" } = useParams();
  const [document, setDocument] = useState<DocumentDetail | null>(null);
  const [nodes, setNodes] = useState<TreeNode[]>([]);
  const [crumbs, setCrumbs] = useState<{ id: string; name: string }[]>([]);
  const [chunks, setChunks] = useState<DocumentChunkSummary[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [selectedNode, setSelectedNode] = useState<TreeNode | null>(null);
  const [selectedChunk, setSelectedChunk] = useState<DocumentChunkSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [treeLoading, setTreeLoading] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const revisionId = document?.current_revision?.id;

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const loaded = await getDocument(did);
      setDocument(loaded);
      const revision = loaded.current_revision?.id;
      const [tree, chunkList] = await Promise.all([
        getDocumentTree(did, { revisionId: revision }),
        getDocumentChunks(did, { revisionId: revision, limit: 50 }),
      ]);
      setNodes(tree.nodes);
      setCrumbs([]);
      setChunks(chunkList.items);
      setCursor(chunkList.next_cursor ?? null);
      setHasMore(chunkList.has_more);
      setSelectedChunk(chunkList.items[0] ?? null);
    } catch (cause) {
      setError(cause);
    } finally {
      setLoading(false);
    }
  }, [did]);

  useEffect(() => {
    void load();
  }, [load]);

  async function openNode(node: TreeNode) {
    setSelectedNode(node);
    if (node.child_count === 0) return;
    setTreeLoading(true);
    try {
      const tree = await getDocumentTree(did, {
        revisionId,
        parent: node.id,
      });
      setNodes(tree.nodes);
      setCrumbs((current) => [...current, { id: node.id, name: node.name ?? node.kind }]);
    } catch (cause) {
      setError(cause);
    } finally {
      setTreeLoading(false);
    }
  }

  async function openCrumb(index: number) {
    setTreeLoading(true);
    try {
      if (index < 0) {
        const tree = await getDocumentTree(did, { revisionId });
        setNodes(tree.nodes);
        setCrumbs([]);
      } else {
        const target = crumbs[index];
        const tree = await getDocumentTree(did, {
          revisionId,
          parent: target.id,
        });
        setNodes(tree.nodes);
        setCrumbs((current) => current.slice(0, index + 1));
      }
    } catch (cause) {
      setError(cause);
    } finally {
      setTreeLoading(false);
    }
  }

  async function loadMore() {
    if (!cursor) return;
    try {
      const next = await getDocumentChunks(did, {
        revisionId,
        limit: 50,
        cursor,
      });
      setChunks((current) => [...current, ...next.items]);
      setCursor(next.next_cursor ?? null);
      setHasMore(next.has_more);
    } catch (cause) {
      setError(cause);
    }
  }

  return (
    <AppShell me={me} project={membership} active="documents" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("explorer.title")}</h1>
          <p className="page-subtitle">
            {document?.title ?? t("explorer.subtitle")}
          </p>
        </div>
        <div className="row">
          <Link className="btn btn-sm" to={`/projects/${membership.project_id}/documents/${did}`}>
            {t("document.title")}
          </Link>
          <Link className="btn btn-sm" to={`/projects/${membership.project_id}/search`}>
            {t("search.title")}
          </Link>
        </div>
      </div>

      {error ? <div className="page-error"><ErrorNotice error={error} onRetry={() => void load()} /></div> : null}

      {loading ? (
        <div className="panel panel-body stack">
          <div className="skeleton" style={{ width: "40%" }} />
          <div className="skeleton" style={{ width: "70%" }} />
        </div>
      ) : (
        <div className="explorer-layout">
          <aside className="panel explorer-tree">
            <div className="panel-head">
              <span className="panel-title">{t("explorer.tree")}</span>
            </div>
            <div className="panel-body stack">
              <div className="row">
                <button className="btn btn-sm btn-ghost" onClick={() => void openCrumb(-1)}>
                  {t("explorer.root")}
                </button>
                {crumbs.map((crumb, index) => (
                  <button
                    key={crumb.id}
                    className="btn btn-sm btn-ghost"
                    onClick={() => void openCrumb(index)}
                  >
                    / {crumb.name}
                  </button>
                ))}
              </div>
              {treeLoading ? (
                <div className="skeleton" style={{ width: "80%" }} />
              ) : nodes.length === 0 ? (
                <p className="muted" style={{ margin: 0 }}>{t("explorer.noNodes")}</p>
              ) : (
                <ul className="tree-list">
                  {nodes.map((node) => (
                    <li key={node.id}>
                      <button
                        className="tree-item"
                        data-selected={selectedNode?.id === node.id}
                        onClick={() => void openNode(node)}
                      >
                        <span>{node.name ?? node.kind}</span>
                        <span className="faint">
                          {node.kind}
                          {node.child_count > 0
                            ? ` · ${t("explorer.childCount", { count: node.child_count })}`
                            : ""}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </aside>

          <section className="stack explorer-main">
            <div className="panel">
              <div className="panel-head">
                <span className="panel-title">
                  {t("explorer.chunks")} · {formatNumber(chunks.length, lang)}
                </span>
              </div>
              <div className="panel-body stack">
                {chunks.length === 0 ? (
                  <EmptyState title={t("explorer.noChunks")} hint={t("explorer.noChunksHint")} />
                ) : (
                  <div className="chunk-list">
                    {chunks.map((chunk) => (
                      <button
                        key={chunk.id}
                        className="chunk-card"
                        data-selected={selectedChunk?.id === chunk.id}
                        onClick={() => setSelectedChunk(chunk)}
                      >
                        <span className="row row-between">
                          <strong>{locatorLabel(chunk.locator as Record<string, unknown>)}</strong>
                          <span className="chip">{chunk.chunk_type}</span>
                        </span>
                        <span className="chunk-excerpt">{chunk.text.slice(0, 180)}</span>
                        <span className="faint">
                          {t("explorer.tokenCount", { count: chunk.token_count })} · #{chunk.ordinal + 1}
                        </span>
                      </button>
                    ))}
                  </div>
                )}
                {hasMore ? (
                  <button className="btn btn-sm" onClick={() => void loadMore()}>
                    {t("explorer.loadMore")}
                  </button>
                ) : null}
              </div>
            </div>

            <div className="panel explorer-detail">
              <div className="panel-head">
                <span className="panel-title">{t("explorer.detail")}</span>
                {selectedNode ? <span className="chip">{selectedNode.kind}</span> : null}
              </div>
              <div className="panel-body stack">
                {selectedChunk ? (
                  <>
                    <div className="row">
                      <span className="chip chip-accent">
                        {locatorLabel(selectedChunk.locator as Record<string, unknown>)}
                      </span>
                      <span className="chip">{selectedChunk.chunk_type}</span>
                      <span className="chip num">
                        {t("explorer.tokenCount", { count: selectedChunk.token_count })}
                      </span>
                    </div>
                    <pre className="chunk-text">{selectedChunk.text}</pre>
                    <div>
                      <div className="label">{t("explorer.locator")}</div>
                      <pre className="mono chunk-locator">
                        {JSON.stringify(selectedChunk.locator, null, 2)}
                      </pre>
                    </div>
                  </>
                ) : (
                  <p className="muted" style={{ margin: 0 }}>{t("explorer.selectHint")}</p>
                )}
              </div>
            </div>
          </section>
        </div>
      )}
    </AppShell>
  );
}
