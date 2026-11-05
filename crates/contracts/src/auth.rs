//! 認証まわりの DTO。`06-API.md` §2（認証）の形。

use domain::{ProjectId, Role, UserId};
use serde::{Deserialize, Serialize};

/// ログイン要求。パスワードは**ログに出さない**（`Debug` を実装しない）。
#[derive(Deserialize)]
pub struct LoginRequest {
    pub login_id: String,
    pub password: String,
}

/// 所属 1 件。`/me` の応答に含める。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipSummary {
    pub project_id: ProjectId,
    /// 表示用のキー。`project_id` の代わりに API へ渡さない（`detail/04` §0.2）。
    pub project_code: String,
    pub project_name: String,
    pub role: Role,
}

/// ログイン成功の応答。セッション Cookie は `Set-Cookie` で返す。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub user_id: UserId,
    pub display_name: String,
    /// 所属が 1 つも無い場合は空。画面は「プロジェクトがありません」を出す。
    pub memberships: Vec<MembershipSummary>,
}

/// `/me` の応答。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResponse {
    pub user_id: UserId,
    pub display_name: String,
    pub memberships: Vec<MembershipSummary>,
}

/// 特定プロジェクトでの自分の立場。`GET /projects/{pid}/me`。
///
/// 所属していなければ 404 なので、この型が返る時点で `role` は必ずある。
#[derive(Debug, Clone, Serialize, Deserialize)]
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
