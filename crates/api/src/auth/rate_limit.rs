//! 認証エンドポイントの IP 単位レート制限。
//!
//! 単一 API プロセス内のトークンバケット。複数ノードではノードごとに
//! 別カウントになるため、水平分割時は共有ストア（Redis 等）が必要。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use contracts::ErrorCode;

use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

const DEFAULT_CAPACITY: f64 = 10.0;
const DEFAULT_REFILL_PER_MIN: f64 = 10.0;
const MAX_BUCKETS: usize = 10_000;

struct Bucket {
    tokens: f64,
    last: Instant,
}

pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    trust_proxy: bool,
    backend: Backend,
    buckets: Mutex<HashMap<String, Bucket>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Memory,
    Pg,
}

impl RateLimiter {
    pub fn from_env() -> Self {
        let capacity = env_positive("AUTH_RATE_LIMIT_CAPACITY").unwrap_or(DEFAULT_CAPACITY);
        let refill_per_min =
            env_positive("AUTH_RATE_LIMIT_REFILL_PER_MIN").unwrap_or(DEFAULT_REFILL_PER_MIN);
        let trust_proxy = std::env::var("TRUST_PROXY")
            .map(|v| v == "1")
            .unwrap_or(false);
        let backend = match std::env::var("AUTH_RATE_LIMIT_BACKEND").as_deref() {
            Ok("pg") => Backend::Pg,
            _ => Backend::Memory,
        };
        let mut limiter = Self::new(capacity, refill_per_min / 60.0, trust_proxy);
        limiter.backend = backend;
        limiter
    }

    pub fn new(capacity: f64, refill_per_sec: f64, trust_proxy: bool) -> Self {
        Self {
            capacity,
            refill_per_sec,
            trust_proxy,
            backend: Backend::Memory,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// 設定されたバックエンドで判定する。PG 障害時はメモリへ退避する
    /// （認証を全停止させない。共有カウントの厳密性より可用性を取る）。
    pub async fn check_with(&self, app: &app::App, key: &str) -> Result<(), u64> {
        if self.backend == Backend::Pg {
            match app
                .check_auth_rate(key, self.capacity.ceil() as i64, 60)
                .await
            {
                Ok(Some(retry)) => return Err(retry),
                Ok(None) => return Ok(()),
                Err(error) => {
                    tracing::warn!(%error, "shared rate limiter unavailable; memory fallback");
                }
            }
        }
        self.check(key)
    }

    pub fn trust_proxy(&self) -> bool {
        self.trust_proxy
    }

    pub fn check(&self, key: &str) -> Result<(), u64> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: &str, now: Instant) -> Result<(), u64> {
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if buckets.len() > MAX_BUCKETS {
            buckets.retain(|_, bucket| {
                now.saturating_duration_since(bucket.last) < Duration::from_secs(3600)
            });
        }

        let bucket = buckets.entry(key.to_owned()).or_insert(Bucket {
            tokens: self.capacity,
            last: now,
        });
        let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.last = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let wait = ((1.0 - bucket.tokens) / self.refill_per_sec).ceil() as u64;
            Err(wait.max(1))
        }
    }
}

fn env_positive(key: &str) -> Option<f64> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v > 0.0)
}

/// ハンドラ用の接続情報。ConnectInfo が無いテストでも失敗しない。
pub struct ClientInfo {
    pub ip: String,
    pub user_agent: Option<String>,
}

impl FromRequestParts<ApiState> for ClientInfo {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ApiState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect| connect.0);
        Ok(Self {
            ip: client_ip(&parts.headers, peer, state.rate_limiter.trust_proxy()),
            user_agent: user_agent(&parts.headers),
        })
    }
}

pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return ip.to_owned();
        }
    }
    peer.map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

pub fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.chars().take(256).collect())
}

pub async fn limit_auth(State(state): State<ApiState>, req: Request, next: Next) -> Response {
    if !is_limited_path(req.uri().path()) {
        return next.run(req).await;
    }

    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0);
    let key = client_ip(req.headers(), peer, state.rate_limiter.trust_proxy());

    if let Err(seconds) = state.rate_limiter.check_with(&state.app, &key).await {
        let trace = req
            .extensions()
            .get::<TraceId>()
            .cloned()
            .unwrap_or_else(|| TraceId(String::new()));
        let mut response = ApiError::new(ErrorCode::AuthRateLimited, &trace)
            .detail("too many authentication attempts")
            .into_response();
        let value = HeaderValue::from_str(&seconds.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("60"));
        response.headers_mut().insert(header::RETRY_AFTER, value);
        return response;
    }

    next.run(req).await
}

fn is_limited_path(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/auth/login" | "/api/v1/auth/oidc/start" | "/api/v1/auth/oidc/callback"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_then_retry_after() {
        let limiter = RateLimiter::new(2.0, 1.0, false);
        let start = Instant::now();
        assert!(limiter.check_at("ip", start).is_ok());
        assert!(limiter.check_at("ip", start).is_ok());
        assert_eq!(limiter.check_at("ip", start), Err(1));
        assert!(limiter
            .check_at("ip", start + Duration::from_secs(1))
            .is_ok());
    }

    #[test]
    fn separate_keys_have_separate_buckets() {
        let limiter = RateLimiter::new(1.0, 1.0, false);
        let start = Instant::now();
        assert!(limiter.check_at("a", start).is_ok());
        assert!(limiter.check_at("b", start).is_ok());
        assert!(limiter.check_at("a", start).is_err());
    }

    #[test]
    fn forwarded_for_is_used_only_when_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.9, 10.0.0.1"),
        );
        let peer: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        assert_eq!(client_ip(&headers, Some(peer), false), "127.0.0.1");
        assert_eq!(client_ip(&headers, Some(peer), true), "203.0.113.9");
    }

    #[test]
    fn user_agent_is_truncated() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_str(&"a".repeat(400)).unwrap(),
        );
        assert_eq!(user_agent(&headers).unwrap().chars().count(), 256);
    }
}
