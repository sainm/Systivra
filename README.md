# Systivra

Software Engineering Intelligence Platform — 設計書・ソースコード・データベース・テストを「証拠」でつなぎ、改修の影響範囲を計算・検証・監査できるようにする分析基盤。

## 設計文書

- 正式版（日本語）: [`docs/design/ja/00-全体設計.md`](docs/design/ja/00-全体設計.md)
- 中国語版（内部参照・**凍結**）: [`docs/design/00-README.md`](docs/design/00-README.md)
- 一致性監査: [`docs/design/review-2026-11-05.md`](docs/design/review-2026-11-05.md)

## 状態

Phase 0（設計）完了。Phase 1 の **骨格（`P1-ENV-01`）と SQL 層（`P1-DB-01`）は実装・検証済み**。
アプリケーションのロジック（`P1-API-01` 以降）は未着手。

できていること：

- 13 crate の Cargo workspace。依存の向きは `02-アーキテクチャ` §3 の表に固定
- `systivra-api` … `/healthz`（DB 非依存）/ `/readyz`（DB 依存）、`X-Trace-Id` の付与と伝播
- 認証 … ログイン / ログアウト / `/me` / `/projects/{pid}/me`。セッション Cookie（HMAC 署名）と PAT の両対応
- 認可 … `06-API.md` §4.1 の権限マトリクスを `domain::Permission` に 1 箇所だけ実装。非所属プロジェクトは 404
- `systivra-worker` … 起動と graceful shutdown の骨格
- マイグレーション 001〜003、3 ロール、RLS、監査ログの月次分区
- 50 テスト、`clippy -D warnings`、`cargo fmt --check` が緑

```bash
cargo test --workspace

# マイグレーションの検証（一時 DB を作って 10 項目を確認し、最後に消す）
PGHOST=localhost PGUSER=postgres ./scripts/verify-db.sh

# 初期アカウント用のパスワードハッシュ
cargo run -p app --example hash_password -- 'your-password'

# API を起動して確認
cp .env.example .env    # SESSION_SECRET を埋める（openssl rand -hex 32）
PORT=8080 DATABASE_URL=postgres://systivra_app@localhost:5432/systivra \
SESSION_SECRET=$(openssl rand -hex 32) SESSION_COOKIE_SECURE=false \
  cargo run --bin systivra-api

curl -i localhost:8080/healthz   # 200
curl -i localhost:8080/readyz    # DB が生きていれば 200、落ちていれば 503

curl -i -X POST localhost:8080/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"login_id":"alice","password":"..."}'
```

## 構成

```text
crates/                    13 crate（domain / contracts / store / providers /
                           engine-rag / engine-graph / engine-code / engine-impact /
                           engine-test / runtime / app / api / worker）
migrations/                001_core / 002_core_ops / 003_authz
deployment/postgres/       roles.sql（3 ロール + 権限。冪等）
deployment/docker/         Dockerfile
docker-compose.yml         db + api + worker
scripts/verify-db.sh       P1-DB-01 の完了条件を機械で確認
.github/workflows/ci.yml   fmt / clippy / test / DB 検証 / 日本語文書の簡体字検査

docs/design/ja/            設計正本（日本語）
  00〜09                  全体設計 9 部
  reference/              付録 3 本（日本語検索 / OSS 評価 / 規模算定）
  detail/                 実装直結の詳細設計（01 Excel 取込 〜 09 テスト実行）
docs/design/               中国語版（内部参照・凍結）+ 一致性監査 review-2026-11-05.md
```

> **日本語版が唯一の正本**。2026-11-05 以降の設計変更は `ja/` にのみ反映する。中国語版は当時の意思決定の記録として凍結し、数字・版・契約の照会には使わない。
