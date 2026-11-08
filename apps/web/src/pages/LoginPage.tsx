import { useEffect, useState, type FormEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useT, type MessageKey } from "../i18n";
import { authConfig, login } from "../lib/api";
import type { AuthConfigResponse, Me } from "../lib/types";
import { ErrorNotice } from "../components/ui";

function ssoErrorKey(reason: string): MessageKey {
  switch (reason) {
    case "state_invalid":
      return "login.ssoErrors.stateInvalid";
    case "state_expired":
      return "login.ssoErrors.stateExpired";
    case "provider_error":
      return "login.ssoErrors.providerError";
    case "exchange_failed":
      return "login.ssoErrors.exchangeFailed";
    case "not_linked":
      return "login.ssoErrors.notLinked";
    case "unavailable":
      return "login.ssoErrors.unavailable";
    default:
      return "login.ssoErrors.internal";
  }
}

export function LoginPage({ onLogin }: { onLogin: (me: Me) => void }) {
  const { t, lang, setLang } = useT();
  const navigate = useNavigate();
  const [loginId, setLoginId] = useState("alice");
  const [password, setPassword] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [oidc, setOidc] = useState<AuthConfigResponse["oidc"]>(null);
  const [ssoError] = useState(() => new URLSearchParams(window.location.search).get("sso_error"));

  useEffect(() => {
    let cancelled = false;
    authConfig()
      .then((config) => {
        if (!cancelled) setOidc(config.oidc ?? null);
      })
      .catch(() => {
        if (!cancelled) setOidc(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      const response = await login(loginId, password);
      onLogin(response);
      navigate("/", { replace: true });
    } catch (cause) {
      setError(cause);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="auth-screen">
      <form className="auth-card" onSubmit={submit}>
        <div className="auth-brand">
          <span className="brand-mark" aria-hidden="true" />
          <div>
            <div className="brand-name auth-brand-name">{t("app.name")}</div>
            <div className="muted auth-brand-tagline">{t("app.tagline")}</div>
          </div>
          <div className="auth-lang-wrap">
            <select
              className="select auth-lang"
              value={lang}
              onChange={(event) => setLang(event.target.value)}
              aria-label={t("lang.label")}
            >
              <option value="ja">{t("lang.ja")}</option>
              <option value="en">{t("lang.en")}</option>
              <option value="zh-CN">{t("lang.zh")}</option>
            </select>
          </div>
        </div>

        <h1 className="page-title auth-title">{t("login.title")}</h1>
        <p className="muted auth-subtitle">{t("login.subtitle")}</p>

        <div className="stack">
          {ssoError ? (
            <div className="error-box" role="alert">
              {t(ssoErrorKey(ssoError))}
            </div>
          ) : null}
          {oidc ? (
            <>
              <a
                className="btn"
                href={`${oidc.start_url}?return_to=${encodeURIComponent("/")}`}
              >
                {t("login.sso", { provider: oidc.provider_label })}
              </a>
              <p className="faint auth-hint">{t("login.or")}</p>
            </>
          ) : null}
          <div className="field">
            <label className="label" htmlFor="login-id">
              {t("login.loginId")}
            </label>
            <input
              id="login-id"
              className="input"
              autoComplete="username"
              value={loginId}
              onChange={(event) => setLoginId(event.target.value)}
            />
          </div>
          <div className="field">
            <label className="label" htmlFor="password">
              {t("login.password")}
            </label>
            <input
              id="password"
              className="input"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
          </div>
          {error ? <ErrorNotice error={error} /> : null}
          <button className="btn btn-primary" type="submit" disabled={submitting}>
            {submitting ? t("login.submitting") : t("login.submit")}
          </button>
          <p className="faint auth-hint">{t("login.hint")}</p>
        </div>
      </form>
    </div>
  );
}
