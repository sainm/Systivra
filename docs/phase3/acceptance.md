# Phase 3 受入記録（Graph + Ontology）

実施日: 2026-11-07  
対象: `08-実装計画.md` Phase 3 / `detail/07-graph-projection.md`  
環境: macOS arm64、PostgreSQL 18.4 + pgvector 0.8.2、Neo4j 2026.05.0 Community、
Rust 1.88 以上、Node.js 24  
Neo4j: `bolt://localhost:7687`（ローカル Community。認証は開発用で実質 permissive）

---

## 1. 受入項目

| # | 受入 | 結果 | 証拠 |
|---|---|---|---|
| 1 | Phase 1/2 の文書データで Document/Section グラフを構築。ノード数が PG と一致（誤差 0） | **PASS** | PG 5 / Neo4j 5、関係 4 / 4、`drift_rate=0.0`。chunk は設計 I-3 により投影しない |
| 2 | 隣接クエリ P95 < 200ms（10 万ノード） | **PASS（合成データ、PG lite）** | 100,000 実体 / 99,999 関係で P95 5.325ms、平均 4.977ms（100 回） |
| 3 | 投影器を 10 分停止して再開。最終的に一致し、重複辺がない | **一部未実施** | 3 秒停止で outbox 追従と復旧を実測。10 分停止は未実施。再構築 2 回で辺 4 / 重複 0 |
| 4 | 文書削除後、1 分以内に関連ノードが消える | **PASS** | 削除ジョブから 4 秒以内に Neo4j から 3 実体が消え、PG / Neo4j が一致 |

受入 1 の件数は、Phase 1 からあるデモ文書 1 件（XLSX）の構造木。実体は Document / Sheet / Section / Region / Table。
「Chunk」は `detail/07` I-3（chunk を投影しない）に従い対象外とした。

---

## 2. 実測値

### 2.1 グラフ一致と漂流

`GET /api/v1/projects/{pid}/graph/stats`:

```json
{"pg_nodes":5,"pg_relationships":4,"neo4j_nodes":5,"neo4j_relationships":4,
 "drift_rate":0.0,"last_event_seq":25,"ontology_version":"v1-ui-1793986889512"}
```

- 再構築 2 回後の Neo4j: `count(r)=4`、`count(DISTINCT r.rel_id)=4`（重複辺 0）。
- 再構築前後の `projection_build.base_event_seq` は 18 → 20 → 22 と前進し、
  `entities_written=5` / `relationships_written=4` / `drift_rate=0` を記録。
- 再構築は PG の `projection_seq` を保持する。実体の `projection_seq` は 1、関係は 0〜2。
  再構築後の outbox イベントで実体 1→2→3、関係 1→2 と前進することを確認。

### 2.2 順序保証（古いイベントの追い越し）

関係 `4143a778-…` に対し、`projection_seq=1, confidence=0.9` のイベントを適用した後、
`projection_seq=0, confidence=0.1` のイベントを後から投入。Neo4j は `0.9 / seq=1` のまま変わらず、
古いイベントが上書きしないことを確認。最後に `seq=2, confidence=1.0` で復旧。

### 2.3 投影器の停止と復旧（短時間）

1. worker を停止。
2. 実体の名前を変更し、`DocumentNodeUpserted` を outbox に積む。
3. 停止中は Neo4j が旧名のままであることを確認。
4. worker を再起動。3 秒以内に新名と `projection_seq=4` が反映され、未消費イベント 0。

10 分停止は実行していない。設計の 10 分は「停止中に積まれたイベントを再開後に全部追従できるか」の
試験であり、本実測は 3 秒版である。再開後の重複辺が 0 であることは 2.1 と併せて確認した。

### 2.4 隣接性能（10 万ノード、合成データ）

- プロジェクト `99999999-9999-4999-8999-999999999999` に 100,000 実体 + 99,999 の
  `CONTAINS` 関係を PG に直接投入（Neo4j には投影しない。PgGraphProvider lite の測定）。
- 隣接取得（1 ホップ）を 100 回実行: **P95 5.325ms / 平均 4.977ms**。
- 測定後に合成データは `graph_meta.relationship` → `graph_meta.entity` の順で全削除済み。
- 合成データは実在の文書由来ではなく、辺の次数分布も一様である。実データの偏りは未検証。

### 2.5 UI

`npm run check:ui:phase3`:

```json
{"entityCount":5,"nodeCount":4,"typeRows":56,
 "publishVersion":"v1-ui-1793986889512","errors":[]}
```

- グラフブラウザ: 実体検索、型フィルタ、SVG サブグラフ、祖先 / 子孫、再構築。
- Ontology: 56 実体型 / 27 関係型の表、版公開。公開ごとに新 version が PUBLISHED、旧版は DEPRECATED。
- 1440px と 607px で確認。607px では一覧と詳細が縦積み（`gridTemplateColumns` 1 列）。
- スクリーンショット: `/tmp/systivra-phase3-graph-1440.png`、
  `/tmp/systivra-phase3-ontology-1440.png`、`/tmp/systivra-phase3-graph-607.png`。
- Phase 1 / Phase 2 のブラウザ受入も再実行し、回帰なし（Phase 2 の記録は `docs/phase2/acceptance.md`）。

---

### 2.6 投影運用の追補（2026-11-08 / `scripts/phase3-graph-ops-eval.py`）

| 項目 | 実測 |
|---|---|
| 影ラベル再構築（G-06） | 17 ノード / 12 辺。BUILDING 中も旧 `:Entity` で読み取り可。検証差分 0 で SWITCHED、影ラベル残 0 |
| ABORT（G-07） | 構築中に `rebuild/abort` → 応答 ABORTED、Neo4j ノード数は旧 17 のまま、影ラベル残 0 |
| 差分追従 | 再構築後に base_seq 以降の outbox を `:Entity` へ再適用（冪等 MERGE）。PG / Neo4j 件数一致 |
| 日次漂流（G-09） | 起動時チェックで drift 0。`:Entity` を 1 件削除 → 次回起動で degraded=true、監査 `graph.drift.detected` 1 件。再構築で復旧 |
| 複数 projector | `locked_by` / `locked_until` + `SKIP LOCKED`。2 worker が 29 イベントを 4 / 25 で分担、dead letter 0、PG / Neo4j 件数一致。stale lock（`locked_until` 過去）は再取得された |
| 完全性表示 | `GET /graph/stats` に未消費イベント数・直近 drift・最終再構築を追加。UI（1440 / 607）で 9 チップ、pageerror 0 |

---

## 3. 実装済み範囲

| 領域 | 実装 |
|---|---|
| Ontology | v1 を `011_graph.sql` で投入。56 実体型 / 27 関係型。7 クラスタ（業務・文書・設計・アプリ・コード/DB・テスト・AI/分析）。閲覧 + 公開 API / 画面 |
| 正本 | `graph_meta.entity` / `relationship` / `projection_state` / `projection_build`。RLS は `core.current_project()` |
| Outbox | 文書構造の保存と同一 tx で `DocumentNodeUpserted` / `RelationshipUpserted`、削除で `ProjectionTombstone` |
| 冪等 / 順序 | 実体・関係の `projection_seq` 条件付き書き込み、関係 `rel_id` の `MERGE`、イベント `event_id` |
| Projector | worker tick 末尾に最大 200 件を drain。失敗 3 回で dead letter。Neo4j 障害時は投影を止め、PG API は継続 |
| 再構築 | `POST /graph/rebuild` が PG の live 行を読み、プロジェクト単位で `clear_project` → 再投影 → `projection_build` 記録 |
| 検索 / 走査 | 実体検索（trigram + ILIKE）、隣接、経路、サブグラフ、祖先 / 子孫。深さ 3 ホップ、上限付き |
| 削除伝播 | `tombstone_document` が実体を論理削除し、関係の `valid_to` を閉じ、`DETACH DELETE` を投影 |
| UI | グラフブラウザ（検索・サブグラフ・トレーサビリティ・統計・再構築）、Ontology 閲覧 / 公開 |
| 契約 | OpenAPI に graph 9 ルートを追加。`apps/web/src/generated/api.ts` を再生成 |

---

## 4. 未実施・降格（偽装しない）

| 項目 | 状態 | 理由 / 次の扱い |
|---|---|---|
| 影ラベルによる無停止再構築（I-5） | **実装** | 影ラベル `Entity__<8hex>` に構築 → 差分追従 → 検証（差分 0）→ ラベル入替。BUILDING / VERIFYING / SWITCHED / FAILED / ABORTED を `projection_build` に残す。`POST /graph/rebuild/abort` で中止でき、旧グラフは無傷（G-06 / G-07 を実測） |
| 日次漂流検出 | **実装** | worker が起動時 + 24 時間ごとに PG / Neo4j の件数を比較し `graph_meta.projection_drift` に記録。劣化時は監査 `graph.drift.detected`。`GET /graph/stats` が未消費イベント数・直近 drift・最終再構築を返し、グラフ画面に表示（G-09 を実測） |
| 投影器 10 分停止 | **未実施** | 3 秒停止で代替。10 分間のイベント蓄積と復旧は未検証 |
| 1,000 実体 / 84 万ノードの負荷 | **未実施** | 10 万ノードの PG lite 隣接のみ合成データで実測。Neo4j 側の 10 万ノード投影は未実施 |
| 複数 projector の並走 | **実装** | outbox に `locked_by` / `locked_until` を追加し、`FOR UPDATE SKIP LOCKED` の claim リースで配る。2 worker 並走で重複・欠落 0、stale lock の再取得を実測 |
| UNWIND バッチ投影 | **実装済み** | 1,000 件ずつ `UNWIND`（`apply_entities_as` / `apply_relationships_as`） |
| 改版切替の tombstone | **未実装** | `DocumentRevisionSuperseded` は未実装。Phase 1 の改版履歴（`/revisions`）が未実装のため、版切替を起こせない。`revision` / `valid_to` の列と削除時の `valid_to` 閉止のみ実装 |
| chunk の投影 | **設計どおり対象外** | I-3 により chunk は投影しない。Phase 3 受入文の「Document/Section/Chunk」は chunk を除く構造実体として解釈 |
| Ontology の型編集 | **未実装** | 閲覧 + 公開のみ。型の追加・属性編集・関係マトリクスは未実装 |
| Cytoscape / 経路ハイライト | **降格** | SVG の静的レイアウト。フィルタと経路ハイライトは無い |
| Neo4j 5.26 LTS | **未検証** | ローカルは 2026.05.0 Community。`docker-compose.yml` に `neo4j:5.26-community` を定義したが、Docker が無いため未起動 |
| docker compose（Neo4j 含む） | **未実行** | 開発機に Docker / compose provider が無い。Podman での Phase 3 構成検証も未実施 |
| 認証付き Neo4j | **未検証** | ローカルは permissive。アプリは user / password を送るが、認証失敗経路は未試験 |

---

## 5. 既知の制約

- `graph_meta.relationship.projection_seq` は `012_graph_relationship_seq.sql` で追加した。
  新規 DB は 001〜012 の順で適用する。既存 DB は 012 のみ追加適用する。
- 再構築は同期 API。実体 / 関係が多いプロジェクトでは HTTP タイムアウトし得る。Phase 5 までに
  ジョブ化（202 + `job_event`）へ移す。
- 再構築は in-place のため、再構築中の書き込みと投影が競合し得る。PG 読み取り後に積まれた
  outbox イベントを worker が先に適用してから `clear_project` すると、その更新は次の再構築まで
  Neo4j に残らない。影ラベル方式（I-5）を実装するまで、再構築は書き込みが少ない時間帯に行う。
- `projection_state.last_event_seq` は「適用済みの最大 outbox seq」で、outbox の採番順と
  コミット順は一致しない。projector は `published_at IS NULL` を読むため取りこぼさないが、
  seq を「ここまで完了」とみなして未消費イベントを飛ばしてはならない。
- `GET /graph/stats` の Neo4j 件数は、Neo4j 停止時に 0 / 0 となり `drift_rate=1.0` になる。
  「Neo4j 障害」と「漂流」を区別するフィールドは無い（契約追加は Phase 5 以降）。
- Ontology の公開は型を複製するだけで、差分承認・ロールバックは無い。公開版は `v1-ui-<epoch>` のように
  一意な文字列を要求する。
- グラフの `revision` は文書 revision の文字列表現。改版切替時に旧版を自動で無効化しない。
- Neo4j には既存の無関係ラベルがあってもよい。`clear_project` は `project_id` で絞る。
- 隣接 API は 1 ホップ固定。経路 / サブグラフ / トレーサビリティは 3 ホップ。

---

## 6. 再現手順

```bash
# 001〜012 + roles
PGHOST=localhost PGUSER=postgres ./scripts/verify-db.sh

# API / worker / Neo4j / Vite を起動した状態で
curl -s -c /tmp/c.txt -H 'Content-Type: application/json' \
  -d '{"login_id":"alice","password":"SystivraDemo1!"}' \
  http://127.0.0.1:18080/api/v1/auth/login

PID=22222222-2222-4222-8222-222222222222
curl -s -b /tmp/c.txt http://127.0.0.1:18080/api/v1/projects/$PID/graph/stats
curl -s -b /tmp/c.txt -X POST http://127.0.0.1:18080/api/v1/projects/$PID/graph/rebuild

cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p api --bin openapi -- --check
cd apps/web && npm run check:i18n && npm run build
cd apps/web && npm run check:ui:phase3   # 要 API + Vite
```
