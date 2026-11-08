import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { chromium } from "playwright-core";

const chrome = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15173";
const providerLabel = process.env.SSO_PROVIDER_LABEL ?? "ローカル IdP";
const database = process.env.PGDATABASE ?? "systivra";
const issuer = process.env.SSO_ISSUER ?? "http://localhost:11540";
const subject = process.env.SSO_SUBJECT ?? "sso-eval-subject";
const userId = process.env.SSO_EVAL_USER_ID ?? "44444444-4444-4444-8444-444444444444";

function psql(sql) {
  execFileSync("psql", ["-v", "ON_ERROR_STOP=1", "-Atc", sql], {
    env: { ...process.env, PGDATABASE: database },
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function seedFixture() {
  psql(`
    INSERT INTO core.app_user (id, login_id, display_name)
    VALUES ('${userId}', 'sso-eval', 'SSO Eval')
    ON CONFLICT (id) DO NOTHING;
    INSERT INTO core.project_member (project_id, user_id, role)
    SELECT id, '${userId}', 'OWNER' FROM core.project ORDER BY code LIMIT 1
    ON CONFLICT (project_id, user_id) DO NOTHING;
    INSERT INTO core.external_identity (user_id, provider, subject)
    VALUES ('${userId}', '${issuer}', '${subject}')
    ON CONFLICT (provider, subject) DO UPDATE SET user_id = EXCLUDED.user_id;
  `);
}

function cleanupFixture() {
  psql(`
    DELETE FROM core.external_identity
     WHERE provider = '${issuer}' AND subject = '${subject}';
    DELETE FROM core.project_member
     WHERE user_id = (
       SELECT id FROM core.app_user WHERE id = '${userId}' AND login_id = 'sso-eval'
     );
    DELETE FROM core.app_user WHERE id = '${userId}' AND login_id = 'sso-eval';
  `);
}

seedFixture();

const browser = await chromium.launch({ executablePath: chrome, headless: true });
const errors = [];

function watch(page) {
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("response", (response) => {
    if (response.status() < 400) return;
    const url = response.url();
    if (url.endsWith("/favicon.ico")) return;
    if (response.status() === 401 && url.includes("/api/v1/auth/me")) return;
    errors.push(`http: ${response.status()} ${url}`);
  });
}

try {
  const errorContext = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    colorScheme: "dark",
  });
  const errorPage = await errorContext.newPage();
  watch(errorPage);
  await errorPage.goto(`${base}/login?sso_error=not_linked`, { waitUntil: "networkidle" });
  await errorPage
    .getByText("この SSO アカウントは製品に紐付いていません")
    .waitFor({ timeout: 15_000 });
  await errorPage.screenshot({ path: "/tmp/systivra-phase8-sso-error-1440.png", fullPage: false });
  await errorContext.close();

  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    colorScheme: "dark",
  });
  const page = await context.newPage();
  watch(page);
  await page.goto(`${base}/login`, { waitUntil: "networkidle" });
  const sso = page.getByRole("link", { name: `${providerLabel} でサインイン` });
  await sso.waitFor({ timeout: 15_000 });
  assert.ok(await page.locator("#password").isVisible(), "password login must stay visible");
  await page.screenshot({ path: "/tmp/systivra-phase8-sso-login-1440.png", fullPage: false });

  await sso.click();
  await page.getByRole("heading", { name: "プロジェクト" }).waitFor({ timeout: 30_000 });
  const landed = new URL(page.url()).pathname;
  assert.equal(landed, "/", "SSO login must return to the app root");
  await page.screenshot({ path: "/tmp/systivra-phase8-sso-after-1440.png", fullPage: false });

  await page.goto(`${base}/security`, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "セキュリティ" }).waitFor();
  await page.locator("table.data tbody tr").first().waitFor({ timeout: 15_000 });
  const sessionRows = await page.locator("table.data tbody tr").count();
  assert.ok(sessionRows >= 1, "current session must be listed");
  await page.getByRole("button", { name: "他の端末をサインアウト" }).click();
  await page.getByText(/件失効しました/).waitFor({ timeout: 15_000 });
  await page.screenshot({ path: "/tmp/systivra-phase8-security-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "セキュリティ" }).waitFor();
  const securityOverflow = await page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
  );
  assert.equal(securityOverflow, false, "security page must not overflow at 607px");
  await page.screenshot({ path: "/tmp/systivra-phase8-security-607.png", fullPage: false });

  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(`${base}/users`, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "ユーザー" }).waitFor();
  await page.locator("table.data tbody tr").first().waitFor({ timeout: 15_000 });
  const memberRows = await page.locator("table.data tbody tr").count();
  assert.ok(memberRows >= 1, "project members must be listed");
  await page.screenshot({ path: "/tmp/systivra-phase8-users-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "ユーザー" }).waitFor();
  const usersOverflow = await page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
  );
  assert.equal(usersOverflow, false, "users page must not overflow at 607px");
  await page.screenshot({ path: "/tmp/systivra-phase8-users-607.png", fullPage: false });

  const narrowContext = await browser.newContext({
    viewport: { width: 607, height: 900 },
    colorScheme: "dark",
  });
  const narrowPage = await narrowContext.newPage();
  watch(narrowPage);
  await narrowPage.goto(`${base}/login`, { waitUntil: "networkidle" });
  await narrowPage
    .getByRole("link", { name: `${providerLabel} でサインイン` })
    .waitFor({ timeout: 15_000 });
  const overflow = await narrowPage.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
  );
  assert.equal(overflow, false, "login page must not overflow at 607px");
  await narrowPage.screenshot({ path: "/tmp/systivra-phase8-sso-login-607.png", fullPage: false });
  await narrowContext.close();

  console.log(
    JSON.stringify({
      providerLabel,
      landed,
      overflow,
      securityOverflow,
      usersOverflow,
      sessionRows,
      memberRows,
      errorNotice: true,
      passwordKept: true,
      errors,
    }),
  );
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
  cleanupFixture();
}
