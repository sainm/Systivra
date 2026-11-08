import { useEffect, useId, useRef, type ReactNode } from "react";
import { useT, type MessageKey } from "../i18n";
import { ApiError } from "../lib/api";
import type { JobResponse } from "../lib/types";

const PHASE_KEY: Record<string, MessageKey> = {
  QUEUED: "phase.QUEUED",
  START: "phase.START",
  EXTRACT: "phase.EXTRACT",
  PARSE: "phase.PARSE",
  DONE: "phase.DONE",
  FAILED: "phase.FAILED",
};

export function statusText(status: string, t: (key: MessageKey) => string): string {
  const key = `status.${status}` as MessageKey;
  const translated = t(key);
  return translated === key ? status : translated;
}

export function StatusBadge({ status }: { status: string }) {
  const { t } = useT();
  const tone =
    status === "SUCCEEDED"
      ? "badge-ok"
      : status === "RUNNING"
        ? "badge-run"
        : status === "FAILED"
          ? "badge-danger"
          : status === "SUCCEEDED_WITH_ERRORS" || status === "PARTIAL"
            ? "badge-warn"
            : "";
  return <span className={`badge ${tone}`}>{statusText(status, t)}</span>;
}

export function ErrorNotice({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const { t } = useT();
  const apiError = error instanceof ApiError ? error : null;
  const title =
    apiError?.status === 401
      ? t("error.unauthorized")
      : apiError
        ? t("error.title")
        : t("error.network");
  const human =
    apiError?.code === "CONFLICT_ALREADY_IMPORTED"
      ? t("error.conflictAlreadyImported")
      : null;
  return (
    <div className="error-box" role="alert">
      <div className="error-title">{title}</div>
      {human ? <div className="muted error-human">{human}</div> : null}
      {apiError ? (
        <div className="error-meta">
          {t("error.code")}: {apiError.code} / {t("error.trace")}: {apiError.traceId || "—"}
          {apiError.detail ? ` / ${apiError.detail}` : ""}
        </div>
      ) : (
        <div className="error-meta">{error instanceof Error ? error.message : String(error)}</div>
      )}
      <div className="row error-actions">
        {onRetry ? (
          <button className="btn btn-sm" onClick={onRetry}>
            {t("common.retry")}
          </button>
        ) : null}
        <span className="faint error-hint">{t("error.hint")}</span>
      </div>
    </div>
  );
}

export function JobStrip({ job }: { job: JobResponse }) {
  const { t } = useT();
  const percent =
    job.progress.files_total > 0
      ? Math.min(100, Math.round((job.progress.files_done / job.progress.files_total) * 100))
      : job.status === "SUCCEEDED"
        ? 100
        : 4;
  const phaseKey = PHASE_KEY[job.progress.phase] ?? "status.RUNNING";
  const phase = t(phaseKey);
  return (
    <div className="job-strip">
      <StatusBadge status={job.status} />
      <span className="muted">
        {t("documents.jobPhase")}: {phase}
      </span>
      <span className="num">
        {job.progress.files_done} / {job.progress.files_total}
      </span>
      <span className="num faint">
        {job.progress.chunks_done} {t("documents.colChunks")}
      </span>
      <div className="progress job-progress" aria-label={`${percent}%`}>
        <span style={{ width: `${percent}%` }} />
      </div>
      {job.errors.length > 0 ? (
        <span className="badge badge-danger">
          {t("documents.jobFailed")}: {job.errors[0].code}
        </span>
      ) : null}
    </div>
  );
}

export function EmptyState({ title, hint, action }: { title: string; hint?: string; action?: ReactNode }) {
  return (
    <div className="empty">
      <strong>{title}</strong>
      {hint ? <span>{hint}</span> : null}
      {action ? <div className="row row-center empty-action">{action}</div> : null}
    </div>
  );
}

export function Modal({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  const { t } = useT();
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = dialogRef.current;
    if (!dialog) return;

    document.body.classList.add("modal-open");
    dialog.focus();

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeRef.current();
        return;
      }
      if (event.key !== "Tab" || !dialog) return;

      const focusable = Array.from(
        dialog.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((element) => element.getAttribute("aria-hidden") !== "true");

      if (focusable.length === 0) {
        event.preventDefault();
        return;
      }

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.body.classList.remove("modal-open");
      previous?.focus();
    };
  }, []);

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        ref={dialogRef}
      >
        <h2 id={titleId}>{title}</h2>
        {children}
        <div className="row row-end modal-actions">
          <button className="btn" onClick={onClose}>
            {t("common.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
