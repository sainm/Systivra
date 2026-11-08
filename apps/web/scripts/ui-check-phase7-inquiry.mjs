import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const chrome =
  process.env.CHROME_BIN ??
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15174";
const password = process.env.SEED_PASSWORD ?? "SystivraDemo1!";
const projectId =
  process.env.PROJECT_ID ?? "22222222-2222-4222-8222-222222222222";
const inquiryId =
  process.env.INQUIRY_ID ?? "f0f0f0f0-f0f0-4f0f-8f0f-f0f0f0f0f0f0";

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

async function counts() {
  return page.locator(".inquiry-counts").first().innerText();
}

async function tables() {
  return page.locator(".inquiry-item").allInnerTexts();
}

try {
  await page.route(
    `**/api/v1/projects/${projectId}/change-inquiries`,
    async (route) => {
      if (route.request().method() !== "POST") {
        await route.continue();
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          inquiry_id: inquiryId,
          impact_id: null,
          job_id: null,
          status: "SUCCEEDED",
          links: {
            events: `/api/v1/projects/${projectId}/change-inquiries/${inquiryId}/events`,
            workbench: null,
          },
        }),
      });
    },
  );

  await page.goto(`${base}/login`, { waitUntil: "networkidle" });
  await page.fill("#login-id", "alice");
  await page.fill("#password", password);
  await page.click('button[type="submit"]');
  await page.waitForURL(`${base}/`);
  await page.goto(`${base}/projects/${projectId}/inquiry`, {
    waitUntil: "networkidle",
  });
  await page.getByRole("button", { name: "影響をたずねる" }).click();
  await page.locator(".inquiry-headline").waitFor({ timeout: 30_000 });

  const authoritativeCounts = await counts();
  const authoritativeTables = await tables();
  await page.getByRole("button", { name: "AI に要約させる" }).click();
  await page.locator(".inquiry-ai").waitFor({ timeout: 30_000 });

  const details = page.locator(".inquiry-ai");
  assert.equal(
    await details.getAttribute("open"),
    null,
    "AI inference must be folded by default",
  );
  assert.equal(await counts(), authoritativeCounts, "counts must not change");
  assert.deepEqual(await tables(), authoritativeTables, "main tables must not change");

  await details.locator("summary").click();
  assert.equal(await page.locator(".inquiry-hypothesis").count(), 1);
  assert.equal(
    await page.locator(".inquiry-hypothesis .chip").first().innerText(),
    "INFERRED",
  );
  assert.ok(
    (await page.locator(".inquiry-hypothesis").innerText()).includes(
      "変更はユーザ編集画面に閉じる",
    ),
  );
  assert.ok(
    (await page.locator(".inquiry-basis").innerText()).includes(
      "class UserEditAction {}",
    ),
  );
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: "/tmp/systivra-phase7-inquiry-1440.png",
    fullPage: false,
  });

  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("button", { name: "影響をたずねる" }).click();
  await page.locator(".inquiry-counts").first().waitFor({ timeout: 30_000 });
  await page.locator(".inquiry-ai").waitFor({ timeout: 30_000 });
  assert.equal(
    await details.getAttribute("open"),
    null,
    "persisted inference must remain folded",
  );
  assert.equal(await counts(), authoritativeCounts, "reload must not change counts");
  assert.deepEqual(await tables(), authoritativeTables, "reload must not change tables");

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("button", { name: "影響をたずねる" }).click();
  await page.locator(".inquiry-counts").first().waitFor({ timeout: 30_000 });
  await page.locator(".inquiry-ai").waitFor({ timeout: 30_000 });
  await page.locator(".inquiry-ai").locator("summary").click();
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - window.innerWidth,
  );
  assert.ok(overflow <= 1, `narrow layout overflows by ${overflow}px`);
  await page.screenshot({
    path: "/tmp/systivra-phase7-inquiry-607.png",
    fullPage: false,
  });

  console.log(
    JSON.stringify({
      hypotheses: await page.locator(".inquiry-hypothesis").count(),
      basis: await page.locator(".inquiry-basis li").count(),
      folded: true,
      authoritativeUnchanged: true,
      overflow,
      errors,
    }),
  );
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
