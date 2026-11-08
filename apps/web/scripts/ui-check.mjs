import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const fixture = path.join(root, "tests/fixtures/walking_skeleton_100p.pdf");
const chrome = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15173";
const password = process.env.SEED_PASSWORD ?? "SystivraDemo1!";

const browser = await chromium.launch({ executablePath: chrome, headless: true });
const context = await browser.newContext({ viewport: { width: 1440, height: 960 }, colorScheme: "dark" });
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

  const projectLink = page.getByText("顧客管理システム").first();
  await projectLink.click();
  await page.waitForURL(/\/documents$/);
  await page.waitForTimeout(1500);
  if ((await page.locator("tbody tr").count()) === 0) {
    await page.setInputFiles('input[type="file"]', fixture);
    await page.locator("tbody tr", { hasText: "walking_skeleton_100p" }).getByText("完了").waitFor({
      timeout: 120_000,
    });
  }

  await page.getByRole("link", { name: "ナレッジ検索" }).click();
  await page.waitForURL(/\/search$/);

  const searchRequests = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/knowledge/search")) {
      searchRequests.push(request.url());
    }
  });

  await page.fill('input[placeholder*="住所"]', "住所チェックの桁数と文字種");
  searchRequests.length = 0;
  await page.getByRole("button", { name: "検索" }).click();
  await page.getByText("住所チェックの桁数と文字種").first().waitFor({ timeout: 30_000 });
  await page.waitForTimeout(300);
  const searchRequestCount = searchRequests.length;
  assert.equal(searchRequestCount, 1, `search requests must be 1, got ${searchRequestCount}`);

  const rawText = await page.locator(".results-pane").innerText();
  assert.ok(!/SRH-/.test(rawText), "search_id must not be visible");
  assert.ok(!/sha256:/.test(rawText), "quote_hash must not be visible");
  assert.ok(!/スコア/.test(rawText), "raw score must not be visible");

  const cards = page.locator("article.result-card");
  const cardCount = await cards.count();
  assert.ok(cardCount > 0, "expected result cards");
  assert.equal(
    await cards.locator(".btn-primary").count(),
    cardCount,
    "each card must have exactly one primary action",
  );
  assert.equal(
    await cards.locator(".btn").count() + await cards.locator("a.btn").count(),
    cardCount,
    "each card must expose a single action",
  );

  const widthClosed = (await page.locator(".results-pane").boundingBox())?.width ?? 0;
  assert.equal(await page.locator(".search-drawer").count(), 0, "drawer must not render before selection");

  await cards.first().getByRole("button", { name: "根拠を表示" }).click();
  await page.locator(".search-drawer").waitFor();
  await page.locator(".search-drawer .quote").waitFor({ timeout: 30_000 });
  const drawerText = await page.locator(".search-drawer").innerText();
  assert.ok(!/sha256:/.test(drawerText), "quote_hash must not be visible in the drawer");
  const widthOpen = (await page.locator(".results-pane").boundingBox())?.width ?? 0;
  assert.ok(widthClosed > widthOpen, `results should be wider without drawer (${widthClosed} > ${widthOpen})`);
  await page.screenshot({ path: "/tmp/systivra-search-open-1440.png" });

  await page.keyboard.press("Escape");
  await page.locator(".search-drawer").waitFor({ state: "detached" });
  const widthAfterClose = (await page.locator(".results-pane").boundingBox())?.width ?? 0;
  assert.equal(widthAfterClose, widthClosed, "results pane should return to full width");
  await page.screenshot({ path: "/tmp/systivra-search-closed-1440.png" });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.getByText("住所チェックの桁数と文字種").first().waitFor({ timeout: 30_000 });
  await page.evaluate(() => window.scrollTo(0, 240));
  const scrollBefore = await page.evaluate(() => window.scrollY);
  await page.locator("article.result-card").first().getByRole("button", { name: "根拠を表示" }).click();
  await page.locator(".search-drawer").waitFor();
  const drawerPosition = await page.locator(".search-drawer").evaluate((element) => getComputedStyle(element).position);
  assert.equal(drawerPosition, "fixed", "narrow drawer must be a fixed sheet");
  assert.equal(await page.locator(".drawer-backdrop").evaluate((element) => getComputedStyle(element).display), "block");
  await page.screenshot({ path: "/tmp/systivra-search-drawer-607.png" });
  await page.getByRole("button", { name: "閉じる" }).click();
  await page.locator(".search-drawer").waitFor({ state: "detached" });
  const scrollAfter = await page.evaluate(() => window.scrollY);
  assert.ok(Math.abs(scrollAfter - scrollBefore) <= 2, `scroll should be preserved (${scrollBefore} -> ${scrollAfter})`);

  await page.getByRole("link", { name: "設計書" }).click();
  await page.waitForURL(/\/documents$/);
  await page.locator("tbody tr .row-title").first().click();
  await page.waitForURL(/\/documents\/[0-9a-f-]+/);
  await page.getByRole("button", { name: "論理削除" }).click();
  await page.locator(".modal").waitFor();
  await page.keyboard.press("Escape");
  await page.locator(".modal").waitFor({ state: "detached" });
  assert.ok(
    (await page.evaluate(() => document.activeElement?.textContent ?? "")).includes("論理削除"),
    "focus should return to the trigger after closing the modal",
  );

  await page.getByRole("button", { name: "論理削除" }).click();
  await page.locator(".modal").waitFor();
  for (let i = 0; i < 8; i += 1) {
    await page.keyboard.press("Tab");
    assert.ok(
      await page.evaluate(() => Boolean(document.querySelector(".modal")?.contains(document.activeElement))),
      "focus must stay inside the modal",
    );
  }
  await page.keyboard.press("Escape");
  await page.locator(".modal").waitFor({ state: "detached" });

  console.log(
    JSON.stringify(
      { searchRequests: searchRequestCount, cardCount, widthClosed, widthOpen, errors },
      null,
      2,
    ),
  );
  assert.equal(errors.length, 0, errors.join("\n"));
} catch (error) {
  console.error(error);
  await page.screenshot({ path: "/tmp/systivra-ui-check-failure.png", fullPage: true });
  console.error(JSON.stringify({ errors }, null, 2));
  process.exitCode = 1;
} finally {
  await browser.close();
}
