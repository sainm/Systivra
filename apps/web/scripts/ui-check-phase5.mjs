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
  await page.goto(`${base}/projects/${projectId}/impact`, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "変更影響をたずねる" }).waitFor();
  await page.getByRole("button", { name: "影響 Draft を作成" }).waitFor();

  await page.getByRole("button", { name: "影響 Draft を作成" }).click();
  await page.locator(".impact-item").first().waitFor({ timeout: 30_000 });
  const entityItems = await page.locator(".impact-item").count();
  const entityEvidence = await page.locator(".impact-evidence").count();
  assert.ok(entityItems >= 1, "entity impact must render evidence-backed items");
  assert.ok(entityEvidence >= 1, "entity impact must render evidence");
  assert.ok(
    (await page.getByText("tests_not_available").count()) >= 1,
    "test degradation must be visible",
  );
  await page.getByText(/NOT_MODIFIED/).first().waitFor();

  await page.getByRole("button", { name: "コード版差分から" }).click();
  await page.getByRole("button", { name: "影響 Draft を作成" }).click();
  await page.getByText("影響する設計書", { exact: true }).waitFor({ timeout: 30_000 });
  await page.getByText("変更行", { exact: true }).waitFor({ timeout: 30_000 });
  const changedHunks = await page.getByText("変更行", { exact: true }).count();
  const pdfLinks = await page.locator('a[href$="/report.pdf"]').count();
  assert.ok(
    pdfLinks >= 1,
    "PDF report download must be visible",
  );
  const diffItems = await page.locator(".impact-item").count();
  const diffEvidence = await page.locator(".impact-evidence").count();
  assert.ok(diffItems >= 1, "diff assessment must add impact items");
  assert.equal(
    await page.locator(".impact-item").evaluateAll((items) =>
      items.filter((item) => !item.querySelector(".impact-evidence")).length,
    ),
    0,
    "every main impact item must have evidence",
  );
  assert.ok(diffEvidence >= diffItems, "each item needs at least one evidence block");
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: "/tmp/systivra-phase5-impact-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.locator(".impact-seed-grid").first().waitFor();
  const seedColumns = await page.locator(".impact-seed-grid").first().evaluate((element) => {
    return getComputedStyle(element).gridTemplateColumns.split(" ").length;
  });
  assert.equal(seedColumns, 1, "impact seed layout must stack on narrow viewport");
  await page.screenshot({ path: "/tmp/systivra-phase5-impact-607.png", fullPage: false });

  console.log(
    JSON.stringify({
      entityItems,
      entityEvidence,
      diffItems,
      diffEvidence,
      changedHunks,
      pdfLinks,
      seedColumns,
      errors,
    }),
  );
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
