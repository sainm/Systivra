-- 021_session_identity.sql
-- IdP 主導ログアウト（back-channel logout）のために、セッションと IdP の
-- subject / sid を紐付ける。パスワードセッションは NULL のまま。

ALTER TABLE core.session ADD COLUMN provider text;
ALTER TABLE core.session ADD COLUMN subject text;
ALTER TABLE core.session ADD COLUMN idp_sid text;

CREATE INDEX session_identity_idx ON core.session (provider, subject);
CREATE INDEX session_idp_sid_idx ON core.session (provider, idp_sid);
