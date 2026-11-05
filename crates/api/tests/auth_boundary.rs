//! 認証が要るルートの境界。
//!
//! DB には繋がない。ここで見たいのは「認証が無いと入れない」ことと
//! 「入力が空なら 400」であること。

use app::App;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Duration;
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> axum::Router {
    let app = App::from_database_url("postgres://nobody@127.0.0.1:1/none").unwrap();
    let session = api::auth::SessionConfig::new(
        b"0123456789abcdef0123456789abcdef".to_vec(),
        Duration::hours(1),
        false,
    );
    api::router(app, session)
}

async fn body_json(res: axum::response::Response) -> serde_json::Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn me_requires_authentication() {
    let res = app()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        res.headers()["content-type"],
        contracts::Problem::CONTENT_TYPE,
        "401 は problem+json で返す"
    );

    let v = body_json(res).await;
    assert_eq!(v["code"], "AUTH_TOKEN_EXPIRED");
    assert!(v["trace_id"].as_str().is_some_and(|t| !t.is_empty()));
}

#[tokio::test]
async fn project_scope_requires_authentication() {
    let res = app()
        .oneshot(
            Request::builder()
                .uri("/api/v1/projects/00000000-0000-0000-0000-000000000000/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // 未認証なら 401。404 にすると「認証さえすれば存在は分かる」ことになる。
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bad_bearer_token_is_rejected() {
    let res = app()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/me")
                .header("authorization", "Bearer not-a-real-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // DB に繋がらないので認証基盤は落ちるが、トークンは不正なので 401 か 500。
    // ここでは「通らないこと」だけを保証する。
    assert!(
        res.status() == StatusCode::UNAUTHORIZED || res.status().is_server_error(),
        "unexpected status: {}",
        res.status()
    );
}

#[tokio::test]
async fn login_rejects_empty_credentials_before_touching_db() {
    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"login_id":"","password":""}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let v = body_json(res).await;
    assert_eq!(v["code"], "VALIDATION_SCHEMA");
}

#[tokio::test]
async fn healthz_is_still_public() {
    // 認証を全体に掛けると liveness probe が 401 で落ちる。その退行検出。
    let res = app()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
