import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  applyBudget,
  artifact,
  buildUrl,
  classify,
  classifyStepError,
  cssEscape,
  locatorFor,
  maskHar,
} from "./runner.mjs";

function fakePage() {
  return {
    locator: (selector) => ({ selector }),
    getByRole: (role, options) => ({ role, options }),
    getByLabel: (label) => ({ label }),
    getByText: (text, options) => ({ text, options }),
  };
}

test("classify separates selector, timeout and network failures", () => {
  assert.equal(classify(new Error("waiting for locator('[data-testid=x]')")), "SELECTOR_NOT_FOUND");
  assert.equal(classify(new Error("strict mode violation: resolved to 2 elements")), "SELECTOR_NOT_FOUND");
  assert.equal(classify(new Error("Timeout 10000ms exceeded.")), "TIMEOUT");
  assert.equal(classify(new Error("net::ERR_CONNECTION_REFUSED")), "NETWORK");
  assert.equal(classify(new Error("something else")), "RUNNER");
});

test("classifyStepError turns duplicate-data assertions into DATA", () => {
  const assertion = new Error("ASSERTION");
  assertion.kind = "ASSERTION";
  assert.equal(
    classifyStepError(assertion, "一意制約違反: 重複があります", "入力値を確認してください"),
    "DATA",
  );
  assert.equal(classifyStepError(assertion, "期待値", "実際値"), "ASSERTION");
  const timeout = new Error("Timeout");
  timeout.kind = "TIMEOUT";
  assert.equal(classifyStepError(timeout, "重複", "重複"), "TIMEOUT");
  assert.equal(classifyStepError(new Error("unique constraint violated")), "RUNNER");
});

test("cssEscape escapes quotes for testid selectors", () => {
  assert.equal(cssEscape('a"b\\c'), 'a\\"b\\\\c');
  assert.equal(cssEscape("plain"), "plain");
});

test("buildUrl keeps the trailing slash and forwards field/label queries", () => {
  const url = new URL(
    buildUrl({
      steps: [
        { action: "goto", target: { path: "/sample/user/edit" } },
        { action: "fill", target: { label: "user.addr" }, value: "x" },
        { action: "assert_text", value: "登録しました", match_mode: "contains" },
      ],
    }),
  );
  assert.equal(url.pathname, "/sample/user/edit/");
  assert.equal(url.searchParams.get("field"), "user.addr");
  assert.equal(url.searchParams.get("label"), "登録しました");
});

test("locatorFor prefers testid, then role, label, text, css", () => {
  const page = fakePage();
  assert.equal(locatorFor(page, { testid: "submit" }).strategy, "testid");
  assert.equal(locatorFor(page, { role: "button", name: "登録" }).strategy, "role");
  assert.equal(locatorFor(page, { label: "住所" }).strategy, "label");
  assert.equal(locatorFor(page, { text: "登録" }).strategy, "text");
  assert.equal(locatorFor(page, { css: "#value" }).strategy, "css");
  assert.equal(locatorFor(page, {}).strategy, "body");
});

test("maskHar redacts cookies and authorization headers", async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "runner-har-"));
  const file = path.join(dir, "network.har");
  await fs.writeFile(
    file,
    JSON.stringify({
      log: {
        entries: [
          {
            request: {
              headers: [
                { name: "Cookie", value: "SESSION=phase6-secret-token-do-not-leak" },
                { name: "Authorization", value: "Bearer abc" },
                { name: "Accept", value: "text/html" },
              ],
              cookies: [{ name: "SESSION", value: "phase6-secret-token-do-not-leak" }],
            },
            response: { headers: [{ name: "Set-Cookie", value: "SESSION=x" }], cookies: [] },
          },
        ],
      },
    }),
    "utf8",
  );
  const result = await maskHar(file);
  assert.equal(result.masked, true);
  assert.equal(result.leak, false);
  const masked = JSON.parse(await fs.readFile(file, "utf8"));
  const headers = masked.log.entries[0].request.headers;
  assert.equal(headers.find((h) => h.name === "Cookie").value, "***");
  assert.equal(headers.find((h) => h.name === "Authorization").value, "***");
  assert.equal(headers.find((h) => h.name === "Accept").value, "text/html");
  assert.equal(masked.log.entries[0].request.cookies[0].value, "***");
  const missing = await maskHar(path.join(dir, "missing.har"));
  assert.deepEqual(missing, { masked: false, reason: "har_missing" });
  await fs.rm(dir, { recursive: true, force: true });
});

test("artifact records existing files and skips missing ones", async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "runner-artifact-"));
  const file = path.join(dir, "failure.png");
  await fs.writeFile(file, Buffer.alloc(16));
  const artifacts = [];
  await artifact(file, "SCREENSHOT", artifacts);
  await artifact(path.join(dir, "missing.png"), "DOM", artifacts);
  await artifact(file, "TRACE", artifacts, "TRUNCATED");
  assert.equal(artifacts.length, 2);
  assert.equal(artifacts[0].kind, "SCREENSHOT");
  assert.equal(artifacts[0].bytes, 16);
  assert.equal(artifacts[1].status, "TRUNCATED");
  await fs.rm(dir, { recursive: true, force: true });
});

test("applyBudget disables traces after the run limit is exceeded", () => {
  const budget = { bytes: 0, traceDisabled: false };
  applyBudget([{ kind: "SCREENSHOT", bytes: 3000 }], budget, 4096);
  assert.equal(budget.traceDisabled, false);
  assert.equal(budget.bytes, 3000);
  applyBudget([{ kind: "DOM", bytes: 2000 }], budget, 4096);
  assert.equal(budget.traceDisabled, true);
  assert.equal(budget.bytes, 5000);
});