//! 認証のユースケース。
//!
//! パスワード検証とトークンのハッシュ化はここに閉じる。
//! `api` は生のパスワードも生のトークンも保存しないし、ハッシュ計算もしない。

use argon2::password_hash::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use chrono::Utc;
use contracts::MembershipSummary;
use domain::{ProjectId, Role, UserId};
use sha2::{Digest, Sha256};

use crate::App;

/// 認証済みの身元。**プロジェクトはまだ決まっていない。**
/// どのプロジェクトで何ができるかは、パスの `{pid}` と突き合わせて決める。
#[derive(Debug, Clone)]
pub struct Authenticated {
    pub user_id: UserId,
    pub display_name: String,
    pub via: Credential,
}

/// 認証手段。監査ログの `actor_type` とは別物（どちらも `USER`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential {
    Session,
    Token,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("store error: {0}")]
    Store(#[from] store::StoreError),
}

impl App {
    /// ログイン ID とパスワードで認証する。
    ///
    /// ユーザーが存在しない場合も**同じだけ時間をかける**（ダミー検証）。
    /// 応答時間の差で「その ID は存在する」と分かってしまうのを防ぐ。
    pub async fn authenticate_password(
        &self,
        login_id: &str,
        password: &str,
    ) -> Result<Option<Authenticated>, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let user = self.store().find_user_by_login(&mut tx, login_id).await?;
        store::commit(tx).await?;

        let Some(user) = user else {
            // 存在しないユーザーに対しても argon2 を 1 回回す。
            let _ = verify_password(password, dummy_hash());
            return Ok(None);
        };

        if !user.is_active {
            let _ = verify_password(password, dummy_hash());
            return Ok(None);
        }

        let Some(hash) = user.password_hash.as_deref() else {
            // SSO のみのユーザー。パスワードログインは受け付けない。
            let _ = verify_password(password, dummy_hash());
            return Ok(None);
        };

        if !verify_password(password, hash) {
            return Ok(None);
        }

        Ok(Some(Authenticated {
            user_id: user.id,
            display_name: user.display_name,
            via: Credential::Session,
        }))
    }

    /// セッション Cookie の `user_id` から身元を組み立てる。
    ///
    /// 毎回 `core.app_user` を引くのは、退職・無効化を即座に反映するため。
    /// Cookie に表示名を入れない（名前を変えたときに古い名前が出る）。
    pub async fn authenticate_session(
        &self,
        user: UserId,
    ) -> Result<Option<Authenticated>, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let row = self.store().find_user_by_id(&mut tx, user).await?;
        store::commit(tx).await?;

        Ok(match row {
            Some(u) if u.is_active => Some(Authenticated {
                user_id: u.id,
                display_name: u.display_name,
                via: Credential::Session,
            }),
            _ => None,
        })
    }

    /// PAT で認証する。失効・期限切れはここで弾く。
    pub async fn authenticate_token(
        &self,
        raw_token: &str,
    ) -> Result<Option<Authenticated>, AuthError> {
        let hash = hash_token(raw_token);

        let mut tx = self.store().begin_unscoped().await?;
        let token = self.store().find_token_by_hash(&mut tx, &hash).await?;
        store::commit(tx).await?;

        let Some(token) = token else {
            return Ok(None);
        };
        if !token.is_live(Utc::now()) {
            return Ok(None);
        }

        // トークンが生きていても、ユーザーが無効化されていれば通さない。
        // 退職者の PAT が残る事故を、ここで止める。
        let mut tx = self.store().begin_as(token.user_id, None).await?;
        let user = self.store().find_user_by_id(&mut tx, token.user_id).await?;
        store::commit(tx).await?;

        let Some(user) = user else {
            return Ok(None);
        };
        if !user.is_active {
            return Ok(None);
        }

        Ok(Some(Authenticated {
            user_id: user.id,
            display_name: user.display_name,
            via: Credential::Token,
        }))
    }

    /// 所属一覧。ログイン直後の画面がこれで決まる。
    ///
    /// `store` の行型ではなく契約型を返す。`api` に `store` を依存させないため。
    pub async fn memberships(&self, user: UserId) -> Result<Vec<MembershipSummary>, AuthError> {
        let mut tx = self.store().begin_as(user, None).await?;
        let rows = self.store().memberships_for(&mut tx, user).await?;
        store::commit(tx).await?;
        Ok(rows
            .into_iter()
            .map(|m| MembershipSummary {
                project_id: m.project_id,
                project_code: m.project_code,
                project_name: m.project_name,
                role: m.role,
            })
            .collect())
    }

    /// パスの `{pid}` が本人のスコープか。所属しなければ `None`。
    ///
    /// これを 403 ではなく 404 にするのは、プロジェクトの存在自体を隠すため
    /// （`detail/04` §4.2）。
    pub async fn role_in(
        &self,
        user: UserId,
        project: ProjectId,
    ) -> Result<Option<Role>, AuthError> {
        let mut tx = self.store().begin_as(user, Some(project)).await?;
        let role = self.store().role_in(&mut tx, user, project).await?;
        store::commit(tx).await?;
        Ok(role)
    }
}

/// PAT のハッシュ。生の値は保存も記録もしない。
pub fn hash_token(raw: &str) -> Vec<u8> {
    Sha256::digest(raw.as_bytes()).to_vec()
}

fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// ユーザーが存在しないときに回すダミー。
///
/// 固定文字列を埋め込まない。書式が壊れていると `PasswordHash::new` が
/// 即座に失敗し、等時性が消える（＝応答時間で ID の存在が分かる）。
/// 初回だけ実際にハッシュを作り、以降は使い回す。
fn dummy_hash() -> &'static str {
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::PasswordHasher;
    use std::sync::OnceLock;

    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(b"not-a-real-password", &salt)
            .expect("argon2 のハッシュ生成に失敗")
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::{rand_core::OsRng, SaltString};

    fn make_hash(pw: &str) -> String {
        let salt = SaltString::generate(&mut OsRng);
        argon2::PasswordHasher::hash_password(&Argon2::default(), pw.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    #[test]
    fn verifies_correct_password() {
        let h = make_hash("correct horse");
        assert!(verify_password("correct horse", &h));
    }

    #[test]
    fn rejects_wrong_password() {
        let h = make_hash("correct horse");
        assert!(!verify_password("wrong horse", &h));
    }

    #[test]
    fn broken_hash_never_verifies() {
        assert!(!verify_password("anything", "not-a-hash"));
    }

    #[test]
    fn token_hash_is_stable_and_not_the_raw_value() {
        let h1 = hash_token("abc123");
        let h2 = hash_token("abc123");
        assert_eq!(h1, h2);
        assert_ne!(h1, b"abc123".to_vec());
        assert_eq!(h1.len(), 32);
    }

    #[test]
    fn dummy_hash_does_not_verify_anything() {
        // ダミーは必ず失敗する。ここが通ると存在しないユーザーでログインできる。
        assert!(!verify_password("", dummy_hash()));
        assert!(!verify_password("password", dummy_hash()));
    }

    #[test]
    fn dummy_hash_is_a_valid_argon2_string() {
        // 書式が壊れていると検証が即座に失敗し、等時性が消える。
        assert!(PasswordHash::new(dummy_hash()).is_ok());
    }
}
