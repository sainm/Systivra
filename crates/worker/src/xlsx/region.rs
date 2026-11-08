use std::collections::{BTreeSet, HashMap};

use serde_json::{Map, Value};

use super::a1::CellRef;
use super::ooxml::{MergedRegion, Sheet};
use super::values::{display_value, CellValue, SheetCells, Styles};

const CHUNK_ROWS: usize = 12;
const CHUNK_TOKENS: i32 = 800;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub id: String,
    pub sheet: String,
    pub row_start: u32,
    pub row_end: u32,
    pub col_start: u32,
    pub col_end: u32,
    pub header_row: Option<u32>,
}

impl Region {
    pub fn width(&self) -> u32 {
        self.col_end - self.col_start + 1
    }

    pub fn range_a1(&self) -> String {
        format!(
            "{}:{}",
            CellRef::new(self.row_start, self.col_start).to_a1(),
            CellRef::new(self.row_end, self.col_end).to_a1()
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct XlsxChunk {
    pub ordinal: i32,
    pub text: String,
    pub locator: Value,
    pub token_count: i32,
    pub char_count: i32,
    pub chunk_type: String,
    pub node_path: Vec<String>,
    pub region_id: String,
    pub row_start: u32,
    pub row_end: u32,
    pub col_start: u32,
    pub col_end: u32,
    pub row_texts: Vec<(u32, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct SheetPlan {
    pub regions: Vec<Region>,
    pub chunks: Vec<XlsxChunk>,
}

pub fn plan_sheet(
    sheet: &Sheet,
    cells: &SheetCells,
    merged: &[MergedRegion],
    styles: &Styles,
    base_ordinal: i32,
) -> SheetPlan {
    let regions = detect_regions(sheet, cells, merged, styles);
    let chunks = plan_chunks(sheet, cells, merged, &regions, base_ordinal);
    SheetPlan { regions, chunks }
}

pub fn detect_regions(
    sheet: &Sheet,
    cells: &SheetCells,
    merged: &[MergedRegion],
    styles: &Styles,
) -> Vec<Region> {
    let occ = occupied(cells, merged);
    let rows: Vec<u32> = occ
        .iter()
        .map(|(r, _)| *r)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut row_bands = bands(&rows);
    row_bands.sort_unstable();
    let mut out = Vec::new();

    for (r1, r2) in row_bands {
        let cols: Vec<u32> = occ
            .range((r1, 0)..=(r2, u32::MAX))
            .map(|(_, c)| *c)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for (c1, c2) in bands(&cols) {
            let points: Vec<(u32, u32)> = occ
                .iter()
                .copied()
                .filter(|(r, c)| *r >= r1 && *r <= r2 && *c >= c1 && *c <= c2)
                .collect();
            if points.is_empty() {
                continue;
            }
            let bounds = Bounds {
                r1: points.iter().map(|(r, _)| *r).min().unwrap(),
                r2: points.iter().map(|(r, _)| *r).max().unwrap(),
                c1: points.iter().map(|(_, c)| *c).min().unwrap(),
                c2: points.iter().map(|(_, c)| *c).max().unwrap(),
            };
            for part in split_blank_rows(bounds, &occ) {
                out.push(Region {
                    id: region_id(&sheet.name, part),
                    sheet: sheet.name.clone(),
                    row_start: part.r1,
                    row_end: part.r2,
                    col_start: part.c1,
                    col_end: part.c2,
                    header_row: None,
                });
            }
        }
    }

    let merged_bounds = merge_small(
        out.iter()
            .map(|r| Bounds {
                r1: r.row_start,
                r2: r.row_end,
                c1: r.col_start,
                c2: r.col_end,
            })
            .collect(),
    );

    let mut regions: Vec<Region> = merged_bounds
        .into_iter()
        .map(|b| Region {
            id: region_id(&sheet.name, b),
            sheet: sheet.name.clone(),
            row_start: b.r1,
            row_end: b.r2,
            col_start: b.c1,
            col_end: b.c2,
            header_row: None,
        })
        .collect();

    for region in &mut regions {
        region.header_row = detect_header(region, cells, merged, styles);
    }
    regions.sort_by_key(|r| (r.row_start, r.col_start));
    regions
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    r1: u32,
    r2: u32,
    c1: u32,
    c2: u32,
}

fn area(b: &Bounds) -> u64 {
    u64::from(b.r2 - b.r1 + 1) * u64::from(b.c2 - b.c1 + 1)
}

fn near(a: &Bounds, b: &Bounds) -> bool {
    let row_gap = if a.r2 < b.r1 {
        b.r1 - a.r2 - 1
    } else if b.r2 < a.r1 {
        a.r1 - b.r2 - 1
    } else {
        0
    };
    let col_gap = if a.c2 < b.c1 {
        b.c1 - a.c2 - 1
    } else if b.c2 < a.c1 {
        a.c1 - b.c2 - 1
    } else {
        0
    };
    row_gap <= 1 && col_gap <= 1
}

fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        r1: a.r1.min(b.r1),
        r2: a.r2.max(b.r2),
        c1: a.c1.min(b.c1),
        c2: a.c2.max(b.c2),
    }
}

fn merge_small(mut regions: Vec<Bounds>) -> Vec<Bounds> {
    loop {
        let mut merged = false;
        'outer: for i in 0..regions.len() {
            if area(&regions[i]) >= 4 {
                continue;
            }
            for j in 0..regions.len() {
                if i == j || !near(&regions[i], &regions[j]) {
                    continue;
                }
                let source = regions.remove(i);
                let target = if j > i { j - 1 } else { j };
                regions[target] = union(regions[target], source);
                merged = true;
                break 'outer;
            }
        }
        if !merged {
            return regions;
        }
    }
}

fn bands(values: &[u32]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for value in values {
        match out.last_mut() {
            Some(last) if *value <= last.1 + 2 => last.1 = *value,
            _ => out.push((*value, *value)),
        }
    }
    out
}

fn split_blank_rows(bounds: Bounds, occ: &BTreeSet<(u32, u32)>) -> Vec<Bounds> {
    let mut out = Vec::new();
    let mut start = bounds.r1;
    let mut run = 0u32;
    for row in bounds.r1..=bounds.r2 {
        let empty = (bounds.c1..=bounds.c2).all(|col| !occ.contains(&(row, col)));
        if empty {
            run += 1;
            if run >= 3 {
                let run_start = row - run + 1;
                if run_start > start {
                    out.push(Bounds {
                        r1: start,
                        r2: run_start - 1,
                        ..bounds
                    });
                }
                start = row + 1;
                run = 0;
            }
        } else {
            run = 0;
        }
    }
    if start <= bounds.r2 {
        out.push(Bounds {
            r1: start,
            r2: bounds.r2,
            ..bounds
        });
    }
    out
}

fn region_id(sheet: &str, b: Bounds) -> String {
    format!(
        "{}:{}:{}-{}:{}",
        sheet,
        b.r1 + 1,
        b.c1 + 1,
        b.r2 + 1,
        b.c2 + 1
    )
}

fn occupied(cells: &SheetCells, merged: &[MergedRegion]) -> BTreeSet<(u32, u32)> {
    let mut out = BTreeSet::new();
    for cell in cells.iter() {
        if !matches!(cell.value, CellValue::Empty)
            || cell.formula.is_some()
            || cell.comment.is_some()
        {
            out.insert((cell.row, cell.col));
        }
    }
    for region in merged {
        for row in region.anchor.row..region.anchor.row + region.rows {
            for col in region.anchor.col..region.anchor.col + region.cols {
                out.insert((row, col));
            }
        }
    }
    out
}

fn detect_header(
    region: &Region,
    cells: &SheetCells,
    merged: &[MergedRegion],
    styles: &Styles,
) -> Option<u32> {
    let mut best: Option<(u32, u32)> = None;
    for row in region.row_start..=region.row_end.min(region.row_start + 2) {
        let score = header_score(region, row, cells, merged, styles);
        if score >= 3 && best.is_none_or(|(_, current)| score > current) {
            best = Some((row, score));
        }
    }
    best.map(|(row, _)| row)
}

fn header_score(
    region: &Region,
    row: u32,
    cells: &SheetCells,
    merged: &[MergedRegion],
    styles: &Styles,
) -> u32 {
    let mut score = 0;
    let mut non_empty = 0u32;
    let mut all_string = true;
    let mut bold = false;
    let mut filled = false;

    for col in region.col_start..=region.col_end {
        let Some(cell) = cells.get(row, col) else {
            continue;
        };
        if matches!(cell.value, CellValue::Empty) {
            continue;
        }
        non_empty += 1;
        if !matches!(cell.value, CellValue::String(_)) {
            all_string = false;
        }
        let style = styles.get(cell.style_id);
        bold |= style.bold;
        filled |= style.filled;
    }

    if non_empty >= 2 && all_string {
        score += 2;
    }
    if merged
        .iter()
        .any(|m| m.anchor.row == row && m.cols > 1 && m.anchor.col < region.col_end)
    {
        score += 2;
    }
    if row < region.row_end && has_type_transition(region, row, cells) {
        score += 2;
    }
    if bold {
        score += 1;
    }
    if filled {
        score += 1;
    }
    if non_empty == region.width() {
        score += 1;
    }
    score
}

fn has_type_transition(region: &Region, row: u32, cells: &SheetCells) -> bool {
    let mut saw_string = false;
    let mut saw_scalar = false;
    for col in region.col_start..=region.col_end {
        if let Some(cell) = cells.get(row, col) {
            if matches!(cell.value, CellValue::String(_)) {
                saw_string = true;
            }
        }
        if let Some(cell) = cells.get(row + 1, col) {
            if matches!(cell.value, CellValue::Number(_) | CellValue::Date(_)) {
                saw_scalar = true;
            }
        }
    }
    saw_string && saw_scalar
}

fn merged_map(merged: &[MergedRegion]) -> HashMap<(u32, u32), (u32, u32)> {
    let mut out = HashMap::new();
    for region in merged {
        for row in region.anchor.row..region.anchor.row + region.rows {
            for col in region.anchor.col..region.anchor.col + region.cols {
                out.insert((row, col), (region.anchor.row, region.anchor.col));
            }
        }
    }
    out
}

fn header_keys(
    region: &Region,
    cells: &SheetCells,
    merged: &[MergedRegion],
) -> Vec<Option<String>> {
    let Some(header_row) = region.header_row else {
        return Vec::new();
    };
    let map = merged_map(merged);
    (region.col_start..=region.col_end)
        .map(|col| {
            let anchor = map
                .get(&(header_row, col))
                .copied()
                .unwrap_or((header_row, col));
            cells
                .get(anchor.0, anchor.1)
                .and_then(|cell| display_value(&cell.value))
        })
        .collect()
}

fn row_lines(
    region: &Region,
    row: u32,
    keys: &[Option<String>],
    cells: &SheetCells,
    map: &HashMap<(u32, u32), (u32, u32)>,
) -> Vec<String> {
    let mut out = Vec::new();
    for col in region.col_start..=region.col_end {
        if map
            .get(&(row, col))
            .is_some_and(|anchor| *anchor != (row, col))
        {
            continue;
        }
        let Some(cell) = cells.get(row, col) else {
            continue;
        };
        let Some(value) = display_value(&cell.value) else {
            continue;
        };
        let value = value.replace(['\r', '\n'], " ");
        let key = keys
            .get((col - region.col_start) as usize)
            .and_then(|key| key.clone())
            .filter(|key| !key.trim().is_empty())
            .unwrap_or_else(|| format!("R{}C{}", row + 1, col + 1));
        out.push(format!("{key}: {value}"));
    }
    out
}

fn title_text(
    region: &Region,
    cells: &SheetCells,
    map: &HashMap<(u32, u32), (u32, u32)>,
) -> Option<String> {
    let end = region.header_row?;
    if end == region.row_start {
        return None;
    }
    let mut parts = Vec::new();
    for row in region.row_start..end {
        for col in region.col_start..=region.col_end {
            if map
                .get(&(row, col))
                .is_some_and(|anchor| *anchor != (row, col))
            {
                continue;
            }
            if let Some(value) = cells
                .get(row, col)
                .and_then(|cell| display_value(&cell.value))
            {
                let value = value.trim();
                if !value.is_empty() {
                    parts.push(value.replace(['\r', '\n'], " "));
                }
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" / "))
    }
}

fn chunk_text(
    sheet: &Sheet,
    region: &Region,
    title: Option<&str>,
    blocks: &[Vec<String>],
) -> String {
    let prefix = match title {
        Some(title) if title == sheet.name || title.ends_with(&format!("/ {}", sheet.name)) => {
            format!("【{title}】")
        }
        Some(title) => format!("【{title} / {}】", sheet.name),
        None => format!("【{} / {}】", sheet.name, region.range_a1()),
    };
    let mut lines = vec![prefix];
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            lines.push("---".into());
        }
        lines.extend(block.iter().cloned());
    }
    lines.join("\n")
}

fn token_count(text: &str) -> i32 {
    engine_rag::tokenize(text).len() as i32
}

fn plan_chunks(
    sheet: &Sheet,
    cells: &SheetCells,
    merged: &[MergedRegion],
    regions: &[Region],
    base_ordinal: i32,
) -> Vec<XlsxChunk> {
    let map = merged_map(merged);
    let mut out = Vec::new();
    let mut ordinal = base_ordinal;

    for region in regions {
        let title = title_text(region, cells, &map);
        let keys = header_keys(region, cells, merged);
        let data_start = region
            .header_row
            .map(|row| row + 1)
            .unwrap_or(region.row_start);
        let blocks: Vec<(u32, Vec<String>)> = (data_start..=region.row_end)
            .map(|row| (row, row_lines(region, row, &keys, cells, &map)))
            .filter(|(_, lines)| !lines.is_empty())
            .collect();
        if blocks.is_empty() {
            continue;
        }

        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        for index in 0..blocks.len() {
            let mut candidate = current.clone();
            candidate.push(index);
            let text = chunk_text(
                sheet,
                region,
                title.as_deref(),
                &candidate
                    .iter()
                    .map(|i| blocks[*i].1.clone())
                    .collect::<Vec<_>>(),
            );
            if !current.is_empty()
                && (current.len() >= CHUNK_ROWS || token_count(&text) > CHUNK_TOKENS)
            {
                groups.push(current);
                current = vec![index];
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            groups.push(current);
        }

        for group in groups {
            let row_start = blocks[group[0]].0;
            let row_end = blocks[*group.last().unwrap()].0;
            let group_blocks: Vec<Vec<String>> =
                group.iter().map(|i| blocks[*i].1.clone()).collect();
            let row_texts: Vec<(u32, String)> = group
                .iter()
                .map(|i| (blocks[*i].0, blocks[*i].1.join(" ")))
                .collect();
            let text = chunk_text(sheet, region, title.as_deref(), &group_blocks);
            if group_blocks.len() == 1 && token_count(&text) > CHUNK_TOKENS {
                for line in &group_blocks[0] {
                    let single = chunk_text(sheet, region, title.as_deref(), &[vec![line.clone()]]);
                    push_chunk(
                        &mut out,
                        &mut ordinal,
                        sheet,
                        region,
                        row_start,
                        row_end,
                        row_texts.clone(),
                        single,
                        merged,
                    );
                }
                continue;
            }
            push_chunk(
                &mut out,
                &mut ordinal,
                sheet,
                region,
                row_start,
                row_end,
                row_texts,
                text,
                merged,
            );
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn push_chunk(
    out: &mut Vec<XlsxChunk>,
    ordinal: &mut i32,
    sheet: &Sheet,
    region: &Region,
    row_start: u32,
    row_end: u32,
    row_texts: Vec<(u32, String)>,
    text: String,
    merged: &[MergedRegion],
) {
    let start = CellRef::new(row_start, region.col_start);
    let end = CellRef::new(row_end, region.col_end);
    let mut locator = Map::new();
    locator.insert("kind".into(), Value::String("xlsx".into()));
    locator.insert("sheet".into(), Value::String(sheet.name.clone()));
    locator.insert(
        "range".into(),
        Value::String(format!("{}:{}", start.to_a1(), end.to_a1())),
    );
    locator.insert(
        "rows".into(),
        Value::Array(vec![Value::from(row_start + 1), Value::from(row_end + 1)]),
    );
    locator.insert("anchor_cell".into(), Value::String(start.to_a1()));
    locator.insert("region_id".into(), Value::String(region.id.clone()));
    if let Some(span) = merged
        .iter()
        .find(|m| {
            row_start >= m.anchor.row
                && row_start < m.anchor.row + m.rows
                && region.col_start >= m.anchor.col
                && region.col_start < m.anchor.col + m.cols
        })
        .map(MergedRegion::to_a1)
    {
        locator.insert("merged_span".into(), Value::String(span));
    }

    out.push(XlsxChunk {
        ordinal: *ordinal,
        char_count: text.chars().count() as i32,
        token_count: token_count(&text),
        text,
        locator: Value::Object(locator),
        chunk_type: "SHEET_REGION".into(),
        node_path: vec![
            "DOCUMENT".into(),
            format!("SHEET:{}", sheet.name),
            format!("REGION:{}", region.id),
        ],
        region_id: region.id.clone(),
        row_start,
        row_end,
        col_start: region.col_start,
        col_end: region.col_end,
        row_texts,
    });
    *ordinal += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xlsx::ooxml::Visibility;
    use crate::xlsx::values::{Cell, Formula};

    fn sheet(name: &str) -> Sheet {
        Sheet {
            name: name.into(),
            part: "xl/worksheets/sheet1.xml".into(),
            visibility: Visibility::Visible,
        }
    }

    fn text_cell(row: u32, col: u32, value: &str) -> Cell {
        Cell {
            row,
            col,
            value: CellValue::String(value.into()),
            formula: None,
            style_id: 0,
            comment: None,
        }
    }

    fn number_cell(row: u32, col: u32, value: f64) -> Cell {
        Cell {
            row,
            col,
            value: CellValue::Number(value),
            formula: None,
            style_id: 0,
            comment: None,
        }
    }

    fn sample_cells() -> SheetCells {
        let mut cells = Vec::new();
        for (col, value) in ["項目名", "入力形式", "必須"].iter().enumerate() {
            cells.push(text_cell(0, col as u32, value));
        }
        for row in 1..=13 {
            cells.push(text_cell(row, 0, &format!("項目{row}")));
            cells.push(text_cell(row, 1, "全角20文字以内"));
            cells.push(text_cell(row, 2, "必須"));
        }
        SheetCells::from_cells(cells)
    }

    #[test]
    fn detects_header_and_regions() {
        let cells = sample_cells();
        let regions = detect_regions(&sheet("画面項目定義"), &cells, &[], &Styles::default());
        assert_eq!(regions.len(), 1, "{regions:?}");
        let region = &regions[0];
        assert_eq!(region.header_row, Some(0));
        assert_eq!((region.row_start, region.row_end), (0, 13));
        assert_eq!((region.col_start, region.col_end), (0, 2));
        assert_eq!(region.id, "画面項目定義:1:1-14:3");
    }

    #[test]
    fn splits_tables_with_two_blank_rows() {
        let mut cells = sample_cells();
        for row in 16..=18 {
            cells.insert(number_cell(row, 0, 1.0));
        }
        let regions = detect_regions(&sheet("Sheet1"), &cells, &[], &Styles::default());
        assert_eq!(regions.len(), 2, "{regions:?}");
        assert_eq!(regions[1].row_start, 16);
    }

    #[test]
    fn joins_gap_of_one_blank_row() {
        let mut cells = sample_cells();
        cells.insert(number_cell(15, 0, 1.0));
        let regions = detect_regions(&sheet("Sheet1"), &cells, &[], &Styles::default());
        assert_eq!(regions.len(), 1, "{regions:?}");
    }

    #[test]
    fn builds_chunks_with_repeated_header_and_12_row_limit() {
        let cells = sample_cells();
        let plan = plan_sheet(&sheet("画面項目定義"), &cells, &[], &Styles::default(), 0);
        assert_eq!(plan.chunks.len(), 2, "{:?}", plan.chunks);
        assert!(plan.chunks[0].text.contains("【画面項目定義 / A1:C14】"));
        assert!(plan.chunks[0].text.contains("項目名: 項目1"));
        assert!(plan.chunks[0].text.contains("---"));
        assert!(plan.chunks.iter().all(|c| c.token_count <= 800));
        assert_eq!(plan.chunks[0].row_start, 1);
        assert_eq!(plan.chunks[0].row_end, 12);
        assert_eq!(plan.chunks[1].row_start, 13);
        assert_eq!(plan.chunks[1].row_end, 13);
    }

    #[test]
    fn skips_merged_children_and_expands_merged_header() {
        let mut cells = Vec::new();
        cells.push(text_cell(0, 0, "画面項目"));
        for col in 1..3 {
            cells.push(text_cell(0, col, ""));
        }
        cells.push(text_cell(1, 0, "住所"));
        cells.push(number_cell(1, 1, 20.0));
        let merged = vec![MergedRegion {
            anchor: CellRef::new(0, 0),
            rows: 1,
            cols: 3,
        }];
        let plan = plan_sheet(
            &sheet("Sheet1"),
            &SheetCells::from_cells(cells),
            &merged,
            &Styles::default(),
            0,
        );
        assert_eq!(plan.regions.len(), 1);
        assert_eq!(plan.regions[0].header_row, Some(0));
        let text = &plan.chunks[0].text;
        assert!(text.contains("画面項目: 住所"), "{text}");
        assert!(text.contains("画面項目: 20"), "{text}");
        assert!(!text.contains("画面項目: 画面項目"), "{text}");
        assert!(!text.contains(": \n"));
    }

    #[test]
    fn uses_merged_title_in_chunk_prefix() {
        let cells = SheetCells::from_cells(vec![
            text_cell(0, 0, "ユーザ編集画面"),
            text_cell(1, 0, "項目名"),
            text_cell(1, 1, "入力形式"),
            text_cell(2, 0, "住所"),
            number_cell(2, 1, 20.0),
        ]);
        let merged = vec![MergedRegion {
            anchor: CellRef::new(0, 0),
            rows: 1,
            cols: 2,
        }];
        let plan = plan_sheet(&sheet("Sheet1"), &cells, &merged, &Styles::default(), 0);
        assert_eq!(plan.regions[0].header_row, Some(1));
        let text = &plan.chunks[0].text;
        assert!(text.starts_with("【ユーザ編集画面 / Sheet1】"), "{text}");
        assert!(text.contains("項目名: 住所"), "{text}");
        assert!(text.contains("入力形式: 20"), "{text}");
    }

    #[test]
    fn uses_rc_keys_without_header() {
        let cells = SheetCells::from_cells(vec![
            number_cell(0, 0, 1.0),
            number_cell(0, 1, 2.0),
            number_cell(1, 0, 3.0),
            number_cell(1, 1, 4.0),
        ]);
        let plan = plan_sheet(&sheet("Sheet1"), &cells, &[], &Styles::default(), 0);
        assert_eq!(plan.regions[0].header_row, None);
        assert!(plan.chunks[0].text.contains("R1C1: 1"));
        assert!(plan.chunks[0].text.contains("R2C2: 4"));
    }

    #[test]
    fn locator_contains_range_rows_and_region() {
        let cells = sample_cells();
        let plan = plan_sheet(&sheet("画面項目定義"), &cells, &[], &Styles::default(), 0);
        let locator = &plan.chunks[0].locator;
        assert_eq!(locator["kind"], "xlsx");
        assert_eq!(locator["sheet"], "画面項目定義");
        assert_eq!(locator["range"], "A2:C13");
        assert_eq!(locator["rows"], serde_json::json!([2, 13]));
        assert_eq!(locator["anchor_cell"], "A2");
        assert_eq!(locator["region_id"], "画面項目定義:1:1-14:3");
    }

    #[test]
    fn formula_without_cache_is_not_searchable() {
        let mut cells = vec![text_cell(0, 0, "項目名"), text_cell(0, 1, "値")];
        cells.push(text_cell(1, 0, "合計"));
        cells.push(Cell {
            row: 1,
            col: 1,
            value: CellValue::Empty,
            formula: Some(Formula {
                text: "SUM(B2:B2)".into(),
                cached: None,
                shared_index: None,
            }),
            style_id: 0,
            comment: None,
        });
        let plan = plan_sheet(
            &sheet("Sheet1"),
            &SheetCells::from_cells(cells),
            &[],
            &Styles::default(),
            0,
        );
        assert!(!plan.chunks[0].text.contains("SUM"));
        assert!(plan.chunks[0].text.contains("項目名: 合計"));
    }

    #[test]
    fn long_tables_split_under_token_limit() {
        let mut cells = Vec::new();
        for (col, value) in ["項目名", "説明", "備考"].iter().enumerate() {
            cells.push(text_cell(0, col as u32, value));
        }
        for row in 1..=1000u32 {
            cells.push(text_cell(row, 0, &format!("項目{row}")));
            cells.push(text_cell(row, 1, "全角20文字以内で入力すること"));
            cells.push(text_cell(row, 2, "必須"));
        }
        let plan = plan_sheet(
            &sheet("Sheet1"),
            &SheetCells::from_cells(cells),
            &[],
            &Styles::default(),
            0,
        );
        assert!(plan.chunks.len() > 80, "{}", plan.chunks.len());
        assert!(plan.chunks.iter().all(|c| c.token_count <= 800));
        assert_eq!(plan.chunks[0].row_start, 1);
        assert_eq!(plan.chunks.last().unwrap().row_end, 1000);
        assert!(plan.chunks.iter().all(|c| c.text.contains("項目名: 項目")));
    }
}
