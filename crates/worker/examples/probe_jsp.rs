//! JSP が実際に解析できるかを測る。
//!
//! ```text
//! cargo run -p worker --example probe_jsp -- <file.jsp>...
//! ```
//!
//! 出す数字：
//! - `html_err`   マスク後の HTML 解析のエラーノード数
//! - `java_err`   scriptlet を合成したメソッドのエラーノード数
//! - `member_err` `<%! %>` を合成したクラス本体のエラーノード数
//! - `blocks`     JSP ブロック数、`el` は EL 式の数（Java ではないので別扱い）

use std::fs;

use worker::jsp;

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: probe_jsp <file.jsp>...");
        std::process::exit(2);
    }

    let mut clean = 0usize;
    let mut dirty = 0usize;
    let mut undetected = 0usize;
    let mut total_blocks = 0usize;

    for path in &paths {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{path}\tREAD_ERROR\t{e}");
                continue;
            }
        };

        // 実運用では Shift_JIS の JSP も来る。まず文字コードを通す。
        let Some(decoded) = worker::text::decide(&bytes) else {
            undetected += 1;
            println!("{path}\tENCODING_UNDETECTED");
            continue;
        };
        let src = decoded.text.as_str();

        let a = jsp::analyze(src);
        let el = src.matches("${").count();
        total_blocks += a.block_count;

        let ok = a.html.is_clean() && a.java.is_clean() && a.members.is_clean();
        if ok {
            clean += 1;
        } else {
            dirty += 1;
        }

        println!(
            "{path}\tenc={}\thtml_err={}\tjava_err={}\tmember_err={}\tblocks={}\tel={}",
            decoded.encoding.name(),
            a.html.errors + a.html.missing,
            a.java.errors + a.java.missing,
            a.members.errors + a.members.missing,
            a.block_count,
            el
        );

        if !a.java.is_clean() {
            if let Some((s, e)) = a.java.first_error {
                // 合成位置を元ファイルへ戻せているかを確認する。
                let mapped = jsp::map_range(&a.sources.statement_segments, s, e);
                let around = mapped
                    .and_then(|(s, e)| src.get(s as usize..e as usize))
                    .unwrap_or("<unmapped>");
                println!("    java first error at synth {s}..{e} -> {around:?}");
            }
        }
    }

    println!("\n--- summary ---");
    println!("files:              {}", paths.len());
    println!("clean:              {clean}");
    println!("with errors:        {dirty}");
    println!("undetected:         {undetected}");
    println!("jsp blocks:         {total_blocks}");
}
