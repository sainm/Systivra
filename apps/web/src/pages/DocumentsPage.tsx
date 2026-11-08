import { useCallback, useEffect, useRef, useState, type DragEvent, type FormEvent } from "react";
import { Link, useNavigate } from "react-router-dom";
import type { MessageKey } from "../i18n";
import { useT } from "../i18n";
import {
  createUploadUrl,
  getJob,
  listDocuments,
  parseDocument,
  putPdf,
  registerDocument,
  ApiError,
} from "../lib/api";
import { useAsyncAction } from "../lib/useAsyncAction";
import { formatDate, formatNumber } from "../lib/format";
import type { DocumentSummary, JobResponse, Me, Membership } from "../lib/types";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice, JobStrip, StatusBadge } from "../components/ui";

const TERMINAL = new Set(["SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"]);
const XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const XLSM_MIME = "application/vnd.ms-excel.sheet.macroEnabled.12";
const DOCX_MIME = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const PPTX_MIME = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

function detectMime(file: File): string | null {
  const name = file.name.toLowerCase();
  if (file.type === "application/pdf" || name.endsWith(".pdf")) return "application/pdf";
  if (file.type === XLSX_MIME || name.endsWith(".xlsx")) return XLSX_MIME;
  if (file.type === XLSM_MIME || name.endsWith(".xlsm")) return XLSM_MIME;
  if (file.type === DOCX_MIME || name.endsWith(".docx")) return DOCX_MIME;
  if (file.type === PPTX_MIME || name.endsWith(".pptx")) return PPTX_MIME;
  return null;
}

export function DocumentsPage({
  me,
  membership,
  onLoggedOut,
}: {
  me: Me;
  membership: Membership;
  onLoggedOut: () => void;
}) {
  const { t, lang } = useT();
  const navigate = useNavigate();
  const fileRef = useRef<HTMLInputElement | null>(null);
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [filter, setFilter] = useState("");
  const [appliedFilter, setAppliedFilter] = useState("");
  const { loading, error, run: runLoad } = useAsyncAction();
  const [uploadError, setUploadError] = useState<unknown>(null);
  const [uploadStep, setUploadStep] = useState<string | null>(null);
  const [job, setJob] = useState<JobResponse | null>(null);
  const [drag, setDrag] = useState(false);

  const load = useCallback(
    async (append: boolean, nextCursor?: string) => {
      await runLoad(async (isCurrent) => {
        const response = await listDocuments(membership.project_id, {
          q: appliedFilter || undefined,
          cursor: nextCursor,
        });
        if (!isCurrent()) return;
        setDocuments((current) =>
          append ? [...current, ...response.items] : response.items,
        );
        setCursor(response.next_cursor ?? null);
        setHasMore(response.has_more);
      });
    },
    [appliedFilter, membership.project_id, runLoad],
  );

  useEffect(() => {
    void load(false);
  }, [load]);

  useEffect(() => {
    if (error instanceof ApiError && error.status === 401) {
      onLoggedOut();
    }
  }, [error, onLoggedOut]);

  async function watchJob(jobId: string) {
    for (let attempt = 0; attempt < 1200; attempt += 1) {
      const current = await getJob(jobId);
      setJob(current);
      if (TERMINAL.has(current.status)) break;
      await new Promise((resolve) => setTimeout(resolve, 600));
    }
    await load(false);
    setUploadStep(null);
  }

  async function ingest(file: File) {
    const mimeType = detectMime(file);
    if (!mimeType) {
      setUploadError(new Error(t("upload.unsupported")));
      return;
    }
    if (file.size > 100 * 1024 * 1024) {
      setUploadError(new Error(t("upload.tooLarge")));
      return;
    }
    setUploadError(null);
    setJob(null);
    setUploadStep("UPLOAD");
    try {
      const upload = await createUploadUrl(membership.project_id, {
        filename: file.name,
        size_bytes: file.size,
        mime_type: mimeType,
      });
      await putPdf(upload.upload_url, file, mimeType);
      setUploadStep("REGISTER");
      const registered = await registerDocument(membership.project_id, {
        object_key: upload.object_key,
        doc_type: "DETAILED_DESIGN",
        title: file.name.replace(/\.(pdf|xlsx|xlsm|docx|pptx)$/i, ""),
        mime_type: mimeType,
      });
      const revisionId = registered.document.current_revision?.id;
      if (!revisionId) throw new Error(t("upload.failed"));
      setUploadStep("PARSE");
      const accepted = await parseDocument(
        registered.document.id,
        revisionId,
        crypto.randomUUID(),
      );
      await watchJob(accepted.job_id);
    } catch (cause) {
      setUploadError(cause);
      setUploadStep(null);
    }
  }

  function onFileChange(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (file) void ingest(file);
  }

  function onDrop(event: DragEvent) {
    event.preventDefault();
    setDrag(false);
    const file = event.dataTransfer.files?.[0];
    if (file) void ingest(file);
  }

  function submitFilter(event: FormEvent) {
    event.preventDefault();
    setAppliedFilter(filter.trim());
  }

  return (
    <AppShell me={me} project={membership} active="documents" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("documents.title")}</h1>
          <p className="page-subtitle">{t("documents.subtitle")}</p>
        </div>
        <div className="actions">
          <span className="chip">{t("common.items", { count: documents.length })}</span>
          <button
            className="btn btn-primary"
            onClick={() => fileRef.current?.click()}
            disabled={uploadStep !== null}
          >
            {uploadStep ? t("documents.uploading") : t("documents.upload")}
          </button>
          <input
            ref={fileRef}
            type="file"
            accept="application/pdf,.pdf,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,.xlsx,application/vnd.ms-excel.sheet.macroEnabled.12,.xlsm,application/vnd.openxmlformats-officedocument.wordprocessingml.document,.docx,application/vnd.openxmlformats-officedocument.presentationml.presentation,.pptx"
            hidden
            onChange={onFileChange}
          />
        </div>
      </div>

      <div
        className="dropzone dropzone-block"
        data-drag={drag}
        onClick={() => fileRef.current?.click()}
        onDragOver={(event) => {
          event.preventDefault();
          setDrag(true);
        }}
        onDragLeave={() => setDrag(false)}
        onDrop={onDrop}
        role="button"
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") fileRef.current?.click();
        }}
      >
        <div className="dropzone-title">{t("documents.dropzone")}</div>
        <div className="faint dropzone-hint">
          {uploadStep
            ? `${t("documents.jobPhase")}: ${uploadStep}`
            : t("documents.uploadHint")}
        </div>
      </div>

      {uploadError ? (
        <div className="page-error">
          <ErrorNotice error={uploadError} />
        </div>
      ) : null}
      {job ? <JobStrip job={job} /> : null}
      {error ? (
        <div className="page-error">
          <ErrorNotice error={error} onRetry={() => void load(false)} />
        </div>
      ) : null}

      <div className="panel">
        <div className="panel-head">
          <span className="panel-title">{t("documents.title")}</span>
          <form className="filters" onSubmit={submitFilter}>
            <input
              className="input"
              placeholder={t("documents.filter")}
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
            />
            <button className="btn btn-sm" type="submit">
              {t("search.submit")}
            </button>
          </form>
        </div>
        {loading ? (
          <div className="panel-body stack">
            <div className="skeleton skeleton-70" />
            <div className="skeleton skeleton-50" />
            <div className="skeleton skeleton-60" />
          </div>
        ) : documents.length === 0 ? (
          <EmptyState
            title={t("documents.empty")}
            hint={t("documents.emptyHint")}
            action={
              <button className="btn btn-primary" onClick={() => fileRef.current?.click()}>
                {t("documents.upload")}
              </button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("documents.colTitle")}</th>
                  <th>{t("documents.colType")}</th>
                  <th className="num">{t("documents.colPages")}</th>
                  <th className="num">{t("documents.colChunks")}</th>
                  <th>{t("documents.colStatus")}</th>
                  <th>{t("documents.colUpdated")}</th>
                </tr>
              </thead>
              <tbody>
                {documents.map((document) => (
                  <tr key={document.id}>
                    <td>
                      <Link
                        className="row-title"
                        to={`/projects/${membership.project_id}/documents/${document.id}`}
                      >
                        {document.title}
                      </Link>
                      {document.business_domain ? (
                        <span className="faint">{document.business_domain}</span>
                      ) : null}
                    </td>
                    <td className="muted">{t(`docType.${document.doc_type}` as MessageKey)}</td>
                    <td className="num">{formatNumber(document.page_count, lang)}</td>
                    <td className="num">{formatNumber(document.chunk_count, lang)}</td>
                    <td>
                      <StatusBadge status={document.parse_status} />
                    </td>
                    <td className="muted num">{formatDate(document.updated_at, lang)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {hasMore ? (
          <div className="panel-body panel-footer">
            <button
              className="btn"
              onClick={() => void load(true, cursor ?? undefined)}
              disabled={loading}
            >
              {t("documents.more")}
            </button>
          </div>
        ) : null}
      </div>
      <div className="row back-row">
        <button className="btn btn-sm btn-ghost" onClick={() => navigate("/")}>
          {t("common.back")}
        </button>
      </div>
    </AppShell>
  );
}
