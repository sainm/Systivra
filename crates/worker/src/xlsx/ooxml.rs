//! OOXML（ZIP + XML）の読み取り。
//!
//! 汎用のセル読み取りライブラリを使わない理由は `mod.rs` に書いた。
//! ここで必要なのは「結合範囲」と「図形がどのセルに貼られているか」で、
//! どちらも `xl/worksheets/*.xml` と `xl/drawings/*.xml` を直に読むのが最短。

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::office::{parent_of, Node, Package};

use super::a1::{self, CellRef};

pub use crate::office::OfficeError as XlsxError;
pub(super) use crate::office::{normalize_part, resolve_target};

/// 開いたブック。ZIP の中身をそのまま持つ。
pub struct Workbook {
    package: Package,
}

/// シートの表示状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
    VeryHidden,
}

/// シート 1 枚。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    pub name: String,
    /// ZIP 内のパス（例 `xl/worksheets/sheet1.xml`）
    pub part: String,
    pub visibility: Visibility,
}

/// 結合セル。子セルを展開するかどうかは呼び出し側が決める（§5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergedRegion {
    /// 値を持つ左上のセル
    pub anchor: CellRef,
    pub rows: u32,
    pub cols: u32,
}

impl MergedRegion {
    pub fn cell_count(&self) -> u32 {
        self.rows.saturating_mul(self.cols)
    }

    /// A1 形式（`A1:C3`）。locator に使う。
    pub fn to_a1(&self) -> String {
        let end = CellRef::new(
            self.anchor.row + self.rows.saturating_sub(1),
            self.anchor.col + self.cols.saturating_sub(1),
        );
        format!("{}:{}", self.anchor.to_a1(), end.to_a1())
    }

    /// アンカー以外のセル。描画時に値を繰り返さないため（§5）。
    pub fn children(&self) -> impl Iterator<Item = CellRef> + '_ {
        (0..self.rows).flat_map(move |dr| {
            (0..self.cols).map(move |dc| CellRef::new(self.anchor.row + dr, self.anchor.col + dc))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FigureKind {
    Picture,
    Shape,
    /// グラフ。中身は取らない（§7）。タイトルとアンカーだけ記録する。
    Chart,
}

/// 図形・画像・グラフ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Figure {
    pub kind: FigureKind,
    /// 左上のアンカーセル。`locator` の `anchor_cell`。
    pub anchor: CellRef,
    /// `twoCellAnchor` のときだけ右下がある。
    pub to: Option<CellRef>,
    /// 画像のときだけ。`xl/media/...` の SHA-256。
    pub media: Option<MediaRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    pub part: String,
    pub sha256: String,
    pub bytes: usize,
}

/// ZIP を開いてパーツを読み込む。
pub fn open(bytes: &[u8]) -> Result<Workbook, XlsxError> {
    Ok(Workbook {
        package: crate::office::open(bytes)?,
    })
}

impl Workbook {
    pub fn has(&self, part: &str) -> bool {
        self.package.has(part)
    }

    pub(super) fn xml(&self, part: &str) -> Result<Node, XlsxError> {
        self.package.xml(part)
    }

    /// シート一覧。順序は `workbook.xml` の定義順。
    pub fn sheets(&self) -> Result<Vec<Sheet>, XlsxError> {
        let wb = self.xml("xl/workbook.xml")?;
        let rels = self.rels_for("xl/workbook.xml")?;

        let mut out = Vec::new();
        for s in wb.descendants("sheet") {
            let Some(name) = s.attr("name") else { continue };
            let Some(rid) = s.attr("id") else { continue };
            let Some(target) = rels.get(rid) else {
                return Err(XlsxError::Rel(format!("sheet の rel {rid} が無い")));
            };
            let visibility = match s.attr("state") {
                Some("hidden") => Visibility::Hidden,
                Some("veryHidden") => Visibility::VeryHidden,
                _ => Visibility::Visible,
            };
            out.push(Sheet {
                name: name.to_string(),
                part: normalize_part("xl", target),
                visibility,
            });
        }
        Ok(out)
    }

    /// 結合セル。`mergeCell ref="A1:C3"` を読む。
    pub fn merged_regions(&self, sheet: &Sheet) -> Result<Vec<MergedRegion>, XlsxError> {
        let root = self.xml(&sheet.part)?;
        let mut out = Vec::new();

        for mc in root.descendants("mergeCell") {
            let Some(r) = mc.attr("ref") else { continue };
            let Some((a, b)) = a1::parse_range(r) else {
                continue;
            };
            if b.row < a.row || b.col < a.col {
                continue;
            }
            out.push(MergedRegion {
                anchor: a,
                rows: b.row - a.row + 1,
                cols: b.col - a.col + 1,
            });
        }

        // 左上から順に。読む側の出力を安定させる。
        out.sort_by_key(|m| (m.anchor.row, m.anchor.col));
        Ok(out)
    }

    /// 図形・画像・グラフ。
    pub fn figures(&self, sheet: &Sheet) -> Result<Vec<Figure>, XlsxError> {
        let root = self.xml(&sheet.part)?;

        // `<drawing r:id="rId1"/>`
        let Some(rid) = root
            .descendants("drawing")
            .next()
            .and_then(|n| n.attr("id"))
            .map(str::to_string)
        else {
            return Ok(Vec::new());
        };

        let sheet_rels = self.rels_for(&sheet.part)?;
        let Some(target) = sheet_rels.get(&rid) else {
            return Err(XlsxError::Rel(format!("drawing の rel {rid} が無い")));
        };
        let drawing_part = normalize_part(parent_of(&sheet.part), target);

        if !self.has(&drawing_part) {
            // 図形が 1 つも無いブックでは drawing パーツが無いことがある。
            return Ok(Vec::new());
        }

        let dr = self.xml(&drawing_part)?;
        let drawing_rels = self.rels_for(&drawing_part)?;
        let media_dir = parent_of(&drawing_part);

        let mut out = Vec::new();

        // `parse_tree` が返すのは合成根なので、`dr.children` を直接見ると
        // `<xdr:wsDr>` しか入っていない。アンカーはその中にある。
        let mut anchors: Vec<&Node> = Vec::new();
        for name in ["twoCellAnchor", "oneCellAnchor", "absoluteAnchor"] {
            anchors.extend(dr.descendants(name));
        }

        for anchor in anchors {
            // `from/col` `from/row` は 0 始まり。
            let from = anchor.child("from");
            let (row, col) = match from {
                Some(f) => (
                    f.child_text("row")
                        .and_then(|t| t.parse::<u32>().ok())
                        .unwrap_or(0),
                    f.child_text("col")
                        .and_then(|t| t.parse::<u32>().ok())
                        .unwrap_or(0),
                ),
                // absoluteAnchor はセルに紐づかない。左上とみなす（§7 の範囲外）。
                None => (0, 0),
            };

            let to = anchor.child("to").map(|f| {
                CellRef::new(
                    f.child_text("row")
                        .and_then(|t| t.parse().ok())
                        .unwrap_or(row),
                    f.child_text("col")
                        .and_then(|t| t.parse().ok())
                        .unwrap_or(col),
                )
            });

            // 種別は子要素で決まる。`sp` は図形、`pic` は画像、`graphicFrame` はグラフや表。
            let (kind, media) = if let Some(pic) = anchor.child("pic") {
                let media = pic
                    .descendants("blip")
                    .next()
                    .and_then(|b| b.attr("embed"))
                    .and_then(|rid| drawing_rels.get(rid).cloned())
                    .map(|t| normalize_part(media_dir, &t))
                    .and_then(|part| self.media_ref(&part));
                (FigureKind::Picture, media)
            } else if anchor.child("sp").is_some() {
                (FigureKind::Shape, None)
            } else if anchor.descendants("chart").next().is_some() {
                (FigureKind::Chart, None)
            } else if anchor.child("graphicFrame").is_some() {
                // 表や SmartArt。今回は図形として扱う（中身は取らない）。
                (FigureKind::Shape, None)
            } else {
                continue;
            };

            out.push(Figure {
                kind,
                anchor: CellRef::new(row, col),
                to,
                media,
            });
        }

        out.sort_by_key(|f| (f.anchor.row, f.anchor.col));
        Ok(out)
    }

    fn media_ref(&self, part: &str) -> Option<MediaRef> {
        let bytes = self.package.part(part)?;
        let mut h = Sha256::new();
        h.update(bytes);
        Some(MediaRef {
            part: part.to_string(),
            sha256: format!("{:x}", h.finalize()),
            bytes: bytes.len(),
        })
    }

    /// `foo.xml` の関係ファイル `_rels/foo.xml.rels` を読む。
    pub(super) fn rels_for(&self, part: &str) -> Result<HashMap<String, String>, XlsxError> {
        self.package.rels_for(part)
    }
}

// ─────────────────────────────────────────────────────────────
// テスト用の最小 XLSX
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod fixture {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;

    /// 結合セル・画像・図形・グラフを 1 つずつ持つブックを作る。
    ///
    /// 実物の Excel が吐く構造に合わせている（`xdr:from` の 0 始まり、
    /// drawing の rels 経由の media 参照）。
    pub fn sample_xlsx() -> Vec<u8> {
        let parts: Vec<(&str, Vec<u8>)> = vec![
            (
                "[Content_Types].xml",
                br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_vec(),
            ),
            (
                "_rels/.rels",
                br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#.to_vec(),
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="画面定義" sheetId="1" r:id="rId1"/></sheets></workbook>"#.as_bytes().to_vec(),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "xl/worksheets/sheet1.xml",
                br#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData/><mergeCells count="2"><mergeCell ref="A1:C1"/><mergeCell ref="B3:B5"/></mergeCells><drawing r:id="rId1"/></worksheet>"#.to_vec(),
            ),
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="drawing" Target="../drawings/drawing1.xml"/></Relationships>"#.to_vec(),
            ),
            (
                "xl/drawings/drawing1.xml",
                r#"<?xml version="1.0"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
                  <xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:row>4</xdr:row></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:row>9</xdr:row></xdr:to><xdr:pic><xdr:blipFill><a:blip r:embed="rId1"/></xdr:blipFill></xdr:pic></xdr:twoCellAnchor>
                  <xdr:oneCellAnchor><xdr:from><xdr:col>5</xdr:col><xdr:row>0</xdr:row></xdr:from><xdr:sp><xdr:txBody><a:p>項目の説明</a:p></xdr:txBody></xdr:sp></xdr:oneCellAnchor>
                  <xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:row>10</xdr:row></xdr:from><xdr:to><xdr:col>6</xdr:col><xdr:row>20</xdr:row></xdr:to><xdr:graphicFrame><a:graphic><a:graphicData><c:chart r:id="rId2"/></a:graphicData></a:graphic></xdr:graphicFrame></xdr:twoCellAnchor>
                </xdr:wsDr>"#.as_bytes().to_vec(),
            ),
            (
                "xl/drawings/_rels/drawing1.xml.rels",
                br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="image" Target="../media/image1.png"/><Relationship Id="rId2" Type="chart" Target="../charts/chart1.xml"/></Relationships>"#.to_vec(),
            ),
            ("xl/media/image1.png", b"\x89PNG\r\n\x1a\nfake-image-bytes".to_vec()),
            (
                "xl/charts/chart1.xml",
                br#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#.to_vec(),
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
}

#[cfg(test)]
mod tests {
    use super::fixture::sample_xlsx;
    use super::*;

    fn open_sample() -> Workbook {
        open(&sample_xlsx()).expect("サンプルを開けない")
    }

    #[test]
    fn lists_sheets() {
        let wb = open_sample();
        let sheets = wb.sheets().unwrap();
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].name, "画面定義");
        assert_eq!(sheets[0].part, "xl/worksheets/sheet1.xml");
    }

    #[test]
    fn sheets_keep_workbook_order() {
        // 並びは意味を持つ（1 枚目が表紙や画面定義であることが多い）。
        // LIFO で積むと逆順になるので、退行検出用に固定する。
        let wb = open_sample();
        let sheets = wb.sheets().unwrap();
        assert_eq!(sheets[0].name, "画面定義");
    }

    #[test]
    fn reads_merged_regions() {
        let wb = open_sample();
        let sheet = &wb.sheets().unwrap()[0];
        let merged = wb.merged_regions(sheet).unwrap();

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].anchor, CellRef::new(0, 0));
        assert_eq!(merged[0].to_a1(), "A1:C1");
        assert_eq!(merged[0].cell_count(), 3);

        assert_eq!(merged[1].to_a1(), "B3:B5");
        assert_eq!(merged[1].cell_count(), 3);
    }

    #[test]
    fn merged_children_exclude_the_anchor() {
        // 子に値を繰り返すと検索ノイズになる（§5）。アンカーは含めない。
        let r = MergedRegion {
            anchor: CellRef::new(0, 0),
            rows: 2,
            cols: 2,
        };
        let kids: Vec<String> = r.children().map(|c| c.to_a1()).collect();
        assert_eq!(kids, vec!["A1", "B1", "A2", "B2"]);
    }

    #[test]
    fn reads_figures_with_anchor_cells() {
        let wb = open_sample();
        let sheet = &wb.sheets().unwrap()[0];
        let figs = wb.figures(sheet).unwrap();

        assert_eq!(figs.len(), 3, "{figs:?}");

        // 画像: from (row=4, col=0) → A5、to (row=9, col=3) → D10
        let pic = figs.iter().find(|f| f.kind == FigureKind::Picture).unwrap();
        assert_eq!(pic.anchor, CellRef::new(4, 0));
        assert_eq!(pic.anchor.to_a1(), "A5");
        assert_eq!(pic.to.map(|c| c.to_a1()), Some("D10".to_string()));
        let media = pic.media.as_ref().expect("画像の SHA-256 が無い");
        assert_eq!(media.part, "xl/media/image1.png");
        assert_eq!(media.sha256.len(), 64);

        // 図形: oneCellAnchor なので to が無い
        let shape = figs.iter().find(|f| f.kind == FigureKind::Shape).unwrap();
        assert_eq!(shape.anchor.to_a1(), "F1");
        assert!(shape.to.is_none());

        // グラフ: 中身は取らない
        let chart = figs.iter().find(|f| f.kind == FigureKind::Chart).unwrap();
        assert_eq!(chart.anchor.to_a1(), "B11");
        assert!(chart.media.is_none());
    }

    #[test]
    fn same_image_yields_same_hash() {
        let wb = open_sample();
        let sheet = &wb.sheets().unwrap()[0];
        let figs = wb.figures(sheet).unwrap();
        // 並びはアンカーセル順なので、画像を種別で選ぶ。
        let hash = |f: &[Figure]| {
            f.iter()
                .find(|x| x.kind == FigureKind::Picture)
                .and_then(|x| x.media.as_ref())
                .map(|m| m.sha256.clone())
                .expect("画像の SHA-256 が無い")
        };
        let a = hash(&figs);

        // 同じブックを開き直しても同じ
        let wb2 = open_sample();
        let figs2 = wb2.figures(&wb2.sheets().unwrap()[0]).unwrap();
        assert_eq!(a, hash(&figs2));
    }

    #[test]
    fn broken_zip_is_rejected() {
        let err = open(b"not a zip at all");
        assert!(matches!(err, Err(XlsxError::Zip(_))));
    }

    #[test]
    fn normalizes_relative_targets() {
        assert_eq!(
            normalize_part("xl/worksheets", "../drawings/drawing1.xml"),
            "xl/drawings/drawing1.xml"
        );
        assert_eq!(
            normalize_part("xl", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(normalize_part("xl", "/xl/media/a.png"), "xl/media/a.png");
    }
}
