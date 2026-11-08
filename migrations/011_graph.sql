CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE SCHEMA graph_meta;

CREATE TABLE graph_meta.ontology_version (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  version text NOT NULL UNIQUE,
  status text NOT NULL CHECK (status IN ('DRAFT','PUBLISHED','DEPRECATED')),
  published_at timestamptz,
  published_by uuid
);

CREATE TABLE graph_meta.entity_type (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  ontology_version_id uuid NOT NULL REFERENCES graph_meta.ontology_version(id),
  name text NOT NULL,
  display_name_ja text NOT NULL,
  parent_type text,
  is_abstract boolean NOT NULL DEFAULT false,
  properties_schema jsonb NOT NULL DEFAULT '{}',
  UNIQUE (ontology_version_id, name)
);

CREATE TABLE graph_meta.relation_type (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  ontology_version_id uuid NOT NULL REFERENCES graph_meta.ontology_version(id),
  name text NOT NULL,
  display_name_ja text NOT NULL,
  source_types text[] NOT NULL,
  target_types text[] NOT NULL,
  cardinality text NOT NULL DEFAULT 'N:M',
  is_transitive boolean NOT NULL DEFAULT false,
  properties_schema jsonb NOT NULL DEFAULT '{}',
  UNIQUE (ontology_version_id, name)
);

CREATE TABLE graph_meta.entity (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  entity_type text NOT NULL,
  natural_key text NOT NULL,
  name text NOT NULL,
  qualified_name text,
  properties jsonb NOT NULL DEFAULT '{}',
  ontology_version text NOT NULL,
  revision text NOT NULL,
  resolution text NOT NULL,
  first_seen_rev text NOT NULL,
  last_seen_rev text NOT NULL,
  projection_seq bigint NOT NULL DEFAULT 0,
  deleted_at timestamptz,
  UNIQUE (project_id, entity_type, natural_key)
);
CREATE INDEX entity_name_trgm ON graph_meta.entity USING gin (name gin_trgm_ops);
CREATE INDEX entity_scope_idx ON graph_meta.entity (project_id, entity_type) WHERE deleted_at IS NULL;

CREATE TABLE graph_meta.relationship (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  src_id uuid NOT NULL REFERENCES graph_meta.entity(id),
  dst_id uuid NOT NULL REFERENCES graph_meta.entity(id),
  rel_type text NOT NULL,
  resolution text NOT NULL,
  confidence real NOT NULL,
  fact_id uuid,
  revision text NOT NULL,
  valid_to timestamptz,
  UNIQUE (project_id, src_id, dst_id, rel_type, revision)
);
CREATE INDEX rel_src_idx ON graph_meta.relationship (src_id, rel_type) WHERE valid_to IS NULL;
CREATE INDEX rel_dst_idx ON graph_meta.relationship (dst_id, rel_type) WHERE valid_to IS NULL;
CREATE INDEX rel_scope_idx ON graph_meta.relationship (project_id) WHERE valid_to IS NULL;

CREATE TABLE graph_meta.projection_state (
  project_id uuid PRIMARY KEY,
  last_event_seq bigint NOT NULL DEFAULT 0,
  last_event_id uuid,
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE graph_meta.projection_build (
  build_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  status text NOT NULL CHECK (status IN
    ('BUILDING','VERIFYING','READY','SWITCHED','FAILED','ABORTED')),
  shadow_label text NOT NULL,
  base_event_seq bigint NOT NULL,
  entities_written bigint NOT NULL DEFAULT 0,
  relationships_written bigint NOT NULL DEFAULT 0,
  drift_rate numeric(6,5),
  started_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz
);

ALTER TABLE graph_meta.entity ENABLE ROW LEVEL SECURITY;
CREATE POLICY entity_scope ON graph_meta.entity
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE graph_meta.relationship ENABLE ROW LEVEL SECURITY;
CREATE POLICY relationship_scope ON graph_meta.relationship
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE graph_meta.projection_state ENABLE ROW LEVEL SECURITY;
CREATE POLICY projection_state_scope ON graph_meta.projection_state
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE graph_meta.projection_build ENABLE ROW LEVEL SECURITY;
CREATE POLICY projection_build_scope ON graph_meta.projection_build
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

INSERT INTO graph_meta.ontology_version (id, version, status, published_at)
VALUES ('00000000-0000-4000-8000-000000000101', 'v1', 'PUBLISHED', now());

INSERT INTO graph_meta.entity_type (ontology_version_id, name, display_name_ja) VALUES
  ('00000000-0000-4000-8000-000000000101', 'BusinessDomain', '業務ドメイン'),
  ('00000000-0000-4000-8000-000000000101', 'BusinessFunction', '業務機能'),
  ('00000000-0000-4000-8000-000000000101', 'BusinessRule', '業務ルール'),
  ('00000000-0000-4000-8000-000000000101', 'BusinessProcess', '業務プロセス'),
  ('00000000-0000-4000-8000-000000000101', 'BusinessEvent', '業務イベント'),
  ('00000000-0000-4000-8000-000000000101', 'Document', '文書'),
  ('00000000-0000-4000-8000-000000000101', 'DocumentRevision', '文書改版'),
  ('00000000-0000-4000-8000-000000000101', 'Section', '節'),
  ('00000000-0000-4000-8000-000000000101', 'Sheet', 'シート'),
  ('00000000-0000-4000-8000-000000000101', 'Region', '領域'),
  ('00000000-0000-4000-8000-000000000101', 'Table', '表'),
  ('00000000-0000-4000-8000-000000000101', 'DesignElement', '設計要素'),
  ('00000000-0000-4000-8000-000000000101', 'Requirement', '要件'),
  ('00000000-0000-4000-8000-000000000101', 'Specification', '仕様'),
  ('00000000-0000-4000-8000-000000000101', 'Application', 'アプリケーション'),
  ('00000000-0000-4000-8000-000000000101', 'Module', 'モジュール'),
  ('00000000-0000-4000-8000-000000000101', 'Package', 'パッケージ'),
  ('00000000-0000-4000-8000-000000000101', 'Screen', '画面'),
  ('00000000-0000-4000-8000-000000000101', 'Form', 'フォーム'),
  ('00000000-0000-4000-8000-000000000101', 'Action', 'アクション'),
  ('00000000-0000-4000-8000-000000000101', 'Controller', 'コントローラ'),
  ('00000000-0000-4000-8000-000000000101', 'Service', 'サービス'),
  ('00000000-0000-4000-8000-000000000101', 'Component', 'コンポーネント'),
  ('00000000-0000-4000-8000-000000000101', 'API', 'API'),
  ('00000000-0000-4000-8000-000000000101', 'Job', 'ジョブ'),
  ('00000000-0000-4000-8000-000000000101', 'Batch', 'バッチ'),
  ('00000000-0000-4000-8000-000000000101', 'Class', 'クラス'),
  ('00000000-0000-4000-8000-000000000101', 'Method', 'メソッド'),
  ('00000000-0000-4000-8000-000000000101', 'Field', 'フィールド'),
  ('00000000-0000-4000-8000-000000000101', 'Interface', 'インターフェース'),
  ('00000000-0000-4000-8000-000000000101', 'Dependency', '依存'),
  ('00000000-0000-4000-8000-000000000101', 'Annotation', '注釈'),
  ('00000000-0000-4000-8000-000000000101', 'Configuration', '設定'),
  ('00000000-0000-4000-8000-000000000101', 'SqlStatement', 'SQL 文'),
  ('00000000-0000-4000-8000-000000000101', 'Database', 'データベース'),
  ('00000000-0000-4000-8000-000000000101', 'Schema', 'スキーマ'),
  ('00000000-0000-4000-8000-000000000101', 'Column', '列'),
  ('00000000-0000-4000-8000-000000000101', 'PrimaryKey', '主キー'),
  ('00000000-0000-4000-8000-000000000101', 'ForeignKey', '外部キー'),
  ('00000000-0000-4000-8000-000000000101', 'View', 'ビュー'),
  ('00000000-0000-4000-8000-000000000101', 'StoredProcedure', 'ストアドプロシージャ'),
  ('00000000-0000-4000-8000-000000000101', 'TestPlan', 'テスト計画'),
  ('00000000-0000-4000-8000-000000000101', 'TestScenario', 'テストシナリオ'),
  ('00000000-0000-4000-8000-000000000101', 'TestCase', 'テストケース'),
  ('00000000-0000-4000-8000-000000000101', 'TestData', 'テストデータ'),
  ('00000000-0000-4000-8000-000000000101', 'TestRun', 'テスト実行'),
  ('00000000-0000-4000-8000-000000000101', 'TestExecution', 'テスト実行記録'),
  ('00000000-0000-4000-8000-000000000101', 'TestResult', 'テスト結果'),
  ('00000000-0000-4000-8000-000000000101', 'Defect', '不具合'),
  ('00000000-0000-4000-8000-000000000101', 'Coverage', 'カバレッジ'),
  ('00000000-0000-4000-8000-000000000101', 'Agent', 'エージェント'),
  ('00000000-0000-4000-8000-000000000101', 'Skill', 'スキル'),
  ('00000000-0000-4000-8000-000000000101', 'Prompt', 'プロンプト'),
  ('00000000-0000-4000-8000-000000000101', 'Tool', 'ツール'),
  ('00000000-0000-4000-8000-000000000101', 'Workflow', 'ワークフロー'),
  ('00000000-0000-4000-8000-000000000101', 'AnalysisSession', '分析セッション');

INSERT INTO graph_meta.relation_type
  (ontology_version_id, name, display_name_ja, source_types, target_types)
VALUES
  ('00000000-0000-4000-8000-000000000101', 'CONTAINS', '含む', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'DECLARES', '宣言する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'EXTENDS', '継承する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'IMPLEMENTS', '実装する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'DEPENDS_ON', '依存する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'CALLS', '呼び出す', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'REFERENCES', '参照する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'IMPORTS', '取り込む', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'READS', '読む', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'WRITES', '書く', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'QUERIES', '問い合わせる', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'MAPPED_TO', '対応する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'TRIGGERS', '起動する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'VALIDATES', '検証する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'HANDLES', '処理する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'DISPATCHES', '振り分ける', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'SUBMITS_TO', '送信する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'DESCRIBES', '記述する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'DEFINES', '定義する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'MENTIONS', '言及する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'IMPLEMENTED_BY', '実装される', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'VERIFIES', '検証する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'GENERATED_FROM', '生成元', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'DERIVED_FROM', '派生元', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'AFFECTS', '影響する', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'EXECUTED_IN', '実行される', '{*}', '{*}'),
  ('00000000-0000-4000-8000-000000000101', 'PRODUCED', '生成する', '{*}', '{*}');
