-- 編集中の版が「どの版から複製したか」を残す。
--
-- 背景: これまでは常に**現在の公開版**を複製していた。v3 を公開した後に
-- 「v2 の型定義に戻して直したい」と思っても、v2 を基底にできなかった。
-- 基底を記録しておかないと、後から「この草稿は何を元にしたか」を説明できない。
ALTER TABLE graph_meta.ontology_version
    ADD COLUMN based_on uuid REFERENCES graph_meta.ontology_version (id);

COMMENT ON COLUMN graph_meta.ontology_version.based_on IS
    'この版を複製した元の版。NULL は初版（または記録前のデータ）。';
