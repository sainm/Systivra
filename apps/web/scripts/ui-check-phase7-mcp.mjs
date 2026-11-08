import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const chrome =
  process.env.CHROME_BIN ??
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15175";
const password = process.env.SEED_PASSWORD ?? "SystivraDemo1!";
const projectId =
  process.env.PROJECT_ID ?? "22222222-2222-4222-8222-222222222222";

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
  await page.getByRole("heading", { name: "AI / Agent 基盤" }).waitFor();
  await page.locator(".ai-mcp-row").first().waitFor({ timeout: 30_000 });

  const rows = await page.locator(".ai-mcp-row").count();
  assert.ok(rows >= 2, "external MCP servers must render");
  assert.ok((await page.getByText("UP", { exact: true }).count()) >= 1);
  assert.ok((await page.getByText("DEGRADED", { exact: true }).count()) >= 1);
  assert.ok((await page.getByText("DOWN", { exact: true }).count()) >= 1);
  assert.ok((await page.getByText("stdio", { exact: true }).count()) >= 1);
  assert.ok((await page.getByText("http", { exact: true }).count()) >= 1);
  assert.ok((await page.getByText("MCP_SCHEMA_CHANGED").count()) >= 1);

  const httpRow = page.locator(".ai-mcp-row").filter({ hasText: "http-tools" });
  await httpRow.getByRole("button", { name: "再検出" }).click();
  await page.getByText("MCP Server の Tool 定義と health を更新しました。").waitFor({
    timeout: 30_000,
  });
  await page.locator(".ai-mcp-row").first().scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "/tmp/systivra-phase7-mcp-1440.png",
    fullPage: false,
  });

  await page.setViewportSize({ width: 607, height: 900 });
  await page.reload({ waitUntil: "networkidle" });
  await page.locator(".ai-mcp-row").first().waitFor({ timeout: 30_000 });
  await page.locator(".ai-mcp-row").first().scrollIntoViewIfNeeded();
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - window.innerWidth,
  );
  assert.ok(overflow <= 1, `narrow layout overflows by ${overflow}px`);
  await page.screenshot({
    path: "/tmp/systivra-phase7-mcp-607.png",
    fullPage: false,
  });

  console.log(
    JSON.stringify({
      rows,
      up: await page.getByText("UP", { exact: true }).count(),
      degraded: await page.getByText("DEGRADED", { exact: true }).count(),
      down: await page.getByText("DOWN", { exact: true }).count(),
      overflow,
      errors,
    }),
  );
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  await browser.close();
}
