use std::collections::{BTreeMap, HashMap};

use chrono::{Duration, NaiveDate};

use crate::office::Node;

use super::a1::{self, CellRef};
use super::ooxml::{Sheet, Workbook, XlsxError};

#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Empty,
    String(String),
    Number(f64),
    Bool(bool),
    Date(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Formula {
    pub text: String,
    pub cached: Option<CellValue>,
    pub shared_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub row: u32,
    pub col: u32,
    pub value: CellValue,
    pub formula: Option<Formula>,
    pub style_id: u32,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedName {
    pub name: String,
    pub sheet_index: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StyleInfo {
    pub bold: bool,
    pub filled: bool,
    pub is_date: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Styles {
    by_id: Vec<StyleInfo>,
}

impl Styles {
    pub fn get(&self, style_id: u32) -> StyleInfo {
        self.by_id
            .get(style_id as usize)
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct SheetCells {
    cells: BTreeMap<(u32, u32), Cell>,
    warnings: Vec<String>,
}

impl SheetCells {
    pub fn get(&self, row: u32, col: u32) -> Option<&Cell> {
        self.cells.get(&(row, col))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Cell> {
        self.cells.values()
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn bounds(&self) -> Option<(u32, u32, u32, u32)> {
        let (first, _) = self.cells.first_key_value()?;
        let (last, _) = self.cells.last_key_value()?;
        Some((first.0, first.1, last.0, last.1))
    }

    #[cfg(test)]
    pub(crate) fn from_cells(cells: impl IntoIterator<Item = Cell>) -> Self {
        let mut out = Self::default();
        for cell in cells {
            out.cells.insert((cell.row, cell.col), cell);
        }
        out
    }

    pub(crate) fn insert(&mut self, cell: Cell) {
        self.cells.insert((cell.row, cell.col), cell);
    }

    pub(crate) fn push_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

impl Workbook {
    pub fn defined_names(&self) -> Result<Vec<DefinedName>, XlsxError> {
        let wb = self.xml("xl/workbook.xml")?;
        let Some(container) = wb.descendants("definedNames").next() else {
            return Ok(Vec::new());
        };
        Ok(container
            .children
            .iter()
            .filter(|n| n.name == "definedName")
            .filter_map(|n| {
                let name = n.attr("name")?;
                Some(DefinedName {
                    name: name.to_string(),
                    sheet_index: n.attr("localSheetId").and_then(|v| v.parse().ok()),
                    text: n.text.trim().to_string(),
                })
            })
            .collect())
    }

    pub fn date1904(&self) -> Result<bool, XlsxError> {
        let wb = self.xml("xl/workbook.xml")?;
        let date1904 = wb
            .descendants("workbookPr")
            .next()
            .and_then(|n| n.attr("date1904"))
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        Ok(date1904)
    }

    pub fn styles(&self) -> Result<Styles, XlsxError> {
        let part = self
            .rels_for("xl/workbook.xml")?
            .into_values()
            .find(|t| t.ends_with("styles.xml"))
            .map(|t| super::ooxml::resolve_target("xl/workbook.xml", &t))
            .unwrap_or_else(|| "xl/styles.xml".to_string());
        if !self.has(&part) {
            return Ok(Styles::default());
        }
        let root = self.xml(&part)?;

        let mut custom_formats = HashMap::new();
        if let Some(num_fmts) = root.descendants("numFmts").next() {
            for nf in num_fmts.children.iter().filter(|n| n.name == "numFmt") {
                let (Some(id), Some(code)) = (nf.attr("numFmtId"), nf.attr("formatCode")) else {
                    continue;
                };
                if let Ok(id) = id.parse::<u32>() {
                    custom_formats.insert(id, code.to_string());
                }
            }
        }

        let fonts: Vec<bool> = root
            .descendants("fonts")
            .next()
            .map(|n| {
                n.children
                    .iter()
                    .filter(|c| c.name == "font")
                    .map(font_is_bold)
                    .collect()
            })
            .unwrap_or_default();
        let fills: Vec<bool> = root
            .descendants("fills")
            .next()
            .map(|n| {
                n.children
                    .iter()
                    .filter(|c| c.name == "fill")
                    .map(fill_is_visible)
                    .collect()
            })
            .unwrap_or_default();

        let mut by_id = Vec::new();
        if let Some(xfs) = root.descendants("cellXfs").next() {
            for xf in xfs.children.iter().filter(|n| n.name == "xf") {
                let num_fmt_id = xf
                    .attr("numFmtId")
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(0);
                let font_id = xf
                    .attr("fontId")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let fill_id = xf
                    .attr("fillId")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                by_id.push(StyleInfo {
                    bold: fonts.get(font_id).copied().unwrap_or(false),
                    filled: fills.get(fill_id).copied().unwrap_or(false),
                    is_date: custom_formats
                        .get(&num_fmt_id)
                        .map(|code| format_is_date(code))
                        .unwrap_or_else(|| builtin_format_is_date(num_fmt_id)),
                });
            }
        }
        Ok(Styles { by_id })
    }

    pub fn sheet_cells(&self, sheet: &Sheet) -> Result<SheetCells, XlsxError> {
        let root = self.xml(&sheet.part)?;
        let styles = self.styles()?;
        let date1904 = self.date1904()?;
        let shared = self.shared_strings()?;
        let comments = self.comments(sheet)?;
        let mut out = SheetCells::default();

        let mut shared_formulas: HashMap<u32, String> = HashMap::new();
        let mut pending_shared: Vec<((u32, u32), u32)> = Vec::new();

        if let Some(data) = root.descendants("sheetData").next() {
            let mut next_row = 0u32;
            for row in data.children.iter().filter(|n| n.name == "row") {
                let row_index = row
                    .attr("r")
                    .and_then(|v| v.parse::<u32>().ok())
                    .map(|v| v.saturating_sub(1))
                    .unwrap_or(next_row);
                next_row = row_index + 1;

                let mut next_col = 0u32;
                for node in row.children.iter().filter(|n| n.name == "c") {
                    let cell_ref = node.attr("r").and_then(a1::parse_cell).unwrap_or_else(|| {
                        let r = CellRef::new(row_index, next_col);
                        next_col += 1;
                        r
                    });
                    next_col = cell_ref.col + 1;

                    let style_id = node
                        .attr("s")
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(0);
                    let style = styles.get(style_id);
                    let kind = node.attr("t").unwrap_or("n");
                    let raw = node.child("v").map(|v| v.text.as_str());
                    let comment = comments.get(&(cell_ref.row, cell_ref.col)).cloned();

                    let mut formula = None;
                    if let Some(f) = node.child("f") {
                        let text = f.text.trim().to_string();
                        let shared_index = f.attr("si").and_then(|v| v.parse::<u32>().ok());
                        if let Some(index) = shared_index {
                            if !text.is_empty() {
                                shared_formulas.insert(index, text.clone());
                            } else {
                                pending_shared.push(((cell_ref.row, cell_ref.col), index));
                            }
                        }
                        formula = Some(Formula {
                            text,
                            cached: None,
                            shared_index,
                        });
                    }

                    let value = match kind {
                        "s" => raw
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .and_then(|index| shared.get(index).cloned())
                            .map(CellValue::String)
                            .unwrap_or(CellValue::Empty),
                        "inlineStr" => node
                            .child("is")
                            .map(|is| CellValue::String(collect_text(is)))
                            .unwrap_or(CellValue::Empty),
                        "str" => raw
                            .map(|v| CellValue::String(v.to_string()))
                            .unwrap_or(CellValue::Empty),
                        "b" => raw
                            .map(|v| {
                                CellValue::Bool(v.trim() == "1" || v.eq_ignore_ascii_case("true"))
                            })
                            .unwrap_or(CellValue::Empty),
                        "e" => raw
                            .map(|v| CellValue::Error(v.trim().to_string()))
                            .unwrap_or(CellValue::Empty),
                        "d" => raw
                            .map(|v| CellValue::Date(v.trim().to_string()))
                            .unwrap_or(CellValue::Empty),
                        _ => raw
                            .and_then(|v| v.trim().parse::<f64>().ok())
                            .map(|number| {
                                if style.is_date {
                                    excel_serial_date(number, date1904)
                                        .map(CellValue::Date)
                                        .unwrap_or(CellValue::Number(number))
                                } else {
                                    CellValue::Number(number)
                                }
                            })
                            .unwrap_or(CellValue::Empty),
                    };

                    if matches!(value, CellValue::Empty) && formula.is_none() && comment.is_none() {
                        continue;
                    }
                    if formula.is_some() && matches!(value, CellValue::Empty) {
                        out.push_warning(format!(
                            "{}!{}: NO_CACHED_VALUE",
                            sheet.name,
                            cell_ref.to_a1()
                        ));
                    }
                    if let Some(f) = &mut formula {
                        f.cached = if matches!(value, CellValue::Empty) {
                            None
                        } else {
                            Some(value.clone())
                        };
                    }

                    out.insert(Cell {
                        row: cell_ref.row,
                        col: cell_ref.col,
                        value,
                        formula,
                        style_id,
                        comment,
                    });
                }
            }
        }

        if !pending_shared.is_empty() {
            for (key, shared_index) in pending_shared {
                if let Some(cell) = out.cells.get_mut(&key) {
                    if let Some(formula) = &mut cell.formula {
                        if let Some(master) = shared_formulas.get(&shared_index) {
                            formula.text = master.clone();
                        }
                    }
                }
            }
        }

        for ((row, col), text) in comments {
            if out.get(row, col).is_none() {
                out.insert(Cell {
                    row,
                    col,
                    value: CellValue::Empty,
                    formula: None,
                    style_id: 0,
                    comment: Some(text),
                });
            }
        }

        Ok(out)
    }

    fn shared_strings(&self) -> Result<Vec<String>, XlsxError> {
        let part = self
            .rels_for("xl/workbook.xml")?
            .into_values()
            .find(|t| t.ends_with("sharedStrings.xml"))
            .map(|t| super::ooxml::resolve_target("xl/workbook.xml", &t))
            .unwrap_or_else(|| "xl/sharedStrings.xml".to_string());
        if !self.has(&part) {
            return Ok(Vec::new());
        }
        let root = self.xml(&part)?;
        let strings: Vec<String> = root
            .descendants("sst")
            .next()
            .map(|sst| {
                sst.children
                    .iter()
                    .filter(|n| n.name == "si")
                    .map(collect_text)
                    .collect()
            })
            .unwrap_or_default();
        Ok(strings)
    }

    fn comments(&self, sheet: &Sheet) -> Result<HashMap<(u32, u32), String>, XlsxError> {
        let part = self
            .rels_for(&sheet.part)?
            .into_values()
            .find(|t| t.ends_with(".xml") && t.contains("comments"))
            .map(|t| {
                if t.starts_with('/') {
                    t.trim_start_matches('/').to_string()
                } else {
                    super::ooxml::resolve_target(&sheet.part, &t)
                }
            });
        let Some(part) = part else {
            return Ok(HashMap::new());
        };
        if !self.has(&part) {
            return Ok(HashMap::new());
        }
        let root = self.xml(&part)?;
        let mut out = HashMap::new();
        for comment in root.descendants("comment") {
            let Some(reference) = comment.attr("ref").and_then(a1::parse_cell) else {
                continue;
            };
            let text = comment.child("text").map(collect_text).unwrap_or_default();
            out.insert((reference.row, reference.col), text);
        }
        Ok(out)
    }
}

fn collect_text(node: &Node) -> String {
    let mut out = String::new();
    collect_text_into(node, &mut out);
    out
}

fn collect_text_into(node: &Node, out: &mut String) {
    if node.name == "rPh" || node.name == "phoneticPr" {
        return;
    }
    if node.name == "t" {
        out.push_str(&node.text);
        return;
    }
    for child in &node.children {
        collect_text_into(child, out);
    }
}

fn font_is_bold(font: &Node) -> bool {
    font.children
        .iter()
        .find(|n| n.name == "b")
        .is_some_and(|b| !matches!(b.attr("val"), Some("0") | Some("false")))
}

fn fill_is_visible(fill: &Node) -> bool {
    let Some(pattern) = fill.child("patternFill") else {
        return false;
    };
    if matches!(pattern.attr("patternType"), None | Some("none")) {
        return false;
    }
    pattern.child("fgColor").is_some_and(|color| {
        color
            .attr("rgb")
            .is_some_and(|rgb| !matches!(rgb, "FFFFFFFF" | "FFFFFF"))
    })
}

fn builtin_format_is_date(id: u32) -> bool {
    matches!(id, 14..=22 | 27..=36 | 45..=47 | 50..=58)
}

fn format_is_date(code: &str) -> bool {
    let mut cleaned = String::new();
    let mut in_quote = false;
    let mut in_bracket = false;
    let mut escaped = false;
    for ch in code.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' | '_' | '*' => escaped = true,
            '"' => in_quote = !in_quote,
            '[' if !in_quote => in_bracket = true,
            ']' if !in_quote => in_bracket = false,
            _ if !in_quote && !in_bracket => cleaned.push(ch),
            _ => {}
        }
    }
    cleaned
        .chars()
        .any(|c| matches!(c.to_ascii_lowercase(), 'y' | 'm' | 'd' | 'h' | 's'))
}

pub(crate) fn excel_serial_date(serial: f64, date1904: bool) -> Option<String> {
    if !serial.is_finite() {
        return None;
    }
    let days = serial.trunc() as i64;
    let (base, adjust) = if date1904 {
        (NaiveDate::from_ymd_opt(1904, 1, 1)?, 0)
    } else if days < 60 {
        (NaiveDate::from_ymd_opt(1899, 12, 31)?, 0)
    } else {
        (NaiveDate::from_ymd_opt(1899, 12, 30)?, 0)
    };
    Some(
        (base + Duration::days(days + adjust))
            .format("%Y-%m-%d")
            .to_string(),
    )
}

pub(crate) fn display_value(value: &CellValue) -> Option<String> {
    match value {
        CellValue::Empty => None,
        CellValue::String(s) => {
            if s.trim().is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        CellValue::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                Some(format!("{}", *n as i64))
            } else {
                Some(format!("{n}"))
            }
        }
        CellValue::Bool(b) => Some(if *b { "TRUE" } else { "FALSE" }.to_string()),
        CellValue::Date(d) => Some(d.clone()),
        CellValue::Error(e) => Some(e.clone()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::xlsx::ooxml::{open, Visibility};

    fn fixture() -> Vec<u8> {
        let parts: Vec<(&str, Vec<u8>)> = vec![
            (
                "[Content_Types].xml",
                br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_vec(),
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr date1904="0"/><sheets><sheet name="画面定義" sheetId="1" r:id="rId1"/><sheet name="非表示" sheetId="2" state="hidden" r:id="rId2"/></sheets><definedNames><definedName name="入力範囲">画面定義!$A$2:$C$2</definedName></definedNames></workbook>"#.as_bytes().to_vec(),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId4" Type="styles" Target="styles.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "xl/sharedStrings.xml",
                r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>項目名</t></si><si><t>住所</t></si><si><r><t>全角</t></r><r><t>20文字</t></r></si><si><t>説明</t><rPh><t>セツメイ</t></rPh></si><si><t> 前後空白 </t></si></sst>"#.as_bytes().to_vec(),
            ),
            (
                "xl/styles.xml",
                r#"<?xml version="1.0"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy/m/d"/></numFmts><fonts count="2"><font/><font><b/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/></patternFill></fill></fills><cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0"/><xf numFmtId="164" fontId="0" fillId="0"/><xf numFmtId="0" fontId="1" fillId="1"/></cellXfs></styleSheet>"#.as_bytes().to_vec(),
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s" s="2"><v>0</v></c><c r="B1" t="s" s="2"><v>1</v></c><c r="C1" t="s" s="2"><v>2</v></c></row><row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2"><v>123</v></c><c r="C2" s="1"><v>45292</v></c></row><row r="3"><c r="A3"><f>SUM(B2:B2)</f><v>123</v></c></row><row r="4"><c r="A4"><f>B2*2</f></c></row><row r="5"><c r="A5" t="b"><v>1</v></c><c r="B5" t="e"><v>#REF!</v></c><c r="C5" t="inlineStr"><is><t>インライン</t></is></c><c r="D5" t="s"><v>4</v></c></row></sheetData></worksheet>"#.as_bytes().to_vec(),
            ),
            (
                "xl/worksheets/sheet2.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.as_bytes().to_vec(),
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

    #[test]
    fn reads_values_and_shared_strings() {
        let wb = open(&fixture()).unwrap();
        let sheet = &wb.sheets().unwrap()[0];
        let cells = wb.sheet_cells(sheet).unwrap();

        assert_eq!(cells.len(), 12);
        assert_eq!(
            cells.get(0, 0).unwrap().value,
            CellValue::String("項目名".into())
        );
        assert_eq!(
            cells.get(0, 2).unwrap().value,
            CellValue::String("全角20文字".into())
        );
        assert_eq!(cells.get(1, 1).unwrap().value, CellValue::Number(123.0));
        assert_eq!(cells.get(4, 0).unwrap().value, CellValue::Bool(true));
        assert_eq!(
            cells.get(4, 1).unwrap().value,
            CellValue::Error("#REF!".into())
        );
        assert_eq!(
            cells.get(4, 2).unwrap().value,
            CellValue::String("インライン".into())
        );
        assert_eq!(
            cells.get(4, 3).unwrap().value,
            CellValue::String(" 前後空白 ".into())
        );
    }

    #[test]
    fn reads_dates_from_styles() {
        let wb = open(&fixture()).unwrap();
        let sheet = &wb.sheets().unwrap()[0];
        let cells = wb.sheet_cells(sheet).unwrap();
        assert_eq!(
            cells.get(1, 2).unwrap().value,
            CellValue::Date("2024-01-01".into())
        );
        let styles = wb.styles().unwrap();
        assert!(styles.get(1).is_date);
        assert!(styles.get(2).bold);
        assert!(styles.get(2).filled);
    }

    #[test]
    fn keeps_formula_and_cached_value() {
        let wb = open(&fixture()).unwrap();
        let sheet = &wb.sheets().unwrap()[0];
        let cells = wb.sheet_cells(sheet).unwrap();

        let cached = cells.get(2, 0).unwrap();
        assert_eq!(cached.value, CellValue::Number(123.0));
        let formula = cached.formula.as_ref().unwrap();
        assert_eq!(formula.text, "SUM(B2:B2)");
        assert_eq!(formula.cached, Some(CellValue::Number(123.0)));

        let missing = cells.get(3, 0).unwrap();
        assert_eq!(missing.value, CellValue::Empty);
        assert_eq!(missing.formula.as_ref().unwrap().text, "B2*2");
        assert!(missing.formula.as_ref().unwrap().cached.is_none());
        assert!(cells
            .warnings()
            .iter()
            .any(|w| w.contains("NO_CACHED_VALUE")));
    }

    #[test]
    fn reads_defined_names_and_visibility() {
        let wb = open(&fixture()).unwrap();
        let sheets = wb.sheets().unwrap();
        assert_eq!(sheets[0].visibility, Visibility::Visible);
        assert_eq!(sheets[1].visibility, Visibility::Hidden);

        let names = wb.defined_names().unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].name, "入力範囲");
        assert_eq!(names[0].text, "画面定義!$A$2:$C$2");
    }

    #[test]
    fn rich_text_ignores_phonetic_runs() {
        let wb = open(&fixture()).unwrap();
        let sheet = &wb.sheets().unwrap()[0];
        let cells = wb.sheet_cells(sheet).unwrap();
        assert_eq!(
            cells.get(0, 2).unwrap().value,
            CellValue::String("全角20文字".into())
        );
    }

    #[test]
    fn converts_serial_dates() {
        assert_eq!(excel_serial_date(1.0, false).unwrap(), "1900-01-01");
        assert_eq!(excel_serial_date(45292.0, false).unwrap(), "2024-01-01");
        assert_eq!(excel_serial_date(0.0, true).unwrap(), "1904-01-01");
    }
}
