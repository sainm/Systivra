// Ontology 画面の受入。DRAFT の作成 → 型/関係型の編集 → マトリクス → 破棄まで。
//
//   npm run check:ui:ontology
//   WEB_BASE=http://localhost:15206 CHROME_BIN=... npm run check:ui:ontology
//
// 前提: API と Vite が起動していること（API 側の DB は何でもよい）。
// 画面から作った DRAFT は最後に API で破棄する。**公開版には触らない。**
import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const chrome =
  process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const base = process.env.WEB_BASE ?? "http://localhost:15206";
const password = process.env.SEED_PASSWORD ?? "SystivraDemo1!";
const projectName = process.env.PROJECT_NAME ?? "顧客管理システム";

const browser = await chromium.launch({ executablePath: chrome, headless: true });
const context = await browser.newContext({
  viewport: { width: 1600, height: 1000 },
  colorScheme: "dark",
});
const page = await context.newPage();
const errors = [];
page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
page.on("response", (response) => {
  if (response.status() < 400) return;
  const url = response.url();
  if (url.endsWith("/favicon.ico")) return;
  // ログイン前の /auth/me は 401 が正常。
  if (response.status() === 401 && url.includes("/api/v1/auth/me")) return;
  // 破棄してから「DRAFT を読む」は 404 が正常（画面は公開版を出す）。
  if (response.status() === 404 && url.includes("/graph/ontology/draft")) return;
  errors.push(`http: ${response.status()} ${url}`);
});

let projectId = "";

try {
  await page.goto(`${base}/login`, { waitUntil: "networkidle" });
  await page.fill("#login-id", "alice");
  await page.fill("#password", password);
  await page.click('button[type="submit"]');
  await page.waitForURL(`${base}/`);

  await page.getByText(projectName).first().click();
  await page.waitForURL(/\/documents$/);
  projectId = new URL(page.url()).pathname.split("/")[2];

  // 前回の残骸があれば先に捨てる（途中で落ちたときの後始末）。
  await context.request.delete(`${base}/api/v1/projects/${projectId}/graph/ontology/draft`);

  await page.goto(`${base}/projects/${projectId}/ontology`, { waitUntil: "networkidle" });

  const publishedRows = await page.locator(".graph-layout .panel").nth(0).locator("tbody tr").count();
  assert.ok(publishedRows > 0, "公開版の実体型が表示されていない");

  // ── DRAFT を作る
  await page.getByRole("button", { name: "型を編集" }).click();
  await page.locator(".chip-warn", { hasText: "編集中" }).waitFor({ timeout: 10_000 });

  const entityRows = page.locator(".graph-layout .panel").nth(0).locator("tbody tr");
  const relationRows = page.locator(".graph-layout .panel").nth(1).locator("tbody tr");

  // ── 実体型: 追加 → 編集 → 削除
  await page.locator('input[placeholder="名前"]').nth(0).fill("E2E_TYPE");
  await page.locator('input[placeholder="表示名"]').nth(0).fill("E2E 型");
  await page.getByRole("button", { name: "型を追加", exact: true }).click();
  const addedRow = entityRows.filter({ hasText: "E2E_TYPE" }).first();
  await addedRow.waitFor({ timeout: 10_000 });
  assert.match(await addedRow.innerText(), /E2E 型/);

  await addedRow.getByRole("button", { name: "編集" }).click();
  // クリック直後に fill すると、行の値がフォームへ反映される前の入力が
  // 上書きされる。「編集中は名前が disabled」を確定信号として待つ。
  await page.waitForFunction(
    () => {
      const input = document.querySelectorAll('input[placeholder="名前"]')[0];
      return input instanceof HTMLInputElement && input.disabled;
    },
    null,
    { timeout: 5000 },
  );
  const nameInput = page.locator('input[placeholder="名前"]').nth(0);
  assert.equal(await nameInput.isDisabled(), true, "編集中は名前を編集できてはいけない");
  await page.locator('input[placeholder="表示名"]').nth(0).fill("E2E 型 更新");
  await page.getByRole("button", { name: "更新" }).first().click();
  // 行が存在するだけでは再描画前を読むことがある。**新しい表示名が出るまで**待つ。
  await entityRows
    .filter({ hasText: "E2E 型 更新" })
    .first()
    .waitFor({ timeout: 10_000 });

  // ── 関係型: 明示的な端点つきで追加（マトリクスが点灯する条件）
  await page.locator('input[placeholder="名前"]').nth(1).fill("E2E_REL");
  await page.locator('input[placeholder="表示名"]').nth(1).fill("E2E 関係");
  await page.locator('input[placeholder^="始点の型"]').fill("Screen");
  await page.locator('input[placeholder^="終点の型"]').fill("Field");
  await page.getByRole("button", { name: "関係型を追加" }).click();
  await relationRows.filter({ hasText: "E2E_REL" }).first().waitFor({ timeout: 10_000 });

  // ── マトリクス: 絞り込み → セル押下でフォームに反映
  await page.getByRole("button", { name: "マトリクス" }).click();
  await page.fill('input[placeholder="型を絞り込む"]', "Screen");
  const heads = await page.locator("table.matrix th.matrix-head").allInnerTexts();
  assert.deepEqual(heads, ["Screen"], `絞り込み後の見出し: ${heads.join(",")}`);

  // 絞り込みは行と列の両方に効くので、Screen だけにすると Field 列が消える。
  // セルを見るために絞り込みを解除する。
  await page.fill('input[placeholder="型を絞り込む"]', "");
  await page.waitForTimeout(300);
  const allHeads = await page.locator("table.matrix th.matrix-head").allInnerTexts();
  const fieldIndex = allHeads.indexOf("Field");
  assert.ok(fieldIndex >= 0, `Field 列が無い: ${allHeads.slice(0, 8).join(",")}`);

  const screenRow = page.locator("table.matrix tbody tr", { hasText: "Screen" }).first();
  const litCell = screenRow.locator("td.matrix-cell").nth(fieldIndex);
  await litCell.waitFor({ timeout: 10_000 });
  assert.match(await litCell.getAttribute("title"), /Screen → Field: .*E2E_REL/);
  await litCell.click();
  const sourceValue = await page.locator('input[placeholder^="始点の型"]').inputValue();
  assert.equal(sourceValue, "Screen", "セル押下で始点が入る");

  // ── 破棄: 公開版は変わらない
  await page.getByRole("button", { name: "編集を閉じる" }).click();
  await context.request.delete(`${base}/api/v1/projects/${projectId}/graph/ontology/draft`);
  await page.reload({ waitUntil: "networkidle" });
  const afterRows = await page.locator(".graph-layout .panel").nth(0).locator("tbody tr").count();
  assert.equal(afterRows, publishedRows, "DRAFT を破棄しても公開版の行数は変わらない");
  assert.equal(
    await page.locator(".chip-warn", { hasText: "編集中" }).count(),
    0,
    "破棄後は編集中の表示が消える",
  );

  assert.deepEqual(errors, [], `画面エラー: ${errors.join(" / ")}`);
  console.log(
    JSON.stringify(
      { publishedRows, filteredHeads: heads, afterRows, errors: errors.length },
      null,
      1,
    ),
  );
} finally {
  // 失敗しても DRAFT を残さない。
  if (projectId) {
    await context.request
      .delete(`${base}/api/v1/projects/${projectId}/graph/ontology/draft`)
      .catch(() => {});
  }
  await browser.close();
}
