//! XLSX の読み取り。結合セルと図形アンカー。
//!
//! 正本：`detail/01-excel-ingestion.md` §3.1 / §5 / §7。
//!
//! **セル値の読み取りライブラリに頼らない理由**：§7 が要求する図形・画像・グラフは
//! `xl/drawings/drawingN.xml` にあり、セルを読むライブラリの多くはこれを無視する。
//! 図形がどのセルに貼られているかが分からないと、「この画面項目の説明図」を
//! 証拠として結び付けられない。だから ZIP と XML を直接読む。
//!
//! ここで扱うのは構造だけ。値の解釈（共有文字列、数式、日付）は別途。

#![forbid(unsafe_code)]

pub mod a1;
pub mod ooxml;

pub use a1::CellRef;
pub use ooxml::{open, Figure, FigureKind, MergedRegion, Sheet, Workbook, XlsxError};
