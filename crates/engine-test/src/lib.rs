//! Phase 6: Test Engineering の中核（IO なしの純ロジック）。
//!
//! 正本: `docs/design/ja/04-コアエンジン.md` §4.6、
//!       `docs/design/ja/detail/09-test-runner.md`、
//!       `docs/design/ja/detail/08-change-inquiry.md` §6 / §7。
//!
//! ここは「証拠のあるものだけを出す」縦断実装である。
//! 特に次を守る（前 Phase と同じ方針）:
//!   - 解析できない記述からルールを捏造しない（`unparsed` に数える）。
//!   - グラフの `VERIFIES` 辺が無いとき、Requirement Coverage を近似で埋めない。
//!   - Runner を実装していないのに PASSED を生成しない。実行結果は外部から
//!     `execution_source` 付きで取り込む。
//!   - コードと文書の不一致は `CONFLICT` として両方残す。

#![forbid(unsafe_code)]

pub mod coverage;
pub mod derive;
pub mod dsl;
pub mod execution;
pub mod rule;

pub const ENGINE_VERSION: &str = concat!("test-", env!("CARGO_PKG_VERSION"));

pub use coverage::{compute_coverage, CoverageCase, CoverageResult, CoverageTotals, MetricResult};
pub use derive::{
    derive_assets, Derivation, DeriveInput, DerivedCase, DerivedDataSet, DerivedScenario,
    ImpactItemSeed,
};
pub use dsl::{
    guard_execution, retry_allowed, validate_dsl, validate_import, ImportContext,
    ASSERTION_ACTIONS, FAILURE_KINDS, V1_ACTIONS,
};
pub use execution::{fingerprint, is_defect_candidate};
pub use rule::{extract_rules, ExtractedRule, RuleExtraction, RuleInput};

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// 入力から安定した UUID を作る。同じ入力なら同じ ID（detail/09 §9.4 の再現性）。
pub(crate) fn stable_id(project: Uuid, parts: &[&str]) -> Uuid {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    let digest = hasher.finalize();
    Uuid::new_v5(&project, &digest[..16])
}

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}
