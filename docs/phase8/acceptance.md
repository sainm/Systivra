# Phase 8 進捗・受入記録（運用引渡し: 復旧・OIDC SSO・セッション失効）

実施日: 2026-11-07  
対象: `08-実装計画.md` Phase 8 / `02-アーキテクチャ` §7 / `detail/04-api-contract`  
環境: macOS arm64、PostgreSQL 18.4、Neo4j 2026.05 Community、Rust 1.99.1、Node.js 26.0.0

> 判定: **Phase 8 の縦断切片（復旧・OIDC SSO・セッション失効・認証強化・SCIM・
> 監査転送）は完了。Phase 8 全体受入は未完了。**
> `pg_dump` + 原本データの暗号化バックアップから別 DB へ復元し、29 表の件数・内容ハッシュ・
> RLS・FTS・Fact 一意性・原本ハッシュを照合した。復元後は全セッションを失効させ、
> Neo4j のプロジェクト投影を空にしてから `graph/rebuild` し、Pg/Neo4j 一致（drift 0）を
> 実測した。RTO は **5.20 秒**（暗号化解除 + clean-room 含む）。
> OIDC は discovery / JWKS / 認可コード + PKCE(S256) / state / nonce を検証し、
> mock IdP との E2E **39/39**。logout / back-channel logout 後の Cookie 再利用は 401、
> 認証の連続失敗は **429 + Retry-After**、アイドル期限は 401。
> SCIM Users / Groups とグループ→ロール同期、パスワードログイン監査、SIEM 転送、
> PITR（専用一時クラスタ）も実測した。
> SAML、真の HA / マルチノード、WAL の常時転送、オフサイト保管の実運用は未完了であり、
> 正式な運用引渡しとは扱わない。

---

## 1. 実装済み範囲

| 領域 | 実装 |
|---|---|
| バックアップ | `scripts/backup.sh`。`pg_dump`（custom）+ `SYSTIVRA_DATA_DIR` の tar + manifest（件数・内容ハッシュ・SHA-256・監査/ジョブ最終時刻・git rev） |
| 復旧訓練 | `scripts/restore-drill.sh`。暗号化バックアップの復号、別 DB へ `pg_restore`、roles 再適用、**全セッション失効**、29 表の件数と 9 表の内容ハッシュ、RLS、`core.project_of`、FTS、evidence 結合、live Fact 重複、原本ハッシュを照合。`CLEAN_ROOM=1` で Neo4j 投影を空にしてから rebuild |
| バックアップ運用 | 暗号化（AES-256-CBC / PBKDF2）、保持期間、`BACKUP_UPLOAD_CMD` によるオフサイトフック、systemd / launchd の定期実行テンプレート |
| PITR | `scripts/pitr-drill.sh`。専用の一時 PostgreSQL クラスタで `archive_mode` + WAL アーカイブ、ベースバックアップ、指定時刻と最新 LSN への復元 |
| グラフ復旧 | 復元 DB で API を起動し `POST /graph/rebuild`。`graph/stats` の Pg/Neo4j node 数一致と `drift_rate=0` を確認 |
| SSO DDL | `018_sso.sql`。`core.external_identity`（provider=issuer、subject、一意制約 2 つ）。認証前参照のため RLS 対象外 |
| OIDC | `openidconnect` 4.1。discovery の遅延キャッシュ、認可コード + PKCE(S256)、state/nonce、ID token の署名・issuer・audience・期限検証 |
| 紐付け | 管理者が `core.external_identity` へ事前登録。未知の subject は自動プロビジョニングせず `not_linked` |
| セッション DDL | `019_session.sql` の `core.session`、`021_session_identity.sql` の provider / subject / idp_sid |
| セッション管理 | 自分のセッション一覧、個別失効、他端末の全失効、アイドルタイムアウト（`SESSION_IDLE_MINUTES`）、期限切れ削除（worker） |
| IdP 主導ログアウト | `POST /auth/oidc/backchannel-logout`。logout token の署名・iss・aud・exp・events・jti リプレイを検証し、sub / sid で失効 |
| SCIM | `022_scim.sql` / `023_first_project.sql` / `029_enterprise_ops.sql`。Users / Groups の作成・一覧・PATCH（active）・削除、Groups PATCH（displayName / members / role）、ETag（`meta.version`）+ If-Match / If-None-Match、Bulk（最大 50 操作、`failOnErrors`）、`SCIM_BEARER_TOKEN`、グループ→ロール同期、グループ解除時の SCIM 由来ロール降格（`role_source`） |
| ユーザー管理 | `/projects/{pid}/members` の一覧・ロール変更、`/users` 画面、OIDC の groups claim 同期 |
| セッション | Cookie は `session_id:user_id:exp` の HMAC 署名。毎リクエスト DB で失効・期限を判定。`logout` は現在のセッションを失効させる。OIDC フローは別 Cookie に state/nonce/verifier/return_to を保持 |
| レート制限 | `/auth/login`、`/auth/oidc/start`、`/auth/oidc/callback` を IP 単位のトークンバケットで制限。既定 容量 10 / 毎分 10 補充。超過は `429 AUTH_RATE_LIMITED` + `Retry-After`。`AUTH_RATE_LIMIT_BACKEND=pg` で `core.rate_limit_bucket` の共有固定窓になり、複数 API で合算する（PG 障害時はメモリへ退避） |
| セキュリティヘッダ | `nosniff` / `Referrer-Policy: no-referrer` / `X-Frame-Options: DENY` / `Permissions-Policy`。HSTS は https 設定時のみ |
| パスワード監査 | `auth.password.login` の SUCCESS / FAILED。login_id は SHA-256、理由を `detail.reason` に残す |
| SSO 監査 | `auth.sso.login` の SUCCESS / FAILED。subject は SHA-256、失敗理由を残す |
| 失敗時 | `/login?sso_error=...` へ 303。`state_invalid` / `state_expired` / `provider_error` / `exchange_failed` / `not_linked` / `unavailable` / `internal` |
| API | `GET /auth/config`、`GET /auth/oidc/start`、`GET /auth/oidc/callback`。未設定の start は `503 AUTH_SSO_UNAVAILABLE` |
| UI | ログイン画面に IdP 名付き SSO 入口。パスワードフォームは併置。理由別エラー、ja / en / zh-CN |
| 監査転送 | `scripts/audit-forward.py`（カーソル付き NDJSON、HMAC 署名）と `scripts/mock-siem.py` |
| 運用手順 | [operations.md](operations.md)。バックアップ / PITR / 復旧 / SSO / セッション失効 / SCIM / レート制限 / 監査 / インシデント |
| 検証 | `scripts/mock-oidc-idp.py`（stdlib + openssl、RS256）、`scripts/phase8-sso-eval.py`、`check:ui:phase8` |

---

## 2. 受入結果

| # | 受入 | 結果 | 証拠 |
|---|---|---|---|
| 1 | 復旧訓練合格 | **PASS** | 29 表の件数一致、9 表の内容ハッシュ一致、原本 392 ファイルの SHA-256 一致 |
| 2 | RTO を実測する | **PASS** | 5.20 秒（復号 + 復元 + clean-room rebuild + stats 完了） |
| 3 | RLS が復元後も効く | **PASS** | 未スコープ chunk 0 件、スコープ内 1839 件、`project_of(document)` 一致 |
| 4 | 検索と証拠が復元される | **PASS** | FTS 1839 件、evidence-chunk 結合 1996 件、live Fact 重複 0 |
| 5 | グラフを再構築できる | **PASS** | rebuild 200、Pg 5 / Neo4j 5、`drift_rate=0.0` |
| 6 | 復元後にセッションを引き継がない | **PASS** | 復元後の live session 0（全失効）、復元 DB のユーザーで再ログイン |
| 7 | OIDC でログインできる | **PASS** | 39/39、`/auth/me` が本人、`last_login_at` 更新、監査 SUCCESS、11ms |
| 8 | state / nonce 改竄を拒否 | **PASS** | tampered state → `state_invalid`、bad nonce → `exchange_failed`、両方 FAILED 監査 |
| 9 | 未知の identity を拒否 | **PASS** | `not_linked`、`identity_unlinked` を監査 |
| 10 | IdP 停止後も中核が動く | **PASS** | OIDC 未設定 API で `/auth/config` は `oidc:null`、start 503、パスワードログイン 200 |
| 11 | logout 後は Cookie を再利用できない | **PASS** | `POST /auth/logout` 204 → 同じ Cookie の `/auth/me` は 401 |
| 12 | 連続失敗を 429 で止める | **PASS** | 正しいログイン 1 回 + 失敗 4 回の後、5 回目が `429` + `Retry-After: 60` |
| 13 | セキュリティヘッダが付く | **PASS** | nosniff / referrer / frame / permissions を確認。http では HSTS なし |
| 14 | パスワードログインを監査する | **PASS** | `auth.password.login` の SUCCESS / FAILED、login_id の生値なし |
| 15 | ログイン UI が 1440/607 で壊れない | **PASS** | SSO ボタン + パスワード併置、607px で横 overflow なし、エラー文言表示 |
| 16 | PITR で指定時刻へ戻せる | **PASS** | 専用クラスタでベースバックアップ + WAL 7 本、指定時刻は id=2 が 0、最新 LSN は 1、**2.85 秒** |
| 17 | clean-room でグラフを再構築できる | **PASS** | Neo4j のプロジェクト投影を 0 にしてから rebuild、Pg/Neo4j 5/5、drift 0 |
| 18 | SCIM でユーザー/グループを管理できる | **PASS** | Users 作成・filter・PATCH active、Groups 作成・role 拡張・member、401（Bearer 無し） |
| 19 | グループ→ロールが反映される | **PASS** | SCIM Group の member と OIDC `groups` claim の両方で `ARCHITECT` を実測 |
| 20 | セッションを一覧・個別失効・全失効できる | **PASS** | 2 セッション → logout-all で 1 件失効 → current 1 件、DELETE で 204 |
| 21 | アイドル期限で失効する | **PASS** | `last_seen_at` を 2 時間前にして `/auth/me` が 401 |
| 22 | 監査を SIEM へ転送できる | **PASS** | NDJSON 326 行、HMAC 検証、再実行 0 行（カーソル 326） |
| 23 | 暗号化・保持・オフサイト・定期実行 | **PASS（フックまで）** | AES-256-CBC の復号復元、30 日保持、`BACKUP_UPLOAD_CMD`、systemd/launchd テンプレート |
| 24 | worker 停止後にジョブが回収される | **PASS** | 心拍切れ + 試行残り → 再クレーム（`attempt` 1→2）、試行を使い切った job → `FAILED` / `WORKER_HEARTBEAT_LOST` で停止、心拍が新しい job は奪わない |
| 25 | 共有レート制限が複数 API で合算される | **PASS** | `AUTH_RATE_LIMIT_BACKEND=pg` の API 2 本（容量 6）に 3 + 3 回失敗 → 7 回目は両方 429 + `Retry-After` |
| 26 | フロントチャネルログアウトでセッションが消える | **PASS** | `iss` 不一致 400、`iss` + `sid` 一致で 200 → `/auth/me` 401、監査 `auth.sso.frontchannel_logout` |
| 27 | SCIM の ETag / PATCH / Bulk / 降格 | **PASS** | ETag = `meta.version`、If-None-Match 304、If-Match 不一致 412、Groups PATCH remove → `VIEWER|SCIM`、Bulk 2 操作（201/200）と `failOnErrors=1` の打切り |

---

### 2.1 NOBYPASSRLS 下のジョブ実行（2026-11-08）

`systivra_scheduler` から `BYPASSRLS` を外し、横断閲覧は `scheduler_scope` 方針
（`core.job` / `core.project` / `ai.approval_request` / `test.test_execution`）で行う。
Azure Database for PostgreSQL では管理者が `BYPASSRLS` を付与できないため（`deployment/azure/README.md`）。
再現は `scripts/scheduler-rls-eval.py`（12 チェック）。

| 項目 | 実測 |
|---|---|
| ロール | `rolbypassrls=false`、`scheduler_scope` 4 表、app へは非適用（`verify-db.sh`） |
| PDF_PARSE | `phase2-revision-eval.py` が v1/v2 の解析・グラフ投影・改版切替まで通過 |
| CODE_SYNC / ANALYZE | DIR リポジトリ同期 → 解析 `files_analyzed=12`、Fact 20 件 |
| TEST_RUN prepare | run が `READY`（worker がケース数を確定） |
| AGENT_RUN / Q-01 | `phase7-agent-loop-eval.py` が完走（予算・能力拒否・heartbeat 含む） |
| セッション掃除 | scheduler の `core.session` DELETE が成功（`GRANT` を追加） |
| 漂流 / 保持 | worker 起動時の drift 検出と保持期限適用が動作（publish はプロジェクト別 tx に修正） |
| Outbox 投影 | 806 イベントを publish、dead letter 0。`projection_state` はプロジェクトごとの scoped tx で更新 |

> 実装上の修正: `projection_state` と outbox の publish を同一の未スコープ tx で書くと
> RLS の `WITH CHECK` に弾かれる。publish は未スコープ、`projection_state` は
> プロジェクト別のスコープ付き tx に分けた。`core.session` の掃除には
> scheduler への `SELECT/UPDATE/DELETE` 付与が要る（`roles.sql`）。

## 3. 実測値

### 3.1 復旧訓練

| 項目 | 値 |
|---|---|
| バックアップ | `20261107T063726Z`、暗号化 dump 2,470,576 bytes、originals 6.3MB |
| 件数照合 | 29 表（監査、ジョブ、文書 65、chunk 1,839、evidence 1,996、embedding 1,946、entity 8、relationship 6、ontology 6、session、external_group） |
| 内容ハッシュ | 9 表が一致（project / user / document / chunk / evidence / fact / entity / relationship / external_identity） |
| RTO | **5.20 秒**（復号 + clean-room rebuild 含む。暗号化なし・clean-room なしは 2.5 秒台） |
| グラフ | clean-room（プロジェクト投影 0）→ rebuild 200、Pg 5 / Neo4j 5、drift 0.0 |
| セッション | 復元後に `revoked_at IS NULL` が 0 件 |
| 原本 | tar の SHA-256 一致、展開 392 ファイル |
| PITR | 専用クラスタ、WAL 7 本、指定時刻復元 2.85 秒 |

RPO は「バックアップ時点の一貫したスナップショット」まで。WAL の継続転送・PITR は
未実装のため、バックアップ間隔（運用で決める）が実質 RPO になる。

### 3.2 OIDC SSO とセキュリティ

| 項目 | 値 |
|---|---|
| E2E | **50 / 50 checks** |
| ログイン往復 | 10〜13 ms（start → IdP authorize → callback → session。最新 11 ms） |
| logout / back-channel | 204 / 200、直後の同一 Cookie は 401。logout token のリプレイ・改竄は 400 |
| レート制限 | 6 回目で 429、`Retry-After: 60`（容量 6 / 毎分 1 補充の試験設定）。共有バックエンドは 2 API へ 3 + 3 回で 7 回目が両方 429 |
| セッション管理 | 2 件 → logout-all で 1 件失効 → current 1 件、DELETE 204、アイドル 401 |
| SCIM | Users 一覧 / 作成 / PATCH active、Groups 作成、Bearer 無し 401、group→ARCHITECT |
| SIEM | 326 行転送、HMAC 検証、再実行 0 行（カーソル 326） |
| 監査 | SUCCESS / FAILED の両方（累積）、SSO の SUCCESS / 各 FAILED 理由 |
| フロー Cookie | HttpOnly / SameSite=Lax / Path=`/api/v1/auth/oidc` / Max-Age=600 |
| セッション Cookie | HttpOnly / SameSite=Lax / Path=/ / Max-Age=43200。`secure` は設定時 |
| PKCE | S256（RFC 7636 のテストベクタを単体テストで固定） |
| ヘッダ | nosniff / no-referrer / DENY / Permissions-Policy、http では HSTS なし |
| UI | 1440 / 607、エラー通知、パスワードフォーム併置、pageerror / 4xx 0 |

### 3.3 ゲート

| ゲート | 結果 |
|---|---|
| `cargo fmt --all -- --check` | 緑 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 緑 |
| `cargo test --workspace` | **211 passed**、0 failed |
| `cargo run -p api --bin openapi -- --check` | `openapi.json is up to date` |
| `npm run check:i18n` / `npm run build` | 536 keys / build 成功 |
| `scripts/verify-db.sh` | 001〜023、SSO / session / SCIM を含め全項目通過 |
| `scripts/restore-drill.sh`（暗号化 + clean-room） | 不合格 0、RTO 5.20 秒 |
| `scripts/pitr-drill.sh` | 2.85 秒、指定時刻 / 最新 LSN の両方 PASS |
| `scripts/phase8-sso-eval.py` | 39 / 39 |
| `scripts/phase8-siem-eval.sh` | 326 行 / 再実行 0 行 |
| `npm run check:ui:phase8` | ログイン / セキュリティ / ユーザー、1440 / 607、pageerror 0 |

---

## 4. 既知の制約

- Neo4j は PG からの導出物として扱い、バックアップに含めない。復旧は `graph/rebuild`。
  clean-room は「プロジェクトの投影を空にする」訓練であり、Neo4j データベース全体の
  空からの復旧ではない。Community 版はオンラインのバックアップ API を持たない。
- PITR 訓練は `initdb` / `pg_ctl` / `pg_basebackup` と `archive_mode` の設定ができる
  環境を前提とする。マネージド PostgreSQL では WAL アーカイブの設定権限が無い場合がある。
  本番の常時 WAL 転送、オフサイト保管、保持期間の運用は未設定。
- バックアップの暗号化鍵は環境変数。鍵管理（KMS / 封筒暗号）は未実装。
  `BACKUP_UPLOAD_CMD` はフックであり、実際のクラウド転送は環境側で設定する。
- OIDC の back-channel logout とフロントチャネル logout（`iss` + `sid`）は実装。
  SAML、IdP 主導のセッション一覧同期は未実装。
- 未知の subject は自動プロビジョニングしない。紐付けは SQL で行う（`06-API.md` §2.0）。
- SCIM は Users / Groups、ETag、Groups の displayName / members / role PATCH、Bulk まで実装。
  `PUT` の完全置換仕様（readOnly 属性の保持など）と任意属性の PATCH は未対応。
  グループ→ロールは最も強いロールを付与し、グループから外れた SCIM 由来メンバーは
  `VIEWER` へ降格する（`OWNER` と手動付与は触らない）。
- ID token の署名鍵ローテーションは discovery キャッシュの再取得で追随するが、
  明示的な定期更新は無い。交換失敗時にキャッシュを捨てる。
- OIDC discovery は HTTPS を前提とする（`localhost` のみ http を許す）。
  `OIDC_REDIRECT_URI` はブラウザから見た Web オリジンに合わせる。
- セッションは 12 時間の絶対期限 + `SESSION_IDLE_MINUTES` のアイドル期限。
  worker が期限切れ・失効済みを 7 日後に削除する。端末一覧 / 個別失効 / 全失効は
  `/auth/sessions` と `/security` で提供する。
- 旧形式の Cookie（session id なし）は 401。アップグレード後は再ログインが必要。
- レート制限は既定で単一 API プロセスのメモリ保持。`AUTH_RATE_LIMIT_BACKEND=pg` で
  `core.rate_limit_bucket` の共有固定窓（窓 60 秒）に切り替わり、複数 API で合算する。
  PG 障害時はメモリへ退避する（厳密性より可用性）。`TRUST_PROXY=1` は前段が
  `X-Forwarded-For` を上書きする構成でのみ使う。
- パスワードログインの無効化（OIDC 専用強制）は未実装。SSO と併存する。
- 監査の失敗記録は `project_id=NULL`（プロジェクト確定前）。既存の監査照会は
  `project_id` 単位のため、ログイン監査は DB / 運用ツールで参照する。
- SIEM 転送はカーソル付きのバッチ転送。at-least-once の保証、転送失敗時の再試行キュー、
  SIEM 側の重複排除は環境側で担保する。HA / マルチノードは対象外。単一 API / worker。

---

## 5. 再現

```bash
# 1. バックアップ（暗号化 + 保持 + オフサイトフック）
PGHOST=localhost PGUSER=<admin> PGDATABASE=systivra \
BACKUP_ENCRYPTION_KEY=<passphrase> BACKUP_RETENTION_DAYS=30 \
BACKUP_UPLOAD_CMD='<dir を受け取るコマンド>' ./scripts/backup.sh

# 2. 復旧訓練（別 DB を作り、終了時に削除する）
PGHOST=localhost PGUSER=<admin> \
BACKUP_ENCRYPTION_KEY=<passphrase> CLEAN_ROOM=1 ./scripts/restore-drill.sh
KEEP=1 ... ./scripts/restore-drill.sh   # 残す場合

# 3. PITR 訓練（専用の一時クラスタ。システムの PG には触れない）
./scripts/pitr-drill.sh

# 4. OIDC SSO / back-channel logout / SCIM / 429 / セッション / 監査の E2E
cargo build -p api
PGHOST=localhost PGUSER=<admin> python3 scripts/phase8-sso-eval.py

# 5. 監査の SIEM 転送
PGHOST=localhost PGUSER=<admin> ./scripts/phase8-siem-eval.sh

# 6. UI（SSO 有効な API を Vite の proxy 先に起動しておく）
cd apps/web
PGHOST=localhost PGUSER=<admin> WEB_BASE=http://localhost:15173 \
  SSO_PROVIDER_LABEL="ローカル IdP" npm run check:ui:phase8
```

SSO を有効にする API の環境変数:

```bash
OIDC_ISSUER=http://localhost:11540
OIDC_CLIENT_ID=systivra-eval
OIDC_CLIENT_SECRET=eval-secret
OIDC_REDIRECT_URI=http://localhost:15173/api/v1/auth/oidc/callback
OIDC_PROVIDER_LABEL="ローカル IdP"
SCIM_BEARER_TOKEN=scim-eval-token

# 共有レート制限の実測は、同じ DB を指す API を 2 本
# （AUTH_RATE_LIMIT_BACKEND=pg、容量 6）同時に起動して 3 + 3 回ずつ失敗させる。
# E2E は SSO 用 API を容量 100、レート制限の試験用 API を容量 6、
# 共有の試験用 API を 18101 / 18102 で起動する。
SESSION_IDLE_MINUTES=60
```

E2E は SSO 用 API を容量 100、レート制限の試験用 API を容量 6 で起動する。

---

## 6. 次ゲート

Phase 8 の正式完了には、少なくとも次が必要。

1. SAML、IdP 主導のセッション一覧同期の顧客要件確定と実装。
2. SCIM の `PUT` 完全置換と任意属性 PATCH。
3. 本番 WAL 常時転送・オフサイト保管・KMS 鍵管理・保持期間の運用設定。
4. Neo4j データベース全体の clean-room 復旧と、Neo4j 停止を跨ぐ手順。
5. HA 配置（API 複数、worker 複数、PG レプリカ）と、共有ストアでのセッション失効・
   SIEM カーソル（レート制限の共有は実装済み）。
6. 顧客環境での運用手順レビュー、SIEM 側の重複排除、監査保持期間。
