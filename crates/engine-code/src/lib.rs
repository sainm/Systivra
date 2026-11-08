//! コード走査、Java/JSP/XML/SQL の構文解析、Fact 正規化、増分判定。
//!
//! この crate はファイル IO を持ち込まず、呼び出し側が渡したバイト列を解析する。
//! リポジトリ走査とスナップショット作成は worker が担い、ここでは言語判定、
//! 文字コード・元バイト偏移、シンボル、Fact を決定的に生成する。

#![forbid(unsafe_code)]

pub mod analyze;
pub mod fact;
pub mod jsp;
pub mod language;
pub mod text;

pub use analyze::{analyze_sources, AnalyzeOptions};
pub use fact::{
    AnalysisError, AnalysisReport, BatchAnalysis, Evidence, Fact, FactObject, FactResolution,
    FactSubject, ParserStats, SourceAnalysis, SourceInput, Symbol, SymbolKind,
};
pub use language::Language;
