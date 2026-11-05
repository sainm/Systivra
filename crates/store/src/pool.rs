use std::time::Duration;

use domain::{ProjectId, UserId};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, Transaction};

/// トランザクション。`app.project_id` を設定済みのものだけを外へ渡す。
pub type Tx<'a> = Transaction<'a, Postgres>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("接続プールの取得に失敗: {0}")]
    Pool(String),
}

/// 接続プールの薄いラッパ。
///
/// 直接 `PgPool` を配らないのは、`SET LOCAL app.project_id` を
/// 迂回したクエリを書けなくするため。`pool()` は用意しない。
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// `DATABASE_URL` から接続する。起動時は DB が落ちていてもよい
    /// （`/healthz` を 200 で返し、`/readyz` で 503 にする）。
    pub async fn connect(database_url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    /// 接続を確立せずにプールを作る。`connect_lazy` は起動を DB に依存させない。
    pub fn connect_lazy(database_url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(database_url)
            .map_err(|e| StoreError::Pool(e.to_string()))?;
        Ok(Self { pool })
    }

    /// 疎通確認。`/readyz` からのみ呼ぶ。
    pub async fn ping(&self) -> Result<(), StoreError> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }

    /// マイグレーションを適用する。起動時ではなく `systivra-migrate` から呼ぶ。
    pub async fn migrate(&self, dir: &str) -> Result<(), StoreError> {
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(dir)).await?;
        migrator.run(&self.pool).await?;
        Ok(())
    }

    /// テナントスコープを設定したトランザクションを開く。
    ///
    /// 第 2 引数はサーバー側コンテキストから取る。**リクエスト本文の
    /// `project_id` を渡してはならない**（`detail/04` §4.2）。
    pub async fn begin_scoped(&self, project: ProjectId) -> Result<Tx<'_>, StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.project_id', $1, true)")
            .bind(project.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    /// ユーザーと（任意で）プロジェクトを設定したトランザクション。
    ///
    /// ログイン経路と「自分の所属一覧」は、プロジェクトが決まる前に走る。
    /// そのため `app.user_id` だけで読める経路が要る（`003_authz.sql`）。
    pub async fn begin_as(
        &self,
        user: UserId,
        project: Option<ProjectId>,
    ) -> Result<Tx<'_>, StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.user_id', $1, true)")
            .bind(user.to_string())
            .execute(&mut *tx)
            .await?;
        if let Some(p) = project {
            sqlx::query("SELECT set_config('app.project_id', $1, true)")
                .bind(p.to_string())
                .execute(&mut *tx)
                .await?;
        }
        Ok(tx)
    }

    /// スコープを設定しないトランザクション。
    ///
    /// 使ってよいのは「プロジェクトに属さない表」だけ（`core.workspace`、
    /// `core.app_user`、ログイン経路）。業務表には使わない。
    pub async fn begin_unscoped(&self) -> Result<Tx<'_>, StoreError> {
        Ok(self.pool.begin().await?)
    }
}

/// 読み取りトランザクションを閉じる。
///
/// `Transaction::commit` は `sqlx::Error` を返す。`app` は sqlx に依存しないので、
/// ここで `StoreError` に包んでから渡す。
pub async fn commit(tx: Tx<'_>) -> Result<(), StoreError> {
    tx.commit().await?;
    Ok(())
}
