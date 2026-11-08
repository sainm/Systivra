import type { ReactNode } from "react";
import { NavLink, useNavigate } from "react-router-dom";
import { LANGUAGES, useT, type MessageKey } from "../i18n";
import { logout } from "../lib/api";
import type { Me, Membership } from "../lib/types";

interface Props {
  me: Me;
  project?: Membership;
  active:
    | "projects"
    | "documents"
    | "search"
    | "indexes"
    | "code"
    | "graph"
    | "impact"
    | "tests"
    | "ai"
    | "inquiry"
    | "security"
    | "users";
  children: ReactNode;
  onLoggedOut: () => void;
}

export function AppShell({ me, project, active, children, onLoggedOut }: Props) {
  const { t, lang, setLang } = useT();
  const navigate = useNavigate();

  async function signOut() {
    try {
      await logout();
    } finally {
      onLoggedOut();
      navigate("/login", { replace: true });
    }
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true" />
          <div>
            <div className="brand-name">{t("app.name")}</div>
            <div className="brand-tagline">{t("app.tagline")}</div>
          </div>
        </div>
        <nav className="nav" aria-label={t("nav.workspace")}>
          <NavLink end to="/" className="nav-link" aria-current={active === "projects" ? "page" : undefined}>
            {t("nav.projects")}
          </NavLink>
          {project ? (
            <>
              <NavLink
                to={`/projects/${project.project_id}/documents`}
                className="nav-link"
                aria-current={active === "documents" ? "page" : undefined}
              >
                {t("nav.documents")}
              </NavLink>
              <NavLink
                to={`/projects/${project.project_id}/search`}
                className="nav-link"
                aria-current={active === "search" ? "page" : undefined}
              >
                {t("nav.search")}
              </NavLink>
              <NavLink
                to={`/projects/${project.project_id}/indexes`}
                className="nav-link"
                aria-current={active === "indexes" ? "page" : undefined}
              >
                {t("nav.indexes")}
              </NavLink>
              <NavLink
                to={`/projects/${project.project_id}/graph`}
                className="nav-link"
                aria-current={active === "graph" ? "page" : undefined}
              >
                {t("nav.graph")}
              </NavLink>
              <NavLink
                to={`/projects/${project.project_id}/code`}
                className="nav-link"
                aria-current={active === "code" ? "page" : undefined}
              >
                {t("nav.code")}
              </NavLink>
              <NavLink
                to={`/projects/${project.project_id}/impact`}
                className="nav-link"
                aria-current={active === "impact" ? "page" : undefined}
              >
                {t("nav.impact")}
              </NavLink>
              <NavLink
                to={`/projects/${project.project_id}/tests`}
                className="nav-link"
                aria-current={active === "tests" ? "page" : undefined}
              >
                {t("nav.tests")}
              </NavLink>
              <NavLink
                to={`/projects/${project.project_id}/ai`}
                className="nav-link"
                aria-current={active === "ai" ? "page" : undefined}
              >
                {t("nav.ai")}
              </NavLink>
              <NavLink
                to={`/projects/${project.project_id}/inquiry`}
                className="nav-link"
                aria-current={active === "inquiry" ? "page" : undefined}
              >
                {t("nav.inquiry")}
              </NavLink>
            </>
          ) : null}
        </nav>
        <div className="topbar-spacer" />
        {project ? (
          <div className="project-chip" title={project.project_name}>
            <span className="num faint">{project.project_code}</span>
            <strong>{project.project_name}</strong>
            <span className="chip">{t(`role.${project.role}` as MessageKey)}</span>
          </div>
        ) : null}
        <select
          className="select topbar-lang"
          aria-label={t("lang.label")}
          value={lang}
          onChange={(event) => setLang(event.target.value)}
        >
          {LANGUAGES.map((language) => (
            <option key={language.code} value={language.code}>
              {t(language.labelKey)}
            </option>
          ))}
        </select>
        <NavLink
          to="/security"
          className="nav-link"
          aria-current={active === "security" ? "page" : undefined}
        >
          {t("nav.security")}
        </NavLink>
        <NavLink
          to="/users"
          className="nav-link"
          aria-current={active === "users" ? "page" : undefined}
        >
          {t("nav.users")}
        </NavLink>
        <span className="topbar-user muted">{me.display_name}</span>
        <button className="btn btn-ghost btn-sm topbar-logout" onClick={signOut}>
          {t("nav.logout")}
        </button>
      </header>
      <main className="page">{children}</main>
    </div>
  );
}
