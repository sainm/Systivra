//! XLSX の結合セルと図形アンカーを実ファイルで確認する。
//!
//! ```text
//! cargo run -p worker --example probe_xlsx -- <file.xlsx>...
//! ```

use std::fs;

use worker::xlsx::{self, FigureKind};

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: probe_xlsx <file.xlsx>...");
        std::process::exit(2);
    }

    for path in &paths {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{path}\tREAD_ERROR\t{e}");
                continue;
            }
        };

        let wb = match xlsx::open(&bytes) {
            Ok(w) => w,
            Err(e) => {
                println!("{path}\tOPEN_ERROR\t{e}");
                continue;
            }
        };

        let sheets = match wb.sheets() {
            Ok(s) => s,
            Err(e) => {
                println!("{path}\tSHEETS_ERROR\t{e}");
                continue;
            }
        };

        println!("{path}\tsheets={}", sheets.len());

        for sheet in &sheets {
            let merged = wb.merged_regions(sheet).unwrap_or_default();
            let figures = wb.figures(sheet).unwrap_or_default();

            let pics = figures
                .iter()
                .filter(|f| f.kind == FigureKind::Picture)
                .count();
            let shapes = figures
                .iter()
                .filter(|f| f.kind == FigureKind::Shape)
                .count();
            let charts = figures
                .iter()
                .filter(|f| f.kind == FigureKind::Chart)
                .count();

            println!(
                "  [{}] merged={} figures={} (pic={} sp={} chart={})",
                sheet.name,
                merged.len(),
                figures.len(),
                pics,
                shapes,
                charts
            );

            for m in merged.iter().take(5) {
                println!(
                    "    merged {} anchor={} cells={}",
                    m.to_a1(),
                    m.anchor.to_a1(),
                    m.cell_count()
                );
            }
            for f in figures.iter().take(5) {
                let media = f
                    .media
                    .as_ref()
                    .map(|m| format!("{} {}B {}", m.part, m.bytes, &m.sha256[..12]))
                    .unwrap_or_default();
                println!(
                    "    {:?} at {}..{} {media}",
                    f.kind,
                    f.anchor.to_a1(),
                    f.to.map(|c| c.to_a1()).unwrap_or_else(|| "-".into())
                );
            }
        }
    }
}
