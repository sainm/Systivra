//! 設計書の種別。`detail/04-api-contract.md` §0.3 の列挙が正本。
//!
//! **`DETAILED_SPEC` は存在しない。** 詳細設計は `DETAILED_DESIGN`。
//! この 2 つを混ぜると DB の CHECK 制約で落ちる（`detail/02` §1 の `kb.document`）。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DocType {
    Requirement,
    BasicDesign,
    DetailedDesign,
    UiSpec,
    DbSpec,
    TestSpec,
    ApiSpec,
    Manual,
    Other,
}

impl DocType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requirement => "REQUIREMENT",
            Self::BasicDesign => "BASIC_DESIGN",
            Self::DetailedDesign => "DETAILED_DESIGN",
            Self::UiSpec => "UI_SPEC",
            Self::DbSpec => "DB_SPEC",
            Self::TestSpec => "TEST_SPEC",
            Self::ApiSpec => "API_SPEC",
            Self::Manual => "MANUAL",
            Self::Other => "OTHER",
        }
    }

    /// Phase 1 の PDF はこの 2 つに落とす。判別できないものを `BASIC_DESIGN` に
    /// ねじ込まない（後から直せなくなる）。
    pub fn pdf_default() -> Self {
        Self::Manual
    }

    pub const ALL: [DocType; 9] = [
        Self::Requirement,
        Self::BasicDesign,
        Self::DetailedDesign,
        Self::UiSpec,
        Self::DbSpec,
        Self::TestSpec,
        Self::ApiSpec,
        Self::Manual,
        Self::Other,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_has_no_detailed_spec() {
        assert!(!DocType::ALL.iter().any(|d| d.as_str() == "DETAILED_SPEC"));
    }

    #[test]
    fn serializes_as_screaming_snake() {
        let v = serde_json::to_value(DocType::DetailedDesign).unwrap();
        assert_eq!(v, "DETAILED_DESIGN");
    }

    #[test]
    fn rejects_unknown_value() {
        let r: Result<DocType, _> = serde_json::from_value(serde_json::json!("DETAILED_SPEC"));
        assert!(r.is_err());
    }
}
