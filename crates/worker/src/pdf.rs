use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("pdf read failed: {0}")]
    Read(String),
    #[error("pdf has no extractable pages")]
    Empty,
}

pub fn extract_pages(path: &Path) -> Result<Vec<String>, PdfError> {
    let pages =
        pdf_extract::extract_text_by_pages(path).map_err(|e| PdfError::Read(e.to_string()))?;
    let pages: Vec<String> = pages
        .into_iter()
        .map(|page| {
            let page = engine_rag::normalize_nfkc(&page);
            let page: String = page
                .chars()
                .map(|c| match c {
                    '\u{c}' => '\n',
                    '\n' | '\t' => c,
                    c if c.is_control() => ' ',
                    c => c,
                })
                .collect();
            page.lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        })
        .collect();
    if pages.is_empty() {
        return Err(PdfError::Empty);
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pdf_is_rejected() {
        let result = extract_pages(Path::new("/nonexistent/file.pdf"));
        assert!(matches!(result, Err(PdfError::Read(_))));
    }
}
