//! 失敗の fingerprint と欠陥候補判定（`detail/09` §7 / §10.2）。

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::hex_lower;

/// NFKC → 連続空白を 1 個 → 前後 trim。
/// 数字列の `<N>` 置換は `assert_count` のときだけ（潰しすぎて別欠陥が合流するのを防ぐ）。
fn normalize(value: &str, squash_numbers: bool) -> String {
    let nfkc: String = value.nfkc().collect();
    let mut out = String::new();
    let mut prev_space = false;
    for ch in nfkc.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    let trimmed = out.trim().to_owned();
    if !squash_numbers {
        return trimmed;
    }
    let mut squashed = String::new();
    let mut in_number = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            if !in_number {
                squashed.push_str("<N>");
                in_number = true;
            }
        } else {
            squashed.push(ch);
            in_number = false;
        }
    }
    squashed
}

pub fn fingerprint(
    case_id: Uuid,
    failure_kind: &str,
    failure_step: i32,
    expected: Option<&str>,
    actual: Option<&str>,
    squash_numbers: bool,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(case_id.as_bytes());
    hasher.update([0]);
    hasher.update(failure_kind.as_bytes());
    hasher.update([0]);
    hasher.update(failure_step.to_string().as_bytes());
    hasher.update([0]);
    hasher.update(normalize(expected.unwrap_or(""), squash_numbers).as_bytes());
    hasher.update([0]);
    hasher.update(normalize(actual.unwrap_or(""), squash_numbers).as_bytes());
    hex_lower(&hasher.finalize())
}

/// `detail/09` §7 の「欠陥候補」列。DATA / ENV / RUNNER / CANCELLED は
/// テスト側・環境側の問題なので欠陥候補にしない。
pub fn is_defect_candidate(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind,
        Some("ASSERTION" | "SELECTOR_NOT_FOUND" | "TIMEOUT" | "NETWORK")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_failure_has_same_fingerprint() {
        let id = Uuid::from_u128(42);
        let a = fingerprint(id, "ASSERTION", 3, Some("３ 件"), Some("4 件"), false);
        let b = fingerprint(id, "ASSERTION", 3, Some("3 件"), Some("4 件"), false);
        assert_eq!(a, b, "NFKC と空白正規化で全角数字・連続空白を吸収する");
    }

    #[test]
    fn different_step_or_case_changes_fingerprint() {
        let id = Uuid::from_u128(42);
        let a = fingerprint(id, "ASSERTION", 3, Some("x"), Some("y"), false);
        let b = fingerprint(id, "ASSERTION", 4, Some("x"), Some("y"), false);
        let c = fingerprint(
            Uuid::from_u128(43),
            "ASSERTION",
            3,
            Some("x"),
            Some("y"),
            false,
        );
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn number_squashing_only_when_requested() {
        let id = Uuid::from_u128(1);
        let plain = fingerprint(id, "ASSERTION", 1, Some("3 件"), Some("3 件"), false);
        let count = fingerprint(id, "ASSERTION", 1, Some("3 件"), Some("4 件"), true);
        let count_same = fingerprint(id, "ASSERTION", 1, Some("7 件"), Some("9 件"), true);
        assert_eq!(count, count_same);
        let plain3 = fingerprint(id, "ASSERTION", 1, Some("3 件"), Some("3 件"), false);
        assert_eq!(plain, plain3);
    }

    #[test]
    fn environment_failures_are_not_defects() {
        assert!(is_defect_candidate(Some("ASSERTION")));
        assert!(is_defect_candidate(Some("NETWORK")));
        assert!(!is_defect_candidate(Some("DATA")));
        assert!(!is_defect_candidate(Some("ENV")));
        assert!(!is_defect_candidate(None));
    }
}
