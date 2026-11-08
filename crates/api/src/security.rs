//! 全応答に付けるセキュリティヘッダ。
//!
//! API は JSON のみを返すため CSP は付けない（Web は別プロセス / nginx）。
//! HSTS は `SESSION_COOKIE_SECURE=true`（https 前提）のときだけ付ける。

use axum::extract::{Request, State};
use axum::http::header::{self, HeaderValue};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;

use crate::ApiState;

pub async fn with_security_headers(
    State(state): State<ApiState>,
    req: Request,
    next: Next,
) -> Response {
    let mut response = next.run(req).await;
    apply(response.headers_mut(), state.session.is_secure());
    response
}

pub fn apply(headers: &mut HeaderMap, secure: bool) {
    insert(headers, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    insert(headers, header::REFERRER_POLICY, "no-referrer");
    insert(headers, header::X_FRAME_OPTIONS, "DENY");
    insert(
        headers,
        header::HeaderName::from_static("permissions-policy"),
        "geolocation=(), microphone=(), camera=()",
    );
    if secure {
        insert(
            headers,
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=31536000; includeSubDomains",
        );
    }
}

fn insert(headers: &mut HeaderMap, name: header::HeaderName, value: &'static str) {
    if !headers.contains_key(&name) {
        headers.insert(name, HeaderValue::from_static(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_headers_are_set() {
        let mut headers = HeaderMap::new();
        apply(&mut headers, false);
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["referrer-policy"], "no-referrer");
        assert_eq!(headers["x-frame-options"], "DENY");
        assert!(headers.contains_key("permissions-policy"));
        assert!(!headers.contains_key("strict-transport-security"));
    }

    #[test]
    fn hsts_only_when_secure() {
        let mut headers = HeaderMap::new();
        apply(&mut headers, true);
        assert_eq!(
            headers["strict-transport-security"],
            "max-age=31536000; includeSubDomains"
        );
    }
}
