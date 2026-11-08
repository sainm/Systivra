#!/usr/bin/env bash
# scripts/verify-db.sh
# P1-DB-01 の完了条件（detail/05 §5.1）を機械で確かめる。
#
# 使い方:
#   ./scripts/verify-db.sh
#   PGHOST=/tmp/pg PGPORT=5432 PGUSER=postgres ./scripts/verify-db.sh
#
# 接続は libpq の標準環境変数（PGHOST / PGPORT / PGUSER / PGPASSWORD）に従う。
# 一時 DB（systivra_verify_<pid>）を作り、最後に必ず消す。
# 前提: psql が PATH にあり、管理者権限のロールで接続できること。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DB="systivra_verify_$$"
FAIL=0

export PGDATABASE="${PGDATABASE:-postgres}"

cleanup() {
  psql -d postgres -q -c "DROP DATABASE IF EXISTS $DB;" >/dev/null 2>&1 || true
}
trap cleanup EXIT

say() { printf '%s\n' "$*"; }
ok()  { printf '  \033[32mok\033[0m   %s\n' "$*"; }
no()  { printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAIL=1; }

psql -d postgres -q -c "CREATE DATABASE $DB;" || { say "一時 DB を作れません"; exit 1; }
export PGDATABASE="$DB"

say "== 適用 =="
# 順序が重要：schema を作ってから権限を付ける。
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/001_core.sql"
ok "001_core.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/002_core_ops.sql"
ok "002_core_ops.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/003_authz.sql"
ok "003_authz.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/004_corpus_document.sql"
ok "004_corpus_document.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/005_corpus_chunk.sql"
ok "005_corpus_chunk.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/006_corpus_evidence.sql"
ok "006_corpus_evidence.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/007_job_events.sql"
ok "007_job_events.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/008_project_of.sql"
ok "008_project_of.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/009_rename_corpus.sql"
ok "009_rename_corpus.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/010_index_build_active.sql"
ok "010_index_build_active.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/011_graph.sql"
ok "011_graph.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/012_graph_relationship_seq.sql"
ok "012_graph_relationship_seq.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/013_code.sql"
ok "013_code.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/014_impact.sql"
ok "014_impact.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/015_test.sql"
ok "015_test.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/016_ai.sql"
ok "016_ai.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/017_inquiry.sql"
ok "017_inquiry.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/018_sso.sql"
ok "018_sso.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/019_session.sql"
ok "019_session.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/021_session_identity.sql"
ok "021_session_identity.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/022_scim.sql"
ok "022_scim.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/023_first_project.sql"
ok "023_first_project.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/024_code_schema_repair.sql"
ok "024_code_schema_repair.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/025_mcp_client.sql"
ok "025_mcp_client.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/026_workflow_actor.sql"
ok "026_workflow_actor.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/027_approval_consume.sql"
ok "027_approval_consume.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/deployment/postgres/roles.sql"
ok "roles.sql（migrations の後）"

say "== RLS =="
psql -q >/dev/null <<'SQL'
INSERT INTO core.workspace (code,name) VALUES ('ws','WS');
INSERT INTO core.project (workspace_id,code,name,source_kind)
  SELECT id,'P1','P1','DIR' FROM core.workspace;
INSERT INTO core.project (workspace_id,code,name,source_kind)
  SELECT id,'P2','P2','DIR' FROM core.workspace;
SQL
PID=$(psql -tAc "SELECT id FROM core.project WHERE code='P1';" | tr -d ' ')

n=$(psql -U systivra_app -tA -c "SET app.project_id='$PID'; SELECT count(*) FROM core.project;" | tail -1)
[ "$n" = "1" ] && ok "project_id 設定時は自プロジェクトのみ（1 件）" || no "自プロジェクトが見えない（${n:-空} 件）"

n=$(psql -U systivra_app -tA -c "SELECT count(*) FROM core.project;" | tail -1)
[ "$n" = "0" ] && ok "未設定時は 0 件・エラーなし" || no "未設定時の挙動が違う（${n:-空} 件）"

say "== 権限分離 =="
if psql -U systivra_app -q -c "UPDATE core.job SET status='RUNNING';" >/dev/null 2>&1; then
  no "app が core.job を UPDATE できてしまう"
else
  ok "app は core.job を UPDATE できない"
fi

if psql -U systivra_app -q -c "UPDATE core.audit_log SET result='x';" >/dev/null 2>&1; then
  no "app が audit_log を UPDATE できてしまう"
else
  ok "audit_log は追記専用"
fi

say "== ジョブ取り出し =="
psql -q >/dev/null <<SQL
INSERT INTO core.job (project_id,kind,priority,status,idempotency_key,input)
VALUES ('$PID','PDF_PARSE',5,'QUEUED','k1','{}'),
       ('$PID','PDF_PARSE',9,'QUEUED','k3','{}'),
       ('$PID','PDF_PARSE',1,'QUEUED','k2','{}');
SQL
# FOR UPDATE は集約と併用できないので、副問い合わせで先に固定する。
got=$(psql -U systivra_scheduler -tA -c \
  "SELECT string_agg(priority::text, ',') FROM (SELECT priority FROM core.job WHERE status='QUEUED' ORDER BY priority DESC, created_at FOR UPDATE SKIP LOCKED) t;" | tail -1)
[ "$got" = "9,5,1" ] && ok "priority DESC で取り出せる（9,5,1）" || no "取り出し順が違う（${got:-空}）"

say "== corpus 分区・索引 =="
n=$(psql -tAc "SELECT count(*) FROM pg_inherits WHERE inhparent='corpus.chunk'::regclass;")
[ "$n" = "32" ] && ok "chunk は 32 分区" || no "chunk 分区数が違う（${n:-空}）"

n=$(psql -tAc "SELECT count(*) FROM pg_inherits WHERE inhparent='corpus.embedding_1024'::regclass;")
[ "$n" = "32" ] && ok "embedding_1024 は 32 分区" || no "embedding 分区数が違う（${n:-空}）"

n=$(psql -tAc "SELECT count(*) FROM pg_indexes WHERE schemaname='corpus' AND indexdef ILIKE '%hnsw%';")
[ "$n" = "32" ] && ok "分区ごとに HNSW が $n 個ある" || no "HNSW が足りない（${n:-0} 個）"

n=$(psql -tAc "SELECT count(*) FROM pg_class WHERE relnamespace='corpus'::regnamespace AND relrowsecurity;")
[ "${n:-0}" -ge 9 ] && ok "corpus の $n 表で RLS 有効" || no "RLS 有効表が足りない（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM information_schema.columns WHERE table_schema='corpus' AND table_name='index_build' AND column_name='is_active';")
[ "$n" = "1" ] && ok "index_build.is_active がある" || no "index_build.is_active が無い"

n=$(psql -tAc "SELECT count(*) FROM pg_indexes WHERE schemaname='corpus' AND indexname='index_build_active_idx';")
[ "$n" = "1" ] && ok "有効索引の一意制約がある" || no "index_build_active_idx が無い"

n=$(psql -tAc "SELECT count(*) FROM corpus.embedding_model;")
[ "$n" = "2" ] && ok "埋め込みモデルが 2 版ある" || no "埋め込みモデル数が違う（${n:-0}）"

n=$(psql -tAc "SELECT has_table_privilege('systivra_scheduler','core.job','INSERT');")
[ "$n" = "t" ] && ok "scheduler は子ジョブを投入できる" || no "scheduler が core.job に INSERT できない"

n=$(psql -tAc "SELECT count(*) FROM pg_policies WHERE schemaname='corpus';")
[ "${n:-0}" -ge 9 ] && ok "corpus の RLS ポリシーが $n 個ある" || no "ポリシーが足りない（${n:-0}）"

say "== corpus 越境 =="
psql -q >/dev/null <<SQL
INSERT INTO corpus.document (id, project_id, doc_type, title)
VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', '$PID', 'MANUAL', 'scope-test');
SQL
n=$(psql -U systivra_app -tA -c "SET app.project_id='$PID'; SELECT count(*) FROM corpus.document;" | tail -1)
[ "$n" = "1" ] && ok "自プロジェクトの corpus.document が見える" || no "corpus の RLS が効いていない（${n:-空} 件）"
n=$(psql -U systivra_app -tA -c "SELECT count(*) FROM corpus.document;" | tail -1)
[ "$n" = "0" ] && ok "未設定時は corpus も 0 件" || no "未設定時に corpus が見える（${n:-空} 件）"

resolved=$(psql -tAc "SELECT core.project_of('document', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "core.project_of が corpus を解決する" || no "project_of の解決先が違う（${resolved:-空}）"

say "== 監査ログ分区 =="
n=$(psql -tAc "SELECT count(*) FROM pg_inherits WHERE inhparent='core.audit_log'::regclass;")
[ "${n:-0}" -ge 12 ] && ok "月次分区が $n 個ある" || no "分区が足りない（${n:-0} 個）"

say "== SET LOCAL 残留（接続再利用）=="
# 同じ接続で「スコープ付き → コミット → スコープ無し」を続けて実行する。
# SET LOCAL はコミット時に「未設定」ではなく空文字へ戻るため、NULLIF が無いと
# ''::uuid で落ちる。接続プールでは必ず起きる。
out=$(psql -U systivra_app -tA \
  -c "BEGIN; SELECT set_config('app.project_id', '$PID', true); COMMIT;" \
  -c "SELECT count(*) FROM core.project;" 2>&1)
last=$(printf '%s\n' "$out" | tail -1)
[ "$last" = "0" ] && ok "コミット後の未スコープ参照が落ちない（0 件）" || no "SET LOCAL 残留で失敗（$last）"

say "== 冪等キー =="
if psql -q -c "INSERT INTO core.job (project_id,kind,status,idempotency_key,input) VALUES ('$PID','PDF_PARSE','QUEUED','k1','{}');" >/dev/null 2>&1; then
  no "同じ idempotency_key で二重登録できてしまう"
else
  ok "同じ idempotency_key は拒否される"
fi

say "== graph_meta =="
n=$(psql -tAc "SELECT count(*) FROM graph_meta.entity_type t JOIN graph_meta.ontology_version v ON v.id=t.ontology_version_id WHERE v.version='v1';")
[ "$n" = "56" ] && ok "ontology v1 の実体型は 56" || no "実体型が違う（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM graph_meta.relation_type t JOIN graph_meta.ontology_version v ON v.id=t.ontology_version_id WHERE v.version='v1';")
[ "$n" = "27" ] && ok "ontology v1 の関係型は 27" || no "関係型が違う（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM information_schema.columns WHERE table_schema='graph_meta' AND table_name='relationship' AND column_name='projection_seq';")
[ "$n" = "1" ] && ok "relationship.projection_seq がある" || no "relationship.projection_seq が無い"

n=$(psql -tAc "SELECT count(*) FROM pg_class WHERE relnamespace='graph_meta'::regnamespace AND relrowsecurity;")
[ "${n:-0}" -ge 4 ] && ok "graph_meta の $n 表で RLS 有効" || no "graph_meta の RLS が足りない（${n:-0}）"

if psql -U systivra_app -q -c "SELECT 1 FROM graph_meta.entity LIMIT 1;" >/dev/null 2>&1; then
  ok "app は graph_meta を読める（未スコープは 0 件）"
else
  no "app が graph_meta を読めない"
fi

say "== code =="
n=$(psql -tAc "SELECT count(*) FROM pg_class WHERE relnamespace='code'::regnamespace AND relrowsecurity;")
[ "${n:-0}" -ge 8 ] && ok "code の $n 表で RLS 有効" || no "code の RLS が足りない（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM pg_policies WHERE schemaname='code';")
[ "${n:-0}" -ge 8 ] && ok "code の RLS ポリシーが $n 個ある" || no "code ポリシーが足りない（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM pg_indexes WHERE schemaname='code' AND indexname='classpath_revision_artifact_idx';")
[ "$n" = "1" ] && ok "classpath revision/artifact の一意制約がある" || no "classpath 一意制約が無い"

n=$(psql -tAc "SELECT is_deferrable FROM information_schema.table_constraints WHERE constraint_schema='code' AND constraint_name='fact_superseded_by_fkey';")
[ "$n" = "YES" ] && ok "fact.superseded_by は遅延 FK" || no "supersede の FK が即時検査（${n:-空}）"

n=$(psql -tAc "SELECT has_table_privilege('systivra_worker','code.fact','INSERT');")
[ "$n" = "t" ] && ok "worker は code.fact を書ける" || no "worker が code.fact に INSERT できない"

psql -q >/dev/null <<SQL
INSERT INTO code.repository (id, project_id, kind, url)
VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc', '$PID', 'DIR', 'repositories/test');
INSERT INTO code.code_revision
  (id, project_id, repository_id, revision, tree_digest, snapshot_key)
VALUES ('dddddddd-dddd-4ddd-8ddd-dddddddddddd', '$PID',
        'cccccccc-cccc-4ccc-8ccc-cccccccccccc', 'r1', '\\x01', 'code/test/r1');
SQL
resolved=$(psql -tAc "SELECT core.project_of('code_revision', 'dddddddd-dddd-4ddd-8ddd-dddddddddddd');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "core.project_of が code_revision を解決する" || no "code_revision のスコープ解決が違う"

psql -q >/dev/null <<SQL
INSERT INTO corpus.evidence
  (id, project_id, source_type, locator, quote, quote_hash, resolution, confidence)
VALUES ('eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee', '$PID', 'CODE',
        jsonb_build_object('path','A.java','byte_start',0,'byte_end',1),
        'a', '\\x00', 'RESOLVED', 1.0);

INSERT INTO code.fact
  (id, project_id, revision_id, subject_kind, subject_key, relation,
   object_kind, object_key, resolution, confidence, analysis_engine,
   engine_version, fact_hash)
VALUES
  ('11111111-1111-4111-8111-111111111111', '$PID',
   'dddddddd-dddd-4ddd-8ddd-dddddddddddd', 'Method', 'A.m', 'CALLS',
   'Method', 'B.m', 'RESOLVED', 0.9, 'tree-sitter', 'test', '\\x01');

INSERT INTO code.fact_evidence (project_id, fact_id, evidence_id, role)
VALUES ('$PID', '11111111-1111-4111-8111-111111111111',
        'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee', 'PRIMARY');

BEGIN;
SET CONSTRAINTS ALL DEFERRED;
UPDATE code.fact
   SET valid_to = now(), superseded_by = '22222222-2222-4222-8222-222222222222'
 WHERE id = '11111111-1111-4111-8111-111111111111';
INSERT INTO code.fact
  (id, project_id, revision_id, subject_kind, subject_key, relation,
   object_kind, object_key, resolution, confidence, analysis_engine,
   engine_version, fact_hash)
VALUES
  ('22222222-2222-4222-8222-222222222222', '$PID',
   'dddddddd-dddd-4ddd-8ddd-dddddddddddd', 'Method', 'A.m', 'CALLS',
   'Method', 'B.m', 'INFERRED', 0.7, 'tree-sitter', 'test', '\\x02');
INSERT INTO code.fact_evidence (project_id, fact_id, evidence_id, role)
VALUES ('$PID', '22222222-2222-4222-8222-222222222222',
        'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee', 'PRIMARY');
COMMIT;
SQL
n=$(psql -tAc "SELECT count(*) FROM code.fact WHERE subject_key='A.m' AND relation='CALLS' AND object_key='B.m' AND analysis_engine='tree-sitter' AND valid_to IS NULL;")
[ "$n" = "1" ] && ok "supersede_then_insert 後の live Fact は 1 行" || no "live Fact が ${n:-空} 行"

say "== impact =="
n=$(psql -tAc "SELECT count(*) FROM pg_class WHERE relnamespace='impact'::regnamespace AND relrowsecurity;")
[ "${n:-0}" -ge 2 ] && ok "impact の $n 表で RLS 有効" || no "impact RLS が足りない（${n:-0}）"

n=$(psql -tAc "SELECT has_table_privilege('systivra_worker','impact.assessment_draft','UPDATE');")
[ "$n" = "t" ] && ok "worker は impact Draft を更新できる" || no "worker が impact Draft を更新できない"

psql -q >/dev/null <<SQL
INSERT INTO impact.assessment_draft (id, project_id, input, status)
VALUES ('ffffffff-ffff-4fff-8fff-ffffffffffff', '$PID', '{}', 'PENDING');
SQL
resolved=$(psql -tAc "SELECT core.project_of('impact_assessment', 'ffffffff-ffff-4fff-8fff-ffffffffffff');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "core.project_of が impact_assessment を解決する" || no "impact のスコープ解決が違う"

say "== test =="
n=$(psql -tAc "SELECT count(*) FROM pg_class WHERE relnamespace='test'::regnamespace AND relrowsecurity;")
[ "$n" = "11" ] && ok "test の $n 表で RLS 有効" || no "test の RLS 有効表が違う（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM pg_policies WHERE schemaname='test';")
[ "$n" = "11" ] && ok "test の RLS ポリシーが $n 個ある" || no "test ポリシーが足りない（${n:-0}）"

n=$(psql -tAc "SELECT has_table_privilege('systivra_worker','test.test_execution','UPDATE');")
[ "$n" = "t" ] && ok "worker は test_execution を更新できる" || no "worker が test_execution を更新できない"

n=$(psql -tAc "SELECT count(*) FROM pg_indexes WHERE schemaname='test' AND indexname='t6_run_env_lock_idx';")
[ "$n" = "1" ] && ok "環境ロックの一意制約がある" || no "環境ロック制約が無い"

psql -q >/dev/null <<SQL
INSERT INTO test.test_plan (id, project_id, code, name, status)
VALUES ('abababab-abab-4aba-8aba-abababababab', '$PID', 'TP-001', 'plan', 'DRAFT');
INSERT INTO test.test_run (id, project_id, name, environment, environment_id, status)
VALUES ('bcbcbcbc-bcbc-4bcb-8bcb-bcbcbcbcbcbc', '$PID', 'run', '{"id":"env-stg"}', 'env-stg', 'QUEUED');
SQL
resolved=$(psql -tAc "SELECT core.project_of('test_plan', 'abababab-abab-4aba-8aba-abababababab');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "core.project_of が test_plan を解決する" || no "test_plan のスコープ解決が違う"
resolved=$(psql -tAc "SELECT core.project_of('test_run', 'bcbcbcbc-bcbc-4bcb-8bcb-bcbcbcbcbcbc');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "core.project_of が test_run を解決する" || no "test_run のスコープ解決が違う"

# 015 が 014 の分支を消していないことの回帰検査。
resolved=$(psql -tAc "SELECT core.project_of('impact_assessment', 'ffffffff-ffff-4fff-8fff-ffffffffffff');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "015 適用後も既存の project_of 分支が残る" || no "015 が古い分支を消した"

say "== ai =="
n=$(psql -tAc "SELECT count(*) FROM information_schema.columns WHERE table_schema='ai' AND table_name='mcp_server' AND column_name IN ('tools','health_detail');")
[ "$n" = "2" ] && ok "MCP Client の tools / health_detail がある" || no "MCP Client 列が足りない（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM information_schema.columns WHERE table_schema='ai' AND table_name='agent_run' AND column_name IN ('requested_by','requested_role');")
[ "$n" = "2" ] && ok "agent_run は依頼者とロールの snapshot を保持する" || no "agent_run の依頼者列が足りない（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM pg_class WHERE relnamespace='ai'::regnamespace AND relrowsecurity;")
[ "$n" = "10" ] && ok "ai の $n 表で RLS 有効" || no "ai の RLS 有効表が違う（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM pg_policies WHERE schemaname='ai';")
[ "$n" = "10" ] && ok "ai の RLS ポリシーが $n 個ある" || no "ai ポリシーが足りない（${n:-0}）"

n=$(psql -tAc "SELECT has_table_privilege('systivra_worker','ai.agent_run','UPDATE');")
[ "$n" = "t" ] && ok "worker は ai.agent_run を更新できる" || no "worker が ai.agent_run を更新できない"

# workflow の capability ノードは起動した人間の権限で動く（026）。
n=$(psql -tAc "SELECT count(*) FROM information_schema.columns WHERE table_schema='ai' AND table_name='workflow_run' AND column_name='requested_by';")
[ "$n" = "1" ] && ok "workflow_run は依頼者を保持する" || no "workflow_run に requested_by が無い"

# 承認は 1 回の実行への許可証。消費済みを持つ列が無いと何度でも使えてしまう。
n=$(psql -tAc "SELECT count(*) FROM information_schema.columns WHERE table_schema='ai' AND table_name='approval_request' AND column_name='consumed_at';")
[ "$n" = "1" ] && ok "approval_request は消費済みを保持する" || no "approval_request に consumed_at が無い"

# worker は job 起動者のロールを実行時に引き直す（降格を即時反映）。
# SELECT だけを与える。書込みは与えない。
n=$(psql -tAc "SELECT has_table_privilege('systivra_worker','core.project_member','SELECT');")
[ "$n" = "t" ] && ok "worker は project_member を読める" || no "worker が project_member を読めない"
n=$(psql -tAc "SELECT has_table_privilege('systivra_worker','core.project_member','INSERT');")
[ "$n" = "f" ] && ok "worker は project_member を書けない" || no "worker が project_member を書ける"

psql -q >/dev/null <<SQL
INSERT INTO ai.workflow_def (id, project_id, code, version, definition, status)
VALUES ('acacacac-acac-4aca-8aca-acacacacacac', '$PID', 'WF-001', 1, '{"nodes":[]}', 'DRAFT');
INSERT INTO ai.workflow_run (id, project_id, workflow_def_id, status, input)
VALUES ('adadadad-adad-4ada-8ada-adadadadadad', '$PID',
        'acacacac-acac-4aca-8aca-acacacacacac', 'PENDING', '{}');
INSERT INTO ai.agent (id, project_id, code)
VALUES ('aeaeaeae-aeae-4aea-8aea-aeaeaeaeaeae', '$PID', 'impact-analyst');
INSERT INTO ai.agent_version
  (id, project_id, agent_id, version, model_profile, prompt_ref, capabilities, budgets)
VALUES ('afafafaf-afaf-4afa-8afa-afafafafafaf', '$PID',
        'aeaeaeae-aeae-4aea-8aea-aeaeaeaeaeae', 1, 'reasoning',
        'prompt://impact-analyst@1', '{knowledge.search}', '{}');
INSERT INTO ai.agent_run
  (id, project_id, agent_version_id, status, input, requested_by, requested_role)
VALUES ('b0b0b0b0-b0b0-4b0b-8b0b-b0b0b0b0b0b0', '$PID',
        'afafafaf-afaf-4afa-8afa-afafafafafaf', 'RUNNING', '{}',
        '33333333-3333-4333-8333-333333333333', 'OWNER');
SQL
resolved=$(psql -tAc "SELECT core.project_of('agent_run', 'b0b0b0b0-b0b0-4b0b-8b0b-b0b0b0b0b0b0');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "core.project_of が agent_run を解決する" || no "agent_run のスコープ解決が違う"
resolved=$(psql -tAc "SELECT core.project_of('workflow_run', 'adadadad-adad-4ada-8ada-adadadadadad');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "core.project_of が workflow_run を解決する" || no "workflow_run のスコープ解決が違う"

# 016 が test / impact の分支を消していないことの回帰検査。
resolved=$(psql -tAc "SELECT core.project_of('test_run', 'bcbcbcbc-bcbc-4bcb-8bcb-bcbcbcbcbcbc');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "016 適用後も test_run の分支が残る" || no "016 が test_run 分支を消した"

say "== inquiry =="
n=$(psql -tAc "SELECT count(*) FROM pg_class WHERE relnamespace='impact'::regnamespace AND relrowsecurity;")
[ "$n" = "2" ] && ok "impact の 2 表で RLS 有効" || no "impact の RLS が足りない（${n:-0}）"

psql -q >/dev/null <<SQL
INSERT INTO impact.change_inquiry
  (id, project_id, question, input_kind, status)
VALUES ('c1c1c1c1-c1c1-4c1c-8c1c-c1c1c1c1c1c1', '$PID', 'q', 'ENTITY', 'QUEUED');
SQL
resolved=$(psql -tAc "SELECT core.project_of('change_inquiry', 'c1c1c1c1-c1c1-4c1c-8c1c-c1c1c1c1c1c1');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "core.project_of が change_inquiry を解決する" || no "change_inquiry のスコープ解決が違う"

say "== sso =="
n=$(psql -tAc "SELECT count(*) FROM pg_constraint
  WHERE conrelid='core.external_identity'::regclass AND contype='u';")
[ "$n" = "2" ] && ok "external_identity の一意制約が 2 個ある" || no "external_identity の一意制約が足りない（${n:-0}）"

uid=$(psql -tAc "WITH ins AS (
  INSERT INTO core.app_user (login_id, display_name)
  VALUES ('sso-user','SSO User') RETURNING id
) SELECT id FROM ins;" | tr -d ' ')
psql -q >/dev/null <<SQL
INSERT INTO core.external_identity (user_id, provider, subject)
VALUES ('$uid', 'https://idp.example.test', 'subject-1');
SQL
n=$(psql -U systivra_app -tAc "SELECT count(*) FROM core.external_identity WHERE subject='subject-1';" | tail -1)
[ "$n" = "1" ] && ok "app は external_identity を読める" || no "app が external_identity を読めない（${n:-0}）"

n=$(psql -tAc "SELECT count(*) FROM pg_class
  WHERE relname='external_identity' AND relrowsecurity;")
[ "$n" = "0" ] && ok "external_identity は RLS 対象外（認証前参照）" || no "external_identity に RLS が付いている"

say "== session =="
n=$(psql -tAc "SELECT has_table_privilege('systivra_app','core.session','INSERT');")
[ "$n" = "t" ] && ok "app は session を発行できる" || no "app が session を発行できない"
n=$(psql -tAc "SELECT count(*) FROM pg_constraint
  WHERE conrelid='core.session'::regclass AND contype='f';")
[ "$n" = "1" ] && ok "session は app_user を参照する" || no "session の FK が無い"
n=$(psql -tAc "SELECT count(*) FROM pg_class
  WHERE relname='session' AND relrowsecurity;")
[ "$n" = "0" ] && ok "session は RLS 対象外（認証中参照）" || no "session に RLS が付いている"
n=$(psql -tAc "SELECT count(*) FROM information_schema.columns
  WHERE table_schema='core' AND table_name='session'
    AND column_name IN ('provider','subject','idp_sid');")
[ "$n" = "3" ] && ok "session に IdP 紐付け列がある" || no "session の IdP 紐付け列が足りない（$n）"

say "== extensions =="
n=$(psql -tAc "SELECT count(*) FROM pg_extension
  WHERE extname NOT IN ('plpgsql','vector','pg_trgm');")
[ "$n" = "0" ] && ok "拡張は plpgsql / vector / pg_trgm だけ" || no "許可外の拡張がある（$n）"
n=$(psql -tAc "SELECT count(*) FROM pg_indexes
  WHERE schemaname='graph_meta' AND indexname='entity_name_trgm';")
[ "$n" = "1" ] && ok "graph_meta.entity に trigram 索引がある" || no "entity_name_trgm が無い"

echo
if [ "$FAIL" -eq 0 ]; then
  printf '\033[32m全て通過\033[0m\n'
else
  printf '\033[31m失敗あり\033[0m\n'
  exit 1
fi
