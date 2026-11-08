-- External MCP Client: normalized tool descriptors and failure details.
-- Agent runs record the initiating user so the worker can re-check role permissions.

ALTER TABLE ai.mcp_server
  ADD COLUMN tools jsonb NOT NULL DEFAULT '[]',
  ADD COLUMN health_detail text;

ALTER TABLE ai.agent_run
  ADD COLUMN requested_by text,
  ADD COLUMN requested_role text NOT NULL DEFAULT 'VIEWER';

UPDATE ai.agent_run
   SET requested_by = 'system'
 WHERE requested_by IS NULL;

ALTER TABLE ai.agent_run
  ALTER COLUMN requested_by SET NOT NULL;
