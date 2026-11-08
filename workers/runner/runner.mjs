#!/usr/bin/env node
/**
 * Phase 6 Playwright Runner.
 *
 * 正本: docs/design/ja/detail/09-test-runner.md §3 / §4 / §5 / §6 / §7 / §8 / §12 / §13
 *
 * この Runner は基盤の外側にある独立プロセスである。API からケースの DSL を読み、
 * ブラウザで実行し、結果を `executions:import` に `execution_source=PLAYWRIGHT`
 * として返す。PASSED / FAILED を推測で作ることはしない。
 *
 *   - 1 case = 1 context（cookie / localStorage を持ち越さない）
 *   - 並列は context 数で稼ぐ（既定 4）
 *   - 失敗時だけ screenshot / DOM / console / trace の 4 点と HAR を残す
 *   - 成功時は trace を保存しない（容量）
 *   - HAR の Cookie / Authorization 等を伏せ字にしてから成果物へ入れる
 *   - TIMEOUT / NETWORK のみ 1 回再試行し、attempt=2 を別行で返す
 */

import fs from "node:fs/promises";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "../..");

const API_BASE = process.env.SYSTIVRA_BASE_URL ?? "http://127.0.0.1:18086";
const WEB_BASE = process.env.WEB_BASE ?? "http://127.0.0.1:18099";
const LOGIN_ID = process.env.SYSTIVRA_LOGIN_ID ?? "alice";
const PASSWORD = process.env.SYSTIVRA_PASSWORD ?? "SystivraDemo1!";
const PARALLEL = Math.max(1, Number(process.env.PARALLEL ?? 4));
const LIMIT = Number(process.env.LIMIT ?? 0);
const STEP_TIMEOUT_MS = Number(process.env.STEP_TIMEOUT_MS ?? 10_000);
const CHROME = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const ARTIFACT_DIR = process.env.ARTIFACT_DIR ?? path.join(ROOT, "data", "phase6-runner");
const SECRET_COOKIE = "phase6-secret-token-do-not-leak";
const RETRYABLE = new Set(["TIMEOUT", "NETWORK"]);
const ENV_SUSPECT = new Set(["SELECTOR_NOT_FOUND", "TIMEOUT", "NETWORK"]);
const DATA_MARKER = /duplicate|重複|一意制約|unique constraint|already exists|前提データ/i;
const ENVIRONMENT_ID = process.env.ENVIRONMENT_ID ?? "env-sample-stg";
const STORAGE_STATE = process.env.STORAGE_STATE ?? "";
const STORAGE_STATE_MAX_AGE_MIN = Number(process.env.STORAGE_STATE_MAX_AGE_MIN ?? 480);
const DATA_MODE = process.env.DATA_MODE ?? (process.env.RESET_URL ? "RESET_ENDPOINT" : process.env.SEED_URL ? "SEED" : "NONE");
const RESET_URL = process.env.RESET_URL ?? "";
const SEED_URL = process.env.SEED_URL ?? "";
const CASE_TIMEOUT_MS = Number(process.env.CASE_TIMEOUT_MS ?? 180_000);
const RUN_TIMEOUT_MS = Number(process.env.RUN_TIMEOUT_MS ?? 86_400_000);
const ENV_LOCK_WAIT_SEC = Number(process.env.ENV_LOCK_WAIT_SEC ?? 600);
const TRACE_MAX_BYTES = Number(process.env.TRACE_MAX_BYTES ?? 20 * 1024 * 1024);
const RUN_ARTIFACT_LIMIT_BYTES = Number(process.env.RUN_ARTIFACT_LIMIT_BYTES ?? 20 * 1024 * 1024 * 1024);

function loadChromium() {
  // 単独インストールを優先し、無ければ apps/web の依存を再利用する。
  for (const base of [import.meta.url, new URL("../../apps/web/package.json", import.meta.url)]) {
    try {
      return createRequire(base)("playwright-core").chromium;
    } catch {
      // try next
    }
  }
  throw new Error("playwright-core が見つかりません。workers/runner で npm install してください。");
}

let cookie = "";
async function api(method, route, body) {
  const response = await fetch(`${API_BASE}/api/v1${route}`, {
    method,
    headers: {
      "content-type": "application/json",
      ...(cookie ? { cookie } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const setCookie = response.headers.getSetCookie?.() ?? [];
  if (setCookie.length > 0) {
    cookie = setCookie.map((value) => value.split(";")[0]).join("; ");
  }
  const text = await response.text();
  const payload = text ? JSON.parse(text) : null;
  if (!response.ok) {
    throw new Error(`${method} ${route}: HTTP ${response.status}: ${text.slice(0, 400)}`);
  }
  return payload;
}

const SENSITIVE = /^(cookie|set-cookie|authorization|proxy-authorization|x-csrf-token)$/i;

export async function maskHar(file) {
  let raw;
  try {
    raw = JSON.parse(await fs.readFile(file, "utf8"));
  } catch {
    return { masked: false, reason: "har_missing" };
  }
  for (const entry of raw?.log?.entries ?? []) {
    for (const header of [
      ...(entry.request?.headers ?? []),
      ...(entry.response?.headers ?? []),
    ]) {
      if (SENSITIVE.test(header.name)) header.value = "***";
    }
    for (const item of [
      ...(entry.request?.cookies ?? []),
      ...(entry.response?.cookies ?? []),
    ]) {
      item.value = "***";
    }
  }
  const text = JSON.stringify(raw);
  try {
    await fs.writeFile(file, text, "utf8");
  } catch {
    return { masked: false, reason: "har_write_failed" };
  }
  return { masked: true, leak: text.includes(SECRET_COOKIE) };
}

export function cssEscape(value) {
  return String(value).replace(/["\\]/g, "\\$&");
}

export function locatorFor(page, target = {}) {
  if (target.testid) {
    return { locator: page.locator(`[data-testid="${cssEscape(target.testid)}"]`), strategy: "testid" };
  }
  if (target.role) {
    const options = target.name ? { name: target.name } : {};
    return { locator: page.getByRole(target.role, options), strategy: "role" };
  }
  if (target.label) {
    return { locator: page.getByLabel(target.label), strategy: "label" };
  }
  if (target.text) {
    return { locator: page.getByText(target.text, { exact: true }), strategy: "text" };
  }
  if (target.css) {
    return { locator: page.locator(target.css), strategy: "css" };
  }
  return { locator: page.locator("body"), strategy: "body" };
}

export function classify(error) {
  const message = String(error?.message ?? error);
  if (/strict mode violation|no element|not found|waiting for locator/i.test(message)) {
    return "SELECTOR_NOT_FOUND";
  }
  if (/timeout|timed out/i.test(message)) return "TIMEOUT";
  if (/net::|ERR_|navigation failed/i.test(message)) return "NETWORK";
  return "RUNNER";
}

export function classifyStepError(error, expected, actual) {
  let kind = error.kind ?? classify(error);
  if (kind === "ASSERTION" && DATA_MARKER.test(`${expected ?? ""} ${actual ?? ""} ${error?.message ?? ""}`)) {
    kind = "DATA";
  }
  return kind;
}

export function buildUrl(dsl) {
  const first = dsl.steps?.[0];
  const raw = first?.target?.path ?? "/";
  const pathname = raw.endsWith("/") ? raw : `${raw}/`;
  const url = new URL(pathname, `${WEB_BASE.replace(/\/$/, "")}/`);
  for (const step of dsl.steps ?? []) {
    if (step.target?.label) {
      url.searchParams.set("field", step.target.label);
      break;
    }
  }
  const assertion = (dsl.steps ?? []).find((step) => step.action === "assert_text");
  if (assertion?.value) url.searchParams.set("label", assertion.value);
  return url.toString();
}

async function executeSteps(page, dsl, consoleLines, artifactDir) {
  const stepResults = [];
  const caseStarted = Date.now();
  let expected = null;
  let actual = null;
  for (const [index, step] of (dsl.steps ?? []).entries()) {
    const started = Date.now();
    try {
      if (Date.now() - caseStarted > CASE_TIMEOUT_MS) {
        const err = new Error(`case timeout (${CASE_TIMEOUT_MS}ms)`);
        err.kind = "TIMEOUT";
        throw err;
      }
      const { locator, strategy } = locatorFor(page, step.target ?? {});
      switch (step.action) {
        case "goto":
          await page.goto(buildUrl(dsl), { waitUntil: "domcontentloaded", timeout: STEP_TIMEOUT_MS });
          break;
        case "fill":
          await locator.fill(step.value ?? "");
          break;
        case "select":
          await locator.selectOption(step.value ?? "");
          break;
        case "check":
          await locator.check();
          break;
        case "uncheck":
          await locator.uncheck();
          break;
        case "click":
          await locator.click();
          break;
        case "press":
          await page.keyboard.press(step.value ?? "Enter");
          break;
        case "upload":
          await locator.setInputFiles(step.value ?? "");
          break;
        case "wait_for":
          if (step.ms) await page.waitForTimeout(Math.min(step.ms, 5000));
          else await locator.waitFor({ state: "visible", timeout: STEP_TIMEOUT_MS });
          break;
        case "screenshot": {
          const file = path.join(artifactDir, `step-${String(index).padStart(2, "0")}.png`);
          await page.screenshot({ path: file });
          break;
        }
        case "field_error": {
          const error = page.locator(".field-error:visible");
          const visible = (await error.count()) > 0;
          actual = visible ? await error.first().innerText() : "(エラー表示なし)";
          expected = step.value ?? "(いずれかのエラー表示)";
          if (!visible || (step.value && !actual.includes(step.value))) {
            const err = new Error("ASSERTION");
            err.kind = "ASSERTION";
            throw err;
          }
          break;
        }
        case "assert_text": {
          actual = await page.locator("body").innerText();
          expected = step.value ?? "";
          const mode = step.match_mode ?? "contains";
          const ok = mode === "exact" ? actual.trim() === expected : actual.includes(expected);
          if (!ok) {
            const err = new Error("ASSERTION");
            err.kind = "ASSERTION";
            throw err;
          }
          break;
        }
        case "assert_value": {
          actual = await locator.inputValue();
          expected = step.value ?? "";
          if (actual !== expected) {
            const err = new Error("ASSERTION");
            err.kind = "ASSERTION";
            throw err;
          }
          break;
        }
        case "assert_visible": {
          if (!(await locator.isVisible())) {
            const err = new Error("ASSERTION");
            err.kind = "ASSERTION";
            throw err;
          }
          break;
        }
        case "assert_hidden": {
          if (await locator.isVisible()) {
            const err = new Error("ASSERTION");
            err.kind = "ASSERTION";
            throw err;
          }
          break;
        }
        case "assert_count": {
          actual = String(await locator.count());
          expected = step.value ?? "0";
          if (actual !== expected) {
            const err = new Error("ASSERTION");
            err.kind = "ASSERTION";
            throw err;
          }
          break;
        }
        case "assert_url": {
          actual = page.url();
          expected = step.value ?? "";
          if (!actual.includes(expected)) {
            const err = new Error("ASSERTION");
            err.kind = "ASSERTION";
            throw err;
          }
          break;
        }
        case "assert_table": {
          actual = await page.locator("body").innerText();
          expected = step.value ?? "";
          if (expected && !actual.includes(expected)) {
            const err = new Error("ASSERTION");
            err.kind = "ASSERTION";
            throw err;
          }
          break;
        }
        default: {
          const err = new Error(`unsupported action: ${step.action}`);
          err.kind = "RUNNER";
          throw err;
        }
      }
      stepResults.push({ ordinal: index, status: "PASSED", strategy, duration_ms: Date.now() - started });
    } catch (error) {
      const kind = classifyStepError(error, expected, actual);
      stepResults.push({
        ordinal: index,
        status: "FAILED",
        failure_kind: kind,
        message: String(error?.message ?? error).slice(0, 200),
        duration_ms: Date.now() - started,
      });
      return {
        ok: false,
        failure_kind: kind,
        failure_step: index,
        expected,
        actual,
        stepResults,
      };
    }
  }
  return { ok: true, failure_kind: null, failure_step: null, expected, actual, stepResults };
}

export async function artifact(file, kind, artifacts, status = null) {
  try {
    const stat = await fs.stat(file);
    artifacts.push({ kind, uri: path.relative(ROOT, file), bytes: stat.size, ...(status ? { status } : {}) });
  } catch {
    // 成果物が無い場合は数えない（合否は保つ）
  }
}

export function applyBudget(artifacts, budget, limit = RUN_ARTIFACT_LIMIT_BYTES) {
  for (const item of artifacts) budget.bytes += item.bytes;
  if (budget.bytes > limit) budget.traceDisabled = true;
}

async function runOnce(browser, testCase, runId, attempt, artifactDir, budget) {
  try { await fs.mkdir(artifactDir, { recursive: true }); } catch { }
  const harFile = path.join(artifactDir, `attempt-${attempt}-network.har`);
  const context = await browser.newContext({
    locale: "ja-JP",
    timezoneId: "Asia/Tokyo",
    viewport: { width: 1280, height: 900 },
    ...(STORAGE_STATE ? { storageState: STORAGE_STATE } : {}),
    // full で無いと headers / cookies が落ち、伏せ字の検証ができない。
    recordHar: { path: harFile, mode: "full" },
  });
  await context.addCookies([{ name: "SESSION", value: SECRET_COOKIE, url: WEB_BASE }]);
  await context.tracing.start({ screenshots: true, snapshots: true });
  const page = await context.newPage();
  page.setDefaultTimeout(STEP_TIMEOUT_MS);
  const consoleLines = [];
  page.on("console", (message) => {
    if (message.type() === "error" || message.type() === "warning") {
      consoleLines.push(`[${message.type()}] ${message.text()}`);
    }
  });
  page.on("pageerror", (error) => consoleLines.push(`[pageerror] ${error.message}`));

  const started = Date.now();
  const result = await executeSteps(page, testCase.dsl, consoleLines, artifactDir);
  const durationMs = Date.now() - started;
  const status = result.ok ? "PASSED" : "FAILED";
  const artifacts = [];

  if (!result.ok) {
    const shot = path.join(artifactDir, `attempt-${attempt}-failure.png`);
    const dom = path.join(artifactDir, `attempt-${attempt}-failure.html`);
    const log = path.join(artifactDir, `attempt-${attempt}-console.log`);
    const trace = path.join(artifactDir, `attempt-${attempt}-trace.zip`);
    try { await page.screenshot({ path: shot }); } catch { /* page may be gone */ }
    try { await fs.writeFile(dom, await page.content(), "utf8"); } catch { /* ignore */ }
    try { await fs.writeFile(log, consoleLines.join("\n") || "(no console output)", "utf8"); } catch { /* ignore */ }
    let traceStatus = null;
    if (budget.traceDisabled) {
      try { await context.tracing.stop(); } catch { /* ignore */ }
    } else {
      try {
        await context.tracing.stop({ path: trace });
        const stat = await fs.stat(trace);
        if (stat.size > TRACE_MAX_BYTES) {
          await fs.truncate(trace, TRACE_MAX_BYTES);
          traceStatus = "TRUNCATED";
        }
      } catch { /* ignore */ }
    }
    await artifact(shot, "SCREENSHOT", artifacts);
    await artifact(dom, "DOM", artifacts);
    await artifact(log, "CONSOLE", artifacts);
    await artifact(trace, "TRACE", artifacts, traceStatus);
  } else {
    // 成功時は trace を保存しない（detail/09 §8.2）。
    try { await context.tracing.stop(); } catch { /* ignore */ }
  }

  try { await context.close(); } catch { /* ignore */ }
  const har = await maskHar(harFile);
  if (!result.ok && har.masked) {
    await artifact(harFile, "HAR", artifacts);
  } else {
    try { await fs.rm(harFile, { force: true }); } catch { /* ignore */ }
  }
  if (!result.ok && artifacts.length === 0) {
    artifacts.push({ kind: "UPLOAD_FAILED", uri: "", bytes: 0 });
  }
  applyBudget(artifacts, budget);

  return {
    execution: {
      case_id: testCase.id,
      attempt,
      status,
      execution_source: "PLAYWRIGHT",
      failure_kind: result.failure_kind,
      failure_step: result.failure_step,
      expected: result.expected,
      actual: result.actual,
      duration_ms: durationMs,
      artifacts,
    },
    har,
    stepResults: result.stepResults,
  };
}

async function writeReport(report) {
  const out = path.join(ROOT, "data", "phase6-runner", "report.json");
  await fs.mkdir(path.dirname(out), { recursive: true });
  await fs.writeFile(out, JSON.stringify(report, null, 2), "utf8");
  console.log(JSON.stringify(report, null, 2));
  console.log(`written: ${out}`);
}

async function abortBeforeRun(reason, extra = {}) {
  await writeReport({
    run_id: null,
    env_abort: { reason, ...extra },
    case_count: 0,
    execution_count: 0,
    note: "実行前に中断。ケースは実行していない。",
  });
  process.exit(1);
}

async function callDataEndpoint(url) {
  const response = await fetch(url, { method: "POST" });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
}

async function main() {
  let storageStateInfo = null;
  if (STORAGE_STATE) {
    let stat;
    try {
      stat = await fs.stat(STORAGE_STATE);
    } catch {
      await abortBeforeRun("storage_state_missing", { file: STORAGE_STATE });
    }
    const ageSeconds = (Date.now() - stat.mtimeMs) / 1000;
    if (ageSeconds > STORAGE_STATE_MAX_AGE_MIN * 60) {
      await abortBeforeRun("storage_state_expired", {
        file: STORAGE_STATE,
        age_seconds: Math.round(ageSeconds),
        max_age_min: STORAGE_STATE_MAX_AGE_MIN,
      });
    }
    storageStateInfo = {
      file: STORAGE_STATE,
      age_seconds: Math.round(ageSeconds),
      max_age_min: STORAGE_STATE_MAX_AGE_MIN,
    };
  }

  const dataPreparation = { mode: DATA_MODE, reset: null, seed: null };
  if (DATA_MODE === "RESET_ENDPOINT") {
    if (!RESET_URL) await abortBeforeRun("data_reset_unconfigured");
    try {
      await callDataEndpoint(RESET_URL);
      dataPreparation.reset = "ok";
    } catch (error) {
      await abortBeforeRun("data_reset_failed", { error: String(error?.message ?? error) });
    }
  }
  if (DATA_MODE === "SEED") {
    if (!SEED_URL) await abortBeforeRun("data_seed_unconfigured");
    try {
      await callDataEndpoint(SEED_URL);
      dataPreparation.seed = "ok";
    } catch (error) {
      await abortBeforeRun("data_seed_failed", { error: String(error?.message ?? error) });
    }
  }

  const chromium = loadChromium();
  await fs.mkdir(ARTIFACT_DIR, { recursive: true });
  const login = await api("POST", "/auth/login", { login_id: LOGIN_ID, password: PASSWORD });
  const pid = process.env.PROJECT_ID || login.memberships[0].project_id;
  const overview = await api("GET", `/projects/${pid}/test/overview`);
  let cases = overview.cases.filter((item) => item.status === "APPROVED" && item.dsl?.steps?.length);
  if (LIMIT > 0) cases = cases.slice(0, LIMIT);
  if (cases.length === 0) throw new Error("APPROVED なケースがありません");

  let runId = process.env.RUN_ID;
  let lockWaitMs = 0;
  if (!runId) {
    const created = await api("POST", `/projects/${pid}/test/runs`, {
      name: `runner-${Date.now()}`,
      plan_id: null,
      environment: {
        id: ENVIRONMENT_ID,
        base_url: WEB_BASE,
        guard_kind: "STAGING",
        allowed_origins: [new URL(WEB_BASE).hostname],
        parallel: PARALLEL,
      },
    });
    runId = created.run_id;
    if (created.queued_behind) {
      const lockStart = Date.now();
      const deadline = lockStart + ENV_LOCK_WAIT_SEC * 1000;
      let ready = false;
      while (Date.now() < deadline) {
        const ov = await api("GET", `/projects/${pid}/test/overview`);
        const run = ov.runs.find((item) => item.id === runId);
        if (run && ["READY", "RUNNING", "DONE"].includes(run.status)) {
          ready = true;
          break;
        }
        await new Promise((resolve) => setTimeout(resolve, 2000));
      }
      lockWaitMs = Date.now() - lockStart;
      if (!ready) {
        await writeReport({
          run_id: runId,
          project_id: pid,
          environment_id: ENVIRONMENT_ID,
          lock_timeout: true,
          lock_wait_ms: lockWaitMs,
          case_count: cases.length,
          execution_count: 0,
          note: "環境ロック待機がタイムアウト。ケースは実行していない。",
        });
        process.exit(1);
      }
    }
  }

  const browser = await chromium.launch({ executablePath: CHROME, headless: true });
  let executions = [];
  const harResults = [];
  const executedIds = new Set();
  let passWithTrace = 0;
  let failedWithFour = 0;
  let envAbort = null;
  let dataAbort = null;
  let runTimeout = false;
  let dataResets = 0;
  let envChecked = false;
  const budget = { bytes: 0, traceDisabled: false };
  const wallStart = Date.now();

  try {
    for (let offset = 0; offset < cases.length; offset += PARALLEL) {
      if (Date.now() - wallStart > RUN_TIMEOUT_MS) {
        runTimeout = true;
        break;
      }
      const batch = cases.slice(offset, offset + PARALLEL);
      const settled = await Promise.all(
        batch.map(async (testCase) => {
          const dir = path.join(ARTIFACT_DIR, runId, testCase.code);
          const first = await runOnce(browser, testCase, runId, 1, dir, budget);
          if (first.execution.status === "FAILED" && RETRYABLE.has(first.execution.failure_kind)) {
            const second = await runOnce(browser, testCase, runId, 2, dir, budget);
            return [first, second];
          }
          return [first];
        }),
      );
      for (const group of settled) {
        for (const item of group) {
          executions.push(item.execution);
          harResults.push(item.har);
          executedIds.add(item.execution.case_id);
          if (item.execution.status === "PASSED") {
            if (item.execution.artifacts.some((a) => a.kind === "TRACE")) passWithTrace += 1;
          } else if (item.execution.status === "FAILED") {
            const kinds = new Set(item.execution.artifacts.map((a) => a.kind));
            if (["SCREENSHOT", "DOM", "CONSOLE", "TRACE"].every((k) => kinds.has(k))) {
              failedWithFour += 1;
            }
          }
        }
      }
      if (!envChecked) {
        const firstIds = [...executedIds].slice(0, 3);
        if (firstIds.length === 3) {
          envChecked = true;
          const wipeout = firstIds.every((id) => {
            const rows = executions.filter((e) => e.case_id === id);
            const first = rows.find((e) => e.attempt === 1);
            return first?.status === "FAILED"
              && ENV_SUSPECT.has(first.failure_kind)
              && rows.every((e) => e.status === "FAILED");
          });
          if (wipeout) {
            const ids = new Set(firstIds);
            for (const e of executions) {
              if (ids.has(e.case_id) && e.status === "FAILED") e.failure_kind = "ENV";
            }
            executions = executions.filter((e) => !ids.has(e.case_id) || e.attempt === 1);
            envAbort = { reason: "first_three_env_suspect", cases: firstIds };
            break;
          }
        }
      }
      const dataHit = executions.find((e) => e.attempt === 1 && e.status === "FAILED" && e.failure_kind === "DATA");
      if (dataHit) {
        let reset = "unconfigured";
        if (RESET_URL) {
          try {
            await callDataEndpoint(RESET_URL);
            dataResets += 1;
            reset = "ok";
          } catch {
            reset = "failed";
          }
        }
        dataAbort = { reason: "data_failure", case_id: dataHit.case_id, reset };
        break;
      }
    }
  } finally {
    await browser.close();
  }
  const wallMs = Date.now() - wallStart;

  const skippedCases = cases.filter((c) => !executedIds.has(c.id));
  if (envAbort || dataAbort || runTimeout) {
    for (const testCase of skippedCases) {
      executions.push({
        case_id: testCase.id,
        attempt: 1,
        status: "SKIPPED",
        execution_source: "PLAYWRIGHT",
        failure_kind: null,
        failure_step: null,
        expected: null,
        actual: null,
        duration_ms: 0,
        artifacts: [],
      });
    }
  }

  let accepted = 0;
  const rejected = [];
  for (let offset = 0; offset < executions.length; offset += 40) {
    const response = await api(
      "POST",
      `/projects/${pid}/test/runs/${runId}/executions:import`,
      { executions: executions.slice(offset, offset + 40) },
    );
    accepted += response.accepted;
    rejected.push(...response.rejected);
  }

  const failureKinds = {};
  for (const e of executions) {
    if (e.status === "FAILED") failureKinds[e.failure_kind] = (failureKinds[e.failure_kind] ?? 0) + 1;
  }
  const caseRows = new Map();
  for (const e of executions) {
    if (!caseRows.has(e.case_id)) caseRows.set(e.case_id, []);
    caseRows.get(e.case_id).push(e);
  }
  const codeById = new Map(cases.map((c) => [c.id, c.code]));
  const flakyCases = [];
  for (const [caseId, rows] of caseRows) {
    const first = rows.find((r) => r.attempt === 1);
    if (first?.status === "FAILED" && rows.some((r) => r.attempt >= 2 && r.status === "PASSED")) {
      flakyCases.push(codeById.get(caseId) ?? caseId);
    }
  }
  const warnings = executions
    .filter((e) => e.status === "FAILED" && e.artifacts.some((a) => a.kind === "UPLOAD_FAILED"))
    .map((e) => `UPLOAD_FAILED:${codeById.get(e.case_id) ?? e.case_id}`);
  const passed = executions.filter((item) => item.status === "PASSED").length;
  const firstPassed = executions.filter((item) => item.attempt === 1 && item.status === "PASSED").length;
  const report = {
    run_id: runId,
    project_id: pid,
    web_base: WEB_BASE,
    environment_id: ENVIRONMENT_ID,
    parallel: PARALLEL,
    case_count: cases.length,
    executed_case_count: executedIds.size,
    execution_count: executions.length,
    passed,
    failed: executions.filter((item) => item.status === "FAILED").length,
    skipped_not_executed: envAbort || dataAbort || runTimeout ? skippedCases.length : 0,
    first_attempt_pass_rate: executedIds.size ? firstPassed / executedIds.size : 0,
    flaky_count: flakyCases.length,
    flaky_cases: flakyCases,
    wall_ms: wallMs,
    wall_seconds: Number((wallMs / 1000).toFixed(2)),
    per_case_ms: Number((wallMs / Math.max(1, executedIds.size)).toFixed(1)),
    lock_wait_ms: lockWaitMs,
    env_abort: envAbort,
    data_abort: dataAbort,
    data_resets: dataResets,
    run_timeout: runTimeout,
    storage_state: storageStateInfo,
    data_preparation: dataPreparation,
    failure_kinds: failureKinds,
    warnings,
    artifacts: {
      failed_with_four: failedWithFour,
      pass_with_trace: passWithTrace,
      har_masked: harResults.filter((item) => item.masked).length,
      har_leaks: harResults.filter((item) => item.leak).length,
      upload_failed: warnings.length,
      trace_truncated: executions.filter((e) => e.artifacts.some((a) => a.status === "TRUNCATED")).length,
      trace_disabled: budget.traceDisabled,
      total_bytes: budget.bytes,
    },
    import: { accepted, rejected },
    note: "SAMPLE 静的サイトに対する実測。実案件の 90 秒/ケースとは異なる。",
  };
  await writeReport(report);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`FAILED: ${error.stack ?? error}`);
    process.exitCode = 1;
  });
}
