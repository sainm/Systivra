ALTER TABLE graph_meta.relationship
  ADD COLUMN projection_seq bigint NOT NULL DEFAULT 0;
