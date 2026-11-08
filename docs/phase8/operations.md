# Phase 8 運用手順（復旧・SSO・セッション・レート制限）

対象: 単一 API / worker / PostgreSQL / Neo4j の開発・検証環境。  
正本の受入値は [`acceptance.md`](acceptance.md)。

## 1. バックアップ

```bash
PGHOST=<host> PGUSER=<admin> PGDATABASE=systivra ./scripts/backup.sh
```

出力（`backups/<UTC>/`）:

| ファイル | 内容 |
|---|---|
| `db.pgdump` | `pg_dump --format=custom --no-owner --no-privileges` |
| `originals.tar.gz` | `SYSTIVRA_DATA_DIR` の原本 |
| `manifest.json` | 件数・内容ハッシュ・SHA-256・最終監査/ジョブ時刻・git rev |

Neo4j は PG からの導出物なので含めない。復旧時に `graph/rebuild` で作り直す。

**RPO**（2 つある。混ぜない）:

| 対象 | RPO | 手段 |
|---|---|---|
| 論理バックアップ | **実行間隔**（毎日なら最大 24 時間） | この `backup.sh` |
| 物理 / PITR | **最大 5 分** | Azure PG の自動バックアップ（WAL アーカイブ） |

`BACKUP_ENCRYPTION_KEY` で AES-256-CBC 暗号化、`BACKUP_RETENTION_DAYS` で保持、
`BACKUP_UPLOAD_CMD` でオフサイトへ渡せる。定期実行は `deployment/systemd/` /
`deployment/launchd/` のテンプレートを使う。

**Azure Database for PostgreSQL を使う場合**（[`deployment/azure/README.md`](../../deployment/azure/README.md)）:

- WAL 常時転送（`archive_mode` / `archive_command`）は **Azure では設定できない**。
  物理バックアップと PITR はプラットフォームの自動バックアップに任せる
  （保持 7〜35 日、`az postgres flexible-server restore` で**新サーバー**に復元）。
- Azure のバックアップ ファイルは**書き出せない**。オフサイト / 論理エクスポートは
  上の `backup.sh` が唯一の手段。`BACKUP_UPLOAD_CMD` の宛先に Azure Blob を指定する。
- `azure.extensions` に `vector,pg_trgm` を入れてからマイグレーションを流す。
- `sslmode=require` を `DATABASE_URL` に明示する。組み込み接続プールの
  トランザクション プーリングは**有効にしない**（`SET LOCAL app.project_id` が
  セッションをまたいで漏れ、RLS が無意味になる）。

## 2. 復旧訓練

```bash
PGHOST=<host> PGUSER=<admin> ./scripts/restore-drill.sh
KEEP=1 PGHOST=<host> PGUSER=<admin> ./scripts/restore-drill.sh
```

1. 別 DB `systivra_restore_<pid>` を作る。
2. `pg_restore` → `roles.sql` → **全セッションを失効**。
3. 29 表の件数、9 表の内容ハッシュ、RLS、`core.project_of`、FTS、evidence 結合、
   live Fact 重複、原本 SHA-256 を照合する。
4. 復元 DB で API を起動し、ログイン → `graph/rebuild` → `graph/stats` の
   Pg/Neo4j 一致と drift 0 を確認する。
5. 終了時に DB を削除する（`KEEP=1` で残す）。

**実測 RTO**: 5.20 秒（暗号化解除 + clean-room rebuild。暗号化・clean-room なしは 2.5 秒台）。

本番の復旧手順:

1. 書込みを止める（API / worker 停止）。
2. 新 DB に `pg_restore --no-owner --no-privileges`。
3. `roles.sql` を適用し、`core.session` の live 行を 0 にする。
4. 原本データを展開し、`SYSTIVRA_DATA_DIR` に置く。
5. API を起動し、`POST /projects/{pid}/graph/rebuild`。
6. `graph/stats` の `drift_rate=0` を確認してから受付を再開する。
7. 利用者に再ログインを求める（復元で全セッションを失効済み）。

## 3. SSO（OIDC）

環境変数:

```bash
OIDC_ISSUER=https://idp.example.com
OIDC_CLIENT_ID=systivra
OIDC_CLIENT_SECRET=...
OIDC_REDIRECT_URI=https://systivra.example.com/api/v1/auth/oidc/callback
OIDC_PROVIDER_LABEL=SSO
```

- `OIDC_*` を 1 つも設定しなければパスワードログインのみ。
- 一部だけ設定すると API は起動に失敗する（無効のまま動く事故を防ぐ）。
- 未知の `sub` は自動登録しない。紐付けは管理 SQL:

```sql
INSERT INTO core.external_identity (user_id, provider, subject, email)
VALUES ('<user uuid>', '<OIDC_ISSUER>', '<IdP の sub>', 'user@example.com');
```

- 監査: `auth.sso.login` の SUCCESS / FAILED。`subject` は SHA-256。
- IdP 停止時: `/auth/oidc/start` は 503。パスワードログインは継続する。
- IdP 側で無効化しても、既存セッションは期限（12 時間）まで残る。
  緊急時は下のセッション失効を行う。

## 4. セッション失効

- 通常のログアウト: `POST /api/v1/auth/logout` が現在のセッションを失効させる。
- 全員ログアウト（Cookie 盗難・退職・IdP 侵害時）:

```sql
UPDATE core.session SET revoked_at = now() WHERE revoked_at IS NULL;
```

- 署名鍵のローテーション（`SESSION_SECRET` 変更）はさらに強力で、全 Cookie が無効になる。
  12 時間の期限より先に効かせたいときに使う。
- セッションの期限は 12 時間（絶対）。`SESSION_IDLE_MINUTES` でアイドル期限を追加できる。
  worker が期限切れ・失効済みを 7 日後に削除する。
- 利用者は `/security` で自分のセッションを一覧・個別失効・他端末一括失効できる。
- IdP 主導ログアウトは `POST /api/v1/auth/oidc/backchannel-logout`（logout token）。
- 古い形式の Cookie（session id なし）は 401。アップグレード後は再ログインが必要。

## 5. 認証レート制限

| 環境変数 | 既定 | 意味 |
|---|---|---|
| `AUTH_RATE_LIMIT_CAPACITY` | 10 | バースト許容量 |
| `AUTH_RATE_LIMIT_REFILL_PER_MIN` | 10 | 毎分の補充数 |
| `TRUST_PROXY` | 未設定 | `1` のときだけ `X-Forwarded-For` 先頭を使う |

- 対象: `/auth/login`、`/auth/oidc/start`、`/auth/oidc/callback`。
- 超過: `429 AUTH_RATE_LIMITED` + `Retry-After`（秒）。
- 単一プロセス内のメモリ保持。API を水平分割するとノードごとに別カウント。
- `TRUST_PROXY=1` は、前段のリバースプロキシが `X-Forwarded-For` を
  上書きする構成でのみ使う。直接公開時に有効化すると偽装で回避される。

## 6. セキュリティヘッダ

全 API 応答:

- `X-Content-Type-Options: nosniff`
- `Referrer-Policy: no-referrer`
- `X-Frame-Options: DENY`
- `Permissions-Policy: geolocation=(), microphone=(), camera=()`
- `Strict-Transport-Security` は `SESSION_COOKIE_SECURE=true` のときだけ。

## 7. 監査

| action | 内容 |
|---|---|
| `auth.password.login` | パスワードログインの SUCCESS / FAILED。login_id は SHA-256 |
| `auth.sso.login` | OIDC の SUCCESS / FAILED。subject は SHA-256、reason 付き |

```sql
SELECT occurred_at, action, result, detail->>'reason'
  FROM core.audit_log
 WHERE action LIKE 'auth.%'
 ORDER BY id DESC LIMIT 50;
```

失敗の `project_id` は NULL（プロジェクト確定前）。既存の監査照会は
プロジェクト単位のため、ログイン監査は DB / 運用ツールで参照する。
監査は追記専用。SIEM 転送は `scripts/audit-forward.py`（カーソル付き NDJSON、
HMAC 署名）を `deployment/systemd/systivra-audit-forward.timer` などで定期実行する。
保持期間は DB の運用で決める（自動削除は未実装）。

### SCIM（ユーザー / グループ）

`SCIM_BEARER_TOKEN` を設定すると `/scim/v2/Users` と `/scim/v2/Groups` が有効になる。
未設定なら 404。Groups は次の拡張でロールを指定する。

```json
{
  "displayName": "systivra-admins",
  "externalId": "systivra-admins",
  "urn:systivra:params:scim:schemas:extension:role": {"role": "ARCHITECT"},
  "members": [{"value": "<user uuid>"}]
}
```

グループのメンバーと、OIDC の `groups` claim が一致すると、最も強いロールを
`core.project_member` に付与する。グループから外しても自動降格しない。
管理画面 `/users` でロールを変更できる（Owner のみ）。

### PITR

```bash
./scripts/pitr-drill.sh
```

専用の一時クラスタで `archive_mode` + WAL アーカイブを作り、ベースバックアップ後の
変更を「指定時刻より前」と「最新 LSN」の両方へ復元する。システムの PostgreSQL には
触れない。**復旧手順そのものの訓練**として価値がある（RTO の実測を含む）。

自前クラスタと同じ設定（`wal_level=replica`、`archive_mode=on`、`archive_command`、
`restore_command`）を本番 PG に適用できるのは**自宅 / コンテナの PostgreSQL だけ**。
マネージド PG では設定権限が無い:

| 環境 | 物理バックアップ / PITR |
|---|---|
| 自前 PostgreSQL | `pitr-drill.sh` の構成をそのまま適用 |
| Azure Database for PostgreSQL | プラットフォームの自動バックアップ + `az postgres flexible-server restore`（Fast Restore / Geo Restore） |
| Amazon RDS | 自動バックアップ + `aws rds restore-db-instance-to-point-in-time` |

Azure / RDS では、**プラットフォーム側の PITR を使う復旧手順**を別途訓練する
（復元先は新サーバーになる点が自前クラスタと違う）。

## 8. インシデント時の最小手順

1. Cookie 盗難の疑い: `UPDATE core.session SET revoked_at=now()` → 全員再ログイン。
2. 総当たり: 429 の `Retry-After` と監査の FAILED を確認。必要なら
   `AUTH_RATE_LIMIT_CAPACITY` を下げて再起動。
3. IdP 侵害: `OIDC_*` を外して再起動（パスワードログインのみ）→ 紐付けと監査を確認。
4. DB 破損: 上の復旧手順。復元後は全セッション失効済み。

## 9. ジョブの異常終了と回収

worker が落ちると `core.job` の行は `RUNNING` のまま残る。worker は起動直後と
毎ティックの先頭で心拍切れを回収するので、通常は**手で触らない**。

```bash
# 心拍が古い RUNNING を確認（WORKER_STALE_SEC 既定 15 秒）
SELECT id, kind, attempt, max_attempts, heartbeat_at
  FROM core.job WHERE status = 'RUNNING'
 ORDER BY heartbeat_at NULLS FIRST;
```

回収の規則:

| 状態 | 動作 |
|---|---|
| 心拍切れ + `attempt < max_attempts` | `QUEUED` に戻して再実行（`attempt` はクレーム時に +1） |
| 心拍切れ + `attempt >= max_attempts` | `FAILED` / `error.code = WORKER_HEARTBEAT_LOST` で**止める** |
| 心拍が新しい | 触らない（生きている job を奪わない） |

回収は `core.job_event` にも残る。`/jobs/{id}/events` を見れば、状態だけが
変わった理由まで追える（静かな状態書換えにしない）:

| 回収種別 | イベント |
|---|---|
| `QUEUED` に戻す | `progress` / `status=QUEUED`, `phase=RECOVERED`, `reason=stalled_heartbeat` |
| `FAILED` で止める | `error` / `status=FAILED`, `code=WORKER_HEARTBEAT_LOST`, `attempt` / `max_attempts` |

実測: `PGDATABASE=<db> python3 scripts/job-recovery-eval.py`。結果は
`data/phase8/job-recovery.json`。

worker のログにも回収が出る:

```text
WARN worker::job: stalled jobs recovered requeued=1 failed=0
INFO worker::job: job claimed job_id=1a592e0a-… kind=NOOP_RECOVERY
ERROR worker::job: job failed … unsupported job kind: NOOP_RECOVERY
WARN worker::job: stalled jobs recovered requeued=0 failed=1
```

```json
{"recoverable":{"status":"FAILED","attempt":2},
 "poison":{"status":"FAILED","attempt":3,"code":"WORKER_HEARTBEAT_LOST"},
 "alive":{"status":"RUNNING","attempt":1}}
```

- `recoverable`: 回収されて再クレームされた（`attempt` 1 → 2）。kind が未対応なので
  処理自体は失敗するが、**回収が効いたこと**は試行回数の増加で分かる。
- `poison`: 試行を使い切っているので再キューされない。ここを `QUEUED` に戻すと、
  プロセスごと落とす job が永久に回り続ける。
- `alive`: 心拍が新しいので奪われない。

処理中は worker が `WORKER_STALE_SEC / 3` 間隔で心拍を打つ。これが無いと、
`WORKER_STALE_SEC` を超える長い job を別 worker が「落ちた」と見なして
**同じ job を二重に実行**する。

処理中の心拍は**実測した**。`phase7-agent-loop-eval.py` が mock LLM に 4 秒の遅延を
入れて `AGENT_RUN` をわざと長くし、`WORKER_STALE_SEC=3`（心拍 1 秒間隔）の
worker を 2 台起動する:

```json
{"heartbeat_during_processing":{"beats":10,"status":"SUCCEEDED_WITH_ERRORS",
 "attempt":1,"workers":2}}
```

- `beats=10`: RUNNING の間に `heartbeat_at` が 10 回更新された。
- `attempt=1`: 2 台目が「落ちた」と誤判定して再キューしなかった。
  心拍が無ければ stale 3 秒で奪われ、`attempt=2` になっていたはず。
- 回収側が本当に効くことは `job-recovery-eval.py` で別に実測済み。
  2 つの実測を合わせて「心拍が新しければ奪われない／途切れれば回収される」
  が成立する。

同じティックで**承認待ちの期限切れ**も処理する。`APPROVAL_TTL_SEC`
（既定 86400 = 24 時間）を過ぎた `PENDING` は `EXPIRED` に閉じる。
承認は「その時点の diff を見て判断する」ものなので、古い依頼をいつまでも
承認可能なまま置かない。`EXPIRED` への決定は 409 で拒否される。
`PGDATABASE=<db> python3 scripts/job-recovery-eval.py` の
`approval_expiry` で実測（`old=EXPIRED` / `fresh=PENDING`）。

## 10. 既知の制約

- マルチノード / HA は未実装。レート制限・セッション失効は単一 API 前提。
- PITR は訓練スクリプトで実測済み。自前 PG では同じ設定を本番に適用できるが、
  マネージド PG（Azure / RDS）では設定権限が無く、プラットフォームの PITR を使う
  （[`deployment/azure/README.md`](../../deployment/azure/README.md) §4）。
- Neo4j Community はオンラインバックアップを持たない。復旧は PG からの再構築。
  clean-room はプロジェクト投影単位で検証済み（DB 全体ではない）。
- SAML、フロントチャネル logout、SCIM の Bulk / 全属性 PATCH は未実装。
- SCIM のグループ解除は自動降格しない（明示変更のみ）。
- パスワードログインの無効化（OIDC 専用強制）は未実装。
- 処理中の心拍は **2 worker で実測済み**。`scripts/phase7-agent-loop-eval.py` の
  `heartbeat_during_processing` を参照（下記）。
  実案件の 90 秒/ケース規模での多重度は未検証。
