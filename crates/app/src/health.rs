//! 起動時の依存チェック。`/readyz` の判定。

use std::time::Instant;

use store::Store;

/// 依存ごとの状態。画面と監視にそのまま出す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dependency {
    Ok { latency_ms: u64 },
    Failed { reason: String },
}

/// readiness の結果。1 つでも失敗なら `ready == false`。
#[derive(Debug, Clone)]
pub struct Readiness {
    pub database: Dependency,
}

impl Readiness {
    pub async fn check(store: &Store) -> Self {
        let started = Instant::now();
        let database = match store.ping().await {
            Ok(()) => Dependency::Ok {
                latency_ms: started.elapsed().as_millis() as u64,
            },
            Err(e) => Dependency::Failed {
                reason: e.to_string(),
            },
        };
        Self { database }
    }

    /// 全依存が OK か。
    pub fn is_ready(&self) -> bool {
        matches!(self.database, Dependency::Ok { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_dependency_is_not_ready() {
        let r = Readiness {
            database: Dependency::Failed {
                reason: "connection refused".into(),
            },
        };
        assert!(!r.is_ready());
    }

    #[test]
    fn ok_dependency_is_ready() {
        let r = Readiness {
            database: Dependency::Ok { latency_ms: 3 },
        };
        assert!(r.is_ready());
    }
}
