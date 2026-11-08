import { useCallback, useEffect, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router-dom";
import type { MessageKey } from "../i18n";
import { useT } from "../i18n";
import {
  activateDocumentRevision,
  deleteDocument,
  documentRevisionContentUrl,
  getDocument,
  getJob,
  listDocumentRevisions,
  parseDocument,
} from "../lib/api";
import { useAsyncAction } from "../lib/useAsyncAction";
import { formatBytes, formatDate } from "../lib/format";
import type {
  DocumentDetail,
  JobResponse,
  Me,
  Membership,
  RevisionSummary,
} from "../lib/types";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice, JobStrip, Modal, StatusBadge, statusText } from "../components/ui";
import { PdfViewer } from "../pdf/PdfViewer";

const TERMINAL = new Set(["SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"]);

export function DocumentPage({
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
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const [document, setDocument] = useState<DocumentDetail | null>(null);
  const [revisions, setRevisions] = useState<RevisionSummary[]>([]);
  const [activating, setActivating] = useState("");
  const { loading, error, run: runLoad, setError } = useAsyncAction();
  const [job, setJob] = useState<JobResponse | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [confirmTitle, setConfirmTitle] = useState("");
  const [deleteError, setDeleteError] = useState<unknown>(null);

  const quote = searchParams.get("quote") ?? "";
  const requestedPage = Number(searchParams.get("page") ?? "1");
  const page = Number.isFinite(requestedPage) && requestedPage > 0 ? Math.floor(requestedPage) : 1;
  const pageCount = document?.current_revision?.page_count ?? 1;
  const mimeType = document?.current_revision?.mime_type ?? "";
  const isPdf = mimeType === "application/pdf";
  const unitLabel = isPdf
    ? t("document.pages")
    : mimeType.includes("wordprocessingml")
      ? t("document.sections")
      : mimeType.includes("presentationml")
        ? t("document.slides")
        : t("document.sheets");

  const load = useCallback(async () => {
    await runLoad(async (isCurrent) => {
      const [loaded, history] = await Promise.all([
        getDocument(did),
        listDocumentRevisions(did),
      ]);
      if (isCurrent()) {
        setDocument(loaded);
        setRevisions(history.items);
      }
    });
  }, [did, runLoad]);

  useEffect(() => {
    void load();
  }, [load]);

  async function runParse() {
    const revision = document?.current_revision;
    if (!revision) return;
    try {
      const accepted = await parseDocument(did, revision.id, crypto.randomUUID());
      for (let attempt = 0; attempt < 1200; attempt += 1) {
        const current = await getJob(accepted.job_id);
        setJob(current);
        if (TERMINAL.has(current.status)) break;
        await new Promise((resolve) => setTimeout(resolve, 600));
      }
      await load();
    } catch (cause) {
      setError(cause);
    }
  }

  async function activateRevision(revisionId: string) {
    setActivating(revisionId);
    try {
      const history = await activateDocumentRevision(
        membership.project_id,
        did,
        revisionId,
      );
      setRevisions(history.items);
      setDocument(await getDocument(did));
    } catch (cause) {
      setError(cause);
    } finally {
      setActivating("");
    }
  }

  async function confirmDelete() {
    if (!document || confirmTitle !== document.title) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      await deleteDocument(did);
      navigate(`/projects/${membership.project_id}/documents`, { replace: true });
    } catch (cause) {
      setDeleteError(cause);
    } finally {
      setDeleting(false);
    }
  }

  function movePage(nextPage: number) {
    const clamped = Math.max(1, Math.min(pageCount || 1, nextPage));
    const next = new URLSearchParams(searchParams);
    next.set("page", String(clamped));
    setSearchParams(next, { replace: true });
  }

  const revision = document?.current_revision ?? null;

  return (
    <AppShell me={me} project={membership} active="documents" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <div className="faint breadcrumb">
            <Link to={`/projects/${membership.project_id}/documents`}>
              {t("nav.documents")}
            </Link>
            {" / "}
            {document?.title}
          </div>
          <h1 className="page-title">{document?.title ?? t("common.loading")}</h1>
          {revision ? (
            <p className="page-subtitle">
              {t(`docType.${document?.doc_type}` as MessageKey)} / {revision.page_count}{" "}
              {t("document.pagesUnit")} / {revision.chunk_count} {t("documents.colChunks")}
            </p>
          ) : null}
        </div>
        <div className="actions">
          {revision ? (
            <Link
              className="btn"
              to={`/projects/${membership.project_id}/documents/${did}/explorer`}
            >
              {t("document.explorer")}
            </Link>
          ) : null}
          {revision ? (
            <button className="btn" onClick={() => void runParse()} disabled={!revision}>
              {revision.parse_status === "NOT_PARSED" ? t("document.parse") : t("document.reparse")}
            </button>
          ) : null}
          <button className="btn btn-danger" onClick={() => setConfirmOpen(true)}>
            {t("document.delete")}
          </button>
        </div>
      </div>

      {error ? (
        <div className="page-error">
          <ErrorNotice error={error} onRetry={() => void load()} />
        </div>
      ) : null}
      {job ? <JobStrip job={job} /> : null}

      <section className="panel panel-body section-gap stack">
        <h2 className="panel-title">{t("document.revisions")}</h2>
        {revisions.length === 0 ? (
          <p className="muted">{t("document.noRevisions")}</p>
        ) : (
          <div className="table-scroll">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("document.revisionId")}</th>
                  <th>{t("document.revisionSource")}</th>
                  <th>{t("document.revisionStatus")}</th>
                  <th>{t("documents.colChunks")}</th>
                  <th>{t("document.revisionAcquired")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {revisions.map((item) => (
                  <tr key={item.id} className="revision-row">
                    <td className="num">{item.id.slice(0, 8)}</td>
                    <td>{item.source_revision ?? "-"}</td>
                    <td>
                      <span
                        className={
                          item.status === "CURRENT" ? "chip chip-success" : "chip"
                        }
                      >
                        {item.status}
                      </span>
                    </td>
                    <td className="num">{item.chunk_count}</td>
                    <td className="num">{formatDate(item.acquired_at, lang)}</td>
                    <td>
                      <span className="row row-wrap">
                        <a
                          className="btn btn-sm"
                          href={documentRevisionContentUrl(did, item.id)}
                          target="_blank"
                          rel="noreferrer"
                        >
                          {t("document.revisionOriginal")}
                        </a>
                        <Link
                          className="btn btn-sm"
                          to={`/projects/${membership.project_id}/search?revision=${item.id}`}
                        >
                          {t("document.revisionSearch")}
                        </Link>
                        {item.status !== "CURRENT" ? (
                          <button
                            className="btn btn-sm"
                            disabled={activating !== ""}
                            onClick={() => void activateRevision(item.id)}
                          >
                            {t("document.revisionActivate")}
                          </button>
                        ) : null}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {loading ? (
        <div className="panel panel-body stack">
          <div className="skeleton skeleton-60" />
          <div className="skeleton skeleton-40" />
        </div>
      ) : !document ? (
        <div className="panel"><EmptyState title={t("error.title")} /></div>
      ) : !revision ? (
        <div className="panel"><EmptyState title={t("document.noRevision")} /></div>
      ) : (
        <div className="split">
          <aside className="aside-stack">
            <div className="panel">
              <div className="panel-head">
                <span className="panel-title">{t("document.meta")}</span>
                <StatusBadge status={revision.parse_status} />
              </div>
              <div className="panel-body">
                <dl className="meta-grid">
                  <div>
                    <dt>{t("document.docType")}</dt>
                    <dd>{t(`docType.${document.doc_type}` as MessageKey)}</dd>
                  </div>
                  <div>
                    <dt>{t("document.domain")}</dt>
                    <dd>{document.business_domain ?? "—"}</dd>
                  </div>
                  <div>
                    <dt>{t("document.status")}</dt>
                    <dd>{statusText(revision.parse_status, t)}</dd>
                  </div>
                  <div>
                    <dt>{t("document.size")}</dt>
                    <dd className="num">{formatBytes(revision.file_size, lang)}</dd>
                  </div>
                  <div>
                    <dt>{unitLabel}</dt>
                    <dd className="num">{revision.page_count}</dd>
                  </div>
                  <div>
                    <dt>{t("document.chunks")}</dt>
                    <dd className="num">{revision.chunk_count}</dd>
                  </div>
                  <div>
                    <dt>{t("document.acquired")}</dt>
                    <dd className="num">{formatDate(revision.acquired_at, lang)}</dd>
                  </div>
                  <div>
                    <dt>{t("document.revision")}</dt>
                    <dd className="num">{revision.id.slice(0, 8)}</dd>
                  </div>
                </dl>
                <div className="hash-block">
                  <div className="label">{t("document.hash")}</div>
                  <div className="mono faint hash-line">{revision.content_hash}</div>
                </div>
              </div>
            </div>
            <button
              className="btn btn-ghost btn-sm"
              onClick={() => navigate(`/projects/${membership.project_id}/documents`)}
            >
              {t("common.back")}
            </button>
          </aside>

          <section className="viewer-pane">
            {isPdf ? (
              <>
                <div className="viewer-toolbar viewer-toolbar-framed">
                  <button className="btn btn-sm" onClick={() => movePage(page - 1)} disabled={page <= 1}>
                    {t("document.prev")}
                  </button>
                  <span className="chip">
                    {t("document.page")} {page} / {Math.max(pageCount, 1)}
                  </span>
                  <button
                    className="btn btn-sm"
                    onClick={() => movePage(page + 1)}
                    disabled={page >= pageCount}
                  >
                    {t("document.next")}
                  </button>
                  {quote ? <span className="chip chip-accent">{t("search.quote")}</span> : null}
                </div>
                <PdfViewer documentId={did} page={page} quote={quote} />
              </>
            ) : (
              <div className="panel panel-body stack">
                <span className="panel-title">{t("document.office")}</span>
                <p className="muted" style={{ margin: 0 }}>{t("document.officeHint")}</p>
                <Link
                  className="btn btn-primary"
                  to={`/projects/${membership.project_id}/search`}
                >
                  {t("search.title")}
                </Link>
              </div>
            )}
          </section>
        </div>
      )}

      {confirmOpen && document ? (
        <Modal title={t("document.deleteTitle")} onClose={() => setConfirmOpen(false)}>
          <p className="muted modal-copy">{t("document.deleteBody")}</p>
          <input
            className="input"
            placeholder={t("document.deletePlaceholder")}
            value={confirmTitle}
            onChange={(event) => setConfirmTitle(event.target.value)}
          />
          {deleteError ? (
            <div className="modal-error">
              <ErrorNotice error={deleteError} />
            </div>
          ) : null}
          <div className="row row-end modal-actions">
            <button className="btn" onClick={() => setConfirmOpen(false)}>
              {t("common.cancel")}
            </button>
            <button
              className="btn btn-danger"
              disabled={confirmTitle !== document.title || deleting}
              onClick={() => void confirmDelete()}
            >
              {t("document.deleteConfirm")}
            </button>
          </div>
        </Modal>
      ) : null}
    </AppShell>
  );
}
