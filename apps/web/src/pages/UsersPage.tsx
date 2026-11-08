import { useCallback, useEffect, useState } from "react";
import { AppShell } from "../components/AppShell";
import { EmptyState, ErrorNotice } from "../components/ui";
import { useT, type MessageKey } from "../i18n";
import { listMembers, updateMemberRole } from "../lib/api";
import type { Me, MemberSummary } from "../lib/types";

const ROLES = ["OWNER", "ARCHITECT", "DEVELOPER", "TESTER", "VIEWER"] as const;

export function UsersPage({ me, onLoggedOut }: { me: Me; onLoggedOut: () => void }) {
  const { t } = useT();
  const project = me.memberships[0];
  const canEdit = project?.role === "OWNER";
  const [members, setMembers] = useState<MemberSummary[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!project) return;
    try {
      const response = await listMembers(project.project_id);
      setMembers(response.members);
      setError(null);
    } catch (cause) {
      setError(cause);
    }
  }, [project]);

  useEffect(() => {
    void load();
  }, [load]);

  async function changeRole(member: MemberSummary, role: string) {
    if (!project) return;
    setBusy(member.user_id);
    setNotice(null);
    try {
      await updateMemberRole(project.project_id, member.user_id, role);
      setNotice(t("users.updated", { name: member.display_name, role }));
      await load();
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(null);
    }
  }

  return (
    <AppShell me={me} active="users" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("users.title")}</h1>
          <p className="page-subtitle">{t("users.subtitle")}</p>
        </div>
        <span className="chip">{members.length}</span>
      </div>

      {notice ? <div className="test-notice">{notice}</div> : null}
      {error ? <ErrorNotice error={error} /> : null}

      <div className="panel">
        <div className="panel-head">
          <span className="panel-title">{t("users.members")}</span>
        </div>
        {members.length === 0 ? (
          <EmptyState title={t("users.empty")} hint={t("users.emptyHint")} />
        ) : (
          <div className="table-wrap">
            <table className="data">
              <thead>
                <tr>
                  <th>{t("users.loginId")}</th>
                  <th>{t("users.displayName")}</th>
                  <th>{t("users.email")}</th>
                  <th>{t("users.status")}</th>
                  <th>{t("users.role")}</th>
                </tr>
              </thead>
              <tbody>
                {members.map((member) => (
                  <tr key={member.user_id}>
                    <td className="num">{member.login_id}</td>
                    <td>{member.display_name}</td>
                    <td className="faint">{member.email ?? "—"}</td>
                    <td>{member.is_active ? t("users.active") : t("users.inactive")}</td>
                    <td>
                      <select
                        className="select"
                        value={member.role}
                        disabled={!canEdit || busy === member.user_id}
                        onChange={(event) => changeRole(member, event.target.value)}
                      >
                        {ROLES.map((role) => (
                          <option key={role} value={role}>
                            {t(`role.${role}` as MessageKey)}
                          </option>
                        ))}
                      </select>
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
