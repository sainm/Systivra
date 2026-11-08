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
  await page.goto(`${base}/projects/${projectId}/code`, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "コード解析" }).waitFor();
  const repositorySelect = page.locator("#code-repository");
  const optionTexts = await repositorySelect.locator("option").allTextContents();
  const analyzed = optionTexts.find((text) => text.includes("phase4-eval"));
  if (analyzed) {
    const value = await repositorySelect
      .locator("option")
      .filter({ hasText: analyzed })
      .getAttribute("value");
    await repositorySelect.selectOption(value);
    await page.waitForTimeout(500);
  }
  await page.selectOption("#code-kind", "GIT");
  await page.locator("#code-branch").waitFor();
  const gitOption = await page.locator("#code-kind option[value='GIT']").count();
  assert.equal(gitOption, 1, "Git repository kind must be selectable");
  await page.screenshot({ path: "/tmp/systivra-phase4-git-kind-1440.png", fullPage: false });
  const gitRepositoryId = await page.locator("#code-repository option").evaluateAll((options) => {
    const match = options.find((option) => option.textContent?.includes("[GIT]"));
    return match?.value ?? "";
  });
  assert.ok(gitRepositoryId, "at least one registered Git repository must be visible");
  await page.selectOption("#code-repository", gitRepositoryId);
  assert.equal(
    (await page.getByTestId("repository-kind").innerText()).trim(),
    "GIT",
    "selected repository kind must be visible",
  );
  const dirRepositoryId = await page.locator("#code-repository option").evaluateAll((options) => {
    const dir = options.filter((option) => option.textContent?.includes("[DIR]"));
    const match =
      dir.find((option) => option.textContent?.includes("repositories/phase4-sample")) ??
      dir.find((option) => option.textContent?.includes("phase4-eval")) ??
      dir[0];
    return match?.value ?? "";
  });
  assert.ok(dirRepositoryId, "the analyzed DIR fixture must be visible");
  await page.selectOption("#code-repository", dirRepositoryId);
  await page.selectOption("#code-kind", "DIR");
  await page.waitForFunction(
    () =>
      document.querySelector('[data-testid="repository-kind"]')?.textContent?.trim() ===
      "DIR",
  );
  await page.locator(".metric-card").first().waitFor({ timeout: 30_000 });
  const revisionItems = page.locator(".tree-item");
  const revisionCount = await revisionItems.count();
  for (let index = 0; index < revisionCount; index += 1) {
    await revisionItems.nth(index).click();
    await page.waitForTimeout(400);
    if ((await page.locator(".quality-warning").count()) > 0) break;
  }
  await page.locator("table.data").first().locator("tbody tr").first().waitFor({
    timeout: 30_000,
  });
  await page
    .locator("table.data")
    .nth(1)
    .locator("tbody tr .chip-danger, tbody tr .chip-warning")
    .first()
    .waitFor({ timeout: 30_000 });

  const metrics = await page.locator(".metric-card").count();
  const revisionRows = await page.locator(".tree-item").count();
  const symbolRows = await page.locator("table.data").first().locator("tbody tr").count();
  const factRows = await page.locator("table.data").nth(1).locator("tbody tr").count();
  const degraded = await page.locator(".quality-warning").count();
  const unresolved = await page.locator(".chip-danger, .chip-warning").count();
  assert.equal(metrics, 5, "five analysis metrics must render");
  assert.ok(revisionRows >= 1, "revision list must render");
  assert.ok(symbolRows >= 1, "symbol table must render");
  assert.ok(factRows >= 1, "fact table must render");
  assert.ok(degraded >= 1, "missing classpath / parser limitations must be visible");
  assert.ok(unresolved >= 1, "unresolved facts must not be hidden");

  await page.locator("table.data").nth(1).locator("tbody tr").first().click();
  await page.locator(".code-quote").first().waitFor();
  const quote = await page.locator(".code-quote").first().innerText();
  assert.ok(quote.length > 0, "selected fact must show source evidence");
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: "/tmp/systivra-phase4-code-1440.png", fullPage: false });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.locator(".code-layout").first().waitFor();
  const columns = await page.locator(".code-layout").first().evaluate((element) => {
    return getComputedStyle(element).gridTemplateColumns.split(" ").length;
  });
  assert.equal(columns, 1, "code layout must stack on narrow viewport");
  await page.screenshot({ path: "/tmp/systivra-phase4-code-607.png", fullPage: false });

  console.log(
    JSON.stringify({
      metrics,
      revisionRows,
      symbolRows,
      factRows,
      degraded,
      unresolved,
      quoteLength: quote.length,
      gitKind: gitOption === 1,
      errors,
    }),
  );
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
