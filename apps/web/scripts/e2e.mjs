import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const fixture = path.join(root, "tests/fixtures/walking_skeleton_100p.pdf");
const chrome = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://127.0.0.1:15173";
const password = process.env.SEED_PASSWORD ?? "SystivraDemo1!";

const browser = await chromium.launch({ executablePath: chrome, headless: true });
const context = await browser.newContext({
  viewport: { width: 1440, height: 960 },
  colorScheme: process.env.WEB_THEME === "light" ? "light" : "dark",
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
  await page.getByText("顧客管理システム").first().waitFor();

  await page.getByText("顧客管理システム").first().click();
  await page.waitForURL(/\/documents$/);
  await page.waitForTimeout(1500);

  const rows = page.locator("tbody tr");
  if ((await rows.count()) > 0) {
    await page.evaluate(async () => {
      const response = await fetch("/api/v1/auth/me", { credentials: "include" });
      const me = await response.json();
      const projectId = me.memberships[0].project_id;
      const list = await fetch(`/api/v1/projects/${projectId}/documents?limit=100`, {
        credentials: "include",
      }).then((r) => r.json());
      for (const item of list.items) {
        await fetch(`/api/v1/documents/${item.id}`, {
          method: "DELETE",
          credentials: "include",
        });
      }
    });
    await page.reload({ waitUntil: "networkidle" });
  }

  await page.setInputFiles('input[type="file"]', fixture);
  await page.getByText("walking_skeleton_100p").first().waitFor({ timeout: 120_000 });
  await page.locator("tbody tr", { hasText: "walking_skeleton_100p" }).getByText("完了").waitFor({
    timeout: 120_000,
  });

  await page.getByRole("link", { name: "ナレッジ検索" }).click();
  await page.waitForURL(/\/search$/);
  await page.fill('input[placeholder*="住所"]', "住所チェックの桁数と文字種");
  await page.getByRole("button", { name: "検索" }).click();
  await page.getByText("住所チェックの桁数と文字種").first().waitFor({ timeout: 30_000 });
  const firstResult = page.locator("article.result-card").first();
  await firstResult.getByRole("button", { name: "根拠を表示" }).click();
  await page.locator("aside.drawer").getByText("PDF_PAGE_TEXT").waitFor().catch(() => {});
  await page.locator("aside.drawer").getByText("住所チェックの桁数と文字種").first().waitFor();
  await page.screenshot({ path: "/tmp/systivra-search-1440.png", fullPage: true });

  await page.locator("aside.drawer").getByRole("link", { name: "原文を開く" }).click();
  await page.waitForURL(/\/documents\/[0-9a-f-]+\?/);
  await page.locator("canvas").waitFor({ timeout: 30_000 });
  await page.locator(".pdf-highlight").first().waitFor({ timeout: 30_000 });
  const highlightCount = await page.locator(".pdf-highlight").count();
  await page.screenshot({ path: "/tmp/systivra-document-1440.png", fullPage: true });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.locator(".pdf-highlight").first().waitFor({ timeout: 30_000 });
  await page.screenshot({ path: "/tmp/systivra-document-607.png", fullPage: true });
  await page.getByRole("link", { name: "ナレッジ検索" }).click();
  await page.waitForURL(/\/search$/);
  await page.fill('input[placeholder*="住所"]', "住所チェックの桁数と文字種");
  await page.getByRole("button", { name: "検索" }).click();
  await page.getByText("住所チェックの桁数と文字種").first().waitFor({ timeout: 30_000 });
  await page.screenshot({ path: "/tmp/systivra-search-607.png", fullPage: true });

  console.log(JSON.stringify({ highlightCount, errors }, null, 2));
  if (highlightCount < 1 || errors.length > 0) process.exitCode = 1;
} catch (error) {
  console.error(error);
  await page.screenshot({ path: "/tmp/systivra-e2e-failure.png", fullPage: true });
  console.error(JSON.stringify({ errors }, null, 2));
  process.exitCode = 1;
} finally {
  await browser.close();
}
