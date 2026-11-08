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
  await page.goto(`${base}/projects/${projectId}/inquiry`, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "変更影響をたずねる" }).waitFor();

  await page.getByRole("button", { name: "影響をたずねる" }).click();
  await page.locator(".inquiry-headline").waitFor({ timeout: 30_000 });
  const counts = {
    items: await page.locator(".inquiry-item").count(),
    drafts: await page.locator(".inquiry-draft").count(),
    counts: await page.locator(".inquiry-counts .metric-card").count(),
  };
  assert.ok(counts.items >= 1, "code items must render");
  assert.ok(counts.drafts >= 1, "draft cases must render");
  assert.equal(counts.counts, 5, "five count cards must render");
  const headline = await page.locator(".inquiry-headline").innerText();
  assert.ok(headline.includes("コード"), headline);
  await page.getByText(/code: NOT_MODIFIED/).first().waitFor();
  await page.getByText(/tests: NOT_EXECUTED/).first().waitFor();
  await page.getByText("DRAFT", { exact: true }).first().waitFor();
  await page.getByText("summary", { exact: true }).waitFor({ timeout: 10_000 });
  assert.ok(
    (await page.getByText("summary").count()) >= 1,
    "event order must be visible",
  );
  const pdfLinks = await page.locator('a[href$="/report.pdf"]').count();
  assert.ok(
    pdfLinks >= 1,
    "Inquiry PDF download must be visible",
  );

  await page.getByRole("button", { name: "テスト計画に入れる" }).click();
  await page.getByText(/計画を作成/).waitFor({ timeout: 30_000 });
  await page.getByText(/実行はまだ行いません/).waitFor();

  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: "/tmp/systivra-q01-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("button", { name: "影響をたずねる" }).click();
  await page.locator(".inquiry-counts").first().waitFor({ timeout: 30_000 });
  const columns = await page.locator(".inquiry-counts").first().evaluate((element) =>
    getComputedStyle(element).gridTemplateColumns.split(" ").length,
  );
  assert.equal(columns, 1, "count cards must stack on a narrow viewport");
  await page.screenshot({ path: "/tmp/systivra-q01-607.png", fullPage: false });

  console.log(
    JSON.stringify({
      ...counts,
      headline,
      columns,
      pdfLinks,
      errors,
    }),
  );
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
