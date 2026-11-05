//! 認証・認可に必要な読み取り。
//!
//! ここだけは `app.project_id` を設定せずに読む表がある
//! （`core.app_user` / `core.api_token` は RLS の対象外）。
//! `core.project_member` は RLS の対象だが、`003_authz.sql` の
//! `project_member_self` ポリシーにより `app.user_id` だけで自分の行を読める。

use chrono::{DateTime, Utc};
use domain::{ProjectId, Role, UserId};
use sqlx::Row;

use crate::pool::{Store, StoreError, Tx};

/// ログインに使うユーザー行。
#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: UserId,
    pub login_id: String,
    pub display_name: String,
    /// argon2id のハッシュ。SSO のみのユーザーは `None`。
    pub password_hash: Option<String>,
    pub is_active: bool,
}

/// PAT（不透明トークン）。生の値は保存しない。`sha256` の結果だけを持つ。
#[derive(Debug, Clone)]
pub struct TokenRow {
    pub user_id: UserId,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// 所属。`/me` の応答に使う。
#[derive(Debug, Clone)]
pub struct MembershipRow {
    pub project_id: ProjectId,
    pub project_code: String,
    pub project_name: String,
    pub role: Role,
}

impl TokenRow {
    /// 有効期限切れ・失効済みでないか。
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && self.expires_at.map(|e| e > now).unwrap_or(true)
    }
}

impl Store {
    /// ログイン ID でユーザーを引く。RLS の対象外なのでスコープ不要。
    pub async fn find_user_by_login(
        &self,
        tx: &mut Tx<'_>,
        login_id: &str,
    ) -> Result<Option<UserRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, login_id, display_name, password_hash, is_active
               FROM core.app_user
              WHERE login_id = $1",
        )
        .bind(login_id)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(row.map(|r| UserRow {
            id: UserId(r.get("id")),
            login_id: r.get("login_id"),
            display_name: r.get("display_name"),
            password_hash: r.get("password_hash"),
            is_active: r.get("is_active"),
        }))
    }

    /// ID でユーザーを引く。セッション Cookie の検証後に使う。
    pub async fn find_user_by_id(
        &self,
        tx: &mut Tx<'_>,
        id: UserId,
    ) -> Result<Option<UserRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, login_id, display_name, password_hash, is_active
               FROM core.app_user
              WHERE id = $1",
        )
        .bind(id.0)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(row.map(|r| UserRow {
            id: UserId(r.get("id")),
            login_id: r.get("login_id"),
            display_name: r.get("display_name"),
            password_hash: r.get("password_hash"),
            is_active: r.get("is_active"),
        }))
    }

    /// トークンのハッシュで PAT を引く。失効判定は呼び出し側で行う。
    pub async fn find_token_by_hash(
        &self,
        tx: &mut Tx<'_>,
        token_hash: &[u8],
    ) -> Result<Option<TokenRow>, StoreError> {
        let row = sqlx::query(
            "SELECT user_id, scopes, expires_at, revoked_at
               FROM core.api_token
              WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(row.map(|r| TokenRow {
            user_id: UserId(r.get("user_id")),
            scopes: r.get("scopes"),
            expires_at: r.get("expires_at"),
            revoked_at: r.get("revoked_at"),
        }))
    }

    /// ユーザーの全所属。ログイン直後の画面がこれで決まる。
    ///
    /// `app.user_id` を設定したトランザクションで呼ぶこと
    /// （`003_authz.sql` の `project_member_self` ポリシーが効く）。
    pub async fn memberships_for(
        &self,
        tx: &mut Tx<'_>,
        user: UserId,
    ) -> Result<Vec<MembershipRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT m.project_id, p.code, p.name, m.role
               FROM core.project_member m
               JOIN core.project p ON p.id = m.project_id
              WHERE m.user_id = $1
                AND p.deleted_at IS NULL
              ORDER BY p.code",
        )
        .bind(user.0)
        .fetch_all(&mut **tx)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let role: String = r.get("role");
                Some(MembershipRow {
                    project_id: ProjectId(r.get("project_id")),
                    project_code: r.get("code"),
                    project_name: r.get("name"),
                    role: Role::parse(&role)?,
                })
            })
            .collect())
    }

    /// 指定プロジェクトでのロール。所属していなければ `None`。
    ///
    /// これが `{pid}` の「スコープ宣言」照合そのもの（`detail/04` §4.2）。
    /// 所属しない場合は 404 にする（403 にしない。存在を隠す）。
    pub async fn role_in(
        &self,
        tx: &mut Tx<'_>,
        user: UserId,
        project: ProjectId,
    ) -> Result<Option<Role>, StoreError> {
        let row = sqlx::query(
            "SELECT role
               FROM core.project_member
              WHERE project_id = $1 AND user_id = $2",
        )
        .bind(project.0)
        .bind(user.0)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(row.and_then(|r| {
            let s: String = r.get("role");
            Role::parse(&s)
        }))
    }
}
