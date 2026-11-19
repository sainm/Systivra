import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const chrome = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15173";
const password = process.env.SEED_PASSWORD ?? "SystivraDemo1!";
const docTitle = process.env.DOC_TITLE;
const question = process.env.QUESTION ?? `MOCK_SLOW_HEARTBEAT UI-CANCEL-${Date.now()} 状態を要約して`;
assert.ok(docTitle, "DOC_TITLE is required");

const browser = await chromium.launch({ executablePath: chrome, headless: true });
const context = await browser.newContext({
  viewport: { width: 1440, height: 1000 },
  colorScheme: "dark",
});
const page = await context.newPage();
const errors = [];
page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
page.on("response", (response) => {
  if (response.status() < 400) return;
  const url = response.url();
  if (url.endsWith("/favicon.ico")) return;
  if (response.status() === 401 && url.includes("/api/v1/auth/me")) return;
  errors.push(`http: ${response.status()} ${url}`);
});

try {
  await page.goto(`${base}/login`, { waitUntil: "networkidle" });
  await page.fill("#login-id", "alice");
  await page.fill("#password", password);
  await page.click('button[type="submit"]');
  await page.waitForURL(`${base}/`);
  await page.getByText("顧客管理システム").first().click();
  await page.waitForURL(/\/documents$/);

  const row = page.locator("tbody tr", { hasText: docTitle }).first();
  await row.waitFor({ timeout: 30_000 });
  await row.locator("a.row-title").click();
  await page.waitForURL(/\/documents\/[^/]+$/);

  await page.getByRole("button", { name: /^(解析を実行|再解析)$/ }).click();
  await page.locator(".job-strip").waitFor({ timeout: 30_000 });
  await page.getByRole("button", { name: "解析を取消" }).click();
  const cancelledBadge = page.locator(".job-strip .badge", { hasText: "取消済み" });
  await cancelledBadge.waitFor({ timeout: 60_000 });
  const docBadge = await cancelledBadge.innerText();
  await page.waitForTimeout(1500);
  await page.screenshot({ path: "/tmp/systivra-cancel-doc-1440.png", fullPage: true });
  await page.setViewportSize({ width: 607, height: 900 });
  await page.waitForTimeout(400);
  await page.screenshot({ path: "/tmp/systivra-cancel-doc-607.png", fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });

  await page.getByRole("link", { name: "AI", exact: true }).click();
  await page.waitForURL(/\/ai$/);
  await page.locator(".ai-agent-row", { hasText: "impact-analyst" }).waitFor({ timeout: 30_000 });
  const runsBefore = await page.locator(".ai-run-row").count();
  await page.getByLabel("依頼内容").fill(question);
  await page
    .locator(".ai-agent-row", { hasText: "impact-analyst" })
    .getByRole("button", { name: "実行" })
    .click();
  await page.waitForFunction(
    (before) => document.querySelectorAll(".ai-run-row").length > before,
    runsBefore,
    { timeout: 30_000 },
  );
  const runRow = page.locator(".ai-run-row").first();
  await runRow.locator("td").nth(1).filter({ hasText: /RUNNING|PENDING|WAITING_APPROVAL/ }).waitFor({ timeout: 30_000 });
  await runRow.getByRole("button", { name: "取消" }).click();
  await runRow.locator("td").nth(1).filter({ hasText: "CANCELLED" }).waitFor({ timeout: 30_000 });
  const runStatus = (await runRow.locator("td").nth(1).innerText()).trim();
  await page.screenshot({ path: "/tmp/systivra-cancel-ai-1440.png", fullPage: true });
  await page.setViewportSize({ width: 607, height: 900 });
  await page.waitForTimeout(400);
  await page.screenshot({ path: "/tmp/systivra-cancel-ai-607.png", fullPage: true });

  console.log(JSON.stringify({ docBadge, runStatus, errors }));
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}