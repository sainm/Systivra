use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FactResolution {
    Resolved,
    Inferred,
    Partial,
    Unresolved,
    Conflict,
}

impl FactResolution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resolved => "RESOLVED",
            Self::Inferred => "INFERRED",
            Self::Partial => "PARTIAL",
            Self::Unresolved => "UNRESOLVED",
            Self::Conflict => "CONFLICT",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Resolved => 4,
            Self::Partial => 3,
            Self::Inferred => 2,
            Self::Conflict => 1,
            Self::Unresolved => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SymbolKind {
    Class,
    Method,
    Field,
    Interface,
    Enum,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Class => "CLASS",
            Self::Method => "METHOD",
            Self::Field => "FIELD",
            Self::Interface => "INTERFACE",
            Self::Enum => "ENUM",
        }
    }

    pub fn entity_type(self) -> &'static str {
        match self {
            Self::Class => "Class",
            Self::Method => "Method",
            Self::Field => "Field",
            Self::Interface => "Interface",
            Self::Enum => "Class",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactSubject {
    pub kind: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactObject {
    pub kind: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub path: String,
    /// Original file byte offset, including a BOM when one exists.
    pub byte_start: u64,
    pub byte_end: u64,
    pub line: i32,
    pub column: i32,
    pub quote: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub relation: String,
    pub subject: FactSubject,
    pub object: FactObject,
    pub resolution: FactResolution,
    pub confidence: f32,
    pub analysis_engine: String,
    pub engine_version: String,
    pub unresolved_reason: Option<String>,
    pub candidates: Vec<String>,
    pub evidence: Vec<Evidence>,
}

impl Fact {
    pub fn inferred(
        relation: &str,
        subject: FactSubject,
        object: FactObject,
        evidence: Evidence,
        candidates: Vec<String>,
        reason: &str,
    ) -> Self {
        Self {
            relation: relation.to_string(),
            subject,
            object,
            resolution: FactResolution::Inferred,
            confidence: 0.65,
            analysis_engine: "tree-sitter".into(),
            engine_version: env!("CARGO_PKG_VERSION").into(),
            unresolved_reason: Some(reason.into()),
            candidates,
            evidence: vec![evidence],
        }
    }

    pub fn resolved(
        relation: &str,
        subject: FactSubject,
        object: FactObject,
        evidence: Evidence,
    ) -> Self {
        Self {
            relation: relation.to_string(),
            subject,
            object,
            resolution: FactResolution::Resolved,
            confidence: 0.98,
            analysis_engine: "tree-sitter".into(),
            engine_version: env!("CARGO_PKG_VERSION").into(),
            unresolved_reason: None,
            candidates: Vec::new(),
            evidence: vec![evidence],
        }
    }

    pub fn unresolved(
        relation: &str,
        subject: FactSubject,
        object: FactObject,
        evidence: Evidence,
        candidates: Vec<String>,
        reason: &str,
    ) -> Self {
        Self {
            relation: relation.to_string(),
            subject,
            object,
            resolution: FactResolution::Unresolved,
            confidence: 0.2,
            analysis_engine: "tree-sitter".into(),
            engine_version: env!("CARGO_PKG_VERSION").into(),
            unresolved_reason: Some(reason.into()),
            candidates,
            evidence: vec![evidence],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub fqn: String,
    pub simple_name: String,
    pub signature: Option<String>,
    pub line_start: i32,
    pub line_end: i32,
    pub byte_start: u64,
    pub byte_end: u64,
    pub modifiers: Vec<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParserStats {
    pub nodes: u64,
    pub errors: u64,
    pub missing: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisError {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub files_seen: u64,
    pub files_analyzed: u64,
    pub files_skipped: u64,
    pub symbols: Vec<Symbol>,
    pub facts: Vec<Fact>,
    pub errors: Vec<AnalysisError>,
    pub parser: ParserStats,
    pub classpath_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInput {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceAnalysis {
    pub path: String,
    pub language: String,
    pub content_hash: Vec<u8>,
    pub size_bytes: i64,
    pub encoding: String,
    pub exact_offsets: bool,
    pub loc: i32,
    /// Little-endian u32 values of original-file line starts.
    pub line_offsets: Vec<u8>,
    pub symbols: Vec<Symbol>,
    pub facts: Vec<Fact>,
    pub parser: ParserStats,
    pub error: Option<AnalysisError>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BatchAnalysis {
    pub files: Vec<SourceAnalysis>,
    pub report: AnalysisReport,
}
