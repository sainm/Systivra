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
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const COOKIE_NAME: &str = "systivra_session";
pub const FLOW_COOKIE_NAME: &str = "systivra_oidc_flow";
const FLOW_TTL_MINUTES: i64 = 10;

/// 検証済みのセッション Cookie。DB の実体は `core.session`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionClaims {
    pub session_id: Uuid,
    pub user_id: UserId,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    key: Vec<u8>,
    ttl: Duration,
    /// `Secure` を付けるか。開発の http では false。
    secure: bool,
    /// 無操作で失効させるまでの時間。未設定なら無効。
    idle: Option<Duration>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionConfigError {
    #[error("SESSION_SECRET が未設定、または 32 バイト未満")]
    MissingSecret,
}

impl SessionConfig {
    pub fn new(key: Vec<u8>, ttl: Duration, secure: bool) -> Self {
        Self {
            key,
            ttl,
            secure,
            idle: None,
        }
    }

    pub fn with_idle(mut self, idle: Option<Duration>) -> Self {
        self.idle = idle;
        self
    }

    pub fn idle_seconds(&self) -> Option<i64> {
        self.idle.map(|idle| idle.num_seconds()).filter(|s| *s > 0)
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
        let idle = std::env::var("SESSION_IDLE_MINUTES")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > 0)
            .map(Duration::minutes);
        Ok(Self::new(secret.into_bytes(), Duration::hours(12), secure).with_idle(idle))
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn signing_key(&self) -> &[u8] {
        &self.key
    }

    /// HSTS を付けてよいか。
    pub fn is_secure(&self) -> bool {
        self.secure
    }

    /// IP などを生値で保存しないための HMAC。同一値は同一結果になる。
    pub fn fingerprint(&self, value: &str) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC は任意長の鍵を受け付ける");
        mac.update(b"fingerprint:");
        mac.update(value.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }

    /// `payload.signature` を組み立てる。
    fn sign(&self, payload_b64: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC は任意長の鍵を受け付ける");
        mac.update(payload_b64.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    /// Cookie の値を作る。`session_id:user_id:exp` を URL-safe base64 にして署名する。
    pub fn issue(&self, session: Uuid, user: UserId, now: DateTime<Utc>) -> String {
        let exp = (now + self.ttl).timestamp();
        let payload = format!("{}:{}:{}", session, user.as_uuid(), exp);
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let sig = self.sign(&payload_b64);
        format!("{payload_b64}.{sig}")
    }

    /// 検証してセッション ID と `user_id` を返す。改竄・期限切れは `None`。
    pub fn verify(&self, raw: &str, now: DateTime<Utc>) -> Option<SessionClaims> {
        let (payload_b64, sig_b64) = raw.split_once('.')?;
        let sig = URL_SAFE_NO_PAD.decode(sig_b64).ok()?;

        // 定数時間比較。`==` で比べると署名の一致バイト数が漏れる。
        let mut mac = HmacSha256::new_from_slice(&self.key).ok()?;
        mac.update(payload_b64.as_bytes());
        mac.verify_slice(&sig).ok()?;

        let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
        let payload = std::str::from_utf8(&payload).ok()?;
        let mut parts = payload.split(':');
        let session_id = parts.next()?.parse().ok()?;
        let user_id = parts.next()?.parse().ok()?;
        let exp: i64 = parts.next()?.parse().ok()?;
        if parts.next().is_some() || exp <= now.timestamp() {
            return None;
        }

        Some(SessionClaims {
            session_id,
            user_id,
        })
    }

    /// 用途を分離した署名付き値。セッション Cookie と OIDC フローの
    /// 取り違えを、署名対象の `kind` で防ぐ。
    pub fn seal(&self, kind: &str, payload: &[u8]) -> String {
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload);
        let signed = format!("{kind}.{payload_b64}");
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC は任意長の鍵を受け付ける");
        mac.update(signed.as_bytes());
        format!(
            "{}.{}",
            payload_b64,
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        )
    }

    pub fn open(&self, kind: &str, raw: &str) -> Option<Vec<u8>> {
        let (payload_b64, sig_b64) = raw.split_once('.')?;
        let sig = URL_SAFE_NO_PAD.decode(sig_b64).ok()?;
        let mut mac = HmacSha256::new_from_slice(&self.key).ok()?;
        mac.update(format!("{kind}.{payload_b64}").as_bytes());
        mac.verify_slice(&sig).ok()?;
        URL_SAFE_NO_PAD.decode(payload_b64).ok()
    }

    pub fn flow_ttl_minutes(&self) -> i64 {
        FLOW_TTL_MINUTES
    }

    pub fn set_flow_cookie(&self, value: &str) -> String {
        let mut c = format!(
            "{FLOW_COOKIE_NAME}={value}; Path=/api/v1/auth/oidc; HttpOnly; SameSite=Lax; Max-Age={}",
            FLOW_TTL_MINUTES * 60
        );
        if self.secure {
            c.push_str("; Secure");
        }
        c
    }

    pub fn clear_flow_cookie(&self) -> String {
        let mut c = format!(
            "{FLOW_COOKIE_NAME}=; Path=/api/v1/auth/oidc; HttpOnly; SameSite=Lax; Max-Age=0"
        );
        if self.secure {
            c.push_str("; Secure");
        }
        c
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
        let session = Uuid::new_v4();
        let user = UserId::new();
        let v = c.issue(session, user, now);
        assert_eq!(
            c.verify(&v, now),
            Some(SessionClaims {
                session_id: session,
                user_id: user,
            })
        );
    }

    #[test]
    fn old_two_part_cookie_is_rejected() {
        let c = cfg();
        let now = Utc::now();
        let payload = format!(
            "{}:{}",
            UserId::new().as_uuid(),
            (now + Duration::hours(1)).timestamp()
        );
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let v = format!("{payload_b64}.{}", c.sign(&payload_b64));
        assert_eq!(c.verify(&v, now), None);
    }

    #[test]
    fn fingerprint_is_stable_and_key_bound() {
        let a = cfg();
        let b = SessionConfig::new(
            b"ffffffffffffffffffffffffffffffff".to_vec(),
            Duration::hours(1),
            false,
        );
        assert_eq!(a.fingerprint("127.0.0.1"), a.fingerprint("127.0.0.1"));
        assert_ne!(a.fingerprint("127.0.0.1"), a.fingerprint("10.0.0.1"));
        assert_ne!(a.fingerprint("127.0.0.1"), b.fingerprint("127.0.0.1"));
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let c = cfg();
        let now = Utc::now();
        let v = c.issue(Uuid::new_v4(), UserId::new(), now);
        let (payload, sig) = v.split_once('.').unwrap();

        // payload だけ差し替える（署名は元のまま）
        let other = URL_SAFE_NO_PAD.encode(
            b"00000000-0000-0000-0000-000000000000:00000000-0000-0000-0000-000000000000:9999999999",
        );
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
        let v = c.issue(Uuid::new_v4(), UserId::new(), issued_at);
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
        let v = a.issue(Uuid::new_v4(), UserId::new(), now);
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
    fn sealed_payload_roundtrips_and_kind_is_bound() {
        let c = cfg();
        let sealed = c.seal("oidc-flow", b"state-and-verifier");
        assert_eq!(
            c.open("oidc-flow", &sealed),
            Some(b"state-and-verifier".to_vec())
        );
        assert_eq!(c.open("session", &sealed), None);
    }

    #[test]
    fn sealed_payload_rejects_tampering_and_other_keys() {
        let c = cfg();
        let other = SessionConfig::new(
            b"ffffffffffffffffffffffffffffffff".to_vec(),
            Duration::hours(1),
            false,
        );
        let sealed = c.seal("oidc-flow", b"payload");
        assert_eq!(other.open("oidc-flow", &sealed), None);
        let mut broken = sealed.clone();
        broken.push('A');
        assert_eq!(c.open("oidc-flow", &broken), None);
    }

    #[test]
    fn flow_cookie_is_scoped_and_short_lived() {
        let c = cfg();
        let header = c.set_flow_cookie("x.y");
        assert!(header.contains("Path=/api/v1/auth/oidc"));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("SameSite=Lax"));
        assert!(header.contains("Max-Age=600"));
        assert!(c.clear_flow_cookie().contains("Max-Age=0"));
    }

    #[test]
    fn parses_cookie_header() {
        let h = "a=1; systivra_session=abc.def; b=2";
        assert_eq!(cookie_value(h, COOKIE_NAME), Some("abc.def"));
        assert_eq!(cookie_value(h, "missing"), None);
    }
}
