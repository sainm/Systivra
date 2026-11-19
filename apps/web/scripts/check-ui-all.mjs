// すべての UI 受入を順に走らせ、**「失敗」と「前提不足」を区別して**報告する。
//
//   node scripts/check-ui-all.mjs            # 前提が揃っているものだけ実行
//   node scripts/check-ui-all.mjs --all      # 前提不足も実行して失敗を見る
//
// 前提（フィクスチャ / 環境変数）が無いだけで赤くなるのは、状態の把握を
// 妨げる。ここでは「走らせて落ちた」と「走らせていない」を分けて出す。
import { spawn } from "node:child_process";
import path from "node:path";

const runAll = process.argv.includes("--all");
const webBase = process.env.WEB_BASE ?? "http://localhost:15173";

// needs: 何が揃っていれば意味のある受入になるか（人が読む説明）。
// env: 実行に必須の環境変数。無ければ SKIP。
const CHECKS = [
  { name: "phase2", needs: "XLSX 設計書（Phase 2 受入で取り込む）" },
  { name: "phase2-revisions", needs: "" },
  { name: "phase3", needs: "" },
  { name: "phase3-ops", needs: "終了状態のグラフ再構築（BUILDING のままだと出ない）" },
  { name: "phase4", needs: "コード解析済みリポジトリ" },
  {
    name: "phase5",
    needs: "設計書がコードシンボルに言及しているフィクスチャ（Phase 4/5 受入）",
  },
  { name: "phase5-inquiry", needs: "" },
  { name: "phase6", needs: "ルール抽出できる解析済み文書" },
  { name: "phase7", needs: "MCP サーバー登録済み" },
  { name: "phase7-inquiry", needs: "見出しを持つ完了済みの問い合わせ" },
  { name: "phase7-mcp", needs: "MCP サーバー登録済み" },
  { name: "phase8", needs: "ローカル IdP 設定済み" },
  { name: "cancel", needs: "DOC_TITLE（対象文書のタイトル）", env: ["DOC_TITLE"] },
  { name: "workflow", needs: "" },
  { name: "ontology", needs: "" },
];

function lastLine(text) {
  const lines = text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  const interesting =
    lines.filter((line) => /Error|assert|Timeout|waiting for|FAILED/.test(line)).pop() ??
    lines.pop() ??
    "";
  return interesting.slice(0, 120);
}

function run(name) {
  return new Promise((resolve) => {
    const child = spawn("npm", ["run", `check:ui:${name}`], {
      cwd: path.resolve(import.meta.dirname, ".."),
      env: { ...process.env, WEB_BASE: webBase },
    });
    let output = "";
    child.stdout.on("data", (chunk) => (output += chunk));
    child.stderr.on("data", (chunk) => (output += chunk));
    child.on("close", (code) => resolve({ code, output }));
  });
}

const results = [];
for (const check of CHECKS) {
  const missingEnv = (check.env ?? []).filter((key) => !process.env[key]);
  if (missingEnv.length > 0 && !runAll) {
    results.push({ name: check.name, status: "SKIP", detail: `env 未設定: ${missingEnv.join(", ")}` });
    continue;
  }
  const { code, output } = await run(check.name);
  if (code === 0) {
    results.push({ name: check.name, status: "PASS", detail: "" });
  } else {
    results.push({
      name: check.name,
      status: "FAIL",
      detail: lastLine(output),
      needs: check.needs,
    });
  }
}

const width = Math.max(...results.map((item) => item.name.length));
for (const item of results) {
  const mark =
    item.status === "PASS" ? "\x1b[32mPASS\x1b[0m" : item.status === "SKIP" ? "\x1b[33mSKIP\x1b[0m" : "\x1b[31mFAIL\x1b[0m";
  const detail = item.detail ? `  ${item.detail}` : "";
  const needs =
    item.status !== "PASS" && item.needs ? `\n${" ".repeat(width + 8)}前提: ${item.needs}` : "";
  console.log(`${mark}  ${item.name.padEnd(width)}${detail}${needs}`);
}

const failed = results.filter((item) => item.status === "FAIL");
console.log(
  `\nPASS ${results.filter((i) => i.status === "PASS").length} / FAIL ${failed.length} / SKIP ${results.filter((i) => i.status === "SKIP").length}`,
);
process.exit(failed.length > 0 ? 1 : 0);
