# Phase 7 進捗・受入記録（AI Agent / MCP / Workflow 縦断実装）

実施日: 2026-11-07  
対象: `08-実装計画.md` Phase 7 / `04-コアエンジン.md` §4.7〜§4.9  
環境: macOS arm64、PostgreSQL 18.4、Rust 1.99.1、Node.js 26.0.0、Neo4j 無効

> 判定: **Phase 7 の縦断切片は完了。Phase 7 全体受入は未完了。**
> Agent 登録（版管理）→ AgentRun、MCP Server のツール公開と `tools/call`、
> Workflow の承認ノードでの停止と再開、承認の留痕までを API / worker / 画面で通した。
> **AI は未設定のまま実行し、生成物を作らないこと（`AI_UNAVAILABLE`）を先に確認した。**
> 2026-11-07 の追補で **Agent のツール呼び出しループ**を実装し、ローカルの
> mock LLM（OpenAI 互換）で「AI 可用」の経路も通した。ツールは Agent の
> `capabilities` と突き合わせ、外れたら run を止める。
> Q-01 へ `narrative` / `hypotheses[]` を接続し、各仮説を `INFERRED`、根拠を
> Q-01 の既知 item に限定した。AI 出力の schema violation と AI 未設定でも
> 主表・件数・risk・resolution は変えず、画面では推定層を既定折り畳みにした。
> 2026-11-07 の追補で **外部 MCP Client** を stdio / HTTP で接続し、
> initialize / tools/list / tools/call、Tool 正規化、schema_hash、health、
> Agent 経由の能力ゲートウェイまで実測した。Schema 変化は自動適応せず DEGRADED にする。
> 2026-11-07 の追補で **MCP の実行可能性を §4.7 に合わせて是正**した。草稿生成は
> 「承認不要（草稿）」なので `approval_required=false` に直し、承認必須は
> `test_run_execute` だけにした。身元が要る `knowledge.search` / `impact.analyze` /
> 草稿生成を `tools/call` と Agent ループから通し、11 本の即時実行を実測した。
> 残る 1 本は承認後の 1 回実行として実測済み（下記 A-14）。
> さらに **承認後の破壊的実行**を実装した。`test.run.execute` 能力を追加し、
> Workflow の非 read ノードは経路上に承認ノードを持つ定義だけ許可する。
> 承認前に実行ノードが走らないことを実測で確かめている。
> CopilotRuntimeProvider と実モデルでのツール選択精度は未実装・未検証である。

---

## 1. 実装済み範囲

| 領域 | 実装 |
|---|---|
| DDL | `016_ai.sql`（ai schema 10 表、全表 RLS、`core.project_of` に 4 分支追加）、MCP Tool / health / Agent 依頼者 snapshot を足す `025_mcp_client.sql`、Workflow の実行主体を足す `026_workflow_actor.sql`、承認の消費を足す `027_approval_consume.sql` |
| AI Provider | `providers::AiProvider`。`OpenAiCompatProvider`（http のみ、`/chat/completions`、`tools` / `tool_calls` 対応）と `DisabledAiProvider`。`model_profile` を環境変数へ写像 |
| Gateway | 能力文字列 → `Capability` の変換、Agent ホワイトリスト、ロール権限、`project_id` のサーバー側注入を 1 か所に固定。予算判定は `runtime::budget_exceeded` |
| Agent | 登録と版の自動繰り上げ、Prompt 保存、`AGENT_RUN` job、**ツール呼び出しループ**（最大 `max_tool_calls`、`tool` メッセージ返送、結果は 2,000 字で切る）、`narrative` / `hypotheses` の構造検証、失敗時も job は正常終了 |
| MCP Client | stdio / HTTP の `initialize` / `tools/list` / `tools/call`、Tool と Schema の正規化、安定 `schema_hash`、health、session header、timeout、256 KiB 上限。Agent は外部 Tool を能力名で照合して呼ぶ |
| MCP Server | 12 ツールの公開、`initialize` / `tools/list` / `tools/call` の JSON-RPC、`schema_hash` の安定記録、入力 Schema の公開。**11 本は即実行**（read 9 + 草稿 write 2）。`test_run_execute` は未承認なら 423 と `approval_id` を返し、承認後に同じ引数で **1 回だけ**実行して許可証を消費する（再呼び出しは 423） |
| MCP の身元 | `tools/call` は要求者の `Authenticated` を能力実行まで渡す（`execute_capability(Some(auth), ..)`）。`knowledge.search` / `impact.analyze` は身元が無いと実行しない。Agent のツールループは `agent_run.requested_by` から身元を復元する（`Authenticated::for_job`） |
| Workflow | JSON 定義（start / capability / approval / end）、`WORKFLOW_RUN` job、承認ノードで `WAITING_APPROVAL`、承認後の再開（冪等）。**SVG の可視化エディタ**（配置・接続・属性・保存）。非 read の capability は **`start → … → approval → … → node` の経路上に承認がある場合だけ**許可（`approval_preceded`）。ノードの実行主体は起動した人間（`026` で `workflow_run.requested_by`） |
| 承認 | `approval_request` の起票・判断・留痕。Workflow の承認ノードは判断後に再開 job を投入。**MCP の承認必須ツールは「1 回の実行への許可証」**として扱い、承認後に同じ引数で呼ぶと 1 回だけ実行して消費する（`027` の `consumed_at`）。PENDING は 24 時間で `EXPIRED` |
| 監査 | `ai.*` の操作を `core.audit_log` に記録 |
| UI | Agent 登録 / 実行 / 実行一覧、MCP Server と 12 ツール、Workflow と承認の画面 |
| **Q-01 連携** | `narrative` / `hypotheses[]`。各仮説は `INFERRED`、`basis[]` は Q-01 の既知 item の `section / entity_id / label / quote / locator`。推定層は既定折り畳み |
| **Impact 叙述** | `POST /impacts/{iid}/narrate`。Q-01 と同じ provider を使い、AI 未設定なら `ai_unavailable`、設定時は `ai_inferred` で report に保存 |

---

## 2. 受入結果

| # | 受入 | 結果 | 証拠 |
|---|---|---|---|
| A-01 | すべての AI Provider を停止しても Phase 1〜6 が動く | **PASS** | `ai_status=unavailable` でも `test/overview` が 150 ケースを返す |
| A-02 | AI 停止時、UI が「AI 未設定」を表示する | **PASS** | 画面に `AI 未設定` と `ai_unavailable` を表示 |
| A-03 | 越権 Prompt を Gateway が拒否し監査に残る | **PASS（ツール境界）** | 未知 capability 400、承認必須 423、viewer 403、Agent ホワイトリスト外の外部 Tool で `AGENT_CAPABILITY_DENIED`。実モデルの Prompt 注入耐性は未検証 |
| A-04 | 同名ツール・別実装の MCP Server に差し替えても Agent 定義が変わらない | **PASS** | HTTP Server を alpha → beta に差し替え、同じ `schema_hash`、同じ Agent v1 のまま外部 Tool を実行。stdio も同じ Agent から実行 |
| A-05 | Workflow が承認ノードで停止し、再開できる | **PASS** | `WAITING_APPROVAL` → 承認 → `SUCCEEDED`。24 時間の実待機は未実施 |
| A-06 | Q-01 の 5 ブロックが AI 停止時も出る | **PASS** | AI 未設定でも 5 段 + `ai_unavailable`。`counts / risk / resolution / path / evidence` は narrate 前後で完全一致 |
| A-07 | Q-01 が AI 叙述と根拠付き仮説を返す | **PASS（mock LLM）** | `narrative` + 1 hypothesis、`resolution=INFERRED`、既知 code evidence 1 件を参照。GET / Markdown / UI に保存・表示。Impact も同じ経路で `narrative_source=AI` を保存 |
| A-08 | 不正な AI 出力は主表を変えず降格する | **PASS** | 未知 basis を `ai_schema_violation` として拒否。narrative / hypotheses は空、5 段と counts / risk は不変 |
| A-09 | 外部 MCP を stdio / HTTP で検出・実行できる | **PASS** | 両 transport で initialize / tools/list / tools/call。入力の `project_id` は Agent 引数を捨ててサーバー側で注入 |
| A-10 | Schema 変更を自動適用しない | **PASS** | hash 変化後は DEGRADED、旧 Tool 定義を保持。明示的な再登録時だけ新 Schema を受理。監査 2 行 |
| A-11 | 外部 Tool の失敗境界を明示する | **PASS** | timeout / oversize / invalid output / DOWN / unknown / 越権 / 書込み承認を個別に拒否または降格 |
| A-12 | MCP の 12 ツールが §4.7 の境界どおりに実行・拒否される | **PASS** | read 9 + 草稿 2 が実行（`knowledge_search` は `SRH-…`、草稿は `test_run_created=false`）。`test_run_execute` は承認必須で、承認後に 1 回だけ実行。未知ツールは 400 |
| A-13 | 承認ノードを経路上に持つ Workflow だけが破壊的操作を実行できる | **PASS** | 承認なしの `test.run.execute` 定義は 400。承認ありは `WAITING_APPROVAL` で停止し、`exec` の node_run が無いことを確認後に承認 → `SUCCEEDED`、test run を 1 件作成（`case_count=1`） |
| A-14 | MCP の承認が「1 回の実行への許可証」として機能する | **PASS** | 未承認呼び出しは 423 + `approval_id`。連打しても承認は増えず、引数が違えば別 ID。承認後の同引数呼び出しで run を 1 件作成、再呼び出しは 423。一覧 API でも `consumed_at` が入り、`workflow_run_id` は `null` |

### 2.1 API + worker E2E

独立 DB `systivra_phase6_verify`、API `18086`、AI 未設定。

```json
{"ai_unavailable":{"ai_status":"unavailable","degraded":["ai_unavailable"]},
 "core_still_works":{"test_cases":24},
 "unknown_capability_rejected":400,
 "agent_versions":[1,2],
 "agent_run":{"status":"FAILED","error":"AI_UNAVAILABLE","output":null},
 "mcp":{"tools":12,"schema_hash":"04df07ac2616","stable":true},
 "mcp_calls":{
   "total":12,
   "read":{"project_list":"ok","graph_query":2,"graph_trace_path":true,
     "code_get_symbol":6,"knowledge_search":1,"knowledge_get_evidence":"found",
     "impact_analyze":"545e2985-…","impact_get_report":true,
     "coverage_get":"degraded"},
   "write_draft":{
     "test_plan_generate":{"plan_id":"5d3900b0-…","appended":false,"drafts":4},
     "test_case_generate":{"plan_id":"5d3900b0-…","appended":true,"drafts":20,
       "plan_case_count":20},
     "same_plan":true},
   "destructive":{"test_run_execute":{"approval_required":423,
     "approval_id":"c8a2137d-…","executed_run_id":"95f9c211-…",
     "replay_blocked":"POLICY_APPROVAL_REQUIRED"}},
   "unknown_tool":"rejected"},
 "role_boundary":{"viewer_register_agent":403,"viewer_ai_overview":403,"viewer_mcp_read":200},
 "workflow":{"paused_at":"gate","approval_status":"APPROVED","final_status":"SUCCEEDED"},
 "workflow_write_capability_rejected":400,
 "workflow_approved_execute":{"approval_before_execute":true,
   "run_id":"12e817f2-…","status":"QUEUED","case_count":1},
 "audit_rows":22}
```

要点:

- Agent は同じ `code` で再登録すると `current_version` が 2 に上がる。
- AI 未設定の run は `FAILED` / `output=null`。**空の narrative を作らない。**
- `ai/overview` の `degraded` は実状態だけを並べる。以前は Workflow エディタを
  実装した後も `workflow_editor_not_implemented` を返し続けていた（画面に
  「未実装」と出ていた）。実装済みの項目は降格として出さない。
- `tools/call` の可否はツール定義の `side_effects` と `04-コアエンジン.md` §4.7
  「人間との境界」で決まる。
  read 9 本はすべて実行でき、草稿 2 本（`test_plan_generate` / `test_case_generate`）
  も実行できる。**承認が要るのは `test_run_execute` だけ**（正式環境での実行）。
  承認必須ツールは権限判定の後に 423 を返し、同時に `approval_request` を起票する
  （権限が無い人に存在を漏らさない）。承認後に**同じ引数**で呼ぶと 1 回だけ実行し、
  許可証は消費される。引数が 1 つでも違えば別の承認が要る。
- `test_plan_generate` は**新しい計画を作る**（`appended=false`）。`test_case_generate`
  は**既存の計画へ追記する**（`appended=true`、`plan_id` 必須）。取り違えると 400 で
  弾く —— 同じ派生器に流すと「同じ計画に積んでいるつもりで毎回新しい計画ができる」。
  追記は計画の `scope` を union で更新し、承認済み計画（`APPROVED`）へは追記しない。
  実測では 4 件の計画に 20 件を追記して `plan_case_count=20`（うち 4 件は id が
  一致して `ON CONFLICT DO NOTHING`）。`test_run_created=false` は Q-01 経由でも
  変わらない（R-16）。
- `DeriveTestPlanResponse` は `appended` と `plan_case_count` を返す。
  `draft_case_count` は**その呼び出しで生成した数**で、計画の総数ではない。
- `knowledge_search` は語彙一致が要る。`UserEdit` は 1 トークン
  （`usereditaction`）に潰れて 0 件になるため、実測は `UserService` で行った。
  日本語形態素分割は **Lindera + 内嵌 IPADIC で実装済み**（単体テストで
  `住所` が切れることを確認）。ただし **保存済み chunk の `tokenized` は古いまま**
  で、`fts` はその生成列なので追随しない。古い DB では `住所` が本文に在っても
  0 件になりうる。`POST /projects/{pid}/knowledge/retokenize` で作り直す
  （[`docs/phase2/acceptance.md`](../phase2/acceptance.md) §5、実測は
  `scripts/knowledge-retokenize-eval.py`）。
- `coverage_get` は snapshot が無ければ `detail=null` + `coverage_not_computed` を
  返す（**0% と書かない**）。この DB では未計算のため `degraded`。
- MCP の承認は「1 回の実行への許可証」として動く。未承認の呼び出しは 423 と
  `approval_id` を返し、同じ引数の連打では同じ ID を返す（承認依頼を増やさない）。
  承認後に**同じ引数**で呼ぶと 1 回だけ実行し、2 回目は再び 423 になる。
  引数が 1 つでも違えば別の承認が要る（payload の jsonb 等価で照合）。
  許可証の有効期限は 10 分で、しかも**起票した本人**しか使えない。
  承認した内容と実行内容をずらさないための制限。
  **起票の再利用も本人で絞る**（`pending_approval` の `requested_by` と、
  承認 ID の安定ハッシュに起票者を混ぜる）。他人の PENDING を返すと、
  相手が承認しても `requested_by` が違って使えず、こちらは 423 のまま詰まる。
  実測: 同一 action / 引数の他人の PENDING を置いても、別の `approval_id` を返す。
- Workflow の capability ノードは、read 以外なら**承認ノードを経路上に持つ定義**
  だけが登録できる（`start → … → approval → … → node`）。承認が無ければ 400。
  実行ノードが承認まで走らないことを実測で確かめている。
- 承認ノードを経路上に置いた定義なら、`test.run.execute` も実行できる。
  実測では `start → gate(approval) → exec(test.run.execute) → end` を登録し、
  `WAITING_APPROVAL` で停止（**この時点で `exec` の node_run は存在しない**）、
  承認後に `SUCCEEDED`、`exec` が `PASSED` で test run `12e817f2-…` を
  `QUEUED`（`case_count=1`）で作った。承認を飛ばす定義は同じ 400 で弾かれる。
- Viewer は AI 画面を開けない（`AgentRun` を持たない）。MCP の読み取りは通る。

実測結果: `data/phase7-eval/report.json`。

### 2.2 UI

`npm run check:ui:phase7`:

```json
{"agents":1,"mcp":1,"workflows":2,"workflowRuns":2,"approvals":5,"columns":1,"errors":[]}
```

- `AI 未設定`、`ai_unavailable`、12 ツール、Workflow `SUCCEEDED`、承認 `APPROVED` を表示。
  件数（`workflows` / `workflowRuns` / `approvals`）は E2E を何回回したかで変わる。
  見るのは**描画されているか**と `errors` が空であること。
- **順序に注意**: `phase7-eval.py` は冒頭で `ai.*` を TRUNCATE する（冪等化のため）。
  UI 検証は MCP Server が 1 件以上ある状態を前提にするので、E2E の後に
  `POST /mcp/servers` で 1 件登録してから実行する。今回も 1 度目は
  「mcp servers must render」で落ち、登録後に通った。
- 承認一覧は**起票元**（Workflow / MCP / Agent）を列で示し、`consumed_at` がある
  ものは「使用済み」を併記する。`APPROVED` のまま残るので、これが無いと
  まだ使えるように見える（実測: 5 セル、「使用済み」が 1 件以上）。
- エディタ: 例 4 ノード → ドラッグ移動 → 承認ノード追加（5 件）→ 保存で v2。
  1440px / 607px。607px で 1 列へ縦積み。
- スクリーンショット: `/tmp/systivra-phase7-ai-1440.png`、
  `/tmp/systivra-phase7-ai-607.png`。

---

### 2.3 Agent のツール呼び出しループ（mock LLM）

`scripts/phase7-agent-loop-eval.py` が独立 DB・API(18088)・worker・
mock LLM(11534) を起動して実測する。既存の 18086 には触れない。

```json
{"tool_loop":{"status":"SUCCEEDED",
  "steps":["complete:PASSED","project_list:PASSED"],
  "tool_calls":1,
  "narrative":"提示された根拠では、影響はユーザ編集画面周辺に限定されると推定されます。"},
 "tool_loop_identity":{"capability":"knowledge_search","status":"PASSED",
   "search_id_returned":true},
 "capability_denied":{"status":"FAILED","error":"AGENT_CAPABILITY_DENIED","output":null},
 "audit_rows":9}
```

- 予算 4 項目のうち `wall_time_sec` / `cost_usd` も実測対象にした。単価
  （`AI_PRICE_INPUT_PER_1M` / `AI_PRICE_OUTPUT_PER_1M`）を設定したうえで
  `max_cost_usd=0.000001` を当てると、1 コール（18 トークン = 1.8e-05 USD）で
  `BUDGET_EXCEEDED` / `budget=max_cost_usd` になる。単価未設定なら
  `cost.priced=false` を返し、**0 ドルとは書かない**（判定していないことを示す）。
- 1 回目の LLM 応答が `tool_calls: [project_list]` を返し、Agent が
  `capabilities` と照合して実行、`role=tool` として結果を返送し、2 回目で
  `narrative` / `hypotheses` を得た。
- **身元が要る能力**も Agent から呼べる。`knowledge.search` だけを持つ Agent に
  `knowledge_search` を返させると step は `PASSED` で、返送された tool 結果に
  実在の `search_id`（`SRH-…`）が入っていた。worker は `agent_run.requested_by`
  から `Authenticated::for_job` を組み立て、**Agent ではなく依頼した人間**の
  権限で Gateway とロール判定を通す。
- `coverage.get` しか許可されていない Agent に同じ `tool_call` を返すと、
  run は `FAILED` / `AGENT_CAPABILITY_DENIED`、`output=null` で止まる。
- ツール結果は 2,000 字で切り、`untrusted` としてモデルへ返す（命令として
  再実行しない）。

実測結果: `data/phase7-eval/agent-loop.json`。

#### 2.3.1 身元の貫通で見つかった権限漏れ（修正済み）

`knowledge.search` を worker から実行した最初の実測は
`permission denied for table project_member` で落ちた。worker は job を起動した
人間のロールを引く必要があるが、`systivra_worker` / `systivra_scheduler` に
`core.project_member` の SELECT が無かった。`roles.sql` に**読み取りのみ**の
GRANT を足して解消した（書込みは与えない）。既存 DB は `roles.sql` を再適用する。

> 設計判断: ロールを job 行に保存した `requested_role` で代替する案は採らなかった。
> 保存時点のロールが現在も有効とは限らず、降格後の権限で実行されうるため。
> 実行のたびに引き直す。

### 2.4 Q-01 の AI 叙述と仮説（mock LLM）

独立 DB、API `18088`、worker、mock LLM `11534`、Vite `15174` で実測した。

```json
{"inquiry_narrative":{"hypotheses":1,"resolution":"INFERRED",
  "basis":1,"degraded":["ai_inferred"],"authoritative_unchanged":true},
 "invalid_ai_output":{"narrative":null,"hypotheses":[],
  "degraded":["ai_schema_violation: ..."],"authoritative_unchanged":true},
 "ai_unavailable":{"narrative":null,"hypotheses":[],
  "degraded":["ai_unavailable"],"authoritative_unchanged":true},
 "ui":{"hypotheses":1,"basis":1,"folded":true,
  "authoritativeUnchanged":true,"overflow":0,"errors":[]}}
```

- AI は `summary.narrative` と `summary.hypotheses[]` だけを更新する。
  `screens / documents / code / tests`、`counts`、`risk`、既存 `resolution` は比較で不変。
- 仮説の `basis` はサービス側の Q-01 item だけを解決する。未知の参照、空 basis、
  `confidence` の範囲外、narrative 欠落は `SCHEMA_VIOLATION` 相当として受理しない。
- AI 未設定・schema violation でも 5 段は残る。`DRAFT`、`code_modified=false`、
  `tests_executed=false` は変更しない。
- UI は推定層を既定で閉じ、展開時に `INFERRED / claim / confidence / basis` を表示する。
  再読込後も同じ状態で、1440 / 607px の主表件数は不変、横 overflow 0。
  スクリーンショット: `/tmp/systivra-phase7-inquiry-1440.png`、
  `/tmp/systivra-phase7-inquiry-607.png`。

実測結果: `data/phase7-eval/agent-loop.json`。

### 2.5 外部 MCP Client（stdio / HTTP）

独立 DB、API `18089`、worker、mock LLM `11535`、Vite `15175`、
mock MCP Server を使って実測した。

```json
{"http_replacement":{"schema_stable":true,
  "implementations":["alpha","beta"],"agent_version":1,
  "project_id_injected":true},
 "schema_change":{"degraded":"DEGRADED","old_tools_retained":true,
  "explicit_reregister_recovered":"UP","audit_rows":2},
 "stdio":{"health":"UP","tools":2},
 "failures":{"timeout":"DOWN","oversize":"DOWN",
  "invalid_output":"DEGRADED","down":"DOWN",
  "unknown_tool":"CAPABILITY_UNSUPPORTED",
  "overreach":"AGENT_CAPABILITY_DENIED",
  "approval":"POLICY_APPROVAL_REQUIRED",
  "approval_did_not_call_server":true,
  "untrusted_isolated":true},
 "ui":{"rows":8,"up":3,"degraded":2,"down":3,
  "overflow":0,"errors":[]}}
```

- 外部 Tool は `name / capability / inputSchema / outputSchema / side_effects /
  timeout_ms` を正規化し、未知 capability や重複名は受理しない。
- `schema_hash` は Tool 名だけでなく Schema、side effects、approval、timeout を含む。
  変化時は旧 Tool を残して DEGRADED とし、次の discover でも自動で UP に戻さない。
  管理者が Server を再登録した場合だけ新しい定義を採用する。
- Agent の Tool 一覧には外部 Tool を含め、Gateway で Agent ホワイトリストとロールを
  再確認する。`write / destructive` は `approval_required=true` とし、この経路では実行しない。
- 外部戻り値は `untrusted=true` と Server 名を付けてモデルへ返す。本文は命令として
  再実行しない。応答は 256 KiB、Tool は 32 件、timeout は最大 60 秒で制限する。
- HTTP は `Mcp-Session-Id` を後続要求へ引き継ぐ。stdio は操作ごとに initialize し、
  応答後に子プロセスを終了する。
- UI は transport / health / tool_count / health_detail / schema_hash と再検出を表示する。
  1440 / 607px、page overflow 0。

実測結果: `data/phase7-mcp/report.json`。

### 2.6 回帰ゲート

- `cargo fmt --all --check`: 成功。
- `cargo clippy --workspace --all-targets -- -D warnings`: 成功。
- `cargo test --workspace`: **230 passed / 0 failed**。
- `cargo run -q -p api --bin openapi -- --check`: `openapi.json is up to date`。
- `npm run check:i18n`: 564 keys。`npm run build`: 成功。
- `npm run check:ui:phase7-inquiry`: hypotheses 1 / basis 1 / folded true /
  authoritative unchanged true / overflow 0 / pageerror・HTTP error 0。
- `npm run check:ui:phase7-mcp`: MCP Server 8 / UP 3 / DEGRADED 2 / DOWN 3 /
  overflow 0 / pageerror・HTTP error 0。
- `PGHOST=localhost PGUSER=sainm ./scripts/verify-db.sh`: 001〜027 全項目成功。

## 3. 未実施・降格

| 項目 | 状態 | 扱い |
|---|---|---|
| CopilotRuntimeProvider | **未実装** | `OpenAiCompatProvider` と `DisabledAiProvider` のみ |
| https / TLS の AI 接続 | **未実装** | `http://` のみ。閉域の vLLM / Ollama を想定 |
| 実モデルでのツール選択精度 | **未検証** | mock LLM は固定の `tool_call` を返す。モデルが適切なツールを選ぶかは測っていない |
| 予算の実消費（tokens / cost） | **一部** | 4 上限すべて判定する。`tool_calls` / `tokens` / `wall_time_sec` / `cost_usd` を実測。cost は `AI_PRICE_*` 設定時のみ（未設定は `priced=false`）。実モデルの単価・為替は未検証 |
| MCP 12 ツールの実装 | **12 / 12** | 能力は 12 個すべて実装。MCP `tools/call` から 11 本は即実行、`test.run.execute` は承認必須（承認後に 1 回だけ実行）。`test.plan_generate` は**新規作成**、`test.case_generate` は**既存 plan への追記**で、取り違えると 400 |
| Markdown 出力 | **実装済み**（Phase 7 の対象外） | `GET /impacts/{iid}/report.md` と `GET /change-inquiries/{iid}/report.md`。Phase 5 追補で実装・実測済み（[`docs/phase5/acceptance.md`](../phase5/acceptance.md)）。Phase 7 で新たに足したものではない |
| React Flow の採用 | **不採用（軽量 SVG）** | 依存を増やさず、ドラッグ配置・接続・属性編集・新版本保存を自前 SVG で実装。ノード座標は `WorkflowNode.x/y` に保存し、実行には使わない |
| Workflow 定義の YAML | **未実装** | JSON で保存（`serde_yaml` を増やさない） |
| 承認の 24 時間停止・再開 | **未実施** | 状態機械としての停止・再開は実測。実時間の待機は未実施 |
| 予算の実測 | **一部** | mock LLM で `max_cost_usd` の超過を実測（`BUDGET_EXCEEDED`）。実モデルの単価・為替・キャッシュ割引は未検証 |
| 実モデルでの Prompt 注入耐性 | **未検証** | mock LLM で Agent ホワイトリスト外 Tool の拒否は実測。実モデルの誘導耐性は未検証 |
| Q-01 の実モデルによる叙述品質 | **未検証** | mock LLM で配線・schema・画面を実測。実モデルの文章品質と仮説精度は未検証 |
| マルチノード / 水平分割 | **未実装** | 単一 API / worker プロセス |

---

## 4. 既知の制約

- AI が未設定のとき、`agent_run` は `FAILED` / `AI_UNAVAILABLE` で閉じる。
  job は `SUCCEEDED_WITH_ERRORS` とし、再試行しない（同じ失敗を繰り返さないため）。
- Q-01 の `/narrate` は既存の `AiProvider` と Agent 出力 schema 検証を共用するが、
  AgentRun は作らない。書き込み先は `summary.narrative` / `summary.hypotheses[]` /
  `summary.degraded` に限定する。
- Q-01 仮説は最大 5 件、各 `basis` は最大 8 件。モデルが返した `resolution` は使わず、
  サーバーが `INFERRED` を設定する。未知の basis は保存しない。
- Agent の `capabilities` はツール呼び出しごとに Gateway で照合する。
  ホワイトリスト外の `tool_call` は `AGENT_CAPABILITY_DENIED` で run を止める。
- `agent_run` は作成時に検証済みの `requested_by / requested_role` を保存する。
  ただし**実行時の権限判定は保存値を使わない**。worker は `requested_by` で
  `core.project_member` を引き直し、降格・除名を即座に反映する
  （`requested_role` は run の記録として残す）。この参照のため worker /
  scheduler に `core.project_member` の SELECT だけを付与している。
- `knowledge.search` / `impact.analyze` / 草稿生成は**身元が無いと実行しない**。
  Workflow の `capability` ノードは身元を渡さないため、これらを置くと
  `CAPABILITY_UNSUPPORTED` で止まる（黙って別主体で実行しない）。
- 外部 Tool の `$ref` Schema は外部 Client では解決せず、discover / call 時に明示エラーにする。
- stdio Server は操作ごとに新しい子プロセスを起動する。長期セッションや Server 側の
  メモリ状態は保持しない。command / args / env は管理者が登録する信頼済み設定として扱う。
- 外部 Tool は Agent の能力ゲートウェイ経由で利用する。プラットフォームの外向き MCP
  `tools/list` は従来どおりローカル 12 Tool だけを公開する。
- 外部 Tool の `write / destructive` は承認必須として Agent 呼び出しを止める。
  **外部**書き込みを承認後に実行する経路は未接続である。混同注意: ローカル 12 Tool の
  `test.run.execute` は承認→1 回実行まで接続済み（MCP の許可証 / Workflow の承認ノード）。
  未接続なのは Agent が外部 MCP Client 経由で呼ぶ Tool のほう。
- 承認は `AgentRun` の write 権限で判断する。専用の `approval:decide` 権限は
  設けていない（`06-API.md` §4.1 の表に無いため）。
- Workflow の capability ノードは、read 以外なら**承認ノードを経路上に持つ定義**
  だけを許可する。判定は「定義のどこかに承認ノードがある」ではなく、`start` から
  その node への経路上に承認があること（状態を `(node, 承認済み)` に広げて到達判定）。
  承認済みでも、実行主体は run を起動した人間のまま。run に `requested_by` が
  無い場合（026 以前の run）は身元が要る能力を実行せず `CAPABILITY_UNSUPPORTED`。
- REST からのテスト実行は `guard_kind` を列挙で検査し、**`PRODUCTION` は受け付けない**
  （設計は「正式環境での実行は承認必要」だが、REST 側に承認を通す経路が未接続）。
  素通しにすると、実行権限だけを持つ人が承認なしで本番に対して流せる。
  未知の値（`PROD` など）も 400。破壊的ケースの判定は**許可リスト**
  （`LOCAL` / `STAGING` 以外は実行しない）で、未知の環境名でも fail-closed。
- `test.run.execute` は `CreateTestRunRequest`（`name` / `plan_id` / `environment`）を
  run の `input` から受け取る。生成した run は `QUEUED` で返り、worker が
  `case_count` を確定して `READY` にする。**Runner は起動しない**（実行は別工程）。
- `core.project_of` は `CREATE OR REPLACE` で全体書換えする。016 は 015 の
  15 分支を保持し、ai の 4 分支を追加した。`verify-db.sh` に回帰検査がある。

---

## 5. 再現

```bash
# 空 DB に 001〜027 と roles を適用
PGHOST=localhost PGUSER=<admin> ./scripts/verify-db.sh

# 既存 DB へ 016 を足す場合
psql -d <db> -v ON_ERROR_STOP=1 -f migrations/016_ai.sql
psql -d <db> -f deployment/postgres/roles.sql

# API / worker（AI 未設定のまま）
PORT=18086 DATABASE_URL=postgres://systivra_app@localhost:5432/<db> \
SESSION_SECRET=$(openssl rand -hex 32) SESSION_COOKIE_SECURE=false \
PUBLIC_BASE_URL=http://127.0.0.1:18086 SYSTIVRA_DATA_DIR=$PWD/data \
  ./target/debug/systivra-api
DATABASE_URL=postgres://systivra_scheduler@localhost:5432/<db> WORKER_TICK_SEC=1 \
SYSTIVRA_DATA_DIR=$PWD/data ./target/debug/systivra-worker

PGHOST=localhost PGUSER=<admin> PGDATABASE=<db> \
SYSTIVRA_BASE_URL=http://127.0.0.1:18086 python3 scripts/phase7-eval.py

cd apps/web && WEB_BASE=http://localhost:15200 npm run check:ui:phase7

# Agent のツール呼び出しループ + Q-01 推定層（独立 DB / API 18088 / mock LLM 11534）
PGHOST=localhost PGUSER=<admin> python3 scripts/phase7-agent-loop-eval.py

# 外部 MCP Client（独立 DB / API 18089 / HTTP + stdio mock / UI 15175）
PGHOST=localhost PGUSER=<admin> python3 scripts/phase7-mcp-eval.py
```

AI を接続する場合は `AI_BASE_URL`（例 `http://127.0.0.1:11434/v1`）と
`AI_MODEL_FAST` / `AI_MODEL_REASONING` / `AI_MODEL_LONG_CONTEXT` を設定する。
`AI_BASE_URL` が無ければ `DisabledAiProvider` になる。
