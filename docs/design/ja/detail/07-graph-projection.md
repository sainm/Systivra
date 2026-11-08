# 詳細設計 07：グラフ投影（Outbox / 冪等 / 全件再構築 / 漂流検出）

対象：Phase 3「Graph + Ontology」。**ADR-002（PostgreSQL を図の正本、Neo4j を再構築可能な投影）** を、実装者がそのまま書ける手順に落とす。

**正本**：データ権威は `02-アーキテクチャ.md` §4.2、DDL は `detail/02-database-ddl.md`、ノード / 辺の形は `05-データモデル.md` §2、Cypher 規約は `04-コアエンジン.md` §3。

---

## 0. 不変条件（破ったら設計違反）

| # | 不変条件 |
|---|---|
| I-1 | **Neo4j にしか無いデータを作らない**。全実体・全関係・全属性は PG に対応する行がある |
| I-2 | 業務書き込みは**必ず `store` → PG トランザクション → `outbox_event`**。`GraphProvider::upsert_*` は投影専用で、業務コードから呼ばない |
| I-3 | **chunk を投影しない**。1,200 万ノードになり影響分析に寄与しない。図は Document / Sheet / Section / Region / 業務実体 / コード実体だけ（84 万ノード / 500 万辺） |
| I-4 | 生成 Cypher は **Cypher 5 サブセット**。`CYPHER 25` 接頭指定と Cypher 25 専用構文は使わない（ADR-013） |
| I-5 | 投影は**何度でもやり直せる**。全件再構築中も旧グラフで読み取りを継続できる |
| I-6 | 漂流は**黙らせない**。日次検出の結果をホームの「グラフ完全性」に表示する |

---

## 1. 権威の対応表

| データ | 正本 | 派生 | 再構築方法 |
|---|---|---|---|
| `graph_meta.entity` / `relationship` | **PostgreSQL** | Neo4j `:Entity` / 関係 | PG の live 行から全件再投影（§5） |
| Ontology（型・関係型・制約） | PostgreSQL（版管理） | 投影側のラベル / 関係型 | 登録簿を読み直す |
| `code.fact` | PostgreSQL | 図の辺 | `detail/06` §6 → 本稿 §2 |
| Chunk / Embedding / Evidence | PostgreSQL | **投影しない** | — |
| `ProjectionState` | `graph_meta.projection_state` | Neo4j の `:ProjectionState` | PG から上書き |

---

## 2. イベント型と payload

`core.outbox_event` に積む。DDL は `detail/02` §2（`event_id` PK、`seq` IDENTITY、`outbox_dead_letter` あり）。

| event_type | 発生箇所 | payload（最小） |
|---|---|---|
| `EntityUpserted` | entity 正規化 | `{project_id, entity_id, uid, entity_type, natural_key, name, qualified_name, revision, resolution, seq}` |
| `RelationshipUpserted` | Fact 受理 | `{project_id, rel_id, src_uid, dst_uid, rel_type, fact_id, resolution, confidence, revision, seq}` |
| `DocumentNodeUpserted` | 設計書構造木 | `{project_id, entity_id, uid, node_kind, parent_uid, revision, seq}`（**chunk は含めない**） |
| `DocumentRevisionSuperseded` | 改版追加 | `{project_id, document_id, old_revision, new_revision, seq}`（worker は旧 revision の Neo4j ノードを削除し、切替先は PG から再投影） |
| `ProjectionTombstone` | 論理削除 | `{project_id, uid, deleted_at, seq}` |
| `FactAccepted` | `detail/06` §6 | Fact の要約 + `entity` / `relationship` への正規化結果 |

**`seq` の扱い（重要）**：`GENERATED ALWAYS AS IDENTITY` は挿入時に採番するため、**コミット順と `seq` 順は一致しないことがある**（長い tx が後から commit する）。したがって：

- `seq` は**進捗表示と再開位置の目安にだけ**使う。
- 正しさは **① `event_id` の冪等、② 実体ごとの `projection_seq` 条件付き書き込み** の 2 つで担保する。
- projector は `published_at IS NULL AND NOT dead_letter` を `FOR UPDATE SKIP LOCKED` で取り続けるので、採番の隙間があっても最終的に全件を処理する。`last_event_seq` を「ここまで完了」と誤信して飛ばさない。

---

## 3. 書き込み側（PG トランザクション）

```text
BEGIN
  SET LOCAL app.project_id = <ctx から注入>;
  INSERT/UPDATE graph_meta.entity ...;          -- projection_seq も更新
  INSERT/UPDATE graph_meta.relationship ...;
  INSERT INTO core.audit_log (...);             -- 追記のみ
  INSERT INTO core.outbox_event
    (event_id, aggregate, aggregate_id, event_type, payload, occurred_at)
    VALUES (gen_random_uuid(), 'entity', $entity_id, 'EntityUpserted', $payload, now());
COMMIT
```

- `entity` の更新と `outbox_event` は**同一トランザクション**。片方だけ成功する状態を作らない。
- `payload.seq` には PG 側の `graph_meta.entity.projection_seq`（単調増加）を入れる。outbox の `seq` とは別物で、**順序逆転の判定にはこちらを使う**。
- 冪等キーは `hash(project_id, operation, input_digest, revision)`。重複送信は同一 `event_id` を返し、2 度目のイベントを積まない。
- 1 回の tx で積むイベントは最大 1,000 件。超える場合はバッチを分割（`UNWIND $batch` の 1,000 件規約と合わせる）。

---

## 4. Projector

### 4.1 取得と適用（疑似コード）

```rust
loop {
    let events = sqlx::query(
        "SELECT event_id, aggregate, event_type, payload, seq
           FROM core.outbox_event
          WHERE published_at IS NULL AND NOT dead_letter
          ORDER BY seq
          LIMIT 5000
          FOR UPDATE SKIP LOCKED"
    ).fetch_all(&pool).await?;

    for chunk in group_by_event_and_relation_type(&events).chunks(1000) {
        match project_unwind_batch(&neo4j, chunk).await {
            Ok(_)  => mark_published_batch(&pool, chunk).await?,
            Err(e) => record_failure_batch(&pool, chunk, e).await?,  // バッチは止めない
        }
    }
    if events.is_empty() { tokio::time::sleep(Duration::from_millis(500)).await; }
}
```

- **1 件の失敗でバッチを止めない**。`attempts` を +1、`last_error` を書き、3 回超で `dead_letter = true` にして `core.outbox_dead_letter` へ退避。閾値超で警報。
- `mark_published` は `UPDATE … SET published_at = now() WHERE event_id = $1 AND published_at IS NULL`。更新件数 0 なら他ワーカーが処理済み。
- 複数 projector が並走しても `SKIP LOCKED` + `event_id` + `projection_seq` で安全。

### 4.2 条件付き書き込み（順序逆転防止）

Cypher 5 の範囲で書く。**ラベルと関係型はパラメータ化できない**ので、Ontology 登録簿で検証した `^[A-Z_]+$` の識別子だけを文字列連結する。

```cypher
// EntityUpserted
MERGE (e:Entity {uid: $uid})
  ON CREATE SET e.project_id = $project_id,
                e.entity_id = $entity_id,
                e.projection_seq = $seq
  ON MATCH  SET e.project_id = $project_id
WITH e
WHERE e.projection_seq <= $seq
 SET e.entity_type = $entity_type,
     e.name = $name,
     e.qualified_name = $qualified_name,
     e.revision = $revision,
     e.resolution = $resolution,
     e.projection_seq = $seq,
     e.last_event_id = $event_id
RETURN e.uid;
```

```cypher
// RelationshipUpserted（rel_type は登録簿検証済み識別子を埋め込む）
MATCH (a:Entity {uid: $src_uid}), (b:Entity {uid: $dst_uid})
MERGE (a)-[r:CALLS {rel_id: $rel_id}]->(b)
  ON CREATE SET r.projection_seq = $seq
WITH r
WHERE r.projection_seq <= $seq
 SET r.resolution = $resolution,
     r.confidence = $confidence,
     r.fact_id = $fact_id,
     r.revision = $revision,
     r.projection_seq = $seq,
     r.last_event_id = $event_id;
```

| 問題 | 機構 |
|---|---|
| 古いイベントが後から届く | `WHERE e.projection_seq <= $seq`。古い `$seq` は `SET` されない |
| 同じイベントの重複送信 | `event_id` 一意 + `MERGE` + `last_event_id`。結果は同一 |
| 部分失敗 | dead letter へ退避、バッチ継続 |
| 削除 | 物理削除せず `ProjectionTombstone`（§6） |

> **`MERGE` のキーは `uid` 単一プロパティ**。`uid = project_id + ":" + entity_id`。Community 版では複合一意制約 / ノードキーの対応が限られるため、`CREATE CONSTRAINT entity_uid FOR (e:Entity) REQUIRE e.uid IS UNIQUE` だけを置く（`05` §2.3）。

### 4.3 バッチ化

- 同一 `event_type` の 1,000 件をまとめ、`UNWIND $batch AS row` で 1 クエリにする。**1 件ずつ `MERGE` しない**（`detail/03` §6.5）。
- `projection_seq` の条件付き書き込みはバッチ内でも行単位で効く。`WITH e WHERE …` を `UNWIND` の直後に置く。
- Neo4j ページキャッシュ 32GB。84 万ノード / 500 万辺なら全載りする。これを超える投影は設計違反（I-3）。

### 4.4 進捗状態

適用後に `graph_meta.projection_state` を更新し、Neo4j の `:ProjectionState` へも mirror する。

```sql
UPDATE graph_meta.projection_state
   SET last_event_seq = GREATEST(last_event_seq, $seq),
       last_event_id  = $event_id,
       updated_at     = now()
 WHERE project_id = $project_id;
```

`GREATEST` により、古いイベントの処理で進捗が後退しない。

---

## 5. 全件再構築（`POST /api/v1/projects/{pid}/graph/rebuild`）

**旧グラフを消してから書き直さない**。影ラベルに構築し、検証してから読み取りを切り替える（I-5）。

> 2026-11-07 の実装は in-place の同期 API である。PG live 行を keyset で 1,000 件ずつ読み、
> Neo4j の project scope を `clear_project` 後に `UNWIND` 再投影する。2.5M 辺は 176.168 秒で
> PG / Neo4j / API 件数一致。**2026-11-08**: 影ラベル再構築・差分追従・ABORT を実装し、
> `scripts/phase3-graph-ops-eval.py` で G-06 / G-07 を実測した。

```text
[1] graph_meta.projection_build に status=BUILDING で 1 行作る
    base_event_seq = 現在の projection_state.last_event_seq
    shadow_label   = 'Entity__' || left(build_id::text, 8)   -- 例 Entity__b7c1e2a9
[2] PG の live entity / relationship を build_id 時点のスナップショットで読む
    → :Entity__b7c1e2a9 ノードと同名の影ラベル付き関係へ UNWIND 1,000 件ずつ書く
[3] [1] 以降に積まれた outbox_event を影ラベルへ追記（差分追従）
[4] VERIFYING：§7 の漂流検出を影ラベルに対して実施
    drift_rate < 0.001 なら status=READY。超えたら FAILED で中止（切替えない）
[5] SWITCHED：読み取り側のラベル指定を影ラベルへアトミックに切替
    （設定 1 個の更新。API 再起動不要）
[6] 旧ラベルは 24 時間残してから DETACH DELETE を分割実行
[7] status=SWITCHED、finished_at を記録
```

| 項目 | 規約 |
|---|---|
| 同時実行 | 同一 `project_id` で `BUILDING` は 1 つだけ。2 つ目は `CONFLICT_REVISION_SUPERSEDED` |
| 中断 | `ABORTED` にして影ラベルを削除。旧グラフは無傷 |
| 切替前の読み取り | 常に旧ラベル。利用者は再構築に気づかない |
| 容量 | 影ラベル分だけ一時的に 2 倍。500GB × 2 を前提に空きを確保（R-24） |
| ログ | `projection_build.entities_written` / `relationships_written` / `drift_rate` を残す |

> **`GraphProvider::upsert_*` は影ラベルにも使える**が、業務コードからは呼ばない（I-2）。再構築ツール内部だけが直接投影 API を使う例外箇所として明記する。

---

## 6. 削除伝播

- PG では**物理削除しない**。`deleted_at` を立て、`ProjectionTombstone` を outbox へ。
- 投影側は `DETACH DELETE` を**分割実行**（1 クエリ 1,000 ノードまで）。巨大な `DETACH DELETE` でトランザクションログを膨らませない。
- tombstone にも `projection_seq` を持たせ、削除より新しい `EntityUpserted` が先に届いても復活しないようにする。
- 削除検証ジョブ：毎日、`deleted_at IS NOT NULL` の実体が Neo4j に残っていないかを照会。1 件でもあれば警報（R-11「幽霊証拠」）。
- **chunk / evidence は投影対象外**なので、文書削除は PG の `is_current=false` とオブジェクトストレージのライフサイクルで完結させる。

---

## 7. 漂流検出

日次バッチ。PG を真として Neo4j を検査する。

```text
1. graph_meta.entity からランダムに 1,000 件（project_id で絞る）
2. 各 uid について PG 側の次数と属性ハッシュを計算
     attr_hash = sha256(NFC( qualified_name | entity_type | revision | resolution ))
3. Neo4j から同じ uid の次数と属性を取得し比較
4. 差分数 / 1000 = drift_rate
5. drift_rate > 0.001 → 警報 + ホームの「グラフ完全性」を DEGRADED に
6. 自動で全件再構築は**しない**。§5 を人手承認で起動する
```

| 差分の種類 | 意味 | 対応 |
|---|---|---|
| Neo4j に無い | Outbox 未消費 / dead letter | `outbox_dead_letter` を確認→再投入 |
| PG に無い | 削除伝播漏れ | tombstone 再送信 |
| 属性差 | 順序逆転 / 古いイベントが上書き | `projection_seq` を確認 |
| 次数差 | 辺の投影漏れ | `RelationshipUpserted` を再送信 |

**完全性の表示**：ホームに「最終投影イベント / 未消費イベント数 / 直近日次 drift_rate / 最終再構築日時」の 4 値を常時出す。グラフが欠けていても**レポートは完全に見えてしまう**ため、この表示が R-08 の唯一の防御になる。

---

## 8. Cypher 5 制約とクエリ規約

| 項目 | 規約 |
|---|---|
| 版 | `neo4j:5.26.32-community` 固定。CI は 5.26 で全クエリを実行 |
| 構文 | Cypher 5 サブセット。`CYPHER 25` 接頭指定、Cypher 25 専用構文は禁止 |
| ラベル / 関係型 | パラメータ化不可。Ontology 登録簿で `^[A-Z_]+$` 検証後に文字列連結。ユーザー入力は絶対に連結しない |
| 値 | すべて `$param`。`uid` で `MATCH` する |
| 可変長走査 | 1 本の巨大 Cypher を書かない。Rust 側で層ごと BFS、`WHERE a.uid IN $uids`（`detail/03` §4） |
| 上限 | サブグラフ 5,000 ノード。UI も同時に制限 |

```rust
let rel = ontology.relationship_type(&req.rel_type)?;   // 未知は即エラー
let cypher = format!(
    "MATCH (a:Entity {{uid:$uid}})-[r:{}]->(b) WHERE r.revision = $rev RETURN b …",
    rel.as_cypher_ident()
);
```

---

## 9. 運用 Runbook

| 症状 | 最初に見る | 打つ手 |
|---|---|---|
| 影響分析の辺が少ない | ホームの drift_rate / 未消費イベント数 | 未消費なら projector 再起動。dead letter を確認 |
| `projection_seq` が後退 | `graph_meta.entity.projection_seq` と payload.seq | 条件付き書き込みの `WHERE` 抜けを修正 |
| 再構築が OOM | `projection_build` とディスク空き | 影ラベルを ABORTED で消し、空きを確保して再実行 |
| 削除済み文書が検索に出る | `is_current` / tombstone 検証ジョブ | PG の論理削除を確認 → tombstone 再送信 |
| Neo4j が落ちた | — | **PG が正本なので業務停止にしない**。投影を一時停止し、復旧後に outbox を再消費。API は `degraded: ["graph_unavailable"]` を返す |
| 図のバックアップが壊れた | — | 再構築（§5）。Neo4j のバックアップは**復旧の近道であって正本ではない** |

---

## 10. 受入テスト（Phase 3）

`08-実装計画.md` Phase 3「文書グラフが構築でき、再構築できる / ノード誤差 0」を具体化する。

| # | 手順 | 期待 |
|---|---|---|
| G-01 | PG に 1,000 実体を書き、outbox を消費 | Neo4j のノード数が一致。**chunk ノード 0** |
| G-02 | 同一 `event_id` を 3 回投げる | ノード / 辺が増えない。`last_event_id` は同一 |
| G-03 | `seq` の大きいイベント → 小さいイベントの順で届ける | 古い値で上書きされない。`projection_seq` が後退しない |
| G-04 | 2 projector を並走 | 重複・欠落 0 |
| G-05 | 1 件を意図的に失敗させる | バッチが止まらず、3 回で dead letter へ |
| G-06 | 全件再構築を実行 | 切替まで旧ラベルで読み取り継続、`drift_rate < 0.001` |
| G-07 | 再構築を途中で ABORT | 旧グラフ無傷、影ラベル削除 |
| G-08 | 論理削除 → tombstone | 24 時間以内に Neo4j から消え、検証ジョブが 0 件 |
| G-09 | 日次漂流検出を手動実行 | 差分 0 / 1000。意図的に 1 辺消すと警報が出る |
| G-10 | CI で全 Cypher を 5.26.32 で実行 | エラー 0。`CYPHER 25` 文字列がソースに存在しない |

---

## 11. 他詳細設計との境目

| 境目 | 本稿 | 相手 |
|---|---|---|
| Fact を作る | 受け取るだけ | `detail/06-code-analysis-engine.md` |
| 設計書の構造木 | `DocumentNodeUpserted` を受ける | `detail/01-excel-ingestion.md`（cell は投影しない） |
| 分区 / RLS | 図には無関係 | `detail/02-database-ddl.md` §6 |
| 走査速度 | BFS の層バッチ規約を守る | `detail/03-performance.md` §4 |
| API | `/api/v1/projects/{pid}/graph/rebuild` は同期 200 + 件数 | `06-API.md` §1 / `detail/04-api-contract.md` §3 |
| 影響分析 | 辺の `resolution` / `confidence` を提供 | `04-コアエンジン.md` §5 |

---

## 12. 実装状況（2026-11-08）

Phase 3 の実装は本書の不変条件 I-1〜I-6 をすべて満たす。I-5（無停止の影ラベル再構築）は
`scripts/phase3-graph-ops-eval.py` で G-06 / G-07 を、I-6（日次漂流と完全性表示）は
G-09 を実測済み。受入の実測値と残る未実施項目（投影器 10 分停止、10 万ノード投影）は
`docs/phase3/acceptance.md` に記録する。

| 項目 | 実装 | 設計との差分 |
|---|---|---|
| Ontology v1 | 56 実体型 / 27 関係型（7 クラスタ: 業務・文書・設計・アプリ・コード/DB・テスト・AI/分析） | 実体型は「約 60」の範囲。関係型は設計の 20 より 7 多い（登録のみで、現時点の投影に使うのは `CONTAINS` だけ） |
| Ontology 公開 | `POST /graph/ontology/publish` が最新 PUBLISHED の型を複製し、新版を PUBLISHED、旧版を DEPRECATED にする | 型の追加・編集 UI は無い（閲覧 + 公開のみ） |
| 正本 / Outbox | `graph_meta.entity` / `relationship` と `core.outbox_event` を同一 tx で書く。`DocumentNodeUpserted` / `RelationshipUpserted` / `ProjectionTombstone` | event payload は本書 §2 の最小項目 + `src_type` / `dst_type` |
| 順序保証 | 実体・関係とも `projection_seq` の条件付き書き込み（`WHERE e.projection_seq <= $seq`） | 関係の `projection_seq` は `012_graph_relationship_seq.sql` で追加 |
| Projector | worker の各 tick 末尾に outbox を最大 5,000 件取得し、event／関係型ごとに最大 1,000 件を `UNWIND`。失敗は 3 回で dead letter | 複数 projector は未実装。単一 worker 前提 |
| 全件再構築 | `POST /graph/rebuild` が影ラベル `Entity__<8hex>` に 1,000 件ずつ投影し、差分追従 → 検証（差分 0）→ ラベル入替 → 旧ラベル削除。`projection_build` に BUILDING / VERIFYING / SWITCHED / FAILED / ABORTED を残す | 読み取りは切替まで旧 `:Entity`。`POST /graph/rebuild/abort` で中止（旧グラフ無傷）。検証差分が 1 件でも切替えない |
| 漂流検出 | `GET /graph/stats` が都度比較。worker が起動時 + 24 時間ごとに `detect_graph_drift` を実行し `graph_meta.projection_drift` に記録、劣化時は監査 `graph.drift.detected` | グラフ画面に未消費イベント数・直近 drift・最終再構築を表示 |
| 削除伝播 | 論理削除 → `ProjectionTombstone` → Neo4j `DETACH DELETE` | 1 実体 1 イベント。1,000 件分割削除は未実装 |
| 構造木の投影 | Document / Sheet / Section / Region / Table を投影 | **chunk は投影しない**（I-3）。Phase 3 受入文の「Document/Section/Chunk」は chunk を除く構造実体として解釈 |
| `revision` / `valid_to` | 実体に `revision` / `first_seen_rev` / `last_seen_rev`、関係に `revision` / `valid_to` | 改版切替時は `DocumentRevisionSuperseded` を outbox に積み、worker が旧 revision の Neo4j ノードを削除して切替先を PG から再投影する（Phase 2 改版管理で実装済み） |
| PgGraphProvider lite | 実体検索・隣接・経路・サブグラフ・トレーサビリティを再帰 CTE で実装。深さは 3 ホップに clamp | 専用 trait は置かず `store` のメソッド群。API から利用 |
| Neo4j | ローカル検証は Neo4j 2026.05.0 Community（`bolt://localhost:7687`） | 設計の 5.26 LTS とは版が異なる。compose は `neo4j:5.26-community` を定義（未検証） |

### 12.1 受入との対応

| 受入 | 結果 |
|---|---|
| Document/Section グラフ構築・ノード数 PG 一致（誤差 0） | **PASS**（PG 5 / Neo4j 5、関係 4 / 4、drift 0.0） |
| 隣接 P95 < 200ms（10 万ノード） | **PASS（合成データ）** P95 5.325ms / 平均 4.977ms / 100 回。PG lite の再帰 CTE で測定 |
| 250 万 Fact 相当の全件再投影 | **PASS（合成）** 5,506 entity / 2,500,005 relationship、173.117s。PG / Neo4j / API 件数一致 |
| 投影器 10 分停止 → 再開で一致・重複辺なし | **一部未実施**。3 秒停止で outbox 追従と復旧を実測。10 分停止は未実施。再構築 2 回で辺の重複 0（`count(r)=count(DISTINCT r.rel_id)`） |
| 文書削除後 1 分以内に関連ノード消滅 | **PASS**（削除ジョブから 4 秒以内、drift 0） |
