use serde_json::json;
use uuid::Uuid;

use app::{ChunkDraft, EvidenceDraft, NodeDraft};

use crate::office::{open, resolve_target, Node, OfficeError};

const CHUNK_TOKENS: i32 = 800;
const TABLE_ROWS: usize = 20;

#[derive(Debug, Clone)]
pub struct PptxPage {
    pub index: i32,
    pub label: String,
    pub nodes: Vec<NodeDraft>,
    pub chunks: Vec<ChunkDraft>,
    pub evidence: Vec<EvidenceDraft>,
}

#[derive(Debug, Clone, Default)]
pub struct PptxIngest {
    pub pages: Vec<PptxPage>,
    pub slide_count: usize,
    pub table_count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum PptxError {
    #[error("PPTX として壊れています: {0}")]
    Broken(String),
    #[error("スライドがありません")]
    Empty,
}

impl PptxError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Broken(_) => "E-PARSE-001",
            Self::Empty => "E-PPTX-001",
        }
    }

    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl From<OfficeError> for PptxError {
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
) -> Result<PptxIngest, PptxError> {
    let package = open(bytes).map_err(PptxError::from)?;
    let presentation = package
        .xml("ppt/presentation.xml")
        .map_err(PptxError::from)?;
    let rels = package
        .rels_for("ppt/presentation.xml")
        .map_err(PptxError::from)?;

    let slides: Vec<String> = presentation
        .descendants("sldId")
        .filter_map(|node| {
            node.attrs
                .iter()
                .filter(|(key, _)| key == "id")
                .map(|(_, value)| value)
                .find(|value| value.starts_with("rId"))
                .and_then(|rid| rels.get(rid))
                .map(|target| resolve_target("ppt/presentation.xml", target))
        })
        .collect();
    if slides.is_empty() {
        return Err(PptxError::Empty);
    }

    let document_node = Uuid::new_v5(
        &Uuid::from_u128(0x6f9619ff_8b86_d011_b42d_00c04fc964ff),
        format!("document:{revision_id}").as_bytes(),
    );
    let mut ingest = PptxIngest::default();
    let mut chunk_ordinal = 0i32;

    for (slide_index, part) in slides.iter().enumerate() {
        let slide_number = (slide_index + 1) as i32;
        let root = package.xml(part).map_err(PptxError::from)?;
        let title = slide_title(&root).unwrap_or_else(|| format!("スライド {slide_number}"));
        let slide_node = slide_node_id(revision_id, slide_number);
        let mut nodes = Vec::new();
        if slide_index == 0 {
            nodes.push(NodeDraft {
                id: document_node,
                revision_id,
                parent_id: None,
                node_type: "DOCUMENT".into(),
                locator: json!({"kind": "pptx"}),
                ordinal: 0,
                text: None,
                meta: json!({"filename": object_key, "parser": "pptx-native"}),
            });
        }
        nodes.push(NodeDraft {
            id: slide_node,
            revision_id,
            parent_id: Some(document_node),
            node_type: "SECTION".into(),
            locator: json!({"kind": "pptx", "slide": slide_number, "title": title}),
            ordinal: slide_number,
            text: Some(title.clone()),
            meta: json!({}),
        });

        let mut content: Vec<&Node> = Vec::new();
        if let Some(tree) = root.descendants("spTree").next() {
            collect_content(tree, &mut content);
        }

        let mut text_shapes: Vec<(String, String)> = Vec::new();
        let mut tables: Vec<(usize, Vec<Vec<String>>)> = Vec::new();
        let mut figures = 0usize;

        for node in content {
            match node.name.as_str() {
                "sp" => {
                    let text = shape_text(node);
                    if text.is_empty() {
                        continue;
                    }
                    let id =
                        shape_id(node).unwrap_or_else(|| format!("shape-{}", text_shapes.len()));
                    nodes.push(NodeDraft {
                        id: Uuid::new_v5(
                            &revision_id,
                            format!("shape:{slide_number}:{id}").as_bytes(),
                        ),
                        revision_id,
                        parent_id: Some(slide_node),
                        node_type: "PARAGRAPH".into(),
                        locator: json!({
                            "kind": "pptx",
                            "slide": slide_number,
                            "shape_id": id,
                            "name": shape_name(node),
                        }),
                        ordinal: text_shapes.len() as i32,
                        text: Some(engine_rag::normalize_nfc(&text)),
                        meta: json!({}),
                    });
                    if !is_title_shape(node) {
                        text_shapes.push((id, text));
                    }
                }
                "graphicFrame" => {
                    let Some(table) = node.descendants("tbl").next() else {
                        continue;
                    };
                    let rows = table_rows(table);
                    if rows.is_empty() {
                        continue;
                    }
                    let table_index = tables.len();
                    nodes.push(NodeDraft {
                        id: Uuid::new_v5(
                            &revision_id,
                            format!("table:{slide_number}:{table_index}").as_bytes(),
                        ),
                        revision_id,
                        parent_id: Some(slide_node),
                        node_type: "TABLE".into(),
                        locator: json!({
                            "kind": "pptx",
                            "slide": slide_number,
                            "table": table_index + 1,
                        }),
                        ordinal: table_index as i32,
                        text: None,
                        meta: json!({"rows": rows.len()}),
                    });
                    tables.push((table_index, rows));
                }
                "pic" => {
                    nodes.push(NodeDraft {
                        id: Uuid::new_v5(
                            &revision_id,
                            format!("figure:{slide_number}:{figures}").as_bytes(),
                        ),
                        revision_id,
                        parent_id: Some(slide_node),
                        node_type: "FIGURE".into(),
                        locator: json!({
                            "kind": "pptx",
                            "slide": slide_number,
                            "name": shape_name(node),
                        }),
                        ordinal: figures as i32,
                        text: None,
                        meta: json!({}),
                    });
                    figures += 1;
                }
                _ => {}
            }
        }

        let mut chunks = Vec::new();
        let mut evidence = Vec::new();
        for (text, ids) in render_text_shapes(&title, slide_number, &text_shapes) {
            let (draft, item) = drafts_for_chunk(
                project_id,
                document_id,
                revision_id,
                object_key,
                &text,
                json!({
                    "kind": "pptx",
                    "slide": slide_number,
                    "title": title,
                    "shape_ids": ids,
                }),
                "TEXT",
                chunk_ordinal,
            );
            chunk_ordinal += 1;
            chunks.push(draft);
            evidence.push(item);
        }
        for (table_index, rows) in &tables {
            for (group, chunk_rows) in rows.chunks(TABLE_ROWS).enumerate() {
                let start = group * TABLE_ROWS;
                let mut lines = vec![format!(
                    "【{title} / スライド {slide_number} / 表{}】",
                    table_index + 1
                )];
                if start > 0 {
                    if let Some(header) = rows.first() {
                        lines.push(header.join(" | "));
                    }
                }
                lines.extend(chunk_rows.iter().map(|row| row.join(" | ")));
                let (draft, item) = drafts_for_chunk(
                    project_id,
                    document_id,
                    revision_id,
                    object_key,
                    &lines.join("\n"),
                    json!({
                        "kind": "pptx",
                        "slide": slide_number,
                        "table": table_index + 1,
                        "row_start": start + 1,
                        "row_end": start + chunk_rows.len(),
                    }),
                    "TABLE",
                    chunk_ordinal,
                );
                chunk_ordinal += 1;
                chunks.push(draft);
                evidence.push(item);
            }
        }

        ingest.slide_count += 1;
        ingest.table_count += tables.len();
        ingest.pages.push(PptxPage {
            index: slide_number,
            label: title,
            nodes,
            chunks,
            evidence,
        });
    }

    Ok(ingest)
}

fn collect_content<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
    for child in &node.children {
        match child.name.as_str() {
            "sp" | "graphicFrame" | "pic" => out.push(child),
            _ => collect_content(child, out),
        }
    }
}

fn slide_title(root: &Node) -> Option<String> {
    for shape in root.descendants("sp") {
        let placeholder = shape
            .descendants("ph")
            .next()
            .and_then(|ph| ph.attr("type"));
        if matches!(placeholder, Some("title" | "ctrTitle")) {
            let text = shape_text(shape);
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    root.descendants("sp")
        .map(shape_text)
        .find(|text| !text.is_empty())
}

fn shape_text(shape: &Node) -> String {
    let mut lines = Vec::new();
    for paragraph in shape.descendants("p") {
        let text = paragraph
            .descendants("t")
            .map(|t| t.text.as_str())
            .collect::<String>();
        let text = text.trim();
        if !text.is_empty() {
            lines.push(text.to_string());
        }
    }
    lines.join("\n")
}

fn is_title_shape(shape: &Node) -> bool {
    shape
        .descendants("ph")
        .next()
        .and_then(|ph| ph.attr("type"))
        .is_some_and(|kind| matches!(kind, "title" | "ctrTitle"))
}

fn shape_id(shape: &Node) -> Option<String> {
    shape
        .descendants("cNvPr")
        .next()
        .and_then(|props| props.attr("id"))
        .map(str::to_string)
}

fn shape_name(shape: &Node) -> Option<String> {
    shape
        .descendants("cNvPr")
        .next()
        .and_then(|props| props.attr("name"))
        .map(str::to_string)
}

fn table_rows(table: &Node) -> Vec<Vec<String>> {
    table
        .descendants("tr")
        .map(|row| {
            row.children
                .iter()
                .filter(|cell| cell.name == "tc")
                .map(|cell| {
                    cell.descendants("t")
                        .map(|t| t.text.trim())
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()
        })
        .filter(|row: &Vec<String>| !row.is_empty())
        .collect()
}

fn render_text_shapes(
    title: &str,
    slide: i32,
    shapes: &[(String, String)],
) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut ids: Vec<String> = Vec::new();
    let prefix = format!("【{title} / スライド {slide}】");

    for (id, text) in shapes {
        let candidate = if current.is_empty() {
            format!("{prefix}\n{text}")
        } else {
            format!("{current}\n\n{text}")
        };
        if !current.is_empty() && engine_rag::tokenize(&candidate).len() as i32 > CHUNK_TOKENS {
            out.push((current, ids.clone()));
            current = format!("{prefix}\n{text}");
            ids = vec![id.clone()];
        } else {
            current = candidate;
            ids.push(id.clone());
        }
    }
    if !current.is_empty() {
        out.push((current, ids));
    }
    out
}

fn slide_node_id(revision_id: Uuid, slide: i32) -> Uuid {
    Uuid::new_v5(&revision_id, format!("slide:{slide}").as_bytes())
}

#[allow(clippy::too_many_arguments)]
fn drafts_for_chunk(
    project_id: Uuid,
    document_id: Uuid,
    revision_id: Uuid,
    object_key: &str,
    text: &str,
    locator: serde_json::Value,
    chunk_type: &str,
    ordinal: i32,
) -> (ChunkDraft, EvidenceDraft) {
    let normalized = engine_rag::normalize_nfc(text);
    let hash = engine_rag::sha256(normalized.as_bytes());
    let chunk_id = Uuid::new_v5(
        &project_id,
        format!("{revision_id}:pptx:{ordinal}:{}", hex_prefix(&hash, 8)).as_bytes(),
    );
    let slide = locator.get("slide").and_then(|v| v.as_i64()).unwrap_or(0);
    let draft = ChunkDraft {
        id: chunk_id,
        project_id,
        document_id,
        revision_id,
        chunk_type: chunk_type.to_string(),
        ordinal,
        text: text.to_string(),
        tokenized: engine_rag::tokenized_text(text),
        token_count: engine_rag::tokenize(text).len() as i32,
        char_count: text.chars().count() as i32,
        content_hash: hash,
        locator: locator.clone(),
        node_path: vec![
            "DOCUMENT".into(),
            format!("SLIDE:{slide}"),
            format!("CHUNK:{ordinal}"),
        ],
        metadata: json!({
            "slide": slide,
            "parser": "pptx-native",
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
        source_type: "PPTX_SLIDE_TEXT".into(),
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
        let presentation = r#"<?xml version="1.0"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst></p:presentation>"#;
        let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="slide" Target="slides/slide1.xml"/></Relationships>"#;
        let slide = r#"<?xml version="1.0"?><p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>画面項目定義</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Body 1"/><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>住所は全角20文字以内。</a:t></a:r></a:p><a:p><a:r><a:t>必須項目とする。</a:t></a:r></a:p></p:txBody></p:sp><p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tr><a:tc><a:t>項目</a:t></a:tc><a:tc><a:t>制約</a:t></a:tc></a:tr><a:tr><a:tc><a:t>住所</a:t></a:tc><a:tc><a:t>全角20文字</a:t></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#;
        let parts: Vec<(&str, Vec<u8>)> = vec![
            (
                "[Content_Types].xml",
                br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_vec(),
            ),
            ("ppt/presentation.xml", presentation.as_bytes().to_vec()),
            ("ppt/_rels/presentation.xml.rels", rels.as_bytes().to_vec()),
            ("ppt/slides/slide1.xml", slide.as_bytes().to_vec()),
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
    fn extracts_slides_text_and_tables() {
        let bytes = fixture();
        let ingest = read(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            "a.pptx",
            &bytes,
        )
        .unwrap();
        assert_eq!(ingest.pages.len(), 1);
        let page = &ingest.pages[0];
        assert_eq!(page.label, "画面項目定義");
        assert_eq!(
            page.chunks.len(),
            2,
            "{:?}",
            page.chunks.iter().map(|c| &c.text).collect::<Vec<_>>()
        );
        assert!(page.chunks[0]
            .text
            .contains("【画面項目定義 / スライド 1】"));
        assert!(page.chunks[0].text.contains("住所は全角20文字以内。"));
        assert_eq!(page.chunks[0].locator["kind"], "pptx");
        assert_eq!(page.chunks[0].locator["slide"], 1);
        assert_eq!(page.chunks[0].locator["shape_ids"][0], "3");
        let table = page
            .chunks
            .iter()
            .find(|c| c.chunk_type == "TABLE")
            .unwrap();
        assert!(table.text.contains("住所 | 全角20文字"), "{}", table.text);
        assert_eq!(table.locator["table"], 1);
        assert!(page.nodes.iter().any(|n| n.node_type == "SECTION"));
        assert!(page.nodes.iter().any(|n| n.node_type == "PARAGRAPH"));
        assert!(page.nodes.iter().any(|n| n.node_type == "TABLE"));
        assert_eq!(ingest.table_count, 1);
    }

    #[test]
    fn empty_presentation_is_rejected() {
        let presentation = r#"<?xml version="1.0"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst/></p:presentation>"#;
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            zw.start_file("ppt/presentation.xml", SimpleFileOptions::default())
                .unwrap();
            zw.write_all(presentation.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let bytes = buf.into_inner();
        let err = read(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            "a.pptx",
            &bytes,
        )
        .unwrap_err();
        assert!(matches!(err, PptxError::Empty));
    }
}
