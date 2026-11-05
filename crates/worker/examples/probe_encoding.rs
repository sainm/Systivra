//! 文字コード判定と偏移の実地確認。
//!
//! ```text
//! cargo run -p worker --example probe_encoding -- <file>...
//! ```
//!
//! 出力は「判定 / 往復一致 / 行数 / 先頭行の引用が原文と一致するか」。
//! Phase 0 の技術検証（Shift_JIS 判定とバイト偏移）で使う。

use std::fs;

use worker::text::{decide, DecodedFile};

/// 判定 → 復号 → 偏移の証明までを 1 回でやる。
///
/// `decide` は判定と復号をまとめた入口（信頼できなければ `None`）。
fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: probe_encoding <file>...");
        std::process::exit(2);
    }

    let mut ok = 0usize;
    let mut undetected = 0usize;
    let mut inexact = 0usize;

    for path in &paths {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{path}\tREAD_ERROR\t{e}");
                continue;
            }
        };

        match decide(&bytes) {
            None => {
                undetected += 1;
                println!("{path}\tENCODING_UNDETECTED\t{} bytes", bytes.len());
            }
            Some(d) => {
                if d.exact {
                    ok += 1;
                } else {
                    inexact += 1;
                }
                report(path, &bytes, &d);
            }
        }
    }

    println!("\n--- summary ---");
    println!("exact:        {ok}");
    println!("not exact:    {inexact}");
    println!("undetected:   {undetected}");
}

fn report(path: &str, original: &[u8], d: &DecodedFile) {
    let lines = d.line_index().starts().len();
    let chars = d.text.chars().count();

    // 先頭行の 1 文字目〜8 文字目を、元バイト列から切り出して照合する。
    let quote_check = match d.byte_range_of(0, 8) {
        Some((s, e)) if (e as usize) <= original.len() => {
            let slice = &original[s as usize..e as usize];
            // 元バイト列を同じエンコーディングで戻すと、テキストの先頭 8 文字になるはず。
            let (back, had_errors) = d.encoding.decode_without_bom_handling(slice);
            let expected: String = d.text.chars().take(8).collect();
            if had_errors {
                "DECODE_ERROR"
            } else if back == expected {
                "ok"
            } else {
                "MISMATCH"
            }
        }
        _ => "NO_RANGE",
    };

    let mut note = String::new();
    if !d.exact {
        // どこで往復が崩れたかを出す。原因（重複写像・外字・未対応文字）を
        // 特定できないと、現場で「偏移が出ない」理由を説明できない。
        let (back, _, unmappable) = d.encoding.encode(&d.text);
        let back = back.into_owned();
        let first = back
            .iter()
            .zip(original.iter())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| back.len().min(original.len()));
        let lo = first.saturating_sub(8);
        let hi = (first + 8).min(original.len());
        let ch_lo = d.text.chars().take(first).count();
        let around: String = d
            .text
            .chars()
            .skip(ch_lo.saturating_sub(4))
            .take(8)
            .collect();
        note = format!(
            "\tdiverged_at={first}\torig={:02X?}\tchars_around={around:?}\tunmappable={unmappable}",
            &original[lo..hi]
        );
    }

    println!(
        "{path}\tenc={}\texact={}\tlines={}\tchars={}\tquote={}{}",
        d.encoding.name(),
        d.exact,
        lines,
        chars,
        quote_check,
        note
    );
}
