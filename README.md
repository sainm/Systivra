# Systivra

Software Engineering Intelligence Platform — 設計書・ソースコード・データベース・テストを「証拠」でつなぎ、改修の影響範囲を計算・検証・監査できるようにする分析基盤。

## 設計文書

- 正式版（日本語）: [`docs/design/ja/00-全体設計.md`](docs/design/ja/00-全体設計.md)

## 状態

Phase 0（設計）完了。Phase 1（Platform Foundation + Walking Skeleton）完了。
**Phase 2（RAG Engine 完成）実装済み**。受入は [`docs/phase2/acceptance.md`](docs/phase2/acceptance.md) を参照。
60 問評価は recall@10 1.0（word / partial / identifier・外字 各 1.0）、30 セル領域 top-3 100%、
P95 39ms。1,000 冊 / 1,200 万 chunk の負荷試験、docling-serve、pg_bigm、外部 rerank は
環境制約で未実施または降格（同記録に明記）。

**Phase 3（Graph + Ontology）実装済み**。受入は [`docs/phase3/acceptance.md`](docs/phase3/acceptance.md)。
Ontology v1（56 実体型 / 27 関係型）、PG 正本 + Outbox + Neo4j 投影（冪等・順序保証）、
実体検索 / 隣接 / 経路 / サブグラフ / トレーサビリティ、PgGraphProvider lite（3 ホップ）、
Ontology 閲覧・公開とグラフブラウザを実装。10 万ノードの隣接は合成データで P95 5.3ms。
影ラベル再構築、日次漂流検出、投影器 10 分停止は未実施または降格（同記録に明記）。
Neo4j のローカル検証は 2026.05 Community で、設計の 5.26 LTS とは版が異なる。

**Phase 4（Code Analysis）の source-only 受入は PASS**。`DIR` / `SNAPSHOT` / `GIT` の
同期、元バイト偏移、Java / JSP / XML / SQL の Symbol / Fact、COPY 永続化、増分再利用、
Fact evidence、Neo4j 再投影、API とコード解析画面を実装した。受入記録は
[`docs/phase4/acceptance.md`](docs/phase4/acceptance.md) を参照。
13,000 Java 相当を **21.79 秒 / 91,000 Symbol / 104,000 Fact**、増分 **1 / 13,000**、
250 万 Fact 相当の COPY を **19.253 秒**、Neo4j の 250 万辺再投影を **176.168 秒**で実測。
SVN は未導入。OpenRewrite / 顧客 classpath と解析による 250 万 Fact 生成は外部依存として
受入境界の外に置く。

**Phase 5（Impact Analysis）の今回指定受入は PASS**。既存グラフとコード Fact、
文書 evidence から決定論的な `ImpactAssessmentDraft` を生成する。**OpenRewrite、
`javac`、顧客 classpath は取得・実行しない**。これらは「任意で使える依存」ではなく、
現行の source-only 受入から明示的に除外した境界である。外部型・動的挙動は
`INFERRED / UNRESOLVED` のまま保持する。顧客案件固有の意味的正解も未提供のため、
品質値は合成 SAMPLE の範囲を超えて主張しない。Draft はコード変更もテスト実行も
行わない。Git unified diff / line seed、Field の読み書き閉包、20 変更 Golden set
（recall 1.0 / precision 0.833）、frontier 限定 6-hop **1.07 秒** / 250 万辺、Markdown / PDF、
真の SSE を実測した。受入は [`docs/phase5/acceptance.md`](docs/phase5/acceptance.md)。
RAG 併用 seed は未接続の境界として残す。

**Phase 5 の Q-01（変更影響問合せ）を追補実装**。自然言語 / ENTITY からシードを解決し、
5 セクション（summary / screens / documents / code / tests）を返す。曖昧なら
`NEEDS_CLARIFICATION` で止まり、`counts` は各 `total` と一致する。`/events` は
`text/event-stream` で、`Last-Event-ID` から再開できる。**Markdown / PDF の
ダウンロード**（Q-01 / Impact）も可能。AI を設定すると `/narrate` で
要約を足せる（未設定なら `ai_unavailable` を出し、5 段はそのまま）。Impact も
`POST /impacts/{iid}/narrate` で同じ扱い。叙述は Markdown の先頭にも入る。
画面からは「テスト計画に入れる」で
DRAFT 計画まで落とせる（`test_run` は作らない）。計画を指定した run は
その計画のケースだけを対象にする。実測は
`data/phase5-inquiry/report.json`。

**Phase 6（Test Engineering）は正式受入まで実装**。検証ルールの抽出、シナリオ / ケース /
データの派生、承認、run 起票、実行、被覆 5 指標と欠陥候補までを通した。
`workers/runner/`（Node + Playwright）が DSL を実行し、
`execution_source=PLAYWRIGHT` で結果を戻す。合成 SAMPLE **24 画面 / 300 ルール**で
**1,200 組**のデータを派生し、承認 120 ケースを **4 並列 / 100 ケース 12.3 秒**
（初回合格率 100%、正解集一致 100%）で実測した。§13 の失敗モード
（先頭 3 件全滅 → ENV へ切替えて停止、flaky 集計、DATA は reset 強制、
成果物 0 件は `UPLOAD_FAILED`、環境ロック待ち）、`storage_state` の鮮度確認、
データ seed / reset、成果物の 20 MB（trace）/ 20 GB（run）上限と `TRUNCATED`、
90 日（trace 30 日）保持と `EXPIRED` 削除（API + worker 日次）を実装・実測した。
`TestCase -[:VERIFIES]-> Requirement` は outbox 経由で Neo4j へ投影し、
resolution は `INFERRED`、Requirement は `natural_key=rule:<uuid>`（ルール由来）。
Runner 単体テストは CI に登録した。受入は
[`docs/phase6/acceptance.md`](docs/phase6/acceptance.md)。**Collector 分離 /
16 ワーカー複数ノード / 実案件 90 秒/ケースは環境・顧客データ依存として未検証**。

**Phase 7（AI Agent / MCP / Workflow）は縦断切片を実装**。Agent の登録と版管理、
`AGENT_RUN` job、能力ゲートウェイ（能力名 → 権限 → `project_id` 注入）、
ツール呼び出しループ（`capabilities` と照合し、外れたら run を停止）、
MCP Server の 12 ツール公開（11 ツールは即実行、`test.run_execute` は承認後に 1 回だけ実行）と JSON-RPC、承認ノードで止まる Workflow と再開、
**SVG の可視化エディタ**（配置・接続・保存）、承認の留痕までを通した。
非 read の Workflow ノードは**経路上に承認ノードがある場合だけ**実行でき、
承認が下りるまで実行ノードは走らない。MCP の承認必須ツールは、承認を
**1 回の実行への許可証**として消費する（同じ引数で 1 回だけ）。**AI 未設定でも中核は動き**、`agent_run` は
`AI_UNAVAILABLE` で閉じて生成物を作らない。受入は
[`docs/phase7/acceptance.md`](docs/phase7/acceptance.md)。Q-01 は
`narrative` と、既知 evidence を basis に持つ `INFERRED` 仮説まで接続した。
外部 MCP Client は stdio / HTTP、Tool Schema、health、Agent 呼び出しまで接続し、
Schema 変化は自動適用せず DEGRADED とする。能力の実行主体は**依頼した人間**で、
worker も実行のたびにロールを引き直す（降格を即時反映）。実モデルでのツール選択精度は未検証。

**Phase 8（運用引渡し）は復旧・OIDC SSO・セッション失効・認証強化・SCIM・監査転送の
縦断切片を実装**。`scripts/backup.sh` は暗号化・保持・オフサイトフック付きで PostgreSQL と
原本データを取得し、`scripts/restore-drill.sh` が復号・別 DB 復元・全セッション失効・
clean-room の graph rebuild まで行う（RTO 5.2 秒）。`scripts/pitr-drill.sh` は専用の
一時クラスタで WAL アーカイブと指定時刻復元を実測する（2.85 秒）。
OIDC は discovery / JWKS / 認可コード + PKCE(S256) / state / nonce を検証し、
mock IdP の E2E 39/39・ログイン 10〜13ms。logout / back-channel logout 後の Cookie 再利用は
401。セッション一覧・個別失効・全失効・アイドル期限を実装。認証 3 経路は IP 単位で制限し、
連続失敗は `429 AUTH_RATE_LIMITED` + `Retry-After`。全応答にセキュリティヘッダを付け、
パスワードログインの成否も監査に残す。SCIM Users / Groups とグループ→ロール同期、
ユーザー管理画面、監査の SIEM 転送を追加した。受入は
[`docs/phase8/acceptance.md`](docs/phase8/acceptance.md)、運用手順は
[`docs/phase8/operations.md`](docs/phase8/operations.md)。SAML / 真の HA /
本番 WAL 常時転送は未完了のため、**Phase 8 全体受入は未完了**。

Phase 1 の受入 A-02〜A-10 は実測済み。A-01 の `docker compose up` だけは開発機に Docker /
compose provider が無いため未実行だが、同じ Dockerfile・初期化 SQL・環境変数で Podman に
db / api / worker / web を起動し、コンテナ経由の E2E まで通している
（詳細と降格は [`docs/phase1/walking-skeleton.md`](docs/phase1/walking-skeleton.md)）。

できていること：

- PDF / XLSX / XLSM / DOCX / PPTX を **アップロード → 解析 → chunk → 索引 → 検索 → 証拠表示** まで通す
- 文書 API … upload-url / 登録 / カーソル一覧 / 論理削除 / 原本取得 / bulk-import（JSONL、部分成功・再開）
- 解析 … PDF はプロセス内 `pdf-extract`、Office は OOXML 直読み（XLSX 値・数式キャッシュ・図形アンカー、
  DOCX 見出し / 段落 / 表、PPTX スライド / 図形 / 表）→ DocNode → 構造チャンク
- 検索 … Lindera(IPADIC) の単語 + 文字 bigram、32 分区 fan-out、RRF（k=60）、任意の簡易 rerank、
  revision / doc_type / business_domain フィルタ
- 索引 … `index_build` によるモデル別索引、READY の有効化・切替、再埋め込み
- 証拠 … `quote` の部分文字列検証、`sha256(NFC(quote))`、検索結果から locator へ遷移
- グラフ … Ontology v1（56 実体型 / 27 関係型）、Document/Sheet/Section/Region/Table の投影、
  PG 正本 + Outbox + Neo4j（`MERGE` + `projection_seq`）、実体検索 / 隣接 / 経路 / サブグラフ /
  トレーサビリティ、再構築 API、Neo4j 障害時も PG API は継続
- コード … `DIR` / `SNAPSHOT` / `GIT` revision、UTF-8 / Shift_JIS / Windows-31J / EUC-JP の
  元バイト偏移、Java / JSP / XML / SQL の Symbol / Fact、Fact / evidence COPY、
  `supersede_then_insert`、未変更 Fact の増分再利用、batch outbox / Neo4j 再投影、
  Fact evidence と source content API。SVN と OpenRewrite / classpath は未接続
- ジョブ … 冪等 enqueue、`SKIP LOCKED`、checkpoint 再開、SSE（`Last-Event-ID` / `done`）
- 監査 … 登録・解析・削除・一括取込・索引操作、ログイン成功 / 失敗を `trace_id` 付きで追記。RLS と 3 ロールで分離
- 認証 … パスワード / OIDC SSO（back-channel logout）、`core.session` による失効可能なセッション、
  一覧・個別失効・全失効・アイドル期限、SCIM Users / Groups とグループ→ロール、IP レート制限（429）、
  セキュリティヘッダ。ログインの連続失敗も監査に残す
- テスト工学 … ルール抽出、Scenario / Case / Data の決定論的派生、DSL 保存時検証、
  承認、環境ロック付き run、実行結果の取込み、8 分類の失敗、fingerprint による
  欠陥候補の再発集約、被覆 5 指標（分子分母・降格を明示）。Runner は別プロセスで実実行
- 画面 … ログイン / プロジェクト / 設計書一覧・取込 / 文書詳細 / 検索 + エビデンスドロワー /
  Chunk Explorer / 索引管理 / 検索検査 / グラフブラウザ / Ontology / コード解析
  / 影響分析（ENTITY / DIFF Draft）/ 問い合わせ（Q-01）/ テスト工学 /
  AI（Agent・MCP・Workflow）（ja / en / zh-CN）
- テスト … `cargo test --workspace`、`clippy -D warnings`、`fmt --check`、OpenAPI 一致、
  i18n キー一致、DB 検証、ブラウザ受入が緑

```bash
cargo test --workspace
PGHOST=localhost PGUSER=postgres ./scripts/verify-db.sh   # 空 DB に 001〜027 を適用して RLS / 分区 / graph / code / impact / test / ai / inquiry / sso / session / scim を確認

各 phase の受入表に書かれた `001〜0xx` や i18n キー数・ツール実行本数は、
その phase を検証した時点の値（履歴）である。現行の `scripts/verify-db.sh` は
**001〜027** を順に適用し、i18n は 564 キー。現行値は
[`docs/phase7/acceptance.md`](docs/phase7/acceptance.md) の §1・§2 を正本とする。
cargo run -p api --bin openapi -- --check                 # 生成 spec とコミット済み spec の一致
cd apps/web && npm ci && npm run check:i18n && npm run build
python3 scripts/phase2-eval.py                            # 50 sheet 生成 → bulk → 60 問 + 30 セル領域
cd apps/web && npm run check:ui:phase2                     # Explorer / 索引管理 / 検索検査（要 API + Vite）
cd apps/web && npm run check:ui:phase3                     # グラフ / Ontology（要 API + Vite + Neo4j）
./scripts/seed-phase4.sh                                  # DIR / SNAPSHOT の Phase 4 SAMPLE
python3 scripts/phase4-eval.py                            # 要 API + worker。同期 / 解析 / 増分 / evidence
python3 scripts/phase4-git-eval.py                        # ローカル bare Git の clone / fetch / SHA / 失敗処理
PHASE4_SCALE_FILES=13000 python3 scripts/phase4-scale-eval.py # 13,000 Java の解析・増分実測
./scripts/run-phase4-fact-scale.sh                         # 2.5M Fact COPY / supersede / Neo4j / Phase 5 6-hop
cd apps/web && npm run check:ui:phase4                     # コード解析画面（要 API + Vite）
psql -d systivra -f scripts/seed-phase5-doc.sql            # Phase 5 の文書 evidence fixture
python3 scripts/phase5-impact-eval.py                     # ENTITY / DIFF / 冪等 / evidence（要 API + worker）
python3 scripts/phase5-golden-eval.py                     # 20 変更 Golden set の recall / precision
cd apps/web && npm run check:ui:phase5                     # 影響 Draft 画面（要 API + Vite）
cd apps/web && npm run check:ui:phase5-inquiry             # 問い合わせ 5 セクション / 計画作成（要 API + Vite）
python3 scripts/phase6-runner-eval.py                     # SAMPLE 站点 + 100 ケース 4 並列の実測（要 API + worker + Chrome）
SYSTIVRA_PDF_FONT=/path/to/NotoSansJP-Regular.ttf \
  python3 scripts/phase5-inquiry-eval.py                  # Q-01 SSE / 曖昧停止 / Markdown / PDF（要 API + worker）
python3 scripts/phase6-test-eval.py                       # ルール / 派生 / 承認 / 環境ロック / 被覆 / 欠陥（要 API + worker）
python3 scripts/phase7-eval.py                            # Agent / MCP 12 ツール / Workflow 承認 / 監査（要 API + worker）
python3 scripts/phase7-agent-loop-eval.py                 # ツール呼び出しループ（独立 DB + mock LLM。既存サービスに触れない）
cargo test -p engine-test --test golden_phase6            # 合成 300 ルール → 900 組以上のデータ
cd apps/web && npm run check:ui:phase6                     # テスト工学画面（要 API + Vite）
cd apps/web && npm run check:ui:phase7                     # AI / MCP / Workflow 画面（要 API + Vite）
cd apps/web && npm run check:ui:workflow                   # Workflow エディタ（要 API + Vite）
BACKUP_ENCRYPTION_KEY=<passphrase> ./scripts/backup.sh      # PG + 原本 + manifest（暗号化・保持・オフサイトフック）
BACKUP_ENCRYPTION_KEY=<passphrase> CLEAN_ROOM=1 ./scripts/restore-drill.sh  # 復号・別 DB 復元・clean-room rebuild
./scripts/pitr-drill.sh                                    # 一時クラスタで WAL アーカイブ + 指定時刻復元
python3 scripts/phase8-sso-eval.py                         # SSO / back-channel / SCIM / セッション / 429（要 cargo build）
./scripts/phase8-siem-eval.sh                              # 監査 NDJSON の SIEM 転送とカーソル
cd apps/web && npm run check:ui:phase8                     # ログイン / セキュリティ / ユーザー（要 API + Vite）
```

### ローカル起動

```bash
# DB（pgvector が必要）
createdb systivra
for f in migrations/0*.sql; do psql -d systivra -v ON_ERROR_STOP=1 -f "$f"; done
psql -d systivra -f deployment/postgres/roles.sql
./scripts/seed-dev.sh          # alice / SystivraDemo1!（開発専用）

# API（Vite の proxy 先は 18080）
PORT=18080 DATABASE_URL=postgres://systivra_app@localhost:5432/systivra \
SESSION_SECRET=$(openssl rand -hex 32) SESSION_COOKIE_SECURE=false \
PUBLIC_BASE_URL=http://127.0.0.1:18080 SYSTIVRA_DATA_DIR=$PWD/data \
  cargo run --bin systivra-api

# worker（別ロール）
DATABASE_URL=postgres://systivra_scheduler@localhost:5432/systivra \
WORKER_TICK_SEC=1 SYSTIVRA_DATA_DIR=$PWD/data cargo run --bin systivra-worker

# 画面
cd apps/web && npm run dev     # http://localhost:15173

# グラフ投影（任意。未起動でも PG のグラフ API は動く）
#   Neo4j 2026.05 Community を bolt://localhost:7687 で起動
#   NEO4J_URI / NEO4J_USER / NEO4J_PASSWORD / NEO4J_DATABASE で接続先を指定
```

ヘルスチェック：`curl -i localhost:18080/healthz`（DB 非依存）/ `readyz`（DB 依存）。
`docker-compose.yml` は db + neo4j + api + worker + web。Docker がある環境では `docker compose up` で
`http://localhost:3000` にログイン画面が出る（A-01 の条件付き項目。Neo4j 込みの構成は未検証）。

## 構成

```text
crates/                    13 crate（domain / contracts / store / providers /
                           engine-rag / engine-graph / engine-code / engine-impact /
                           engine-test / runtime / app / api / worker）
migrations/                001〜024（core / corpus / job_event / project_of / index_build / graph / code / impact / test / ai / inquiry / sso / session / scim / code repair）
deployment/postgres/       roles.sql（3 ロール + 権限。冪等）
deployment/docker/         API / worker の Dockerfile
apps/web/                  Vite + React + Tailwind。Dockerfile は nginx 配信
workers/runner/            Node + Playwright のテスト実行プロセス（JSON DSL）
samples/sample-web/        Runner の被テストシステム（合成 SAMPLE 静的サイト）
docker-compose.yml         db + neo4j + api + worker + web
scripts/verify-db.sh       空 DB に全マイグレーションを適用し RLS / 分区 / HNSW / グラフを確認
scripts/seed-dev.sh        開発ユーザーとプロジェクト
scripts/make-fixture.sh    100 ページ日本語 PDF fixture
scripts/phase2-eval.py     Phase 2 の 60 問評価と 30 セル領域の top-3 測定
scripts/seed-phase4.sh     Phase 4 の 12 ファイル SAMPLE
scripts/phase4-eval.py     コード同期 / 解析 / 増分 / COPY / 元バイト evidence の API E2E
scripts/phase4-git-eval.py ローカル bare Git の clone / fetch / SHA / 同期失敗 E2E
scripts/phase4-scale-eval.py 13,000 Java の解析時間・Symbol / Fact・増分率
scripts/run-phase4-fact-scale.sh 専用 DB で 250 万 Fact COPY / Neo4j / Phase 5 6-hop
scripts/seed-phase5-doc.sql Phase 5 のコード・文書 evidence 接続用 fixture
scripts/phase5-impact-eval.py ENTITY / DIFF / 冪等 / evidence の API E2E
scripts/phase5-golden-eval.py 20 変更 Golden set の recall / precision / evidence
scripts/phase5-inquiry-eval.py Q-01 SSE / SSE 再開 / Markdown / PDF / 計画化
scripts/phase6-test-eval.py   Phase 6 ルール / 派生 / 承認 / 被覆 / 欠陥候補の API E2E
scripts/phase6-runner-eval.py 100 ケース / 4 並列の Runner 受入と成果物 / HAR
scripts/phase6-failure-eval.py §13 失敗モード / storage_state / seed / reset / 環境ロック
scripts/phase6-retention-eval.py trace 上限 / run 上限 / 保持期限 / 削除 API の E2E
scripts/phase6-sample-eval.py 24 画面 / 300 ルール SAMPLE と正解集の実測
scripts/phase6-graph-eval.py TestCase -[:VERIFIES]-> Requirement の outbox → Neo4j 投影
workers/runner/runner.test.mjs Runner 単体テスト（CI の runner ジョブ）
scripts/phase7-agent-loop-eval.py Agent ツール呼び出し + Q-01 推定層 / schema 境界の E2E
scripts/phase7-mcp-eval.py  外部 MCP stdio / HTTP / Schema 変化 / 失敗境界の E2E
apps/web/scripts/e2e.mjs   ログイン → 取込 → 解析 → 検索 → 原文ハイライトのブラウザ受入
apps/web/scripts/ui-check-phase2.mjs  Explorer / 索引管理 / 検索検査のブラウザ受入
apps/web/scripts/ui-check-phase3.mjs  グラフ / Ontology / 公開のブラウザ受入
apps/web/scripts/ui-check-phase4.mjs  コード解析 / 品質警告 / Fact evidence のブラウザ受入
apps/web/scripts/ui-check-phase5.mjs  影響 Draft / evidence / 狭幅レイアウトのブラウザ受入
apps/web/scripts/ui-check-phase6.mjs  テスト工学 / 被覆 5 指標 / 狭幅レイアウトのブラウザ受入
apps/web/scripts/ui-check-phase7-inquiry.mjs Q-01 推定層の折畳み / 根拠 / 主表不変のブラウザ受入
apps/web/scripts/ui-check-phase7-mcp.mjs 外部 MCP の transport / health / 再検出のブラウザ受入
docs/phase1/                Phase 1 の受入記録と引き継ぎ
docs/phase2/                Phase 2 の受入記録（実測値と未実施項目）
docs/phase3/                Phase 3 の受入記録（実測値と降格・未実施項目）
docs/phase4/                Phase 4 source-only 受入・性能実測・外部依存の境界
docs/phase5/                Phase 5 Impact 正式受入・Golden set・性能・外部境界
docs/phase6/                Phase 6 テスト工学の正式受入・性能実測・外部境界
docs/phase7/                Phase 7 AI / MCP / Workflow の進捗・未実施項目
docs/phase8/                Phase 8 復旧訓練 / OIDC SSO の進捗・未実施項目
.github/workflows/ci.yml   fmt / clippy / test / DB 検証 / OpenAPI / フロント / 文書検査

docs/design/ja/            設計正本（日本語）
  00〜09                  全体設計 9 部
  reference/              付録 3 本（日本語検索 / OSS 評価 / 規模算定）
  detail/                 実装直結の詳細設計（01 Excel 取込 〜 09 テスト実行）
```

> **日本語版が唯一の正本**。設計変更は `ja/` にのみ反映する。中国語版と監査報告は 2026-11-06 に削除した。
> 数字・版・契約の照会には `ja/` と `docs/phase1/`〜`docs/phase7/` を使う。
