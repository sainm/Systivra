use serde_json::json;
use uuid::Uuid;

use app::{ChunkDraft, EvidenceDraft, NodeDraft};

use crate::office::{open, Node, OfficeError};

const CHUNK_TOKENS: i32 = 800;
const TABLE_ROWS: usize = 20;

#[derive(Debug, Clone)]
struct DocxSection {
    heading_path: Vec<String>,
    level: u8,
    parent: Option<usize>,
    paragraphs: Vec<(usize, String)>,
    tables: Vec<DocxTable>,
}

#[derive(Debug, Clone)]
struct DocxTable {
    index: usize,
    rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct DocxPage {
    pub index: i32,
    pub label: String,
    pub nodes: Vec<NodeDraft>,
    pub chunks: Vec<ChunkDraft>,
    pub evidence: Vec<EvidenceDraft>,
}

#[derive(Debug, Clone, Default)]
pub struct DocxIngest {
    pub pages: Vec<DocxPage>,
    pub paragraph_count: usize,
    pub table_count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum DocxError {
    #[error("DOCX として壊れています: {0}")]
    Broken(String),
    #[error("本文がありません")]
    Empty,
}

impl DocxError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Broken(_) => "E-PARSE-001",
            Self::Empty => "E-DOCX-001",
        }
    }

    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl From<OfficeError> for DocxError {
    fn from(value: OfficeError) -> Self {
        Self::Broken(value.to_string())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn read(
    project_id: Uuid,
    document_id: Uuid,
    revision_id: Uuid,
    object_key: &str,
    bytes: &[u8],
) -> Result<DocxIngest, DocxError> {
    let package = open(bytes).map_err(DocxError::from)?;
    let root = package.xml("word/document.xml").map_err(DocxError::from)?;
    let body = root.descendants("body").next().ok_or(DocxError::Empty)?;
    let sections = collect_sections(body);
    if sections.is_empty() {
        return Err(DocxError::Empty);
    }

    let document_node = Uuid::new_v5(
        &Uuid::from_u128(0x6f9619ff_8b86_d011_b42d_00c04fc964ff),
        format!("document:{revision_id}").as_bytes(),
    );
    let mut ingest = DocxIngest::default();
    let mut chunk_ordinal = 0i32;

    for (section_index, section) in sections.iter().enumerate() {
        let page_index = (section_index + 1) as i32;
        let label = if section.heading_path.is_empty() {
            "(body)".to_string()
        } else {
            section.heading_path.join(" / ")
        };
        let section_node = section_node_id(revision_id, section_index);
        let parent_id = section
            .parent
            .map(|index| section_node_id(revision_id, index))
            .unwrap_or(document_node);

        let mut nodes = Vec::new();
        if section_index == 0 {
            nodes.push(NodeDraft {
                id: document_node,
                revision_id,
                parent_id: None,
                node_type: "DOCUMENT".into(),
                locator: json!({"kind": "docx"}),
                ordinal: 0,
                text: None,
                meta: json!({"filename": object_key, "parser": "docx-native"}),
            });
        }
        nodes.push(NodeDraft {
            id: section_node,
            revision_id,
            parent_id: Some(parent_id),
            node_type: "SECTION".into(),
            locator: json!({"kind": "docx", "section_path": section.heading_path}),
            ordinal: section_index as i32,
            text: section.heading_path.last().cloned(),
            meta: json!({"level": section.level}),
        });
        for (paragraph_index, text) in &section.paragraphs {
            nodes.push(NodeDraft {
                id: Uuid::new_v5(
                    &revision_id,
                    format!("paragraph:{section_index}:{paragraph_index}").as_bytes(),
                ),
                revision_id,
                parent_id: Some(section_node),
                node_type: "PARAGRAPH".into(),
                locator: json!({
                    "kind": "docx",
                    "section_path": section.heading_path,
                    "paragraph": paragraph_index + 1,
                }),
                ordinal: *paragraph_index as i32,
                text: Some(engine_rag::normalize_nfc(text)),
                meta: json!({}),
            });
        }
        for table in &section.tables {
            nodes.push(NodeDraft {
                id: Uuid::new_v5(
                    &revision_id,
                    format!("table:{section_index}:{}", table.index).as_bytes(),
                ),
                revision_id,
                parent_id: Some(section_node),
                node_type: "TABLE".into(),
                locator: json!({
                    "kind": "docx",
                    "section_path": section.heading_path,
                    "table": table.index + 1,
                }),
                ordinal: table.index as i32,
                text: None,
                meta: json!({"rows": table.rows.len()}),
            });
        }

        let mut chunks = Vec::new();
        let mut evidence = Vec::new();
        for (text, locator, chunk_type) in render_section(section) {
            let (draft, item) = drafts_for_chunk(
                project_id,
                document_id,
                revision_id,
                object_key,
                &text,
                locator,
                chunk_type,
                chunk_ordinal,
            );
            chunk_ordinal += 1;
            chunks.push(draft);
            evidence.push(item);
        }

        ingest.paragraph_count += section.paragraphs.len();
        ingest.table_count += section.tables.len();
        ingest.pages.push(DocxPage {
            index: page_index,
            label,
            nodes,
            chunks,
            evidence,
        });
    }

    Ok(ingest)
}

fn collect_sections(body: &Node) -> Vec<DocxSection> {
    let mut sections: Vec<DocxSection> = Vec::new();
    let mut current: Option<usize> = None;
    let mut stack: Vec<(u8, usize)> = Vec::new();
    let mut paragraph_index = 0usize;
    let mut table_index = 0usize;

    for child in &body.children {
        match child.name.as_str() {
            "p" => {
                let text = paragraph_text(child);
                let Some(level) = heading_level(child) else {
                    if !text.is_empty() {
                        let index = current.unwrap_or_else(|| {
                            sections.push(DocxSection {
                                heading_path: Vec::new(),
                                level: 0,
                                parent: None,
                                paragraphs: Vec::new(),
                                tables: Vec::new(),
                            });
                            current = Some(sections.len() - 1);
                            sections.len() - 1
                        });
                        sections[index].paragraphs.push((paragraph_index, text));
                        paragraph_index += 1;
                    }
                    continue;
                };
                let heading = text.trim().to_string();
                if heading.is_empty() {
                    continue;
                }
                while stack.last().is_some_and(|(l, _)| *l >= level) {
                    stack.pop();
                }
                let parent = stack.last().map(|(_, index)| *index);
                let mut path: Vec<String> = stack
                    .iter()
                    .map(|(_, index)| {
                        sections[*index]
                            .heading_path
                            .last()
                            .cloned()
                            .unwrap_or_default()
                    })
                    .collect();
                path.push(heading.clone());
                sections.push(DocxSection {
                    heading_path: path,
                    level,
                    parent,
                    paragraphs: Vec::new(),
                    tables: Vec::new(),
                });
                let index = sections.len() - 1;
                stack.push((level, index));
                current = Some(index);
            }
            "tbl" => {
                let rows = table_rows(child);
                if rows.is_empty() {
                    continue;
                }
                let index = current.unwrap_or_else(|| {
                    sections.push(DocxSection {
                        heading_path: Vec::new(),
                        level: 0,
                        parent: None,
                        paragraphs: Vec::new(),
                        tables: Vec::new(),
                    });
                    current = Some(sections.len() - 1);
                    sections.len() - 1
                });
                sections[index].tables.push(DocxTable {
                    index: table_index,
                    rows,
                });
                table_index += 1;
            }
            _ => {}
        }
    }

    sections
}

fn paragraph_text(paragraph: &Node) -> String {
    let mut out = String::new();
    collect_runs(paragraph, &mut out);
    out.trim().to_string()
}

fn collect_runs(node: &Node, out: &mut String) {
    match node.name.as_str() {
        "t" => {
            out.push_str(&node.text);
            return;
        }
        "tab" => {
            out.push('\t');
            return;
        }
        "br" | "cr" => {
            out.push('\n');
            return;
        }
        "delText" | "instrText" => return,
        _ => {}
    }
    for child in &node.children {
        collect_runs(child, out);
    }
}

fn heading_level(paragraph: &Node) -> Option<u8> {
    let properties = paragraph.child("pPr");
    if let Some(style) = properties
        .and_then(|p| p.child("pStyle"))
        .and_then(|p| p.attr("val"))
    {
        let lower = style.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("heading") {
            if let Ok(level) = rest.trim().parse::<u8>() {
                return (1..=9).contains(&level).then_some(level);
            }
        }
        if let Some(rest) = style.strip_prefix("見出し") {
            if let Ok(level) = rest.trim().parse::<u8>() {
                return (1..=9).contains(&level).then_some(level);
            }
        }
        if matches!(lower.as_str(), "title" | "表題") {
            return Some(1);
        }
    }
    properties
        .and_then(|p| p.child("outlineLvl"))
        .and_then(|p| p.attr("val"))
        .and_then(|value| value.parse::<u8>().ok())
        .map(|value| value.saturating_add(1).min(9))
}

fn table_rows(table: &Node) -> Vec<Vec<String>> {
    table
        .descendants("tr")
        .map(|row| {
            row.children
                .iter()
                .filter(|cell| cell.name == "tc")
                .map(|cell| {
                    cell.descendants("p")
                        .map(paragraph_text)
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()
        })
        .filter(|row: &Vec<String>| !row.is_empty())
        .collect()
}

fn render_section(section: &DocxSection) -> Vec<(String, serde_json::Value, String)> {
    let mut out = Vec::new();
    let prefix = if section.heading_path.is_empty() {
        "【本文】".to_string()
    } else {
        format!("【{}】", section.heading_path.join(" / "))
    };

    let mut current = String::new();
    let mut first_paragraph = None;
    for (paragraph_index, text) in &section.paragraphs {
        let candidate = if current.is_empty() {
            format!("{prefix}\n{text}")
        } else {
            format!("{current}\n\n{text}")
        };
        if !current.is_empty() && engine_rag::tokenize(&candidate).len() as i32 > CHUNK_TOKENS {
            out.push((
                current,
                json!({
                    "kind": "docx",
                    "section_path": section.heading_path,
                    "paragraph": first_paragraph.unwrap_or(0) + 1,
                }),
                "TEXT".to_string(),
            ));
            current = format!("{prefix}\n{text}");
            first_paragraph = Some(*paragraph_index);
        } else {
            if current.is_empty() {
                first_paragraph = Some(*paragraph_index);
            }
            current = candidate;
        }
    }
    if !current.is_empty() {
        out.push((
            current,
            json!({
                "kind": "docx",
                "section_path": section.heading_path,
                "paragraph": first_paragraph.unwrap_or(0) + 1,
            }),
            "TEXT".to_string(),
        ));
    }

    for table in &section.tables {
        for (group, rows) in table.rows.chunks(TABLE_ROWS).enumerate() {
            let start = group * TABLE_ROWS;
            let mut lines = vec![format!(
                "{} / 表{}】",
                prefix.trim_end_matches('】'),
                table.index + 1
            )];
            if start > 0 {
                if let Some(header) = table.rows.first() {
                    lines.push(header.join(" | "));
                }
            }
            lines.extend(rows.iter().map(|row| row.join(" | ")));
            out.push((
                lines.join("\n"),
                json!({
                    "kind": "docx",
                    "section_path": section.heading_path,
                    "table": table.index + 1,
                    "row_start": start + 1,
                    "row_end": start + rows.len(),
                }),
                "TABLE".to_string(),
            ));
        }
    }

    out
}

fn section_node_id(revision_id: Uuid, section_index: usize) -> Uuid {
    Uuid::new_v5(&revision_id, format!("section:{section_index}").as_bytes())
}

#[allow(clippy::too_many_arguments)]
fn drafts_for_chunk(
    project_id: Uuid,
    document_id: Uuid,
    revision_id: Uuid,
    object_key: &str,
    text: &str,
    locator: serde_json::Value,
    chunk_type: String,
    ordinal: i32,
) -> (ChunkDraft, EvidenceDraft) {
    let normalized = engine_rag::normalize_nfc(text);
    let hash = engine_rag::sha256(normalized.as_bytes());
    let chunk_id = Uuid::new_v5(
        &project_id,
        format!("{revision_id}:docx:{ordinal}:{}", hex_prefix(&hash, 8)).as_bytes(),
    );
    let section = locator
        .get("section_path")
        .and_then(|v| v.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_default();
    let draft = ChunkDraft {
        id: chunk_id,
        project_id,
        document_id,
        revision_id,
        chunk_type,
        ordinal,
        text: text.to_string(),
        tokenized: engine_rag::tokenized_text(text),
        token_count: engine_rag::tokenize(text).len() as i32,
        char_count: text.chars().count() as i32,
        content_hash: hash,
        locator: locator.clone(),
        node_path: vec![
            "DOCUMENT".into(),
            format!("SECTION:{section}"),
            format!("PARAGRAPH:{ordinal}"),
        ],
        metadata: json!({
            "section": section,
            "parser": "docx-native",
            "parser_version": env!("CARGO_PKG_VERSION"),
            "tokenizer": "lindera-ipadic",
            "tokenizer_version": engine_rag::TOKENIZER_VERSION,
            "filename": object_key,
        }),
        embedding: engine_rag::embed(text),
    };
    let evidence = EvidenceDraft {
        id: Uuid::new_v5(&project_id, format!("evidence:{chunk_id}").as_bytes()),
        project_id,
        source_type: "DOCX_SECTION_TEXT".into(),
        document_id: Some(document_id),
        revision_id: Some(revision_id),
        chunk_id: Some(chunk_id),
        locator,
        quote: normalized,
        quote_hash: engine_rag::quote_hash(text),
        resolution: "RESOLVED".into(),
        confidence: 1.0,
    };
    (draft, evidence)
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

    fn fixture() -> Vec<u8> {
        let document = r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>3. 画面設計</w:t></w:r></w:p><w:p><w:r><w:t>住所は全角20文字以内とする。</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>3.2 入力チェック</w:t></w:r></w:p><w:p><w:r><w:t>必須項目を検証する。</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>項目</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>制約</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>住所</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>全角20文字</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
        let parts: Vec<(&str, Vec<u8>)> = vec![
            (
                "[Content_Types].xml",
                br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_vec(),
            ),
            ("word/document.xml", document.as_bytes().to_vec()),
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

    #[test]
    fn extracts_sections_and_tables() {
        let bytes = fixture();
        let ingest = read(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            "a.docx",
            &bytes,
        )
        .unwrap();
        assert_eq!(
            ingest.pages.len(),
            2,
            "{:?}",
            ingest.pages.iter().map(|p| &p.label).collect::<Vec<_>>()
        );
        assert_eq!(ingest.pages[0].label, "3. 画面設計");
        assert_eq!(ingest.pages[1].label, "3. 画面設計 / 3.2 入力チェック");
        assert!(ingest.pages[0].chunks[0].text.contains("【3. 画面設計】"));
        let table_chunk = ingest.pages[1]
            .chunks
            .iter()
            .find(|c| c.chunk_type == "TABLE")
            .expect("table chunk");
        assert!(
            table_chunk.text.contains("住所 | 全角20文字"),
            "{}",
            table_chunk.text
        );
        assert_eq!(table_chunk.locator["kind"], "docx");
        assert_eq!(table_chunk.locator["table"], 1);
        assert!(ingest.pages[0]
            .nodes
            .iter()
            .any(|n| n.node_type == "DOCUMENT"));
        assert!(ingest.pages[0]
            .nodes
            .iter()
            .any(|n| n.node_type == "SECTION"));
        assert!(ingest.pages[0]
            .nodes
            .iter()
            .any(|n| n.node_type == "PARAGRAPH"));
        assert_eq!(ingest.paragraph_count, 2);
        assert_eq!(ingest.table_count, 1);
    }

    #[test]
    fn empty_document_is_rejected() {
        let document = r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#;
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            zw.start_file("word/document.xml", SimpleFileOptions::default())
                .unwrap();
            zw.write_all(document.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let bytes = buf.into_inner();
        let err = read(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            "a.docx",
            &bytes,
        )
        .unwrap_err();
        assert!(matches!(err, DocxError::Empty));
    }
}
