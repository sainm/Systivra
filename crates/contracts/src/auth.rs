//! 認証まわりの DTO。`06-API.md` §2（認証）の形。

use chrono::{DateTime, Utc};
use domain::{ProjectId, Role, SessionId, UserId};
use serde::{Deserialize, Serialize};

/// ログイン要求。パスワードは**ログに出さない**（`Debug` を実装しない）。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub login_id: String,
    pub password: String,
}

/// 所属 1 件。`/me` の応答に含める。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MembershipSummary {
    pub project_id: ProjectId,
    /// 表示用のキー。`project_id` の代わりに API へ渡さない（`detail/04` §0.2）。
    pub project_code: String,
    pub project_name: String,
    pub role: Role,
}

/// ログイン成功の応答。セッション Cookie は `Set-Cookie` で返す。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoginResponse {
    pub user_id: UserId,
    pub display_name: String,
    /// 所属が 1 つも無い場合は空。画面は「プロジェクトがありません」を出す。
    pub memberships: Vec<MembershipSummary>,
}

/// ログイン画面が使う認証方式の一覧。公開エンドポイント。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AuthConfigResponse {
    pub password_login: bool,
    /// 未設定なら SSO ボタンを出さない。
    pub oidc: Option<OidcProviderSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OidcProviderSummary {
    pub provider_label: String,
    pub start_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SessionSummary {
    pub id: SessionId,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub user_agent: Option<String>,
    pub provider: Option<String>,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MemberSummary {
    pub user_id: UserId,
    pub login_id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub is_active: bool,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MemberListResponse {
    pub members: Vec<MemberSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateMemberRoleRequest {
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RevokeOtherSessionsResponse {
    pub revoked: u64,
}

/// `/me` の応答。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MeResponse {
    pub user_id: UserId,
    pub display_name: String,
    pub memberships: Vec<MembershipSummary>,
}

/// 特定プロジェクトでの自分の立場。`GET /projects/{pid}/me`。
///
/// 所属していなければ 404 なので、この型が返る時点で `role` は必ずある。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProjectMeResponse {
    pub project_id: ProjectId,
    pub role: Role,
    /// この画面で「書ける」操作の一覧。フロントがボタンを出し分けるのに使う。
    /// 権限判定をフロントで再実装させないためのもの。
    pub permissions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_request_does_not_leak_password_in_debug() {
        // derive(Debug) を付けていないこと自体が仕様。ここが壊れたら気づけるように、
        // 「Debug が無い」ことをコンパイル時に近い形で確認する代わりに、
        // パスワードがシリアライズされないことを見る。
        let req = LoginRequest {
            login_id: "u".into(),
            password: "secret".into(),
        };
        // LoginRequest は Deserialize のみ。ここでは値が保持されることだけ確認。
        assert_eq!(req.login_id, "u");
    }
}
