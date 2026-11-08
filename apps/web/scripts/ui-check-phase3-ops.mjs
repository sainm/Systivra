import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const chrome = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15200";
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
  await page.goto(`${base}/projects/${projectId}/graph`, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "グラフ", exact: true }).waitFor();
  await page.getByText("未消費イベント", { exact: false }).first().waitFor({ timeout: 30_000 });
  await page.getByText("最終再構築", { exact: false }).first().waitFor();

  const chips = await page.locator(".page-head .chip").count();
  assert.ok(chips >= 6, `graph stats chips must render: ${chips}`);
  const body = await page.locator("body").innerText();
  assert.ok(body.includes("SWITCHED") || body.includes("ABORTED"), "last rebuild status must be visible");
  assert.ok(!body.includes("undefined"), "no undefined in stats");

  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: "/tmp/systivra-phase3-ops-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "グラフ", exact: true }).waitFor();
  await page.screenshot({ path: "/tmp/systivra-phase3-ops-607.png", fullPage: false });

  console.log(JSON.stringify({ chips, errors }));
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}