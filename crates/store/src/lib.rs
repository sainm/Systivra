//! PG アクセス全般。マイグレーション、Outbox、ジョブキュー。
//!
//! 境界（`02-アーキテクチャ` §3）：`store` は **モジュール横断の業務判断を持たない**。
//! ここに置けるのは SQL と、その結果を領域型へ移す変換だけ。
//!
//! テナント分離は RLS で強制する。アプリはトランザクション開始時に
//! `SET LOCAL app.project_id` を設定する（`detail/05` §5.1）。設定を忘れると
//! **0 行になる**（エラーにならない）。そのため `begin_scoped` 以外で
//! 業務クエリを書かない。

pub mod auth;
pub mod pool;

pub use auth::{MembershipRow, TokenRow, UserRow};
pub use pool::{commit, Store, StoreError, Tx};
