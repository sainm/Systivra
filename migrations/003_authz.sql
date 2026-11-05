-- 003_authz.sql
-- 認証直後に「自分が入っているプロジェクト」を引くための経路を開ける。
--
-- 001 の RLS は `app.project_id` だけを見る。しかしログイン直後は
-- まだプロジェクトが決まっていないので、`core.project` も `core.project_member` も
-- 0 行になり、画面が空になる。ここで `app.user_id` を見るポリシーを足す。
--
-- 複数の PERMISSIVE ポリシーは OR で合成される。既存の project_id ポリシーは
-- そのまま生きているので、プロジェクトを跨いだ読み取りができるようにはならない。
-- 見えるのは「自分が所属する行」だけ。

CREATE POLICY project_member_self ON core.project_member
  FOR SELECT
  USING (user_id = core.current_user());

CREATE POLICY project_self ON core.project
  FOR SELECT
  USING (EXISTS (
    SELECT 1
      FROM core.project_member m
     WHERE m.project_id = core.project.id
       AND m.user_id = core.current_user()
  ));

-- 所属一覧は「自分」で引くので、user_id 側の索引が要る。
CREATE INDEX IF NOT EXISTS project_member_user_idx
  ON core.project_member (user_id);
