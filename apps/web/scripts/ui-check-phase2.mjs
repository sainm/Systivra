import path from "node:path";
import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const chrome = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15173";
const password = process.env.SEED_PASSWORD ?? "SystivraDemo1!";

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

  const row = page.locator("tbody tr", { hasText: "Phase2 Eval XLSX" }).first();
  await row.waitFor({ timeout: 30_000 });
  await row.locator("a.row-title").click();
  await page.waitForURL(/\/documents\/[^/]+$/);
  await page.getByRole("link", { name: "チャンク探索" }).click();
  await page.waitForURL(/\/explorer$/);
  await page.locator(".chunk-card").first().waitFor({ timeout: 30_000 });
  const chunkCount = await page.locator(".chunk-card").count();
  assert.ok(chunkCount > 0, "explorer must show chunks");

  const treeItems = page.locator(".tree-item");
  await treeItems.first().waitFor();
  const rootCount = await treeItems.count();
  assert.ok(rootCount > 0, "tree must show sheets");
  await treeItems.first().click();
  await page.waitForTimeout(300);
  const regionCount = await treeItems.count();
  assert.ok(regionCount > 0, "sheet must expand to regions");
  await treeItems.first().click();
  await page.waitForTimeout(300);
  const detailText = await page.locator(".chunk-text").first().innerText();
  assert.ok(detailText.length > 0, "chunk detail must render");
  const explorerHref = page.url();
  await page.screenshot({ path: "/tmp/systivra-phase2-explorer-1440.png", fullPage: false });

  await page.getByRole("link", { name: "索引管理" }).click();
  await page.waitForURL(/\/indexes$/);
  await page.locator("table.data tbody tr").first().waitFor({ timeout: 30_000 });
  const buildRows = await page.locator("table.data tbody tr").count();
  assert.ok(buildRows >= 1, "index builds must be listed");
  assert.ok(
    (await page.locator("table.data tbody tr", { hasText: "READY" }).count()) >= 1,
    "at least one READY build expected",
  );
  await page.screenshot({ path: "/tmp/systivra-phase2-indexes-1440.png", fullPage: false });

  await page.getByRole("link", { name: "ナレッジ検索" }).click();
  await page.waitForURL(/\/search$/);
  await page.fill(".search-query", "住所 全角20文字以内");
  await page.getByRole("checkbox", { name: "検査" }).check();
  await page.getByRole("button", { name: "検索" }).click();
  await page.locator(".result-inspect").first().waitFor({ timeout: 30_000 });
  const inspectChips = await page.locator(".result-inspect .chip").count();
  assert.ok(inspectChips >= 2, "inspection must show score details");
  await page.screenshot({ path: "/tmp/systivra-phase2-inspect-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.goto(explorerHref, { waitUntil: "networkidle" });
  await page.locator(".chunk-card").first().waitFor({ timeout: 30_000 });
  await page.waitForTimeout(500);
  const columns = await page.locator(".explorer-layout").evaluate((element) => {
    return getComputedStyle(element).gridTemplateColumns.split(" ").length;
  });
  assert.equal(columns, 1, "explorer must stack on narrow viewport");
  await page.screenshot({ path: "/tmp/systivra-phase2-explorer-607.png", fullPage: false });

  console.log(
    JSON.stringify({
      chunkCount,
      rootCount,
      regionCount,
      buildRows,
      inspectChips,
      errors,
    }),
  );
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
