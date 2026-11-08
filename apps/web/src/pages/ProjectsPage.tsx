import { Link } from "react-router-dom";
import type { MessageKey } from "../i18n";
import { useT } from "../i18n";
import type { Me } from "../lib/types";
import { AppShell } from "../components/AppShell";
import { EmptyState } from "../components/ui";

export function ProjectsPage({ me, onLoggedOut }: { me: Me; onLoggedOut: () => void }) {
  const { t } = useT();
  return (
    <AppShell me={me} active="projects" onLoggedOut={onLoggedOut}>
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("projects.title")}</h1>
          <p className="page-subtitle">{t("projects.subtitle")}</p>
        </div>
        <span className="chip">{t("common.items", { count: me.memberships.length })}</span>
      </div>

      {me.memberships.length === 0 ? (
        <div className="panel">
          <EmptyState title={t("projects.empty")} hint={t("projects.emptyHint")} />
        </div>
      ) : (
        <div className="project-grid">
          {me.memberships.map((membership) => (
            <Link
              key={membership.project_id}
              to={`/projects/${membership.project_id}/documents`}
              className="project-card"
            >
              <div className="row row-between">
                <span className="chip num">{membership.project_code}</span>
                <span className="chip">{t(`role.${membership.role}` as MessageKey)}</span>
              </div>
              <h2>{membership.project_name}</h2>
              <div className="card-footer">
                <span className="faint num">{membership.project_id.slice(0, 8)}</span>
                <span className="btn btn-sm">{t("projects.open")}</span>
              </div>
            </Link>
          ))}
        </div>
      )}
    </AppShell>
  );
}
