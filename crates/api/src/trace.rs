//! `X-Trace-Id` の付与と伝播。
//!
//! 上流（LB / APM）が付けた値があれば**引き継ぐ**。無ければ作る。
//! 引き継ぐ際は形式を検査する。ヘッダをそのまま反射すると、
//! 改行混入などでヘッダインジェクションになる。

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

pub const HEADER: &str = "x-trace-id";

/// リクエスト拡張に入れる。ハンドラとエラー生成が読む。
#[derive(Debug, Clone)]
pub struct TraceId(pub String);

impl TraceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 受け入れてよい形か。16〜64 文字の 16 進（ハイフン可）。
/// それ以外は「無い」と見なして新しい ID を振る。
fn is_acceptable(v: &str) -> bool {
    (16..=64).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-' || b == b'_')
}

pub async fn with_trace_id(mut req: Request, next: Next) -> Response {
    let incoming = req
        .headers()
        .get(HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| is_acceptable(v))
        .map(str::to_owned);

    let id = incoming.unwrap_or_else(|| Uuid::new_v4().simple().to_string());

    req.extensions_mut().insert(TraceId(id.clone()));

    let mut res = next.run(req).await;

    if let Ok(v) = HeaderValue::from_str(&id) {
        res.headers_mut().insert(HeaderName::from_static(HEADER), v);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_hex_ids() {
        assert!(is_acceptable("4bf92f3577b34da6a3ce929d0e0e4736"));
        assert!(is_acceptable("4bf92f35-77b3-4da6-a3ce-929d0e0e4736"));
    }

    #[test]
    fn rejects_short_and_dirty_ids() {
        assert!(!is_acceptable("abc"));
        assert!(!is_acceptable("4bf92f3577b34da\r\nX-Evil: 1"));
        assert!(!is_acceptable("４bf92f3577b34da6a3ce929d0e0e4736"));
    }
}
