-- 026: Workflow の実行主体
--
-- capability ノードは「その workflow を起動した人間」の権限で動く。
-- 承認ノードを挟んだ破壊的操作（test.run.execute）を実行するには、
-- 誰の依頼で走っている run なのかを DB に残しておく必要がある。
--
-- ai.workflow_run は RLS 済みなので、ポリシーは変更しない。
-- core.project_of は新規テーブルを作らないため書き換えない。

ALTER TABLE ai.workflow_run
  ADD COLUMN IF NOT EXISTS requested_by text;
