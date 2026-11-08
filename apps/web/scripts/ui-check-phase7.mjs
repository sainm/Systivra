import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const chrome = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15173";
const password = process.env.SEED_PASSWORD ?? "SystivraDemo1!";
const projectId = process.env.PROJECT_ID ?? "22222222-2222-4222-8222-222222222222";

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
  await page.goto(`${base}/projects/${projectId}/ai`, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "AI / Agent 基盤" }).waitFor();
  await page.locator(".ai-agent-row").first().waitFor({ timeout: 30_000 });

  const counts = {
    agents: await page.locator(".ai-agent-row").count(),
    mcp: await page.locator(".ai-mcp-row").count(),
    workflows: await page.locator(".ai-workflow-row").count(),
    workflowRuns: await page.locator(".ai-workflow-run-row").count(),
    approvals: await page.locator(".ai-approval-row").count(),
  };
  assert.ok(counts.agents >= 1, "agents must render");
  assert.ok(counts.mcp >= 1, "mcp servers must render");
  assert.ok(counts.workflows >= 1, "workflows must render");
  assert.ok(counts.workflowRuns >= 1, "workflow runs must render");
  assert.ok(counts.approvals >= 1, "approvals must render");
  assert.ok(
    (await page.getByText("AI 未設定").count()) >= 1,
    "AI unavailable must be explicit",
  );
  assert.ok(
    (await page.getByText("ai_unavailable").count()) >= 1,
    "degradation reason must be visible",
  );
  assert.ok((await page.getByText("SUCCEEDED").count()) >= 1, "workflow must have succeeded");
  assert.ok((await page.getByText("APPROVED").count()) >= 1, "approval must be recorded");
  // 承認の起票元（Workflow / MCP）と「使用済み」が出ていること。
  const approvalCells = await page
    .locator(".ai-approval-row")
    .first()
    .locator("td")
    .count();
  assert.equal(approvalCells, 5, "approval row must show the source column");
  assert.ok(
    (await page.getByText("使用済み").count()) >= 1,
    "consumed approval must be visible",
  );
  assert.ok(
    (await page.locator(".chip").filter({ hasText: "12" }).count()) >= 1,
    "12 MCP tools must be shown",
  );

  await page.getByRole("button", { name: "Agent を登録" }).click();
  await page.getByText("Agent を登録しました").waitFor({ timeout: 30_000 });
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: "/tmp/systivra-phase7-ai-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.locator(".ai-form-grid").first().waitFor();
  const columns = await page.locator(".ai-form-grid").first().evaluate((element) =>
    getComputedStyle(element).gridTemplateColumns.split(" ").length,
  );
  assert.equal(columns, 1, "agent form must stack on a narrow viewport");
  await page.screenshot({ path: "/tmp/systivra-phase7-ai-607.png", fullPage: false });

  console.log(JSON.stringify({ ...counts, columns, errors }));
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
