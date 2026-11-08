use serde_json::json;
use uuid::Uuid;

use app::{ChunkDraft, EvidenceDraft, NodeDraft};

use super::a1::CellRef;
use super::ooxml::{open, FigureKind, Sheet, Visibility, XlsxError};
use super::region::{plan_sheet, Region, XlsxChunk};
use super::values::SheetCells;

const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_CELLS: usize = 500_000;
const MAX_SHEETS: usize = 50;

#[derive(Debug, Clone)]
pub struct XlsxPage {
    pub index: i32,
    pub sheet: String,
    pub nodes: Vec<NodeDraft>,
    pub chunks: Vec<ChunkDraft>,
    pub evidence: Vec<EvidenceDraft>,
}

#[derive(Debug, Clone, Default)]
pub struct XlsxIngest {
    pub pages: Vec<XlsxPage>,
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("暗号化されたブックです")]
    Encrypted,
    #[error("サイズまたはセル数が上限を超えています: {0}")]
    TooLarge(String),
    #[error("OOXML として壊れています: {0}")]
    Broken(String),
    #[error("取り込めるシートがありません")]
    Empty,
}

impl IngestError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Encrypted => "E-XLSX-002",
            Self::TooLarge(_) => "E-XLSX-003",
            Self::Broken(_) => "E-XLSX-001",
            Self::Empty => "E-XLSX-005",
        }
    }

    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl From<XlsxError> for IngestError {
    fn from(value: XlsxError) -> Self {
        match value {
            XlsxError::Zip(message) => Self::Broken(message),
            other => Self::Broken(other.to_string()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn read(
    project_id: Uuid,
    document_id: Uuid,
    revision_id: Uuid,
    object_key: &str,
    file_size: u64,
    bytes: &[u8],
    include_hidden: bool,
) -> Result<XlsxIngest, IngestError> {
    if file_size > MAX_FILE_BYTES {
        return Err(IngestError::TooLarge(format!(
            "{} bytes > {MAX_FILE_BYTES}",
            file_size
        )));
    }
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Err(IngestError::Encrypted);
    }

    let workbook = open(bytes)?;
    let sheets = workbook.sheets()?;
    if sheets.is_empty() {
        return Err(IngestError::Empty);
    }
    if sheets.len() > MAX_SHEETS {
        return Err(IngestError::TooLarge(format!(
            "{} sheets > {MAX_SHEETS}",
            sheets.len()
        )));
    }

    let imported: Vec<Sheet> = sheets
        .into_iter()
        .filter(|s| include_hidden || s.visibility == Visibility::Visible)
        .collect();
    if imported.is_empty() {
        return Err(IngestError::Empty);
    }

    let document_node_id_value = document_node_id(revision_id);
    let mut ingest = XlsxIngest::default();
    let mut ordinal = 0i32;
    let mut total_cells = 0usize;
    let styles = workbook.styles()?;

    let mut parsed = Vec::with_capacity(imported.len());
    for sheet in &imported {
        let cells = workbook.sheet_cells(sheet)?;
        let merged = workbook.merged_regions(sheet)?;
        let figures = workbook.figures(sheet)?;
        total_cells += cells.len();
        if total_cells > MAX_CELLS {
            return Err(IngestError::TooLarge(format!(
                "{total_cells} cells > {MAX_CELLS}"
            )));
        }
        ingest.warnings.extend(cells.warnings().iter().cloned());
        let plan = plan_sheet(sheet, &cells, &merged, &styles, ordinal);
        ordinal += plan.chunks.len() as i32;
        parsed.push((sheet.clone(), cells, plan, figures));
    }

    for (sheet_index, (sheet, cells, plan, figures)) in parsed.into_iter().enumerate() {
        let index = (sheet_index + 1) as i32;
        let sheet_node = sheet_node_id(revision_id, &sheet);
        let mut nodes = Vec::new();
        if sheet_index == 0 {
            nodes.push(document_node(revision_id, object_key));
        }
        nodes.push(NodeDraft {
            id: sheet_node,
            revision_id,
            parent_id: Some(document_node_id_value),
            node_type: "SHEET".into(),
            locator: json!({"kind": "xlsx", "sheet": sheet.name}),
            ordinal: index,
            text: None,
            meta: json!({"sheet_index": sheet_index, "visible": sheet.visibility == Visibility::Visible}),
        });

        for (region_index, region) in plan.regions.iter().enumerate() {
            nodes.push(region_node(
                revision_id,
                &sheet,
                sheet_node,
                region,
                region_index,
            ));
        }
        for (figure_index, figure) in figures.iter().enumerate() {
            let anchor = figure.anchor.to_a1();
            let locator = json!({
                "kind": "xlsx",
                "sheet": sheet.name,
                "anchor_cell": anchor,
                "to_cell": figure.to.map(|to| to.to_a1()),
                "offset": {"row": figure.anchor.row, "col": figure.anchor.col},
            });
            nodes.push(NodeDraft {
                id: Uuid::new_v5(
                    &project_id,
                    format!(
                        "figure:{revision_id}:{}:{anchor}:{:?}:{figure_index}",
                        sheet.name, figure.kind
                    )
                    .as_bytes(),
                ),
                revision_id,
                parent_id: Some(sheet_node),
                node_type: "FIGURE".into(),
                locator,
                ordinal: (plan.regions.len() + figure_index) as i32,
                text: None,
                meta: json!({
                    "figure_kind": match figure.kind {
                        FigureKind::Picture => "PICTURE",
                        FigureKind::Shape => "SHAPE",
                        FigureKind::Chart => "CHART",
                    },
                    "media_sha256": figure.media.as_ref().map(|m| m.sha256.clone()),
                    "media_bytes": figure.media.as_ref().map(|m| m.bytes),
                }),
            });
        }

        let mut chunks = Vec::with_capacity(plan.chunks.len());
        let mut evidence = Vec::with_capacity(plan.chunks.len());
        for chunk in &plan.chunks {
            let (draft, items) =
                drafts_for_chunk(project_id, document_id, revision_id, &sheet, &cells, chunk);
            chunks.push(draft);
            evidence.extend(items);
        }

        ingest.pages.push(XlsxPage {
            index,
            sheet: sheet.name.clone(),
            nodes,
            chunks,
            evidence,
        });
    }

    Ok(ingest)
}

fn document_node_id(revision_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::from_u128(0x6f9619ff_8b86_d011_b42d_00c04fc964ff),
        format!("document:{revision_id}").as_bytes(),
    )
}

fn document_node(revision_id: Uuid, object_key: &str) -> NodeDraft {
    NodeDraft {
        id: document_node_id(revision_id),
        revision_id,
        parent_id: None,
        node_type: "DOCUMENT".into(),
        locator: json!({"kind": "xlsx"}),
        ordinal: 0,
        text: None,
        meta: json!({"filename": object_key, "parser": "xlsx-native"}),
    }
}

fn sheet_node_id(revision_id: Uuid, sheet: &Sheet) -> Uuid {
    Uuid::new_v5(
        &revision_id,
        format!("sheet:{}:{}", sheet.name, sheet.part).as_bytes(),
    )
}

fn region_node(
    revision_id: Uuid,
    sheet: &Sheet,
    parent_id: Uuid,
    region: &Region,
    ordinal: usize,
) -> NodeDraft {
    NodeDraft {
        id: Uuid::new_v5(
            &revision_id,
            format!("region:{}:{}", sheet.name, region.id).as_bytes(),
        ),
        revision_id,
        parent_id: Some(parent_id),
        node_type: "REGION".into(),
        locator: json!({
            "kind": "xlsx",
            "sheet": sheet.name,
            "range": region.range_a1(),
            "region_id": region.id,
        }),
        ordinal: ordinal as i32,
        text: None,
        meta: json!({"header_row": region.header_row.map(|r| r + 1)}),
    }
}

fn drafts_for_chunk(
    project_id: Uuid,
    document_id: Uuid,
    revision_id: Uuid,
    sheet: &Sheet,
    cells: &SheetCells,
    chunk: &XlsxChunk,
) -> (ChunkDraft, Vec<EvidenceDraft>) {
    let normalized = engine_rag::normalize_nfc(&chunk.text);
    let hash = engine_rag::sha256(normalized.as_bytes());
    let range = chunk
        .locator
        .get("range")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let chunk_id = Uuid::new_v5(
        &project_id,
        format!(
            "{revision_id}:{}:{range}:{}",
            sheet.name,
            hex_prefix(&hash, 8)
        )
        .as_bytes(),
    );
    let tokenized = engine_rag::tokenized_text(&chunk.text);
    let token_count = engine_rag::tokenize(&chunk.text).len() as i32;
    let draft = ChunkDraft {
        id: chunk_id,
        project_id,
        document_id,
        revision_id,
        chunk_type: chunk.chunk_type.clone(),
        ordinal: chunk.ordinal,
        text: chunk.text.clone(),
        tokenized,
        token_count,
        char_count: chunk.char_count,
        content_hash: hash,
        locator: chunk.locator.clone(),
        node_path: chunk.node_path.clone(),
        metadata: json!({
            "sheet": sheet.name,
            "region_id": chunk.region_id,
            "rows": [chunk.row_start + 1, chunk.row_end + 1],
            "parser": "xlsx-native",
            "parser_version": env!("CARGO_PKG_VERSION"),
            "tokenizer": "lindera-ipadic",
            "tokenizer_version": engine_rag::TOKENIZER_VERSION,
            "cells": cells.len(),
        }),
        embedding: engine_rag::embed(&chunk.text),
    };
    let evidence = EvidenceDraft {
        id: Uuid::new_v5(&project_id, format!("evidence:{chunk_id}").as_bytes()),
        project_id,
        source_type: "XLSX_REGION_TEXT".into(),
        document_id: Some(document_id),
        revision_id: Some(revision_id),
        chunk_id: Some(chunk_id),
        locator: chunk.locator.clone(),
        quote: normalized,
        quote_hash: engine_rag::quote_hash(&chunk.text),
        resolution: "RESOLVED".into(),
        confidence: 1.0,
    };
    let mut items = vec![evidence];
    for (row, row_text) in &chunk.row_texts {
        let row_quote = engine_rag::normalize_nfc(row_text);
        if row_quote.trim().is_empty() {
            continue;
        }
        let start = CellRef::new(*row, chunk.col_start);
        let end = CellRef::new(*row, chunk.col_end);
        items.push(EvidenceDraft {
            id: Uuid::new_v5(
                &project_id,
                format!("evidence:{chunk_id}:row:{row}").as_bytes(),
            ),
            project_id,
            source_type: "XLSX_ROW".into(),
            document_id: Some(document_id),
            revision_id: Some(revision_id),
            chunk_id: Some(chunk_id),
            locator: json!({
                "kind": "xlsx",
                "sheet": sheet.name,
                "range": format!("{}:{}", start.to_a1(), end.to_a1()),
                "row": row + 1,
                "region_id": chunk.region_id,
                "parent_range": chunk.locator.get("range"),
            }),
            quote: row_quote,
            quote_hash: engine_rag::quote_hash(row_text),
            resolution: "RESOLVED".into(),
            confidence: 1.0,
        });
    }
    (draft, items)
}

fn hex_prefix(bytes: &[u8], len: usize) -> String {
    let mut out = String::with_capacity(len * 2);
    for byte in bytes.iter().take(len) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;

    use super::*;

    fn fixture(hidden: bool) -> Vec<u8> {
        let state = if hidden { r#" state="hidden""# } else { "" };
        let parts: Vec<(String, Vec<u8>)> = vec![
            (
                "[Content_Types].xml".into(),
                br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_vec(),
            ),
            (
                "xl/workbook.xml".into(),
                format!(r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="画面項目定義" sheetId="1"{state} r:id="rId1"/></sheets></workbook>"#).into_bytes(),
            ),
            (
                "xl/_rels/workbook.xml.rels".into(),
                br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="sharedStrings" Target="sharedStrings.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "xl/sharedStrings.xml".into(),
                r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>項目名</t></si><si><t>入力形式</t></si><si><t>住所</t></si><si><t>全角20文字以内</t></si></sst>"#.as_bytes().to_vec(),
            ),
            (
                "xl/worksheets/sheet1.xml".into(),
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2" t="s"><v>3</v></c></row><row r="3"><c r="A3" t="s"><v>2</v></c><c r="B3" t="s"><v>3</v></c></row></sheetData></worksheet>"#.as_bytes().to_vec(),
            ),
        ];

        let mut buf = Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let opts = SimpleFileOptions::default();
            for (name, data) in parts {
                zw.start_file(name, opts).unwrap();
                zw.write_all(&data).unwrap();
            }
            zw.finish().unwrap();
        }
        buf.into_inner()
    }

    fn ids() -> (Uuid, Uuid, Uuid) {
        (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3))
    }

    #[test]
    fn builds_pages_nodes_chunks_and_evidence() {
        let (project, document, revision) = ids();
        let bytes = fixture(false);
        let ingest = read(
            project,
            document,
            revision,
            "design/screen.xlsx",
            bytes.len() as u64,
            &bytes,
            false,
        )
        .unwrap();

        assert_eq!(ingest.pages.len(), 1);
        let page = &ingest.pages[0];
        assert_eq!(page.index, 1);
        assert_eq!(page.sheet, "画面項目定義");
        assert_eq!(page.chunks.len(), 1);
        assert_eq!(page.evidence.len(), 3);
        assert!(page
            .evidence
            .iter()
            .any(|item| item.source_type == "XLSX_ROW"));
        assert!(page.nodes.iter().any(|n| n.node_type == "DOCUMENT"));
        assert!(page.nodes.iter().any(|n| n.node_type == "SHEET"));
        assert!(page.nodes.iter().any(|n| n.node_type == "REGION"));
        assert_eq!(page.chunks[0].chunk_type, "SHEET_REGION");
        assert_eq!(page.chunks[0].locator["kind"], "xlsx");
        assert!(page.chunks[0].text.contains("項目名: 住所"));
        assert_eq!(page.evidence[0].quote, page.chunks[0].text);
        assert!(!page.chunks[0].tokenized.is_empty());
        assert_eq!(page.chunks[0].embedding.len(), 1024);
    }

    #[test]
    fn chunk_ids_are_deterministic() {
        let (project, document, revision) = ids();
        let bytes = fixture(false);
        let first = read(
            project,
            document,
            revision,
            "a.xlsx",
            bytes.len() as u64,
            &bytes,
            false,
        )
        .unwrap();
        let second = read(
            project,
            document,
            revision,
            "a.xlsx",
            bytes.len() as u64,
            &bytes,
            false,
        )
        .unwrap();
        assert_eq!(first.pages[0].chunks[0].id, second.pages[0].chunks[0].id);
    }

    #[test]
    fn skips_hidden_sheets_by_default() {
        let (project, document, revision) = ids();
        let bytes = fixture(true);
        let err = read(
            project,
            document,
            revision,
            "a.xlsx",
            bytes.len() as u64,
            &bytes,
            false,
        )
        .unwrap_err();
        assert!(matches!(err, IngestError::Empty));

        let ingest = read(
            project,
            document,
            revision,
            "a.xlsx",
            bytes.len() as u64,
            &bytes,
            true,
        )
        .unwrap();
        assert_eq!(ingest.pages.len(), 1);
    }

    #[test]
    fn rejects_encrypted_ole_container() {
        let (project, document, revision) = ids();
        let err = read(
            project,
            document,
            revision,
            "a.xlsx",
            16,
            &[0xD0, 0xCF, 0x11, 0xE0, 0, 0, 0, 0],
            false,
        )
        .unwrap_err();
        assert!(matches!(err, IngestError::Encrypted));
        assert_eq!(err.code(), "E-XLSX-002");
    }

    #[test]
    fn rejects_oversized_files() {
        let (project, document, revision) = ids();
        let err = read(
            project,
            document,
            revision,
            "a.xlsx",
            MAX_FILE_BYTES + 1,
            b"",
            false,
        )
        .unwrap_err();
        assert!(matches!(err, IngestError::TooLarge(_)));
        assert_eq!(err.code(), "E-XLSX-003");
    }
}
