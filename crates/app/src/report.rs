use std::fs::File;
use std::path::PathBuf;

use printpdf::{Mm, PdfDocument};

use crate::AppError;

const PAGE_WIDTH_MM: f32 = 210.0;
const PAGE_HEIGHT_MM: f32 = 297.0;
const MARGIN_MM: f32 = 14.0;
const FONT_SIZE: f32 = 8.5;
const LINE_HEIGHT_MM: f32 = 4.8;
const LINES_PER_PAGE: usize = 52;

pub fn render_text_pdf(title: &str, text: &str) -> Result<Vec<u8>, AppError> {
    let font_path = pdf_font_path().ok_or_else(|| {
        AppError::CapabilityUnsupported(
            "PDF 日本語フォントがありません。SYSTIVRA_PDF_FONT を設定してください".into(),
        )
    })?;
    let mut font = File::open(&font_path).map_err(|error| {
        AppError::Validation(format!(
            "PDF フォントを開けません {}: {error}",
            font_path.display()
        ))
    })?;
    let lines = wrap_report_lines(text, 82);
    let (doc, first_page, first_layer) =
        PdfDocument::new(title, Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Report");
    let font = doc
        .add_external_font_with_subsetting(&mut font, true)
        .map_err(|error| AppError::Validation(format!("PDF フォント読込失敗: {error}")))?;
    for (page_index, page_lines) in lines.chunks(LINES_PER_PAGE).enumerate() {
        let (page, layer) = if page_index == 0 {
            (first_page, first_layer)
        } else {
            doc.add_page(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Report")
        };
        let layer = doc.get_page(page).get_layer(layer);
        for (line_index, line) in page_lines.iter().enumerate() {
            let y = PAGE_HEIGHT_MM - MARGIN_MM - line_index as f32 * LINE_HEIGHT_MM;
            layer.use_text(line, FONT_SIZE, Mm(MARGIN_MM), Mm(y), &font);
        }
    }
    doc.save_to_bytes()
        .map_err(|error| AppError::Validation(format!("PDF 生成失敗: {error}")))
}

fn pdf_font_path() -> Option<PathBuf> {
    let configured = std::env::var_os("SYSTIVRA_PDF_FONT").map(PathBuf::from);
    configure_or_default(configured)
}

fn configure_or_default(configured: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = configured.filter(|path| path.is_file()) {
        return Some(path);
    }
    [
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

fn wrap_report_lines(text: &str, max_width: usize) -> Vec<String> {
    let mut output = Vec::new();
    for raw in text.lines() {
        let line = raw
            .trim_end()
            .trim_start_matches('#')
            .trim()
            .replace("|---", "")
            .replace('|', " ");
        if line.is_empty() {
            output.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut width = 0usize;
        for ch in line.chars() {
            let char_width = if ch.is_ascii() { 1 } else { 2 };
            if width + char_width > max_width && !current.is_empty() {
                output.push(current);
                current = String::new();
                width = 0;
            }
            current.push(ch);
            width += char_width;
        }
        if !current.is_empty() {
            output.push(current);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_wide_japanese_characters() {
        let lines = wrap_report_lines("# 見出し\n\n日本語テキスト", 8);
        assert_eq!(lines[0], "見出し");
        assert!(lines.iter().any(|line| line == "日本語テ"));
    }

    #[test]
    fn generated_pdf_contains_extractable_japanese_text() {
        if pdf_font_path().is_none() {
            return;
        }
        let pdf = render_text_pdf(
            "変更影響レポート",
            "# 影響レポート\n\n影響するコード\nUserEditAction.userService",
        )
        .unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        let text = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        assert!(text.contains("影響するコード"), "{text}");
        assert!(text.contains("UserEditAction.userService"), "{text}");
    }
}
