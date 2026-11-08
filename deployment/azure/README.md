# Azure Database for PostgreSQL へのデプロイ

Systivra の PostgreSQL を **Azure Database for PostgreSQL Flexible Server**（以降
「Azure PG」）で動かすための手引。ローカル / コンテナの PostgreSQL と同じ
マイグレーション（`migrations/001` 〜 `027`）をそのまま使う。

> 正本: [`docs/design/ja/detail/02-database-ddl.md`](../../docs/design/ja/detail/02-database-ddl.md) §0.1
> 運用手順: [`docs/phase8/operations.md`](../../docs/phase8/operations.md)

## 0. 何がそのまま動くか

| 設計 | Azure PG |
|---|---|
| RLS 全表 + `current_setting('app.project_id')` | 動く（`SET LOCAL` の独自 GUC も可） |
| `core.project_of()` の `SECURITY DEFINER` | 動く（オブジェクトは admin が所有） |
| `ALTER DEFAULT PRIVILEGES` / `GRANT` / `REVOKE` / `CREATE ROLE` | 動く（`azure_pg_admin` で実行） |
| `CREATE DATABASE` | 動く（ただし検証はローカルで。下記 §6） |
| `vector`（pgvector）+ `pg_trgm` | 動く。**`vector` は許可リストへの登録が要先**（§2） |
| `sha256()` | PG 11+ 組み込み。`pgcrypto` は不要 |
| パーティション / HNSW 索引 | 未使用（設計に無い） |
| `postgresql.conf` の調参 | 設計は依存しない。Azure の「サーバー パラメーター」制約は無影響 |

**主バージョンは 16 以上を選ぶ。** §1 の横断閲覧は方針（RLS policy）方式に改めたので
16 未満でも動くが、PG16 以上が Azure の推奨かつ長期サポート対象。

## 1. ロールは `BYPASSRLS` を使わない

`deployment/postgres/roles.sql` は `systivra_scheduler` を**属性なし**で作り、
横断閲覧を RLS 方針 `scheduler_scope`（`FOR ALL TO systivra_scheduler USING (true)`）
で表現する。対象は許可リストの 4 表:

```
core.job              claim / requeue / heartbeat
core.project          drift 監視のプロジェクト一覧
ai.approval_request   期限切れ承認の一括 close
test.test_execution   artifact の保持期限掃き
```

Azure PG では PG16 未満、`azure_pg_admin` が非管理者ロールに `BYPASSRLS` を
付与できない（[公式ドキュメント](https://learn.microsoft.com/ja-jp/azure/postgresql/security/security-access-control)）。
方針方式にしておけば Azure / RDS / Cloud SQL / 自宅サーバーで同じ SQL が動き、
`pg_policies` を見れば「誰が何を横断できるか」が一覧できる。

## 2. 拡張の許可リスト（**マイグレーションの前に**）

Azure PG は `CREATE EXTENSION` できる拡張をサーバー パラメーター
`azure.extensions` で制限する。`migrations/005_corpus_chunk.sql` が
`CREATE EXTENSION IF NOT EXISTS vector` を実行するので、**マイグレーションより先**に
許可リストへ入れる。

```bash
az postgres flexible-server parameter set \
  --resource-group "$RG" --server-name "$SERVER" \
  --name azure.extensions --value vector,pg_trgm
```

確認:

```sql
SHOW azure.extensions;
-- 例: azure_ai,pg_diskann,pg_trgm,vector
```

`vector` という名前は pgvector の拡張名そのもの（バイナリ名も `vector`）。
`IF NOT EXISTS` が付いていても、許可リストに無いとエラーになる。

## 3. 接続文字列

Azure PG は **TLS 必須**。管理者のログイン名は `<admin>@<server>` 形式。

```bash
# .env（抜粋）
DATABASE_URL=postgres://systivra_app@my-pg-server.postgres.database.azure.com:5432/systivra?sslmode=require
```

worker は別ロールを使う（`roles.sql` の 3 ロール構成）:

```bash
DATABASE_URL=postgres://systivra_scheduler@my-pg-server.postgres.database.azure.com:5432/systivra?sslmode=require
```

- `sslmode=require` を**明示**する。未指定だと sqlx は `prefer` で交渉し、
  ある日平文にフォールバックしうる。
- `sslmode=verify-full` を使う場合は root 証明書のパスを `sslrootcert` で渡す。
- 接続プールは API / worker それぞれ最大 16。Azure の SKU 別上限を超えない。
  Azure の上限は SKU に対して自動設定されるため、**サーバー パラメーターの
  `max_connections` は変更しない**（下げると予約接続を圧迫する）。

## 4. バックアップ / PITR（Azure 側の機能を使う）

| 観点 | ローカル構成 | Azure PG |
|---|---|---|
| バックアップ | `scripts/backup.sh`（pg_dump 毎日） | 自動スナップショット（毎日）+ WAL |
| PITR | `scripts/pitr-drill.sh`（自前クラスタで archive_mode） | プラットフォームの PITR |
| 保持期間 | `BACKUP_RETENTION_DAYS` | 7〜35 日（作成時または後から設定） |
| エクスポート | ローカルファイル | **Azure のバックアップは書き出せない** |

### 論理バックアップ（`scripts/backup.sh`）

**そのまま使える。** Azure からでも `pg_dump --format=custom` は動く。
オフサイト / 論理エクスポートは Azure バックアップでは代替できないので、
これが唯一の手段。`BACKUP_UPLOAD_CMD` に Azure Blob を宛先にすると永続化できる:

```bash
BACKUP_UPLOAD_CMD='az storage blob upload --account-name "$ACCT" -c backups -f "$1" -n "$(basename "$1")"'
```

### 物理バックアップ / PITR

Azure の自動バックアップを使う。`archive_mode` / `archive_command` は
**Azure では設定できない**（自前クラスタの `pitr-drill.sh` がやっていたこと）。
代わりに:

```bash
# 指定時刻へ復元（**新しいサーバー**ができる。元のサーバーは上書きされない）
az postgres flexible-server restore \
  --resource-group "$RG" --name "$SERVER-restored" \
  --source-server "$SERVER" --restore-time "2026-11-08T14:00:00Z"
```

- 早い復元: Fast Restore（スナップショットから、WAL 再生なし）
- 別リージョン: Geo Restore（geo 冗長バックアップの設定が必要。**作成時のみ設定可**）
- 保持期間: `az postgres flexible-server update --backup-retention <7..35>`

### RPO を分けて書く

| 対象 | RPO |
|---|---|
| Azure プラットフォーム PITR | **最大 5 分**（WAL アーカイブ遅延） |
| `scripts/backup.sh` の論理バックアップ | **実行間隔**（毎日なら最大 24 時間） |

「RPO = スナップショット時点」という表現はローカル構成の話で、Azure では
**もっと良い**（5 分）。ただし論理バックアップのほうは 24 時間のままなので、
復旧手順を選ぶときはどちらの RPO を使うかを明記する。

## 5. HA と複数プロセス

- Azure PG の **zone-redundant HA** は DB のフェールオーバーのみ。
  API / worker は引き続き単一プロセス（`docs/phase8/operations.md` §10 の
  「マルチノード未実装」は変わらない）。DB が冗長でもアプリは落ちる。
- worker が複数いる構成は **job の `FOR UPDATE SKIP LOCKED` と心拍で二重実行を
  防ぐ**設計になっている（`docs/phase8/operations.md` §9）。Azure でも同じ。

## 6. `verify-db.sh` は Azure で実行しない

`scripts/verify-db.sh` は**一時 DB を作って drop する**。マネージド インスタンスで
実行すると実のデータベースの建銷になり、課金とゴミが残る。空 DDL の検証は
ローカル（またはコンテナ）の PostgreSQL で行う。

```bash
# ローカル / コンテナの PG で実行
PGHOST=localhost PGUSER=postgres ./scripts/verify-db.sh
```

`roles.sql` の**適用だけ**を Azure で行うことは問題ない（マイグレーション後の
権限設定が目的）:

```bash
PGPASSWORD=... psql -h "$SERVER.postgres.database.azure.com" -U "admin@$SERVER" \
  -d systivra -v ON_ERROR_STOP=1 -f deployment/postgres/roles.sql
```

## 7. 組み込み接続プールを使ってはいけない

Azure PG のポータルに「接続プール」があるが、**トランザクション プーリングを
有効にしてはならない**。`SET LOCAL app.project_id` はトランザクション境界で
効く設定で、セッションを別のクライアントに使い回されると**プロジェクト境界が
別セッションに漏れる**。RLS が意味を失う。

プールを使う場合は:
- プールを経由しない**専用の接続**を使う、または
- `pgbouncer` を `session` モードで自前運用する（`transaction` モードは不可）

## 8. デプロイ順序（まとめ）

1. サーバー作成（PG16 以上、TLS 有効、保持期間を設定。geo 冗長は**作成時のみ**）
2. `azure.extensions` に `vector,pg_trgm` を追加
3. `migrations/001` 〜 `027` を順に適用
4. `deployment/postgres/roles.sql` を適用
5. `DATABASE_URL` を `?sslmode=require` 付きで API / worker に設定
6. `scripts/backup.sh` を定期実行（`BACKUP_UPLOAD_CMD` で Blob へ）
7. PITR は Azure の自動バックアップに任せる（`pitr-drill.sh` は手順の訓練用）
