# 詳細設計 09：テスト実行（Test Runner / Execution）

対象：`test.test_run` を作り、`test.test_case` の DSL を実行し、成果物を残し、被覆と欠陥につなぐまで。

```text
Q-01「どうテストするか」（detail/08 §6）        本稿
  既存ケース / 草稿 / 手順 / 未カバー   ──→   計画（DRAFT）
                                              ↓ 人間が承認（T-03）
                                             実行（T-05）──→ 成果物
                                                              ↓
                                                     被覆（T-06） / 欠陥（T-07）
```

`detail/08` は**提案**まで。本稿は**実行**。この 2 つを分けないと「Q-01 を見ただけで CI が走る」事故が起きる。

---

## 1. 位置づけと正本

> **2026-11-08 の実装状況**（詳細は `docs/phase6/acceptance.md`）。
> `workers/runner/` が本稿の JSON DSL を実行し、`executions:import` に
> `execution_source=PLAYWRIGHT` を付けて結果を戻す。
> 1 case = 1 context、4 並列、失敗時のみ 4 点成果物と HAR、HAR は伏せ字化。
> SAMPLE **24 画面 / 300 ルール**の合成 fixture と正解集を追加し、
> 1,200 組のデータ派生・100 ケース / 4 並列 12.3 秒・初回合格率 100% を実測した。
> `storage_state` の鮮度確認、データ seed / reset、成果物の保持期限（90 日 / trace 30 日）
> と容量上限（trace 20 MB / run 20 GB、`TRUNCATED` / `EXPIRED`）、
> §13 の失敗モード（先頭 3 件全滅 → `ENV` 中断、flaky 集計、`DATA` の reset 強制、
> 成果物 0 件の `UPLOAD_FAILED`、環境ロック待ち）を実装し E2E で実測した。
> Runner 単体テストは CI に登録した。
> **未実装**: Collector の分離、16 ワーカー複数ノード、実案件 90 秒/ケースの性能。
> 100 ケース / 4 並列は合成静的 SAMPLE で約 12〜15 秒であり、
> 実案件の 90 秒/ケースを裏付ける計測ではない。

| 項目 | 正本 |
|---|---|
| DSL の語彙・Playwright Runner の採用判断 | `04-コアエンジン.md` §4.6 |
| 被覆 5 指標の定義 | `04-コアエンジン.md` §4.6「被覆定義」 |
| テーブル定義 | `detail/02-database-ddl.md`（`test` スキーマ） |
| API（実行 / 進捗 / 単回 / 被覆 / 欠陥） | `06-API.md` §3.3、`detail/04-api-contract.md` §0 |
| 画面 | `07-UI.md` T-01〜T-07 |
| 速度 | `detail/03-performance.md` §1 |
| Q-01 への供給 | `detail/08-change-inquiry.md` §6 / §7 |

### 1.1 やらないこと

| やらない | 理由 |
|---|---|
| `.spec.ts` の生成 | sandbox で任意コード実行になる。検証できず、Playwright 版更新で全ケースが壊れる（§4.6 で既に不採用） |
| DSL からの任意 JavaScript 実行 | `custom_step` は既定無効。有効化は管理者 + allowlist |
| テストの自動承認 | `status=DRAFT` → `APPROVED` は `test:approve` を持つ人間だけ |
| Q-01 からの直接実行 | `detail/08` §7。計画に入れるまで |
| 本番環境での破壊的ケース | `guard.kind=PRODUCTION` では `@destructive` を実行しない（§6.4） |
| AI による合否判定 | 合否は DSL の判定式だけ。AI は失敗の要約（`narrative`）まで |

---

## 2. 規模と前提

`reference/scale-model.md` の A-02 から導く。

| 項目 | 値 | 導出 |
|---|---|---|
| テスト対象画面 | **1,000+** | JSP 1,000 本 ≒ 画面 1,000 |
| 1 画面あたりケース | 6 | 正常 1 / 境界 2 / 異常 2 / 権限 1 |
| 横断ケース | 500 | 二重送信、状態遷移、ロールバック、同時実行 |
| **総ケース数** | **7,000** | 1,000 × 6 + 500 = 6,500 → 切上げ |
| 1 変更あたりの回帰 | 30〜100 | 影響画面 3〜10 × 6 + 横断 |
| 全回帰 | 7,000 | 夜間バッチ |
| 1 ケース平均実行時間 | **90 秒** | 画面遷移 3〜4、入力 10〜20、待機込み |
| 1 ケース P95 | 4 分 | 一覧画面の全件描画、帳票プレビュー |

> **なぜ 90 秒か**。1 ステップ 10 秒上限（§4.6）で、平均ケースは 8〜12 ステップ。全ステップが成功する限り 10 秒に張り付かない。90 秒は「画面描画待ち + 入力 + 判定」の実測想定値であり、Phase 6 W1 に SAMPLE 24 画面で実測して置き換える。

**キャパシティの帰結**：

- 1 変更の回帰 100 ケースは 4 並列で 45 分 → **受入 60 分に収まる**（§10）。
- 全回帰 7,000 ケースは 16 並列で 11 時間 → 夜間で回る。32 並列なら 5.5 時間。
- したがって Runner の設計目標は「速さ」ではなく **「並列度を上げても壊れないこと」**（環境ロック、データ衝突、成果物の量）。

---

## 3. 全体像

3 プロセスに分ける。同居させない。

```text
┌──────────────┐   ① enqueue     ┌─────────────────┐
│  API         │ ──────────────→ │ job (core.job)  │
│  POST        │                 │ kind=TEST_RUN   │
│  /test-runs  │ ←── 202 ─────── │ payload=run_id  │
└──────────────┘                 └────────┬────────┘
                                          │ ② claim（SKIP LOCKED）
                                 ┌────────▼─────────┐
                                 │  Scheduler       │  環境ロック、並列度、優先度
                                 │  （Rust）        │  case → execution へ展開
                                 └────────┬─────────┘
                                          │ ③ N 個の case ジョブ
                        ┌─────────────────┼─────────────────┐
                        │                 │                 │
                 ┌──────▼─────┐    ┌──────▼─────┐    ┌──────▼─────┐
                 │ Runner #1  │    │ Runner #2  │ … │ Runner #16 │
                 │ browser 1  │    │ browser 1  │    │ browser 1  │
                 │ context 逐次│    │ context 逐次│    │ context 逐次│
                 └──────┬─────┘    └──────┬─────┘    └──────┬─────┘
                        │ ④ 結果 + 成果物                    │
                        └─────────────────┼─────────────────┘
                                          │
                              ┌───────────▼────────────┐
                              │ Collector              │
                              │ test_execution 更新     │
                              │ artifact 登録           │
                              │ 欠陥候補の fingerprint  │
                              └───────────┬────────────┘
                                          │ ⑤ 全 case 完了
                              ┌───────────▼────────────┐
                              │ Coverage 計算 + snapshot│
                              └────────────────────────┘
```

**分離の理由**：

| 分離 | 理由 |
|---|---|
| Scheduler と Runner | Runner はクラッシュする（ブラウザは落ちる）。Scheduler が生きていれば再配置できる |
| Runner ごとに browser 1 つ | case ごとに browser を起動すると 1.5 秒 × 7,000 = 3 時間の純損失 |
| case ごとに context を新規作成 | cookie / localStorage の持ち越しを防ぐ。context 作成は約 150ms |
| Collector を別に | 成果物アップロードの失敗で case の合否を落とさない |

---

## 4. ステップ DSL

### 4.1 形

`test.test_step`（1 行 = 1 ステップ）と、API / ファイル入出力では同じものを JSON で表す。

```json
{
  "v": 1,
  "case_code": "TC-USER-EDIT-004",
  "title": "住所に半角英字を入力した場合、エラーメッセージが表示される",
  "preconditions": ["ログイン済み（権限: 一般ユーザ）", "ユーザ編集画面を開いている"],
  "steps": [
    {"action": "goto",         "target": {"path": "/user/edit?id=1"}},
    {"action": "fill",         "target": {"label": "住所"}, "value": "Tokyo"},
    {"action": "click",        "target": {"role": "button", "name": "登録"}},
    {"action": "field_error",  "target": {"label": "住所"}, "value": "住所は全角20文字以内で入力してください"}
  ],
  "data": {"ref": "dataset://user-addr-boundary@2"},
  "automation": {"type": "playwright", "selector_strategy": "role_first"}
}
```

`v` は必須。将来の語彙追加で既存ケースを壊さないため、Runner は**未知の `action` を `SKIPPED` にする**（`ERROR` にしない）。これで DSL の追加が後方互換になる。

### 4.2 アクション一覧（v1。これで足りないなら語彙を足す。`custom_step` に逃げない）

| action | 引数 | 意味 | 判定 |
|---|---|---|---|
| `goto` | `target.path` または `target.url` | 遷移 | 失敗 → `SELECTOR_NOT_FOUND` ではなく `NETWORK` |
| `fill` | `target`, `value` | 入力（既存値を消してから） | — |
| `select` | `target`, `value` | 選択 | — |
| `check` / `uncheck` | `target` | チェック | — |
| `click` | `target` | クリック | — |
| `press` | `target?`, `value` | キー（`Enter` 等） | — |
| `upload` | `target`, `value`（fixture 参照） | ファイル添付 | — |
| `wait_for` | `target` または `ms` | 待機 | `ms` は 5,000 上限 |
| `assert_text` | `target?`, `value`, `match` | テキスト検証 | **判定** |
| `assert_value` | `target`, `value` | 入力値検証 | **判定** |
| `assert_visible` / `assert_hidden` | `target` | 表示 | **判定** |
| `assert_count` | `target`, `value` | 件数（一覧） | **判定** |
| `assert_url` | `value`, `match` | URL | **判定** |
| `assert_table` | `target`, `rows` | 表の行一致（部分一致可） | **判定** |
| `field_error` | `target`, `value?` | 項目エラー表示 + 文言 | **判定** |
| `screenshot` | `value`（名前） | 記録 | — |
| `custom_step` | `name`, `args` | **既定無効**。管理者が allowlist に載せたものだけ | 実装依存 |

`match`: `exact`（既定） / `contains` / `regex`。`regex` は **RE2 構文のみ**（破滅的後方参照を排除）。日本語を正規表現で書くと全角・半角で破綻するので、`contains` を推奨し、UI でも既定を `contains` にする。

### 4.3 ターゲット解決

`selector_strategy` は既定 `role_first`。上から順に試し、**最初に一意に決まったもの**を使う。

| 優先 | 指定 | 解決 | 落とし穴 |
|---|---|---|---|
| 1 | `testid` | `[data-testid="..."]` | 最も安定。新規実装ではこれを埋める（U-03） |
| 2 | `role` + `name` | ARIA role + アクセシブル名 | 日本語の `name` は全角スペース混入に注意 |
| 3 | `label` | `<label>` の関連付け | JSP で `for` が無いと解決できない |
| 4 | `text` | 可視テキスト完全一致 | 文言変更で壊れる。表示文言を変える案件では使わない |
| 5 | `css` | CSS セレクタ | **最後の手段**。`nth-child` を含むと `selector_brittle=true` |

- 解決結果が **0 件** → `SELECTOR_NOT_FOUND`。**2 件以上** → 同じく `SELECTOR_NOT_FOUND`（`strict mode violation`）。曖昧なまま先頭を押さない。
- 解決した要素は `step_results[].resolved` に残す（`{strategy, selector, text}`）。失敗時に「なぜその要素を選んだか」を後から追えるようにする。
- `selector_brittle=true` のケースは T-05 で警告表示し、被覆には算入するが「安定性スコア」を別に下げる。

### 4.4 データバインディング

`value` は次の 3 形だけ。

| 形 | 例 | 解決 |
|---|---|---|
| リテラル | `"東京都"` | そのまま |
| データ参照 | `"${data.address}"` | `test.test_data_row` から |
| fixture 参照 | `"fixture://csv/user-100@1"` | オブジェクトストレージの固定ファイル |

**禁止**：環境変数参照、シークレット参照、式展開（`${a + b}`）。DSL はデータであり、プログラムではない。

### 4.5 DSL の検証（保存時。実行時ではない）

`test_case` の保存 / 承認時に JSON Schema で検証し、次を**拒否**する。

| 拒否する内容 | 理由 |
|---|---|
| 未知の `action`（`v` が同一なのに語彙にない） | 実行時に静かに `SKIPPED` になるのを防ぐ |
| `wait_for.ms > 5000` | 固定待機は flaky の温床 |
| `css` に `nth-child` / `:has(` | 構造変更で即壊れる |
| `target.url` が `environment.base_url` のオリジン外 | 外部サイトを踏ませない |
| 判定アクションが 0 件のケース | 何も確かめていないケースを `APPROVED` にしない |
| `value` に `${env.` / `${secret.` | §4.4 |

検証は `POST /test-scenarios/{sid}/cases:generate` と `POST /test-cases/{cid}:approve` の両方で走らせる。生成時だけだと、手で直した後に壊れる。

---

## 5. 環境定義

### 5.1 `environment`

```json
{
  "id": "env-stg-01",
  "base_url": "https://stg.example.internal/app",
  "guard": {"kind": "STAGING", "allow_destructive": true, "allowed_origins": ["stg.example.internal"]},
  "auth": {"type": "storage_state", "ref": "secret://env-stg-01/storage.json", "max_age_min": 480},
  "data": {"seed": "fixture://seed/user-basic@3", "cleanup": "RESET_ENDPOINT", "reset_path": "/test/reset"},
  "locale": {"lang": "ja-JP", "timezone": "Asia/Tokyo", "encoding": "utf-8"},
  "viewport": {"width": 1280, "height": 900},
  "limits": {"parallel": 4, "case_timeout_sec": 180}
}
```

`guard.kind` は 3 値。

| kind | `allow_destructive` | 用途 |
|---|---|---|
| `LOCAL` | 自由 | 開発者ローカル |
| `STAGING` | 設定可（既定 `true`） | 常用。テストデータを壊してよい |
| `PRODUCTION` | **`false` 固定** | 参照系のみ。`@destructive` は実行前に `SKIPPED` |

`PRODUCTION` を選ぶには `test:execute` に加えて `X-Confirm-Action`（`06-API.md` §4.2）と承認が要る。**設定で `PRODUCTION` にできても、実行はできない**ようにする（二重の歯止め）。

### 5.2 認証

- `storage_state`（Playwright の保存済みセッション）を既定にする。ログインをケースのステップに書かない。ログインを各ケースで繰り返すと、失敗の原因が「ログイン」に汚染される。
- `max_age_min` を超えた `storage_state` は Runner が**実行を止める**（`ENV` 失敗）。期限切れのまま流すと全ケースが `SELECTOR_NOT_FOUND` になり、原因究明に時間を溶かす。
- `auth.type=form` も許すが、その場合のログイン手順は `environment.auth.steps` に 1 箇所だけ書き、全ケースで共有する。

> **2026-11-08 の実装**: Runner は `STORAGE_STATE` と `STORAGE_STATE_MAX_AGE_MIN`
> （既定 480）を受け取り、mtime が古い / ファイルが無い場合は run を作らずに
> `ENV` で中断する。フォームログインの共有手順は未実装（`storage_state` のみ）。

### 5.3 データ

| 方式 | 意味 | 使う場面 |
|---|---|---|
| `NONE` | 何もしない | 参照系のみ |
| `SEED` | 実行前に fixture を投入 | 登録・更新系 |
| `RESET_ENDPOINT` | `reset_path` を叩いて初期化 | 被テストシステムにリセット API がある |
| `SNAPSHOT` | DB スナップショット復元 | 重い。夜間の全回帰 |

**同じ環境で 2 つの run を同時に流さない。** `environment.id` 単位でアドバイザリロックを取る（`pg_try_advisory_lock(hashtext(env_id))`）。取れなければ `QUEUED` のまま待つ。データ投入が衝突すると、失敗の原因が「テストのバグ」ではなく「他の run」になる。

> **2026-11-08 の実装**: ロックは部分一意索引による「活性 run は 1 つ」で表現し、
> Runner は `queued_behind` のとき `READY` まで待ってから実行する。
> データは `DATA_MODE=NONE/SEED/RESET_ENDPOINT` + `SEED_URL` / `RESET_URL` で
> run 開始時に投入 / 初期化する。`SNAPSHOT` は未実装。

### 5.4 ロケール

`lang=ja-JP` / `timezone=Asia/Tokyo` を Runner が必ず設定する。ここを省くと、和暦表示・日付境界・全角入力の検証が開発者の端末設定に依存する。CI に載せた瞬間に落ちる類の壊れ方をする。

---

## 6. Runner

### 6.1 ワーカー

| 項目 | 値 | 備考 |
|---|---|---|
| 1 ワーカー = browser プロセス | 1 | chromium 既定 |
| 1 ワーカーが同時に持つ context | 1 | 逐次。並列はワーカー数で稼ぐ |
| ワーカーあたりメモリ | 400〜600 MB | context 1 + page 1〜3 |
| 16 ワーカーのノード | 16 GB | 余裕を見て 2 GB/ワーカー |
| browser 起動 | run 開始時に 1 回 | case ごとに起動しない |
| context 作成 / 破棄 | case ごと | 約 150ms |

### 6.2 ライフサイクル

```text
run 開始
  → 環境ロック取得（取れなければ待機）
  → storage_state の鮮度確認（期限切れ → ENV で中断）
  → data.seed（方式に応じて）
  → browser 起動（ワーカー数分）
  → case を 1 件ずつ claim（SKIP LOCKED）
       → context 作成（storage_state を注入、locale 設定）
       → step を順に実行（各 10 秒上限）
       → 判定失敗 → 残りステップを SKIPPED にし、成果物収集
       → Collector へ結果送信
       → context 破棄
  → 全 case 完了 or 中断
  → Coverage 計算 + snapshot 保存
  → 環境ロック解放、browser 終了
```

### 6.3 タイムアウト階層

| 層 | 既定 | 上限 | 超過時 |
|---|---|---|---|
| 1 ステップ | 10 秒 | 30 秒 | そのステップを `FAILED`（`TIMEOUT`）、ケースは打切り |
| 1 ケース | 180 秒 | 600 秒 | `TIMEOUT` |
| 1 run | 24 時間 | 24 時間 | 残りを `SKIPPED`、run を `CANCELLED` |
| 待機（`wait_for`） | 指定値 | 5,000ms | 保存時に拒否（§4.5） |

**暗黙待機**：`click` / `fill` の前に対象が可視・有効になるまで最大 5 秒待つ（Playwright の既定挙動に合わせる）。この 5 秒を DSL に書かせない。

### 6.4 再試行

| `failure_kind` | 再試行 | 理由 |
|---|---|---|
| `TIMEOUT` | **1 回** | 環境の一時的な遅さの可能性 |
| `NETWORK` | **1 回** | 同上 |
| `ASSERTION` | **しない** | 欠陥を隠す |
| `SELECTOR_NOT_FOUND` | **しない** | 画面が変わった。再試行しても変わらない |
| `DATA` / `ENV` / `RUNNER` | **しない**（run を止める） | 環境の問題。ケースを回しても無駄 |
| `CANCELLED` | しない | — |

再試行は `test.test_execution.attempt` を +1 して**別行**で残す。上書きしない。「1 回目は落ちて 2 回目は通った」は記録すべき事実であり、flaky の唯一の証拠。

### 6.5 並列度の上げ方

`environment.limits.parallel` を超えないこと。加えて、run 全体の並列度はノード数 × ワーカー数で上限を切る。

```text
実効並列 = min(run.parallel, 空きワーカー数, 環境ロック保持者 1)
```

**環境ロックが 1 なので、同じ STAGING に対する並列は「1 run 内のワーカー」だけ**。複数 run を同時に流したければ環境を分ける。ここを緩めると、テストデータの取り合いで失敗率が跳ね上がる。

---

## 7. 失敗分類（`failure_kind`）

`test.test_execution.failure_kind` に入る値はこの 8 つだけ。CHECK 制約で縛る。

| 値 | 意味 | 典型例 | 欠陥候補 |
|---|---|---|---|
| `ASSERTION` | 期待と実際が違う | エラー文言が違う | **する** |
| `SELECTOR_NOT_FOUND` | 要素が 0 件 or 複数 | `data-testid` が消えた | **する**（実装変更なら） |
| `TIMEOUT` | 制限時間超過 | 一覧の描画が 10 秒超 | する（性能劣化） |
| `NETWORK` | 通信失敗 / 5xx | 画面が 500 を返す | **する** |
| `DATA` | データ起因 | 一意制約違反、前提データ欠落 | しない（テスト側） |
| `ENV` | 環境不備 | 認証切れ、reset 失敗、URL 不達 | しない |
| `RUNNER` | 実行基盤の異常 | ブラウザクラッシュ、成果物書込失敗 | しない |
| `CANCELLED` | 中止 | 人が止めた | しない |

**`DATA` と `ASSERTION` を混ぜない**ことが運用上いちばん効く。混ぜると「テストが落ちている」のか「テストが悪い」のかが分からなくなり、誰もテストを信じなくなる。

> **2026-11-08 の実装**: `ASSERTION` の期待値・実測値に重複 / 一意制約 /
> 前提データ欠落の語が現れた場合だけ `DATA` へ寄せる（`classifyStepError`）。
> Runner は `DATA` を受けると reset を 1 回強制し、run を止める。
> 先頭 3 件が `SELECTOR_NOT_FOUND` / `TIMEOUT` / `NETWORK` で全滅した場合は
> 3 件を `ENV` に再分類して run を止め、残りを `SKIPPED` として取り込む。

---

## 8. 成果物（artifacts）

### 8.1 置き場所

```text
s3://sep-artifacts/{project_id}/{test_run_id}/{execution_id}/
    failure.png           スクリーンショット（失敗時）
    failure.html          DOM スナップショット
    console.log           console + pageerror
    trace.zip             Playwright trace（失敗時のみ）
    network.har           失敗ステップ前後 30 秒
    step-04.png           明示 `screenshot` アクションの出力
```

`test.test_execution.artifacts` は**参照のみ**を持つ（`[{"kind":"TRACE","uri":"s3://...","bytes":8123456}]`）。PG に実体を入れない。

### 8.2 取得方針

| 成果物 | 成功時 | 失敗時 |
|---|---|---|
| スクリーンショット | 最終画面のみ（`keep_on_pass=true` のとき） | 失敗ステップ + 最終 |
| DOM | なし | あり |
| console | なし | あり（`error` / `warning`） |
| trace | **なし** | あり |
| HAR | なし | あり |

**成功時に trace を残さない**理由：trace は 1 ケース 5〜20 MB。7,000 ケースで 100 GB を超える。失敗時だけ取れば、容量は失敗率に比例する（通常 5% 未満）。

### 8.3 保持と上限

| 項目 | 既定 | 変更 |
|---|---|---|
| 保持期間（画像 / DOM / log） | 90 日 | プロジェクト設定 |
| 保持期間（trace） | 30 日 | 同上 |
| 1 trace の上限 | 20 MB | 超過は trace を切る（`artifacts` に `TRUNCATED` を記録） |
| 1 run の合計上限 | 20 GB | 超過したら以降 trace 無効 |

保持期限は `artifacts.uri` を消すのではなく、オブジェクトを削除して `artifacts` に `EXPIRED` を立てる。**リンク切れではなく「期限切れ」と分かる**ようにする。

> **2026-11-08 の実装**: Runner が trace 20 MB 超過を切り詰めて `status=TRUNCATED` を、
> run 合計 20 GB 超過後の trace 省略を記録する。worker が起動時 + 24 時間間隔で
> 期限（90 日 / trace 30 日、`ARTIFACT_RETENTION_DAYS` / `TRACE_RETENTION_DAYS`）を
> 適用し、`POST /projects/{pid}/test/artifacts:expire` と
> `POST /projects/{pid}/test/runs/{rid}/artifacts:cleanup` で手動適用 / run 単位削除ができる。
> 期限判定はファイルの mtime。実体は `ARTIFACT_ROOT`（既定はリポジトリ直下）配下の
> ローカル FS で、S3 は未実装。

---

## 9. 被覆計算

### 9.1 5 指標

`04-コアエンジン.md` §4.6 の定義を、そのまま SQL にする。

```sql
-- 対象 run の「合格したケース」
WITH passed AS (
  SELECT DISTINCT e.test_case_id
    FROM test.test_execution e
   WHERE e.test_run_id = $1
     AND e.status = 'PASSED'
     AND e.attempt = (SELECT max(attempt) FROM test.test_execution
                       WHERE test_run_id = $1 AND test_case_id = e.test_case_id)
),
appr AS (
  SELECT tc.* FROM test.test_case tc WHERE tc.project_id = $2 AND tc.status = 'APPROVED'
)
SELECT
  -- Rule Coverage: 合格ケースが covers_rules に持つルール / 対象ルール
  (SELECT count(DISTINCT r) FROM appr a, unnest(a.covers_rules) r
    WHERE a.id IN (SELECT * FROM passed))
    ::numeric / NULLIF((SELECT count(*) FROM test.validation_rule WHERE project_id = $2), 0) AS rule,
```

4 指標（`rule` / `screen` / `impact` / `regression`）は PG だけで閉じる。**`requirement` だけは閉じない**（§9.2）。

### 9.2 Requirement Coverage の扱い

`test.test_case` は `covers_rules` と `covers_impacts` しか持たない。要件までは PG で辿れない。したがって：

```text
Requirement Coverage
  = (:TestCase)-[:VERIFIES]->(:Requirement) を持ち、かつ
    その TestCase が当該 run で PASSED である Requirement 数
    / 対象 Requirement 数
```

グラフ（`detail/07`）に対して 1 クエリで取る。

```cypher
MATCH (r:Requirement {project_id: $pid})
OPTIONAL MATCH (tc:TestCase)-[:VERIFIES]->(r)
WITH r, collect(tc.uid) AS tcs
RETURN count(r) AS total,
       count(CASE WHEN any(uid IN tcs WHERE uid IN $passed_uids) THEN 1 END) AS covered
```

**縮退**：グラフ未構築、または `VERIFIES` 辺が未生成（Phase 6 前半）のときは、`Rule --DERIVED_FROM--> Requirement` で近似し、`coverage_snapshot.detail.degraded = ["REQUIREMENT_VIA_RULE"]` を立てる。近似値であることを**画面に出す**（T-06 に「近似」バッジ）。黙って別の数字を出さない。

> **2026-11-08 の実装**: 被覆は PG の `graph_meta` を直接読む
> （`TestCase.natural_key = test_case.id`、`VERIFIES` は `resolution=INFERRED`）。
> `derive` が Requirement（`natural_key = rule:<uuid>`）/ TestCase / `VERIFIES` を投影し、
> **同じ transaction で `core.outbox_event` にも積む**。worker の projector が
> Neo4j へ MERGE し、PG と Neo4j の件数・解決状態が一致することを実測した
> （`scripts/phase6-graph-eval.py`）。実要件の抽出は未接続のため Requirement は
> ルール由来であり、`REQUIREMENT_INFERRED_FROM_RULE` を立てる。
> `VERIFIES` 辺が 0 件のときだけ 0 + `REQUIREMENT_VIA_RULE` / `GRAPH_VERIFIES_UNAVAILABLE`
> に縮退する。被覆計算は PG を読み、Neo4j の有無に依存しない。

### 9.3 算入除外（再掲。ここが解釈の余地になる）

算入しない：`DRAFT` のケース / 一度も実行していないケース / `BLOCKED` `SKIPPED` / `covers` が空のケース。

**「生成したが未実行」は被覆 0**。`08-実装計画.md` Phase 6 の受入と同じ。ここを「生成したから 1」にすると、被覆率が意味を失う。

### 9.4 スナップショット

`test.coverage_snapshot` は run 完了時に 1 行。`scope` に `{test_run_id, plan_id?, case_filter}`、`detail` に分子分母を全部入れる。

```json
{
  "requirement": {"num": 182, "den": 240, "degraded": ["REQUIREMENT_VIA_RULE"]},
  "rule":        {"num": 271, "den": 300},
  "screen":      {"num": 19,  "den": 24},
  "impact":      {"num": 33,  "den": 37},
  "regression":  {"num": 91,  "den": 100}
}
```

分子分母を残す理由：率だけだと「分母が縮んで率が上がった」を見抜けない。**同じ test_run からは常に同じ数値**（`08-実装計画.md` 受入）。再計算は `POST /projects/{pid}/coverage?test_run_id=` で冪等に行う。

---

## 10. 欠陥化

### 10.1 自動起票は既定でしない

`test.defect` は**人間が起票**する（`POST /test-executions/{eid}/defects`）。既定で自動起票しない理由：

- `ASSERTION` の多くは「期待値が古い」であり、欠陥ではない。
- 自動起票すると欠陥一覧がノイズで埋まり、T-07 が使われなくなる。

代わりに T-05 で「欠陥候補」として上位に出す。候補判定は §7 の「欠陥候補」列 + 同一 fingerprint の発生回数。

### 10.2 fingerprint（重複排除）

```text
fingerprint = sha256(
    test_case_id
  + failure_kind
  + 最初に失敗した step の index
  + normalize(expected) + "\u0000" + normalize(actual)
)

normalize: NFKC → 連続空白を 1 個 → 前後 trim → 数字列を <N> に置換
```

数字を `<N>` に潰すのは、「3 件」と「4 件」の差で別欠陥にしないため。ただし**潰しすぎると別の欠陥が合流する**ので、`<N>` 置換は `assert_count` に限る。

同一 fingerprint の open 欠陥があれば新規起票を促さず、既存欠陥に「再発」として実行を紐付ける。`test.defect.execution_id` は代表 1 件だけ持つ設計なので、再発回数は `detail` 側で集計する（スキーマを増やさない）。

### 10.3 欠陥 → 影響項目

`test.defect.impact_item_id` は `ImpactReport.items[].id`。これがあることで「この欠陥は、どの変更の、どの影響項目を守れなかったのか」が言える。**ここが閉ループの終点**。

---

## 11. 速度

### 11.1 見積り式

```text
wall_sec = ceil(cases × avg_case_sec / parallel)
         + launch_overhead
         + failure_overhead
         + seed_overhead

launch_overhead  = parallel × 1.5s
failure_overhead = fail_count × 12s（成果物収集 + trace 出力）
seed_overhead    = 方式依存（SEED なら 10〜60s、SNAPSHOT なら 5〜20 分）
```

### 11.2 受入値の根拠（100 ケース / 4 並列 / 60 分）

| 項 | 計算 | 値 |
|---|---|---|
| 純実行 | 100 × 90 / 4 | 2,250s = 37.5 分 |
| 起動 | 4 × 1.5s | 6s |
| 失敗成果物（5% と仮定） | 5 × 12s | 60s |
| seed（`SEED`） | — | 30s |
| データ初期化 | — | 20s |
| **合計** | | **約 40 分** |

受入 `< 60 分` に対して 20 分の余裕。この余裕は「P95 のケースが数件混ざる」ことに使う。**余裕を並列度を下げる方向に使わない**（並列を上げても壊れない設計であることが §6.5 の主張）。

### 11.3 全回帰

| 並列 | 純実行 | 実効（+10% オーバーヘッド） | 判断 |
|---|---|---|---|
| 8 | 21.9h | 24h | 1 晩で回らない |
| 16 | 10.9h | **12h** | 夜間で回る（既定） |
| 32 | 5.5h | 6h | 週末 + 日中にも回せる |

`detail/03-performance.md` §1 の「テスト実行（100 ケース）< 60min / < 30min」と整合。**30 分の目標値**は「失敗 0 件、P95 が 2 件以下」のときだけ達成できる。平均が 90 秒から 70 秒に下がれば 30 分に入る。Phase 6 W1 の実測で置き換える。

### 11.4 遅い原因の切り分け（T-05 に出す）

| 症状 | 見る場所 |
|---|---|
| 全ケースが一律に遅い | seed / 環境。Runner ではない |
| 特定画面だけ遅い | 被テストシステムの性能。`TIMEOUT` が並ぶ |
| 待機で稼いでいる | `step_results[].duration_ms` の上位。`wait_for` の多用 |
| 起動が遅い | browser を case ごとに起動していないか（§6.1） |

---

## 12. 安全

| 危険 | 対策 |
|---|---|
| 本番を壊す | `guard.kind=PRODUCTION` は `@destructive` を `SKIPPED`。さらに `allow_destructive` を `true` にできない（スキーマで固定） |
| 外部サイトを踏む | `allowed_origins` 外への `goto` を保存時に拒否（§4.5） |
| 任意コード実行 | `custom_step` 既定無効。`.spec.ts` 生成なし |
| シークレットの露出 | DSL に書けない（§4.4）。Runner が secret manager から取得。成果物の HAR / console は `Authorization` / `Cookie` を伏せる |
| 認証情報の残留 | context を case ごとに破棄。`storage_state` は毎 run 読み直す |
| テストデータの汚染 | 環境ロック（§5.3）。`cleanup` を必ず定義 |
| 暴走 | run 24h 上限、step 10s 上限、`wait_for` 5s 上限 |
| 成果物の肥大 | trace は失敗時のみ、run 合計 20 GB 上限 |

**HAR のマスキング**は必須。`Cookie` / `Set-Cookie` / `Authorization` / `X-CSRF-TOKEN` を伏せ字にする。伏せ字を忘れると、成果物がそのまま認証情報の置き場になる。

---

## 13. 失敗モード

| # | 症状 | 原因 | 扱い |
|---|---|---|---|
| 1 | 全ケースが `SELECTOR_NOT_FOUND` | `storage_state` 期限切れでログイン画面に飛んでいる | 最初の 3 件で `ENV` に切替え、run を止める（全滅を防ぐ） |
| 2 | 全ケースが `TIMEOUT` | 環境が落ちている / ネットワーク断 | 同上 |
| 3 | 同じケースだけ毎回落ちる | 本物の欠陥 or 期待値が古い | `ASSERTION`。T-03 で期待値を直すか欠陥起票 |
| 4 | 1 回目だけ落ちる | flaky（アニメーション、非同期描画） | `attempt` が 2 以上で PASSED のケースを flaky として集計。`wait_for` の追加ではなく、判定を `assert_visible` に寄せる |
| 5 | `strict mode violation` | セレクタが 2 件以上に一致 | `SELECTOR_NOT_FOUND`。`testid` を埋める（U-03） |
| 6 | データ重複で落ちる | 前回の run の `cleanup` 漏れ | `DATA`。run 開始時に `reset` を強制 |
| 7 | 成果物が 0 件 | Collector が落ちた / S3 権限 | case の合否は保つ。`artifacts` に `UPLOAD_FAILED` を記録し、run に警告 |
| 8 | 被覆が前回と違う | 分母（`APPROVED` ケース数）が変わった | 異常ではない。`detail` の分子分母で説明できること |
| 9 | 同じ環境で 2 run が交錯 | 環境ロックが効いていない | 設計違反。ロックは `pg_try_advisory_lock` で取得し、取れなければ `QUEUED` |
| 10 | run が終わらない | ケースが `BLOCKED` のまま collector 待ち | 全 case の終端状態を 5 分ごとに監査し、`RUNNING` が 30 分動かなければ `RUNNER` で落とす |

> **2026-11-08 の実装**: 1 / 2（先頭 3 件全滅 → `ENV` 中断、残り `SKIPPED`）、
> 4（flaky 集計）、6（`DATA` で reset 強制）、7（成果物 0 件 → `UPLOAD_FAILED` + run 警告）、
> 9（環境ロック待ち）を Runner に実装し、`scripts/phase6-failure-eval.py` で実測した。
> 10 の 30 分監査は runner 側のケース上限（180 秒）と run 上限（24 時間）までで、
> 全 case の終端監査は API / worker 側の未実装。

---

## 14. 受入

| ID | 受入項目 | 判定 |
|---|---|---|
| R-01 | 100 ケースを 4 並列で実行し、60 分以内に完了する | 実測 |
| R-02 | 同じ `test_run` から被覆を 2 回計算し、5 指標すべて同値 | 再現性 |
| R-03 | `DRAFT` ケースが被覆に算入されない | 意図的に混ぜて確認 |
| R-04 | 「生成したが未実行」のケースが被覆 0 になる | T-06 表示 |
| R-05 | `ASSERTION` 失敗が再試行されない | `attempt=1` のみ |
| R-06 | `TIMEOUT` が 1 回だけ再試行され、`attempt=2` の行が残る | DB 確認 |
| R-07 | `PRODUCTION` 環境で `@destructive` が `SKIPPED` になる | 設定を変えても実行できないこと |
| R-08 | `allowed_origins` 外への `goto` が保存時に拒否される | 422 |
| R-09 | 失敗時に screenshot / DOM / console / trace の 4 点が出る | 成果物確認 |
| R-10 | 成功時に trace が保存されない | 容量 |
| R-11 | HAR の `Cookie` / `Authorization` が伏せ字 | 目視 + grep |
| R-12 | 同じ環境に 2 run を投入すると、2 つ目が `QUEUED` のまま待つ | ロック |
| R-13 | 同一 fingerprint の失敗が、欠陥候補として 1 件にまとまる | T-05 |
| R-14 | DSL に `wait_for.ms=6000` を入れると保存が拒否される | 422 |
| R-15 | `v=1` に未知の `action` を入れたケースが保存時に拒否される | 422 |
| R-16 | Q-01 から `test-plans:derive` した計画が `DRAFT` で T-01 に現れ、`test_run` が作られない | `detail/08` §7 |
| R-17 | SAMPLE 24 画面 / 300 ルールで、派生ケースの初回合格率が 60% 以上 | `08-実装計画.md` Phase 6 |
R-16 が**この 2 つの詳細設計の接ぎ目**。ここが壊れると「Q-01 を見ただけで CI が走る」に戻る。

---

## 15. 他詳細設計との境目

| 境目 | 本稿 | 相手 |
|---|---|---|
| ルール抽出 / シナリオ派生 | 受け取る | `04-コアエンジン.md` §4.6 |
| ケースの `covers` 生成 | 保存・検証する | `detail/08-change-inquiry.md` §6 |
| テスト計画への受渡し | 実行側 | `detail/08-change-inquiry.md` §7 |
| テーブル | 使う | `detail/02-database-ddl.md`（`test` スキーマ） |
| API 共通規約（UUID / エラー / 202 / 冪等） | 従う | `detail/04-api-contract.md` §0 |
| 影響項目 ID | `covers_impacts` / `defect.impact_item_id` | `04-コアエンジン.md` §4.5 |
| Requirement Coverage のグラフ参照 | クエリを投げる | `detail/07-graph-projection.md` |
| 画面（T-01〜T-07） | ワイヤーは持たない | `07-UI.md` §2.2 / §3 |
| 速度の受入値 | 実行時間 | `detail/03-performance.md` §1 |

---

## 16. 未決（本稿が抱える穴）

| ID | 内容 | 期限 | 影響 |
|---|---|---|---|
| R-a | `data-testid` を被テストシステムに埋められるか（U-03 と同じ） | Phase 6 前 | 埋められない場合、`role` / `label` 依存になり安定性が落ちる。`selector_brittle` の割合が指標になる |
| R-b | 被テストシステムにリセット API があるか | Phase 6 前 | 無ければ `SNAPSHOT` 復元。夜間全回帰が 20 分長くなる |
| R-c | 1 ケース 90 秒の妥当性 | Phase 6 W1 | SAMPLE 24 画面で実測して置換。80 秒を切れば 30 分目標に入る |
| R-d | flaky の許容率 | Phase 6 W3 | 5% を超えると T-05 が信用されない。超えたら `wait_for` ではなく判定の作り直し |
