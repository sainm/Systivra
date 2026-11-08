#!/usr/bin/env bash
# Reproduce the Phase 4 2.5M Fact benchmark in an isolated database.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DB="systivra_phase4_fact_scale_$$"
PORT="${PHASE4_FACT_SCALE_PORT:-18120}"
LOG="$ROOT/data/phase4-scale/api.log"
API_PID=""
WORKER_PID=""

cleanup() {
  if [ -n "$WORKER_PID" ] && kill -0 "$WORKER_PID" 2>/dev/null; then
    kill "$WORKER_PID" 2>/dev/null || true
    wait "$WORKER_PID" 2>/dev/null || true
  fi
  if [ -n "$API_PID" ] && kill -0 "$API_PID" 2>/dev/null; then
    kill "$API_PID" 2>/dev/null || true
    wait "$API_PID" 2>/dev/null || true
  fi
  dropdb --if-exists "$DB" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"
cargo build -q -p api --bin systivra-api -p worker --bin systivra-worker

dropdb --if-exists "$DB"
createdb "$DB"
for migration in migrations/*.sql; do
  psql -X -q -v ON_ERROR_STOP=1 -d "$DB" -f "$migration"
done
psql -X -q -v ON_ERROR_STOP=1 -d "$DB" -f deployment/postgres/roles.sql
PGDATABASE="$DB" "$ROOT/scripts/seed-dev.sh"

mkdir -p "$(dirname "$LOG")"
PORT="$PORT" \
DATABASE_URL="postgres://systivra_app@localhost:5432/$DB" \
SESSION_SECRET="phase4-fact-scale-session-secret-0123456789abcdef" \
SESSION_COOKIE_SECURE=false \
PUBLIC_BASE_URL="http://127.0.0.1:$PORT" \
SYSTIVRA_DATA_DIR="$ROOT/data" \
NEO4J_URI="${NEO4J_URI:-bolt://127.0.0.1:7687}" \
NEO4J_USER="${NEO4J_USER:-neo4j}" \
NEO4J_PASSWORD="${NEO4J_PASSWORD:-neo4j}" \
NEO4J_DATABASE="${NEO4J_DATABASE:-neo4j}" \
RUST_LOG=warn \
"$ROOT/target/debug/systivra-api" >"$LOG" 2>&1 &
API_PID=$!

DATABASE_URL="postgres://systivra_scheduler@localhost:5432/$DB" \
SYSTIVRA_DATA_DIR="$ROOT/data" \
WORKER_TICK_SEC=1 \
WORKER_STALE_SEC=300 \
NEO4J_URI="${NEO4J_URI:-bolt://127.0.0.1:7687}" \
NEO4J_USER="${NEO4J_USER:-neo4j}" \
NEO4J_PASSWORD="${NEO4J_PASSWORD:-neo4j}" \
NEO4J_DATABASE="${NEO4J_DATABASE:-neo4j}" \
RUST_LOG=warn \
"$ROOT/target/debug/systivra-worker" >"$ROOT/data/phase4-scale/worker.log" 2>&1 &
WORKER_PID=$!

for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null

SYSTIVRA_BASE_URL="http://127.0.0.1:$PORT" \
PHASE4_FACT_DB="$DB" \
PHASE4_FACT_COUNT="${PHASE4_FACT_COUNT:-2500000}" \
PHASE4_SUPERSEDE_COUNT="${PHASE4_SUPERSEDE_COUNT:-25000}" \
PHASE4_FACT_KEEP_DB=1 \
python3 "$ROOT/scripts/phase4-fact-scale-eval.py"
