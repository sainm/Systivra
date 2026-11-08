import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import type { MessageKey } from "../i18n";
import { useT } from "../i18n";
import { getEvidence, search } from "../lib/api";
import { useAsyncAction } from "../lib/useAsyncAction";
import { asLocator, type PdfLocator } from "../lib/types";
import type {
  DocType,
  EvidenceResponse,
  Me,
  Membership,
  SearchResponse,
  SearchResult,
} from "../lib/types";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice } from "../components/ui";

const DOC_TYPES: DocType[] = [
  "DETAILED_DESIGN",
  "BASIC_DESIGN",
  "UI_SPEC",
  "DB_SPEC",
  "API_SPEC",
  "TEST_SPEC",
  "REQUIREMENT",
  "MANUAL",
  "OTHER",
];

const SEARCH_CACHE_TTL_MS = 60_000;
const searchCache = new Map<string, { at: number; response: SearchResponse }>();
const inFlightSearches = new Map<string, Promise<SearchResponse>>();

export function clearSearchCache() {
  searchCache.clear();
  inFlightSearches.clear();
}

function pageRange(locator: PdfLocator): string {
  if (locator.kind === "xlsx") {
    return [locator.sheet, locator.range].filter(Boolean).join(" ") || "—";
  }
  if (locator.kind === "docx") {
    return (locator.section_path ?? []).join(" / ") || "—";
  }
  if (!locator.page) return "—";
  if (locator.page_end && locator.page_end !== locator.page) {
    return `${locator.page}–${locator.page_end}`;
  }
  return String(locator.page);
}

function sourceHref(projectId: string, result: SearchResult, locator: PdfLocator): string {
  return `/projects/${projectId}/documents/${result.document.id}?page=${
    locator.page ?? 1
  }&quote=${encodeURIComponent(result.quote)}`;
}

export function SearchPage({
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
  const [searchParams, setSearchParams] = useSearchParams();
  const initialQuery = searchParams.get("q") ?? "";
  const initialType = (searchParams.get("type") as DocType | null) ?? "";
  const revisionFilter = searchParams.get("revision") ?? "";
  const [query, setQuery] = useState(initialQuery);
  const [docType, setDocType] = useState<DocType | "">(initialType);
  const [inspect, setInspect] = useState(searchParams.get("inspect") === "1");
  const [rerank, setRerank] = useState(searchParams.get("rerank") === "1");
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [selected, setSelected] = useState<SearchResult | null>(null);
  const [evidence, setEvidence] = useState<EvidenceResponse | null>(null);
  const triggerRef = useRef<HTMLElement | null>(null);
  const drawerCloseRef = useRef<HTMLButtonElement | null>(null);
  const hadSelectionRef = useRef(false);

  const searchAction = useAsyncAction();
  const evidenceAction = useAsyncAction();
  const { loading: searchLoading, error: searchError, run: runSearchAction } = searchAction;
  const {
    loading: evidenceLoading,
    error: evidenceError,
    run: runEvidence,
    reset: resetEvidence,
  } = evidenceAction;

  const runSearch = useCallback(
    async (text: string, type: DocType | "", options?: { force?: boolean; rerank?: boolean }) => {
      const trimmed = text.trim();
      if (!trimmed) return;

      setSelected(null);
      setEvidence(null);
      resetEvidence();
      triggerRef.current = null;

      const useRerank = options?.rerank ?? false;
      const key = `${pid}\u0000${type}\u0000${trimmed}\u0000${revisionFilter}\u0000${useRerank ? "rerank" : "plain"}`;
      const cached = searchCache.get(key);
      if (!options?.force && cached && Date.now() - cached.at < SEARCH_CACHE_TTL_MS) {
        setResponse(cached.response);
        return;
      }

      let request = inFlightSearches.get(key);
      if (!request) {
        request = search(pid, {
          query: trimmed,
          mode: "hybrid",
          filters: {
            ...(type ? { doc_types: [type] } : {}),
            ...(revisionFilter ? { revision_id: revisionFilter } : {}),
          },
          top_k: 12,
          include: ["locator", "quote", "scores"],
          rerank: useRerank,
        });
        inFlightSearches.set(key, request);
        void request
          .finally(() => {
            inFlightSearches.delete(key);
          })
          .catch(() => undefined);
      }

      await runSearchAction(async (isCurrent) => {
        const result = await request;
        searchCache.set(key, { at: Date.now(), response: result });
        if (isCurrent()) setResponse(result);
      });
    },
    [pid, resetEvidence, runSearchAction],
  );

  useEffect(() => {
    if (initialQuery) void runSearch(initialQuery, initialType, { rerank });
  }, [initialQuery, initialType, rerank, runSearch]);

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!query.trim()) return;

    const next = new URLSearchParams();
    next.set("q", query.trim());
    if (docType) next.set("type", docType);
    if (inspect) next.set("inspect", "1");
    if (rerank) next.set("rerank", "1");

    const nextKey = `${query.trim()}\u0000${docType}\u0000${inspect}\u0000${rerank}`;
    const currentKey = `${initialQuery}\u0000${initialType}\u0000${
      searchParams.get("inspect") === "1"
    }\u0000${searchParams.get("rerank") === "1"}`;
    if (nextKey === currentKey) {
      void runSearch(query, docType, { force: true, rerank });
    } else {
      setSearchParams(next);
    }
  }

  async function openEvidence(result: SearchResult, trigger?: HTMLElement) {
    if (trigger) triggerRef.current = trigger;
    setSelected(result);
    setEvidence(null);

    const evidenceId = result.evidence_id;
    if (!evidenceId) {
      resetEvidence();
      return;
    }

    await runEvidence(async (isCurrent) => {
      const loaded = await getEvidence(evidenceId);
      if (isCurrent()) setEvidence(loaded);
    });
  }

  const closeEvidence = useCallback(() => {
    setSelected(null);
    setEvidence(null);
    resetEvidence();
    const trigger = triggerRef.current;
    triggerRef.current = null;
    trigger?.focus();
  }, [resetEvidence]);

  useEffect(() => {
    if (!selected) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeEvidence();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [selected, closeEvidence]);

  useEffect(() => {
    if (!selected) {
      hadSelectionRef.current = false;
      return;
    }

    if (!hadSelectionRef.current) {
      drawerCloseRef.current?.focus();
      hadSelectionRef.current = true;
    }

    if (!window.matchMedia("(max-width: 900px)").matches) return;
    document.body.classList.add("drawer-open");
    return () => document.body.classList.remove("drawer-open");
  }, [selected]);

  const locator = selected ? asLocator(selected.locator) : null;

  return (
    <AppShell me={me} project={membership} active="search" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("search.title")}</h1>
          <p className="page-subtitle">{t("search.subtitle")}</p>
        </div>
        {response ? (
          <div className="row">
            <span className="chip">
              {t("search.candidates")}: {response.total_candidates}
            </span>
            <span className="chip num">{response.latency_ms} ms</span>
            {response.degraded.length > 0 ? (
              <span className="badge badge-warn">
                {t("search.degraded")}: {response.degraded.length}
              </span>
            ) : null}
          </div>
        ) : null}
      </div>

      <form className="panel panel-body search-form" onSubmit={submit}>
        <div className="filters">
          {revisionFilter ? (
            <span className="chip chip-accent" title={revisionFilter}>
              {t("search.revisionScoped")}: {revisionFilter.slice(0, 8)}
            </span>
          ) : null}
          <input
            className="input search-query"
            placeholder={t("search.placeholder")}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            autoFocus
          />
          <select
            className="select"
            value={docType}
            onChange={(event) => setDocType(event.target.value as DocType | "")}
            aria-label={t("search.docType")}
          >
            <option value="">{t("common.all")}</option>
            {DOC_TYPES.map((type) => (
              <option key={type} value={type}>
                {t(`docType.${type}` as MessageKey)}
              </option>
            ))}
          </select>
          <label className="check">
            <input
              type="checkbox"
              checked={rerank}
              onChange={(event) => setRerank(event.target.checked)}
            />
            {t("search.rerank")}
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={inspect}
              onChange={(event) => setInspect(event.target.checked)}
            />
            {t("search.inspect")}
          </label>
          <button className="btn btn-primary" type="submit" disabled={searchLoading}>
            {searchLoading ? t("search.searching") : t("search.submit")}
          </button>
        </div>
      </form>

      {searchError ? (
        <div className="page-error">
          <ErrorNotice error={searchError} onRetry={() => void runSearch(query, docType, { rerank })} />
        </div>
      ) : null}

      {inspect && response ? (
        <div className="row row-wrap inspect-strip">
          <span className="chip">{t("search.lexicalMode")}: {response.lexical_mode}</span>
          {response.degraded.map((item) => (
            <span key={item} className="badge badge-warn">{item}</span>
          ))}
        </div>
      ) : null}

      <div className={`search-layout${selected ? " has-evidence" : ""}`}>
        <section className="stack results-pane">
          {searchLoading ? (
            <div className="panel panel-body stack">
              <div className="skeleton skeleton-50" />
              <div className="skeleton skeleton-80" />
              <div className="skeleton skeleton-65" />
            </div>
          ) : response && response.results.length === 0 ? (
            <div className="panel">
              <EmptyState title={t("search.empty")} hint={t("search.emptyHint")} />
            </div>
          ) : !response ? (
            <div className="panel">
              <EmptyState title={t("search.title")} hint={t("search.placeholder")} />
            </div>
          ) : (
            <>
              <div className="row row-between">
                <span className="panel-title">
                  {t("search.results")} · {response.results.length}
                </span>
              </div>
              {response.results.map((result) => {
                const resultLocator = asLocator(result.locator);
                return (
                  <article
                    key={result.chunk_id}
                    className="result-card"
                    data-selected={selected?.chunk_id === result.chunk_id}
                  >
                    <div className="result-head">
                      <div>
                        <div className="result-title">{result.document.title}</div>
                        <div className="result-meta">
                          {t(`docType.${result.document.doc_type}` as MessageKey)} ·{" "}
                          {resultLocator.kind === "xlsx" || resultLocator.kind === "docx"
                            ? pageRange(resultLocator)
                            : resultLocator.kind === "pptx"
                              ? `${t("search.slide")} ${resultLocator.slide ?? "—"}`
                              : `${t("search.page")} ${pageRange(resultLocator)}`}
                        </div>
                        {inspect ? (
                          <div className="result-inspect">
                            <span className="chip num">
                              {t("search.score")}: {result.score.toFixed(4)}
                            </span>
                            {result.rerank_score !== null && result.rerank_score !== undefined ? (
                              <span className="chip num">
                                {t("search.rerankScore")}: {result.rerank_score.toFixed(4)}
                              </span>
                            ) : null}
                            {result.sources.map((source) => (
                              <span key={source} className="chip">
                                {source}
                              </span>
                            ))}
                          </div>
                        ) : null}
                      </div>
                    </div>
                    <div>
                      <div className="label">{t("search.quote")}</div>
                      <div className="quote quote-clamp">{result.quote}</div>
                    </div>
                    <div className="result-actions">
                      {result.evidence_id ? (
                        <button
                          className="btn btn-primary btn-sm"
                          onClick={(event) => void openEvidence(result, event.currentTarget)}
                        >
                          {t("search.openEvidence")}
                        </button>
                      ) : (
                        <Link
                          className="btn btn-primary btn-sm"
                          to={sourceHref(pid, result, resultLocator)}
                        >
                          {t("evidence.openSource")}
                        </Link>
                      )}
                    </div>
                  </article>
                );
              })}
            </>
          )}
        </section>

        {selected ? (
          <>
            <div className="drawer-backdrop" onClick={closeEvidence} aria-hidden="true" />
            <aside className="drawer search-drawer" aria-label={t("evidence.title")}>
              <div className="drawer-head">
                <span className="panel-title">{t("evidence.title")}</span>
                <button
                  ref={drawerCloseRef}
                  className="btn btn-ghost btn-sm"
                  onClick={closeEvidence}
                  aria-label={t("common.close")}
                >
                  {t("common.close")}
                </button>
              </div>
              <div className="drawer-body">
                <div className="evidence-doc">
                  <strong>{selected.document.title}</strong>
                  <span className="result-meta">
                    {t(`docType.${selected.document.doc_type}` as MessageKey)} ·{" "}
                    {t("evidence.page")} {locator ? pageRange(locator) : "—"}
                  </span>
                </div>
                {evidenceLoading ? (
                  <div className="skeleton skeleton-80" />
                ) : evidenceError ? (
                  <ErrorNotice
                    error={evidenceError}
                    onRetry={() => void openEvidence(selected)}
                  />
                ) : evidence ? (
                  <>
                    <div className="label">{t("evidence.quote")}</div>
                    <div className="quote">{evidence.quote}</div>
                    {evidence.context_before ? (
                      <div>
                        <div className="label">{t("evidence.contextBefore")}</div>
                        <div className="context">{evidence.context_before}</div>
                      </div>
                    ) : null}
                    {evidence.context_after ? (
                      <div>
                        <div className="label">{t("evidence.contextAfter")}</div>
                        <div className="context">{evidence.context_after}</div>
                      </div>
                    ) : null}
                    <Link
                      className="btn btn-primary"
                      to={sourceHref(pid, selected, locator ?? {})}
                    >
                      {t("evidence.openSource")}
                    </Link>
                  </>
                ) : (
                  <p className="muted drawer-empty">{t("evidence.missing")}</p>
                )}
              </div>
            </aside>
          </>
        ) : null}
      </div>
    </AppShell>
  );
}
