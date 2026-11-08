use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use domain::ProjectId;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadClaims {
    pub project_id: ProjectId,
    pub object_key: String,
    pub max_size: i64,
    pub exp: i64,
}

#[derive(Debug, Clone)]
pub struct UploadConfig {
    key: Vec<u8>,
    ttl: Duration,
    base_url: String,
}

impl UploadConfig {
    pub fn new(key: Vec<u8>, ttl: Duration) -> Self {
        let base_url = std::env::var("PUBLIC_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:8080".to_string());
        Self {
            key,
            ttl,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub fn issue(
        &self,
        project: ProjectId,
        object_key: &str,
        max_size: i64,
        now: DateTime<Utc>,
    ) -> (String, DateTime<Utc>) {
        let expires_at = now + self.ttl;
        let claims = UploadClaims {
            project_id: project,
            object_key: object_key.to_string(),
            max_size,
            exp: expires_at.timestamp(),
        };
        let payload = serde_json::to_vec(&claims).expect("claims は常にシリアライズできる");
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload);
        let signature = self.sign(&payload_b64);
        (format!("{payload_b64}.{signature}"), expires_at)
    }

    pub fn verify(&self, token: &str, now: DateTime<Utc>) -> Option<UploadClaims> {
        let (payload_b64, signature) = token.split_once('.')?;
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        let mut mac = HmacSha256::new_from_slice(&self.key).ok()?;
        mac.update(payload_b64.as_bytes());
        mac.verify_slice(&signature).ok()?;

        let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
        let claims: UploadClaims = serde_json::from_slice(&payload).ok()?;
        if claims.exp <= now.timestamp() {
            return None;
        }
        Some(claims)
    }

    pub fn upload_url(&self, token: &str) -> String {
        format!("{}/api/v1/uploads/{token}", self.base_url)
    }

    fn sign(&self, payload_b64: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC は任意長の鍵を受け付ける");
        mac.update(payload_b64.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }
}

pub fn new_object_key(project: ProjectId, filename: &str) -> String {
    let filename = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let safe: String = filename
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ' | '（' | '）' | '(' | ')')
            {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim().trim_matches('.').replace("..", "_");
    let safe = if safe.is_empty() {
        "document.pdf".to_string()
    } else {
        safe
    };
    format!("raw/{project}/{}/{safe}", Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> UploadConfig {
        UploadConfig::new(
            b"0123456789abcdef0123456789abcdef".to_vec(),
            Duration::minutes(15),
        )
    }

    #[test]
    fn token_roundtrips_and_expires() {
        let cfg = cfg();
        let now = Utc::now();
        let project = ProjectId::new();
        let (token, _) = cfg.issue(project, "raw/x/y.pdf", 100, now);
        let claims = cfg.verify(&token, now).unwrap();
        assert_eq!(claims.project_id, project);
        assert_eq!(claims.object_key, "raw/x/y.pdf");
        assert!(cfg.verify(&token, now + Duration::minutes(16)).is_none());
    }

    #[test]
    fn tampering_is_rejected() {
        let cfg = cfg();
        let now = Utc::now();
        let (token, _) = cfg.issue(ProjectId::new(), "raw/x/y.pdf", 100, now);
        let (payload, signature) = token.split_once('.').unwrap();
        let other = URL_SAFE_NO_PAD.encode(b"{}");
        assert!(cfg.verify(&format!("{other}.{signature}"), now).is_none());
        assert!(cfg.verify(&format!("{payload}.AAAA"), now).is_none());
    }

    #[test]
    fn object_key_stays_under_project_prefix() {
        let project = ProjectId::new();
        let key = new_object_key(project, "../../etc/passwd");
        assert!(key.starts_with(&format!("raw/{project}/")));
        assert!(!key.contains(".."));
    }
}
