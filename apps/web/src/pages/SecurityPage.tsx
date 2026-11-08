import { useCallback, useEffect, useState } from "react";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice } from "../components/ui";
import { useT } from "../i18n";
import { listSessions, logoutAll, revokeSession } from "../lib/api";
import { formatDate } from "../lib/format";
import type { Me, SessionSummary } from "../lib/types";

export function SecurityPage({ me, onLoggedOut }: { me: Me; onLoggedOut: () => void }) {
  const { t, lang } = useT();
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const response = await listSessions();
      setSessions(response.sessions);
      setError(null);
    } catch (cause) {
      setError(cause);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function revoke(id: string) {
    setBusy(id);
    setNotice(null);
    try {
      await revokeSession(id);
      setNotice(t("security.revoked"));
      await load();
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(null);
    }
  }

  async function revokeOthers() {
    setBusy("others");
    setNotice(null);
    try {
      const response = await logoutAll();
      setNotice(t("security.revokedOthers", { count: response.revoked }));
      await load();
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(null);
    }
  }

  return (
    <AppShell me={me} active="security" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("security.title")}</h1>
          <p className="page-subtitle">{t("security.subtitle")}</p>
        </div>
        <button
          className="btn"
          onClick={revokeOthers}
          disabled={busy === "others"}
        >
          {t("security.revokeOthers")}
        </button>
      </div>

      {notice ? <div className="test-notice">{notice}</div> : null}
      {error ? <ErrorNotice error={error} /> : null}

      <div className="panel">
        <div className="panel-head">
          <span className="panel-title">{t("security.current")}</span>
          <span className="chip">{sessions.length}</span>
        </div>
        {sessions.length === 0 ? (
          <EmptyState title={t("security.empty")} hint={t("security.emptyHint")} />
        ) : (
          <div className="table-wrap">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("security.provider")}</th>
                  <th>{t("security.userAgent")}</th>
                  <th>{t("security.created")}</th>
                  <th>{t("security.lastSeen")}</th>
                  <th>{t("security.expires")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {sessions.map((session) => (
                  <tr key={session.id}>
                    <td>
                      {session.provider ?? t("security.password")}
                      {session.current ? (
                        <span className="chip security-current">{t("security.current")}</span>
                      ) : null}
                    </td>
                    <td className="faint">
                      {session.user_agent ?? t("security.unknown")}
                    </td>
                    <td>{formatDate(session.created_at, lang)}</td>
                    <td>{formatDate(session.last_seen_at, lang)}</td>
                    <td>{formatDate(session.expires_at, lang)}</td>
                    <td>
                      <button
                        className="btn btn-sm btn-ghost"
                        onClick={() => revoke(session.id)}
                        disabled={session.current || busy === session.id}
                      >
                        {t("security.revoke")}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </AppShell>
  );
}
