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
  await page.getByRole("link", { name: "グラフ" }).click();
  await page.waitForURL(/\/graph$/);
  await page.locator(".graph-node").first().waitFor({ timeout: 30_000 });
  const entityCount = await page.locator(".tree-item").count();
  const nodeCount = await page.locator(".graph-node").count();
  assert.ok(entityCount >= 1, "entities must be listed");
  assert.ok(nodeCount >= 1, "subgraph must render nodes");
  await page.locator(".tree-item").first().click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: "/tmp/systivra-phase3-graph-1440.png", fullPage: false });

  await page.getByRole("link", { name: "Ontology を開く" }).click();
  await page.waitForURL(/\/ontology$/);
  await page.locator("table.data").first().waitFor({ timeout: 30_000 });
  const typeRows = await page.locator("table.data").first().locator("tbody tr").count();
  assert.ok(typeRows >= 50, `entity types expected, got ${typeRows}`);
  const publishVersion = `v1-ui-${Date.now()}`;
  await page.fill('input[placeholder*="v1.1"]', publishVersion);
  await page.getByRole("button", { name: "公開" }).click();
  await page.locator(".chip", { hasText: publishVersion }).first().waitFor({ timeout: 30_000 });
  await page.screenshot({ path: "/tmp/systivra-phase3-ontology-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.goto(`${base}/projects/22222222-2222-4222-8222-222222222222/graph`, {
    waitUntil: "networkidle",
  });
  await page.locator(".graph-node").first().waitFor({ timeout: 30_000 });
  const columns = await page.locator(".graph-layout").evaluate((element) => {
    return getComputedStyle(element).gridTemplateColumns.split(" ").length;
  });
  assert.equal(columns, 1, "graph layout must stack on narrow viewport");
  await page.screenshot({ path: "/tmp/systivra-phase3-graph-607.png", fullPage: false });

  console.log(JSON.stringify({ entityCount, nodeCount, typeRows, publishVersion, errors }));
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
