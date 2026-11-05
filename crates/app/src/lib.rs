//! ユースケース編成。トランザクション境界・冪等・Outbox はここが持つ。
//!
//! 境界（`02-アーキテクチャ` §3）：`app` は**解析・検索アルゴリズムを持たない**。
//! `api` と `worker` の両方から呼ばれる。だから `app` に HTTP の型を置かない。

#![forbid(unsafe_code)]

pub mod auth;
pub mod health;

pub use auth::{AuthError, Authenticated, Credential};
pub use health::Readiness;

use store::Store;

/// アプリケーションの実行時状態。プロセスごとに 1 つ。
#[derive(Clone)]
pub struct App {
    store: Store,
}

impl App {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// `DATABASE_URL` から組み立てる。
    ///
    /// `api` / `worker` に `store` を依存させないための入口。
    /// こうしておくと、HTTP 層から SQL を書こうとしても型が無い。
    pub fn from_database_url(url: &str) -> Result<Self, store::StoreError> {
        Ok(Self::new(Store::connect_lazy(url)?))
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// 依存の疎通をまとめて確認する。`/readyz` から呼ぶ。
    ///
    /// `/healthz`（プロセスが生きているか）とは別物。ここが落ちても
    /// プロセスは 200 を返し続ける（Kubernetes の liveness / readiness の分離）。
    pub async fn readiness(&self) -> Readiness {
        Readiness::check(&self.store).await
    }
}
