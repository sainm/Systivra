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
  await page.getByRole("link", { name: "編集" }).first().click();
  await page.getByRole("heading", { name: "Workflow エディタ" }).waitFor();
  await page.locator(".wf-node").first().waitFor({ timeout: 30_000 });

  const before = await page.locator(".wf-node").count();
  assert.ok(before >= 3, `expected at least 3 nodes, got ${before}`);

  const target = page.locator(".wf-node").nth(1);
  const start = await target.boundingBox();
  assert.ok(start, "node must have a box");
  await page.mouse.move(start.x + 40, start.y + 20);
  await page.mouse.down();
  await page.mouse.move(start.x + 160, start.y + 120, { steps: 8 });
  await page.mouse.up();
  const moved = await target.boundingBox();
  assert.ok(moved && moved.x > start.x + 40, `node did not move: ${start.x} -> ${moved?.x}`);

  await page.getByRole("button", { name: "+ 承認" }).click();
  await page.waitForFunction(
    (count) => document.querySelectorAll(".wf-node").length === count + 1,
    before,
  );
  const after = await page.locator(".wf-node").count();

  await page.getByRole("button", { name: "新しい版として保存" }).click();
  await page.getByText("保存しました").waitFor({ timeout: 30_000 });
  const versionLabel = await page.locator(".page-head .chip").first().innerText();

  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: "/tmp/systivra-workflow-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.locator(".wf-layout").waitFor({ timeout: 30_000 });
  const columns = await page.locator(".wf-layout").first().evaluate((element) =>
    getComputedStyle(element).gridTemplateColumns.split(" ").length,
  );
  assert.equal(columns, 1, "editor must stack on a narrow viewport");
  await page.screenshot({ path: "/tmp/systivra-workflow-607.png", fullPage: false });

  console.log(JSON.stringify({ before, after, moved: true, versionLabel, columns, errors }));
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
