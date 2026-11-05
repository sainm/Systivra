//! 認証ミドルウェアと、パスの `{pid}` を照合する抽出子。
//!
//! 順序（`detail/05` §5.2）：`trace_id → 認証 → パス {pid} の照合 → SET LOCAL → 権限`。
//! このファイルが前半 2 つ、`ProjectScope` が 3 つ目を受け持つ。

pub mod session;

use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use contracts::ErrorCode;
use domain::{ProjectId, Role};

use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub use session::SessionConfig;

/// 認証を要求するミドルウェア。
///
/// 通れば `Authenticated` をリクエスト拡張に入れる。以降のハンドラは
/// `Extension<Authenticated>` で受け取る。失敗は 401（Cookie 無し・PAT 不正）。
pub async fn require_auth(State(state): State<ApiState>, mut req: Request, next: Next) -> Response {
    let trace = req
        .extensions()
        .get::<TraceId>()
        .cloned()
        .unwrap_or_else(|| TraceId(String::new()));

    let authenticated = match extract_credential(&req) {
        Some(Credential::Token(t)) => state.app.authenticate_token(&t).await,
        Some(Credential::Session(raw)) => match state.session.verify(&raw, chrono::Utc::now()) {
            Some(user) => state.app.authenticate_session(user).await,
            None => return unauthorized(&trace, "session cookie is invalid or expired"),
        },
        None => return unauthorized(&trace, "no credential"),
    };

    match authenticated {
        Ok(Some(a)) => {
            req.extensions_mut().insert(a);
            next.run(req).await
        }
        Ok(None) => unauthorized(&trace, "credential rejected"),
        Err(e) => {
            // 認証基盤の障害は 401 ではなく 500。401 にすると
            // 「パスワードが違う」と誤解され、利用者が入力をやり直し続ける。
            tracing::error!(error = %e, "authentication backend failed");
            ApiError::new(ErrorCode::Internal, &trace)
                .detail("authentication is temporarily unavailable")
                .into_response()
        }
    }
}

enum Credential {
    Token(String),
    Session(String),
}

/// `Authorization: Bearer` を優先し、無ければ Cookie を見る。
fn extract_credential(req: &Request) -> Option<Credential> {
    if let Some(v) = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return Some(Credential::Token(v.to_owned()));
    }

    let cookies = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())?;

    session::cookie_value(cookies, session::COOKIE_NAME).map(|v| Credential::Session(v.to_owned()))
}

fn unauthorized(trace: &TraceId, detail: &str) -> Response {
    ApiError::new(ErrorCode::AuthTokenExpired, trace)
        .detail(detail)
        .into_response()
}

use axum::response::IntoResponse;

// ─────────────────────────────────────────────────────────────
// パスの `{pid}` を本人のスコープと照合する
// ─────────────────────────────────────────────────────────────

/// スコープが確定したリクエスト。**これを持つハンドラだけが業務データに触れる。**
///
/// `{pid}` は「スコープ宣言」であって「対象指定」ではない（`detail/04` §4.2）。
/// 所属していなければ **404**。403 にすると「そのプロジェクトは存在する」と
/// 漏れる。
#[derive(Debug, Clone, Copy)]
pub struct ProjectScope {
    pub project: ProjectId,
    pub role: Role,
}

impl axum::extract::FromRequestParts<ApiState> for ProjectScope {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &ApiState,
    ) -> Result<Self, Self::Rejection> {
        let trace = parts
            .extensions
            .get::<TraceId>()
            .cloned()
            .unwrap_or_else(|| TraceId(String::new()));

        let Some(auth) = parts.extensions.get::<app::Authenticated>().cloned() else {
            // ミドルウェアを通っていないルート。実装の配線ミス。
            tracing::error!("ProjectScope used on a route without require_auth");
            return Err(ApiError::new(ErrorCode::Internal, &trace)
                .detail("route is not protected")
                .into_response());
        };

        let params =
            axum::extract::Path::<std::collections::HashMap<String, String>>::from_request_parts(
                parts, state,
            )
            .await
            .map_err(|_| {
                ApiError::new(ErrorCode::ValidationSchema, &trace)
                    .detail("path parameter is missing")
                    .into_response()
            })?;

        let raw = params.get("pid").ok_or_else(|| {
            ApiError::new(ErrorCode::ValidationSchema, &trace)
                .detail("{pid} is required")
                .into_response()
        })?;

        let project: ProjectId = raw.parse().map_err(|_| {
            // UUID でない `{pid}` は 404。形式エラーを返すと、
            // 総当たりで「どの形式が存在するか」を探られる。
            ApiError::new(ErrorCode::NotFound, &trace).into_response()
        })?;

        match state.app.role_in(auth.user_id, project).await {
            Ok(Some(role)) => Ok(Self { project, role }),
            Ok(None) => Err(ApiError::new(ErrorCode::NotFound, &trace).into_response()),
            Err(e) => {
                tracing::error!(error = %e, "scope lookup failed");
                Err(ApiError::new(ErrorCode::Internal, &trace).into_response())
            }
        }
    }
}

/// 現在の身元。ハンドラの引数に置くだけで認証を要求できる。
pub type CurrentUser = axum::Extension<app::Authenticated>;
