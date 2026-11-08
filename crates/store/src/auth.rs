//! 認証・認可に必要な読み取り。
//!
//! ここだけは `app.project_id` を設定せずに読む表がある
//! （`core.app_user` / `core.api_token` は RLS の対象外）。
//! `core.project_member` は RLS の対象だが、`003_authz.sql` の
//! `project_member_self` ポリシーにより `app.user_id` だけで自分の行を読める。

use chrono::{DateTime, Utc};
use domain::{ProjectId, Role, UserId};
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

/// IdP と紐付くセッション情報。パスワードセッションは全て `None`。
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionIdentity<'a> {
    pub provider: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub idp_sid: Option<&'a str>,
}

/// セッション発行の入力。
pub struct NewSession<'a> {
    pub id: Uuid,
    pub user: UserId,
    pub expires_at: DateTime<Utc>,
    pub ip_hash: Option<&'a [u8]>,
    pub user_agent: Option<&'a str>,
    pub identity: SessionIdentity<'a>,
}

/// セッション一覧の 1 行。
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub user_agent: Option<String>,
    pub provider: Option<String>,
    pub subject: Option<String>,
}

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

    /// セッションを発行する。`ip_hash` は生 IP を保存しないための HMAC。
    pub async fn insert_session(
        &self,
        tx: &mut Tx<'_>,
        session: &NewSession<'_>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO core.session
               (id, user_id, expires_at, ip_hash, user_agent, provider, subject, idp_sid)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(session.id)
        .bind(session.user.0)
        .bind(session.expires_at)
        .bind(session.ip_hash)
        .bind(session.user_agent)
        .bind(session.identity.provider)
        .bind(session.identity.subject)
        .bind(session.identity.idp_sid)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// セッションが失効・期限切れでなく、本人のものかを 1 クエリで見る。
    pub async fn session_is_live(
        &self,
        tx: &mut Tx<'_>,
        id: Uuid,
        user: UserId,
        idle_seconds: Option<i64>,
    ) -> Result<bool, StoreError> {
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS(
               SELECT 1 FROM core.session
                WHERE id = $1 AND user_id = $2
                  AND revoked_at IS NULL AND expires_at > now()
                  AND ($3::bigint IS NULL
                       OR last_seen_at > now() - ($3 * interval '1 second'))
             )",
        )
        .bind(id)
        .bind(user.0)
        .bind(idle_seconds)
        .fetch_one(&mut **tx)
        .await?;
        Ok(live)
    }

    /// 最終アクセスを間引いて更新する（5 分未満の連続要求では書かない）。
    pub async fn touch_session(&self, tx: &mut Tx<'_>, id: Uuid) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE core.session
                SET last_seen_at = now()
              WHERE id = $1
                AND last_seen_at < now() - interval '1 minute'",
        )
        .bind(id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// 現在のセッションを失効させる。冪等。
    pub async fn revoke_session(&self, tx: &mut Tx<'_>, id: Uuid) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE core.session
                SET revoked_at = now()
              WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// ユーザーの有効なセッション一覧。現在のセッション判定は呼び出し側で行う。
    pub async fn list_sessions(
        &self,
        tx: &mut Tx<'_>,
        user: UserId,
    ) -> Result<Vec<SessionRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, created_at, last_seen_at, expires_at, user_agent, provider, subject
               FROM core.session
              WHERE user_id = $1
                AND revoked_at IS NULL
                AND expires_at > now()
              ORDER BY last_seen_at DESC",
        )
        .bind(user.0)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| SessionRow {
                id: r.get("id"),
                created_at: r.get("created_at"),
                last_seen_at: r.get("last_seen_at"),
                expires_at: r.get("expires_at"),
                user_agent: r.get("user_agent"),
                provider: r.get("provider"),
                subject: r.get("subject"),
            })
            .collect())
    }

    /// 本人の指定セッションを失効させる。他人のセッションは触らない。
    pub async fn revoke_user_session(
        &self,
        tx: &mut Tx<'_>,
        id: Uuid,
        user: UserId,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE core.session
                SET revoked_at = now()
              WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(id)
        .bind(user.0)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 共有（PG）レート制限。固定窓のカウントを原子的に進める。
    /// 戻り値は (窓内カウント, 窓終了までの秒数)。
    pub async fn hit_rate_limit(
        &self,
        tx: &mut Tx<'_>,
        key: &str,
        window_seconds: i64,
    ) -> Result<(i64, i64), StoreError> {
        let row = sqlx::query(
            "INSERT INTO core.rate_limit_bucket AS b (bucket_key, window_start, count)
             VALUES ($1, now(), 1)
             ON CONFLICT (bucket_key) DO UPDATE
                SET count = CASE
                        WHEN b.window_start < now() - make_interval(secs => $2) THEN 1
                        ELSE b.count + 1
                    END,
                    window_start = CASE
                        WHEN b.window_start < now() - make_interval(secs => $2) THEN now()
                        ELSE b.window_start
                    END
             RETURNING count::bigint AS count,
                       GREATEST(
                           1,
                           ceil(EXTRACT(EPOCH FROM (
                               window_start + make_interval(secs => $2) - now()
                           )))::bigint
                       ) AS retry_after",
        )
        .bind(key)
        .bind(window_seconds as f64)
        .fetch_one(&mut **tx)
        .await?;
        Ok((row.get("count"), row.get("retry_after")))
    }

    /// 現在のセッション以外を全て失効させる。
    pub async fn revoke_other_sessions(
        &self,
        tx: &mut Tx<'_>,
        user: UserId,
        keep: Uuid,
    ) -> Result<u64, StoreError> {
        let result = sqlx::query(
            "UPDATE core.session
                SET revoked_at = now()
              WHERE user_id = $1
                AND id <> $2
                AND revoked_at IS NULL",
        )
        .bind(user.0)
        .bind(keep)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected())
    }

    /// IdP 主導ログアウト。subject または IdP の sid に一致するセッションを失効させる。
    pub async fn revoke_sessions_by_identity(
        &self,
        tx: &mut Tx<'_>,
        provider: &str,
        subject: Option<&str>,
        idp_sid: Option<&str>,
    ) -> Result<u64, StoreError> {
        let result = sqlx::query(
            "UPDATE core.session
                SET revoked_at = now()
              WHERE provider = $1
                AND revoked_at IS NULL
                AND (($2::text IS NOT NULL AND subject = $2)
                     OR ($3::text IS NOT NULL AND idp_sid = $3))",
        )
        .bind(provider)
        .bind(subject)
        .bind(idp_sid)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected())
    }

    /// 期限切れ・失効済みのセッションを掃除する。
    pub async fn cleanup_sessions(
        &self,
        tx: &mut Tx<'_>,
        older_than_days: i64,
    ) -> Result<u64, StoreError> {
        let result = sqlx::query(
            "DELETE FROM core.session
              WHERE expires_at < now() - ($1 * interval '1 day')
                 OR (revoked_at IS NOT NULL
                     AND revoked_at < now() - ($1 * interval '1 day'))",
        )
        .bind(older_than_days)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected())
    }

    /// 外部 identity（OIDC の issuer + subject）からユーザーを引く。
    ///
    /// `core.external_identity` は認証前に引くため RLS の対象外。
    /// 紐付けは管理者が事前に作り、未知の subject は自動プロビジョニングしない。
    pub async fn find_user_by_external_identity(
        &self,
        tx: &mut Tx<'_>,
        provider: &str,
        subject: &str,
    ) -> Result<Option<UserRow>, StoreError> {
        let row = sqlx::query(
            "SELECT u.id, u.login_id, u.display_name, u.password_hash, u.is_active
               FROM core.external_identity i
               JOIN core.app_user u ON u.id = i.user_id
              WHERE i.provider = $1 AND i.subject = $2",
        )
        .bind(provider)
        .bind(subject)
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

    /// SSO ログイン成功時刻を残す。紐付けが消えていた場合は何もしない。
    pub async fn touch_external_identity(
        &self,
        tx: &mut Tx<'_>,
        provider: &str,
        subject: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE core.external_identity
                SET last_login_at = now()
              WHERE provider = $1 AND subject = $2",
        )
        .bind(provider)
        .bind(subject)
        .execute(&mut **tx)
        .await?;
        Ok(())
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
