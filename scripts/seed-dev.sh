#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DB="${PGDATABASE:-systivra}"
PASSWORD="${SEED_PASSWORD:-SystivraDemo1!}"

export PATH="$HOME/.cargo/bin:$PATH"
HASH="$(cd "$ROOT" && cargo run -q -p app --example hash_password -- "$PASSWORD")"

psql -d "$DB" -v ON_ERROR_STOP=1 -v hash="$HASH" <<'SQL'
INSERT INTO core.workspace (id, code, name)
VALUES ('11111111-1111-4111-8111-111111111111', 'WS-DEMO', 'デモワークスペース')
ON CONFLICT (id) DO NOTHING;

INSERT INTO core.project (id, workspace_id, code, name, source_kind)
VALUES (
  '22222222-2222-4222-8222-222222222222',
  '11111111-1111-4111-8111-111111111111',
  'PRJ-DEMO', '顧客管理システム', 'DIR'
)
ON CONFLICT (id) DO NOTHING;

INSERT INTO core.app_user (id, login_id, display_name, password_hash)
VALUES (
  '33333333-3333-4333-8333-333333333333',
  'alice',
  'Alice',
  :'hash'
)
ON CONFLICT (login_id) DO UPDATE SET password_hash = EXCLUDED.password_hash;

INSERT INTO core.project_member (project_id, user_id, role)
VALUES (
  '22222222-2222-4222-8222-222222222222',
  '33333333-3333-4333-8333-333333333333',
  'OWNER'
)
ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role;
SQL

printf 'seeded user alice / project PRJ-DEMO\n'
