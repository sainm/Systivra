-- Chunk 間の関係（Phase 3）。
--
-- なぜ graph_meta.relationship ではだめか:
--   あちらの端点は graph_meta.entity。chunk は entity ではない
--   （natural_key 体系に chunk id が無い）。2,800 個の chunk を実体に
--   投影するとグラフが倍になり、コード知識グラフの意味も薄まる。
--   粒度が違うので別テーブルにする。
--
-- 粒度の約束:
--   文書内部の階層   → corpus.doc_node.parent_id
--   章 → 章 / 章 → コード → graph_meta.relationship（Section は実体）
--   段落 → 段落      → ここ
CREATE TABLE corpus.chunk_relation (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id   uuid NOT NULL,
    -- chunk は分割テーブルで主キーが (id, project_id)。FK も同じ組で張る
    -- （id だけを参照しようとすると「一意制約が無い」で落ちる）。
    src_chunk_id uuid NOT NULL,
    dst_chunk_id uuid NOT NULL,
    relation     text NOT NULL,
    confidence   real NOT NULL DEFAULT 0.5,
    method       text NOT NULL DEFAULT 'MANUAL',
    evidence_id  uuid REFERENCES corpus.evidence (id),
    note         text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    verified_at  timestamptz,
    CONSTRAINT chunk_relation_src_fk FOREIGN KEY (src_chunk_id, project_id)
        REFERENCES corpus.chunk (id, project_id) ON DELETE CASCADE,
    CONSTRAINT chunk_relation_dst_fk FOREIGN KEY (dst_chunk_id, project_id)
        REFERENCES corpus.chunk (id, project_id) ON DELETE CASCADE,
    CONSTRAINT chunk_relation_no_self CHECK (src_chunk_id <> dst_chunk_id),
    CONSTRAINT chunk_relation_confidence_range CHECK (confidence >= 0 AND confidence <= 1),
    CONSTRAINT chunk_relation_method_valid CHECK (method IN ('MANUAL', 'RULE', 'IMPORTED')),
    UNIQUE (project_id, src_chunk_id, dst_chunk_id, relation)
);

CREATE INDEX chunk_relation_src_idx
    ON corpus.chunk_relation (project_id, src_chunk_id, relation);
CREATE INDEX chunk_relation_dst_idx
    ON corpus.chunk_relation (project_id, dst_chunk_id, relation);

COMMENT ON COLUMN corpus.chunk_relation.method IS
    'MANUAL=人が付けた / RULE=規則で推定 / IMPORTED=外部取り込み。自動抽出は未対応。';

GRANT SELECT, INSERT, UPDATE, DELETE ON corpus.chunk_relation TO systivra_app;
GRANT SELECT ON corpus.chunk_relation TO systivra_scheduler;
