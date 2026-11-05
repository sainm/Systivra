//! セッション Cookie。
//!
//! **サーバー側にセッション表を持たない**（`detail/02` に該当表が無い）。
//! Cookie は `user_id` と有効期限だけを持ち、HMAC で改竄を防ぐ。
//!
//! 代償として、ログアウトは「Cookie を消す」だけで、盗まれた Cookie は
//! 期限まで有効。強制失効は署名鍵の更新（全員ログアウト）しかない。
//! これは Phase 1 の割り切り。プロジェクトからの除外は毎リクエスト
//! `core.project_member` を見るので**即座に効く**（越権は起きない）。

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use domain::UserId;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const COOKIE_NAME: &str = "systivra_session";

#[derive(Debug, Clone)]
pub struct SessionConfig {
    key: Vec<u8>,
    ttl: Duration,
    /// `Secure` を付けるか。開発の http では false。
    secure: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionConfigError {
    #[error("SESSION_SECRET が未設定、または 32 バイト未満")]
    MissingSecret,
}

impl SessionConfig {
    pub fn new(key: Vec<u8>, ttl: Duration, secure: bool) -> Self {
        Self { key, ttl, secure }
    }

    /// 環境変数から。`SESSION_SECRET` は 32 バイト以上を要求する。
    ///
    /// 短い鍵を許すと、総当たりで偽造 Cookie を作られる。
    pub fn from_env() -> Result<Self, SessionConfigError> {
        let secret = std::env::var("SESSION_SECRET").unwrap_or_default();
        if secret.len() < 32 {
            return Err(SessionConfigError::MissingSecret);
        }
        let secure = std::env::var("SESSION_COOKIE_SECURE")
            .map(|v| v != "false")
            .unwrap_or(true);
        Ok(Self::new(secret.into_bytes(), Duration::hours(12), secure))
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// `payload.signature` を組み立てる。
    fn sign(&self, payload_b64: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC は任意長の鍵を受け付ける");
        mac.update(payload_b64.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    /// Cookie の値を作る。`user_id:exp` を URL-safe base64 にして署名する。
    pub fn issue(&self, user: UserId, now: DateTime<Utc>) -> String {
        let exp = (now + self.ttl).timestamp();
        let payload = format!("{}:{}", user.as_uuid(), exp);
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let sig = self.sign(&payload_b64);
        format!("{payload_b64}.{sig}")
    }

    /// 検証して `user_id` を返す。改竄・期限切れは `None`。
    pub fn verify(&self, raw: &str, now: DateTime<Utc>) -> Option<UserId> {
        let (payload_b64, sig_b64) = raw.split_once('.')?;
        let sig = URL_SAFE_NO_PAD.decode(sig_b64).ok()?;

        // 定数時間比較。`==` で比べると署名の一致バイト数が漏れる。
        let mut mac = HmacSha256::new_from_slice(&self.key).ok()?;
        mac.update(payload_b64.as_bytes());
        mac.verify_slice(&sig).ok()?;

        let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
        let payload = std::str::from_utf8(&payload).ok()?;
        let (uuid_part, exp_part) = payload.rsplit_once(':')?;

        let exp: i64 = exp_part.parse().ok()?;
        if exp <= now.timestamp() {
            return None;
        }

        uuid_part.parse().ok()
    }

    pub fn set_cookie(&self, value: &str) -> String {
        let mut c = format!(
            "{COOKIE_NAME}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
            self.ttl.num_seconds()
        );
        if self.secure {
            c.push_str("; Secure");
        }
        c
    }

    pub fn clear_cookie(&self) -> String {
        let mut c = format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
        if self.secure {
            c.push_str("; Secure");
        }
        c
    }
}

/// `Cookie` ヘッダから 1 つ取り出す。見つからなければ `None`。
pub fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SessionConfig {
        SessionConfig::new(
            b"0123456789abcdef0123456789abcdef".to_vec(),
            Duration::hours(1),
            false,
        )
    }

    #[test]
    fn issued_token_verifies() {
        let c = cfg();
        let now = Utc::now();
        let u = UserId::new();
        let v = c.issue(u, now);
        assert_eq!(c.verify(&v, now), Some(u));
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let c = cfg();
        let now = Utc::now();
        let v = c.issue(UserId::new(), now);
        let (payload, sig) = v.split_once('.').unwrap();

        // payload だけ差し替える（署名は元のまま）
        let other = URL_SAFE_NO_PAD.encode(b"00000000-0000-0000-0000-000000000000:9999999999");
        assert_eq!(c.verify(&format!("{other}.{sig}"), now), None);

        // 署名だけ壊す
        let mut broken = sig.to_string();
        broken.push('A');
        assert_eq!(c.verify(&format!("{payload}.{broken}"), now), None);
    }

    #[test]
    fn expired_token_is_rejected() {
        let c = cfg();
        let issued_at = Utc::now() - Duration::hours(2);
        let v = c.issue(UserId::new(), issued_at);
        assert_eq!(c.verify(&v, Utc::now()), None);
    }

    #[test]
    fn different_key_cannot_verify() {
        let a = cfg();
        let b = SessionConfig::new(
            b"ffffffffffffffffffffffffffffffff".to_vec(),
            Duration::hours(1),
            false,
        );
        let now = Utc::now();
        let v = a.issue(UserId::new(), now);
        assert_eq!(b.verify(&v, now), None);
    }

    #[test]
    fn cookie_attributes_are_set() {
        let c = cfg();
        let h = c.set_cookie("x.y");
        assert!(h.contains("HttpOnly"));
        assert!(h.contains("SameSite=Lax"));
        assert!(h.contains("Path=/"));
        assert!(!h.contains("Secure"));
    }

    #[test]
    fn parses_cookie_header() {
        let h = "a=1; systivra_session=abc.def; b=2";
        assert_eq!(cookie_value(h, COOKIE_NAME), Some("abc.def"));
        assert_eq!(cookie_value(h, "missing"), None);
    }
}
