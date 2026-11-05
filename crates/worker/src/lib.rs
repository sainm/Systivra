//! ジョブ取得と実行。
//!
//! 境界（`02-アーキテクチャ` §3）：`worker` は **`app` を迂回して状態を変えない**。
//! ジョブの取り出しは `SELECT … FOR UPDATE SKIP LOCKED`（`detail/09` §3 の
//! Scheduler 役）。DB ロールは `systivra_scheduler`（`BYPASSRLS`）を使う。
//!
//! Phase 1 では骨格のみ。`P1-JOB-01` で取り出しループを実装する。

#![forbid(unsafe_code)]

pub mod text;

/// worker の 1 反復。`P1-JOB-01` で実装する。
pub async fn tick() {
    // TODO(P1-JOB-01): scheduler ロールで job を取り、
    // app.project_id を設定してからハンドラを呼ぶ。
}
