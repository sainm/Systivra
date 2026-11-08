-- 027: 承認を「1 回の実行への許可証」にする
--
-- MCP の write / destructive ツールは承認必須。従来は 423 を返すだけで、
-- 承認しても実行する経路が無かった（Workflow の承認ノードとは別系統）。
--
--   tools/call → 423 + approval_request(PENDING)
--   人が approve
--   tools/call（同じ引数）→ 承認を**消費**して 1 回だけ実行
--   もう一度 → 423
--
-- 消費済みを表す列が無いと、同じ承認で何度でも実行できてしまう。
-- status の enum を増やさないのは、承認の履歴（APPROVED のまま）を壊さないため。

ALTER TABLE ai.approval_request
  ADD COLUMN IF NOT EXISTS consumed_at timestamptz;

-- 未消費の承認を引くための索引。部分索引なので小さく保てる。
CREATE INDEX IF NOT EXISTS t7_approval_pass_idx
  ON ai.approval_request (project_id, action, decided_at DESC)
  WHERE status = 'APPROVED' AND consumed_at IS NULL;
