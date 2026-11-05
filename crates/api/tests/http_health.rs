//! HTTP 境界の最小契約テスト。
//!
//! DB には繋がない（`connect_lazy`）。ここで確かめたいのは
//! 「DB が落ちていてもプロセスは 200 を返す」という liveness / readiness の分離。

use app::App;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app_with_unreachable_db() -> axum::Router {
    // ポート 1 は誰も listen していない。接続は即座に拒否される。
    // `store` は api の依存に無い（境界）。だから組み立ては app に任せる。
    let app = App::from_database_url("postgres://nobody@127.0.0.1:1/none")
        .expect("lazy 接続のプールは作れるはず");
    api::router(app)
}

#[tokio::test]
async fn healthz_is_200_without_database() {
    let res = app_with_unreachable_db()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["status"], "ok");
}

#[tokio::test]
async fn healthz_always_sets_trace_id() {
    let res = app_with_unreachable_db()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let id = res
        .headers()
        .get("x-trace-id")
        .expect("X-Trace-Id が無い")
        .to_str()
        .unwrap();
    // 32 桁の 16 進（Uuid::simple）
    assert_eq!(id.len(), 32);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn incoming_trace_id_is_propagated() {
    let given = "4bf92f3577b34da6a3ce929d0e0e4736";
    let res = app_with_unreachable_db()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("x-trace-id", given)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.headers()["x-trace-id"], given);
}

#[tokio::test]
async fn dirty_trace_id_is_replaced() {
    let res = app_with_unreachable_db()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("x-trace-id", "abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let got = res.headers()["x-trace-id"].to_str().unwrap();
    assert_ne!(got, "abc");
    assert_eq!(got.len(), 32);
}

#[tokio::test]
async fn readyz_is_503_when_database_is_unreachable() {
    let res = app_with_unreachable_db()
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ready"], false);
    assert_eq!(v["database"]["state"], "FAILED");
}
