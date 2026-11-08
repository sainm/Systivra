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
  await page.goto(`${base}/projects/${projectId}/tests`, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "テスト工学" }).waitFor();
  await page.locator(".test-case-row").first().waitFor({ timeout: 30_000 });

  const metrics = {
    rules: await page.locator(".test-rule-row").count(),
    cases: await page.locator(".test-case-row").count(),
    runs: await page.locator(".test-run-row").count(),
    executions: await page.locator(".test-execution-row").count(),
    defects: await page.locator(".test-defect-row").count(),
    coverageCards: await page.locator(".test-coverage-card").count(),
  };
  assert.ok(metrics.rules >= 1, "validation rules must be visible");
  assert.ok(metrics.cases >= 1, "draft/approved cases must be visible");
  assert.equal(metrics.coverageCards, 5, "five coverage metrics must render");
  assert.ok(
    (await page.getByText("runner_external_process").count()) >= 1,
    "external runner boundary must be explicit",
  );
  assert.ok(
    (await page.getByText("REQUIREMENT_INFERRED_FROM_RULE").count()) >= 1,
    "inferred requirement must be labelled",
  );

  await page.getByRole("button", { name: "ルールを抽出" }).click();
  await page.getByText("ルールを抽出しました").waitFor({ timeout: 30_000 });
  await page.getByRole("button", { name: "被覆を再計算" }).click();
  await page.getByText("被覆を再計算しました").waitFor({ timeout: 30_000 });

  await page.locator("details summary").filter({ hasText: "実行 run" }).click();
  await page.locator("details summary").filter({ hasText: "実行結果" }).click();
  const artifactUi = {
    cleanupButtons: await page.locator(".test-run-row button").count(),
    expiredChips: await page.locator(".test-execution-row td:last-child .chip", { hasText: "EXPIRED" }).count(),
    truncatedChips: await page.locator(".test-execution-row td:last-child .chip", { hasText: "TRUNCATED" }).count(),
  };
  assert.ok(artifactUi.cleanupButtons >= 1, "run artifact cleanup button must render");
  assert.ok(artifactUi.expiredChips >= 1, "EXPIRED artifacts must be labelled");
  assert.ok(artifactUi.truncatedChips >= 1, "TRUNCATED artifacts must be labelled");

  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: "/tmp/systivra-phase6-tests-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.locator(".test-coverage-grid").first().waitFor();
  const columns = await page.locator(".test-coverage-grid").first().evaluate((element) =>
    getComputedStyle(element).gridTemplateColumns.split(" ").length,
  );
  assert.equal(columns, 1, "coverage grid must stack on a narrow viewport");
  await page.screenshot({ path: "/tmp/systivra-phase6-tests-607.png", fullPage: false });

  console.log(JSON.stringify({ ...metrics, artifactUi, columns, errors }));
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
