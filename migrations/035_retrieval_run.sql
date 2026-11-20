-- Retrieval Engine の実行記録（Phase 2）。
--
-- 既存の corpus.retrieval_log は「検索 1 回」の扁平なログで、
-- 多段（vector / lexical / graph / hierarchy / fuse / rerank / context）を
-- 表せない。用途が違うので別テーブルにする（retrieval_log は /search 用に残す）。
CREATE TABLE corpus.retrieval_run (
    id              uuid PRIMARY KEY,
    project_id      uuid NOT NULL,
    query           text NOT NULL,
    strategy        text NOT NULL,
    strategy_reason text NOT NULL DEFAULT '',
    request         jsonb NOT NULL,
    response_stats  jsonb NOT NULL,
    degraded        text[] NOT NULL DEFAULT '{}',
    latency_ms      integer NOT NULL,
    created_by      uuid,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX retrieval_run_project_created_idx
    ON corpus.retrieval_run (project_id, created_at DESC);

CREATE TABLE corpus.retrieval_stage (
    id              uuid PRIMARY KEY,
    run_id          uuid NOT NULL REFERENCES corpus.retrieval_run (id) ON DELETE CASCADE,
    seq             integer NOT NULL,
    stage           text NOT NULL,
    input_summary   jsonb NOT NULL,
    output_summary  jsonb NOT NULL,
    candidates      integer NOT NULL,
    kept            integer NOT NULL,
    latency_ms      integer NOT NULL,
    UNIQUE (run_id, seq)
);

-- 原文は入れない方針（機密の混入を避ける）。必要なら chunk を引き直す。
COMMENT ON COLUMN corpus.retrieval_stage.input_summary IS
    '原文ではなく要約（query の長さ・ハッシュ等）。trace は UI 表示用。';

-- 権限は API ロールに合わせる（既存の corpus テーブルと同じ扱い）。
GRANT SELECT, INSERT ON corpus.retrieval_run TO systivra_app;
GRANT SELECT, INSERT ON corpus.retrieval_stage TO systivra_app;
GRANT SELECT ON corpus.retrieval_run TO systivra_scheduler;
GRANT SELECT ON corpus.retrieval_stage TO systivra_scheduler;
