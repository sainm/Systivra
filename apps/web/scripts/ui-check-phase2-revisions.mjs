import assert from "node:assert/strict";
import fs from "node:fs";
import { chromium } from "playwright-core";

const chrome = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15200";
const password = process.env.SEED_PASSWORD ?? "SystivraDemo1!";
const report = JSON.parse(
  fs.readFileSync(new URL("../../../data/phase2-revision/report.json", import.meta.url), "utf8"),
);
const projectId = report.document ? "22222222-2222-4222-8222-222222222222" : "";
const documentId = report.document.id;

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
  await page.goto(`${base}/projects/${projectId}/documents/${documentId}`, {
    waitUntil: "networkidle",
  });
  await page.locator(".revision-row").first().waitFor({ timeout: 30_000 });

  const rows = await page.locator(".revision-row").count();
  assert.equal(rows, 2, `revision rows: ${rows}`);
  const body = await page.locator("body").innerText();
  assert.ok(body.includes("CURRENT"), "current badge must render");
  assert.ok(body.includes("SUPERSEDED"), "superseded badge must render");
  const searchHref = await page.locator(".revision-row a", { hasText: "この版で検索" }).first().getAttribute("href");
  assert.ok(searchHref && searchHref.includes("revision="), searchHref || "");

  await page.getByRole("button", { name: "この版を最新に" }).first().click();
  await page.waitForFunction(
    () => {
      const rows = [...document.querySelectorAll(".revision-row")];
      return rows.length === 2 && rows.filter((row) => row.textContent.includes("CURRENT")).length === 1;
    },
    null,
    { timeout: 30_000 },
  );
  const after = await page.locator(".revision-row").count();
  assert.equal(after, 2);
  const currentRow = await page.locator(".revision-row", { hasText: "CURRENT" }).count();
  assert.equal(currentRow, 1, "exactly one CURRENT row after activation");

  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: "/tmp/systivra-phase2-revisions-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.locator(".revision-row").first().waitFor();
  await page.screenshot({ path: "/tmp/systivra-phase2-revisions-607.png", fullPage: false });

  console.log(JSON.stringify({ rows, currentRow, errors }));
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}