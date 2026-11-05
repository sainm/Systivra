//! JSP の解析。
//!
//! 正本：`detail/06-code-analysis-engine.md` §4.3。
//!
//! JSP は HTML + 指令 + scriptlet の混在で、どちらの grammar でも単独では解析できない。
//! 方針は **等長プレースホルダ**：
//!
//! 1. JSP 構文（`<% %>` など）を**同じバイト長・同じ改行位置**の空白に置き換える
//! 2. 置き換えた本文を HTML grammar に渡す（バイト偏移がそのまま使える）
//! 3. scriptlet の中身だけを別途 Java grammar に渡し、位置を元に戻す
//!
//! 置換で長さか改行位置がずれたら、そのファイルの位置情報は使えない。
//! だから置換の不変条件はプロパティテストで守る（R-02）。

#![forbid(unsafe_code)]

pub mod parse;

pub use parse::{parse_html, parse_java, ParseStats};

/// scriptlet の種別。中身をどの grammar に渡すかが変わる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `<% ... %>` スクリプトレット（文の並び）
    Scriptlet,
    /// `<%= ... %>` 式（値を出力）
    Expression,
    /// `<%! ... %>` 宣言（クラスメンバ）
    Declaration,
    /// `<%@ ... %>` 指令（JSP の構文。Java ではない）
    Directive,
    /// `<%-- ... --%>` コメント（HTML に出さない）
    Comment,
}

impl Kind {
    /// 中身が Java として解析できるか。
    pub fn is_java(self) -> bool {
        matches!(self, Self::Scriptlet | Self::Expression | Self::Declaration)
    }
}

/// 元ファイル内の 1 つの JSP 構文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    /// `<%` の開始位置（元ファイルのバイト）
    pub start: u32,
    /// `%>` の直後（元ファイルのバイト）
    pub end: u32,
    /// 中身の開始位置（タグを除いた本文）
    pub body_start: u32,
    pub body_end: u32,
}

impl Block {
    pub fn body<'a>(&self, source: &'a str) -> &'a str {
        &source[self.body_start as usize..self.body_end as usize]
    }
}

/// マスク結果。
#[derive(Debug, Clone)]
pub struct Masked {
    /// 元と同じバイト長・同じ改行位置のテキスト。
    pub text: String,
    pub blocks: Vec<Block>,
}

impl Masked {
    /// 元とバイト長が同じであること。ここが崩れたら位置情報は全部無効。
    pub fn same_len(&self, original: &str) -> bool {
        self.text.len() == original.len()
    }
}

/// JSP 構文を空白（改行は保持）に置き換える。
///
/// バイト列を直接書き換えるのは、`char` 単位の処理だとバイト長がずれるから。
/// マーカーはすべて ASCII なので、UTF-8 の途中で切れることはない。
pub fn mask(source: &str) -> Masked {
    let mut buf = source.as_bytes().to_vec();
    let mut blocks = Vec::new();

    let mut i = 0usize;
    while i + 1 < buf.len() {
        if buf[i] != b'<' || buf[i + 1] != b'%' {
            i += 1;
            continue;
        }

        // 開始タグの判定。`<%--` を `<%` より先に見る。
        let (kind, open_len) = if starts_with(&buf, i, b"<%--") {
            (Kind::Comment, 4)
        } else if starts_with(&buf, i, b"<%@") {
            (Kind::Directive, 3)
        } else if starts_with(&buf, i, b"<%=") {
            (Kind::Expression, 3)
        } else if starts_with(&buf, i, b"<%!") {
            (Kind::Declaration, 3)
        } else {
            (Kind::Scriptlet, 2)
        };

        let body_start = i + open_len;
        let close: &[u8] = if kind == Kind::Comment {
            b"--%>"
        } else {
            b"%>"
        };

        let Some(close_at) = find_close(&buf, body_start, close, kind == Kind::Comment) else {
            // 閉じないタグ。ここで打ち切ると以降が全部ずれるので、この 1 つは無視して進む。
            i += 2;
            continue;
        };

        let end = close_at + close.len();
        blocks.push(Block {
            kind,
            start: i as u32,
            end: end as u32,
            body_start: body_start as u32,
            body_end: close_at as u32,
        });

        // 空白で埋める。改行だけは残す（行番号を保つため）。
        for b in &mut buf[i..end] {
            if *b != b'\n' && *b != b'\r' {
                *b = b' ';
            }
        }

        i = end;
    }

    Masked {
        // 置換後は ASCII 空白と元の UTF-8 が混ざるが、境界は必ず ASCII マーカー上なので
        // 不正なバイト列にはならない。
        text: String::from_utf8(buf).expect("マスクで UTF-8 が壊れた"),
        blocks,
    }
}

// ─────────────────────────────────────────────────────────────
// Java の合成
// ─────────────────────────────────────────────────────────────

/// 合成した Java ソースの一部が、元ファイルのどこに対応するか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// 合成ソース内の開始位置
    pub synth_start: u32,
    /// 元ファイル内の開始位置
    pub orig_start: u32,
    pub len: u32,
}

/// 合成結果。文の並び（メソッド本体）と、宣言（クラスメンバ）を分けて作る。
#[derive(Debug, Clone)]
pub struct Sources {
    pub statements: String,
    pub statement_segments: Vec<Segment>,
    pub members: String,
    pub member_segments: Vec<Segment>,
}

/// scriptlet を順に連結して 1 つのメソッドに合成する。
///
/// JSP エンジン（Jasper）がやっていることと同じ。個別に解析すると括弧が合わない。
/// `<%= %>` は式なので `Object __eN = (expr);` に包んで文にする。
pub fn synthesize(source: &str, blocks: &[Block]) -> Sources {
    let mut statements = String::from(parse::JAVA_STMT_PREFIX);
    let mut statement_segments = Vec::new();
    let mut members = String::from(parse::JAVA_MEMBER_PREFIX);
    let mut member_segments = Vec::new();
    let mut expr_no = 0usize;

    for b in blocks {
        let body = b.body(source);
        let len = b.body_end - b.body_start;

        match b.kind {
            Kind::Scriptlet => {
                statement_segments.push(Segment {
                    synth_start: statements.len() as u32,
                    orig_start: b.body_start,
                    len,
                });
                statements.push_str(body);
                // 断片どうしが token として癒着しないよう区切る。
                statements.push('\n');
            }
            Kind::Expression => {
                expr_no += 1;
                statements.push_str("Object __e");
                statements.push_str(&expr_no.to_string());
                statements.push_str(" = (");
                statement_segments.push(Segment {
                    synth_start: statements.len() as u32,
                    orig_start: b.body_start,
                    len,
                });
                statements.push_str(body);
                statements.push_str(");\n");
            }
            Kind::Declaration => {
                member_segments.push(Segment {
                    synth_start: members.len() as u32,
                    orig_start: b.body_start,
                    len,
                });
                members.push_str(body);
                members.push('\n');
            }
            Kind::Directive | Kind::Comment => {}
        }
    }

    statements.push_str(parse::JAVA_STMT_SUFFIX);
    members.push_str(parse::JAVA_MEMBER_SUFFIX);

    Sources {
        statements,
        statement_segments,
        members,
        member_segments,
    }
}

/// 合成ソースの範囲を、元ファイルのバイト範囲に戻す。
///
/// 両端が別のセグメントにあってもよい（`<% if { %>` 〜 `<% } %>` のような
/// ブロックは、間の HTML ごと 1 つの範囲になる）。戻せなければ `None`。
pub fn map_range(segments: &[Segment], synth_start: usize, synth_end: usize) -> Option<(u32, u32)> {
    let start = map_offset(segments, synth_start)?;
    let end = map_offset(segments, synth_end)?;
    (end >= start).then_some((start, end))
}

fn map_offset(segments: &[Segment], offset: usize) -> Option<u32> {
    let o = offset as u32;
    for seg in segments {
        let seg_end = seg.synth_start + seg.len;
        if o >= seg.synth_start && o <= seg_end {
            return Some(seg.orig_start + (o - seg.synth_start));
        }
    }
    None
}

/// 解析のまとめ。
#[derive(Debug, Clone)]
pub struct Analyzed {
    pub html: ParseStats,
    pub java: ParseStats,
    pub members: ParseStats,
    pub sources: Sources,
    pub html_text: String,
    pub block_count: usize,
}

/// マスク → HTML 解析 → Java 合成 → Java 解析、を 1 回で行う。
pub fn analyze(source: &str) -> Analyzed {
    let masked = mask(source);
    let html = parse::parse_html(&masked.text);
    let sources = synthesize(source, &masked.blocks);
    let java = parse::parse_java(&sources.statements);
    let members = parse::parse_java(&sources.members);

    Analyzed {
        html,
        java,
        members,
        sources,
        html_text: masked.text,
        block_count: masked.blocks.len(),
    }
}

fn starts_with(buf: &[u8], at: usize, pat: &[u8]) -> bool {
    buf.len() >= at + pat.len() && &buf[at..at + pat.len()] == pat
}

/// 閉じタグを探す。Java の文字列・文字リテラルの中の `%>` を拾わない。
///
/// `<% String s = "%>"; %>` のようなコードは実在する。素朴に `%>` を探すと
/// そこで切れて、以降の対応が全部ずれる。
fn find_close(buf: &[u8], from: usize, close: &[u8], is_comment: bool) -> Option<usize> {
    let mut i = from;
    let mut quote: Option<u8> = None;
    let mut escaped = false;

    while i < buf.len() {
        let b = buf[i];

        if is_comment {
            if b == b'-' && starts_with(buf, i, close) {
                return Some(i);
            }
            i += 1;
            continue;
        }

        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == q {
                    quote = None;
                }
            }
            None => {
                if b == b'"' || b == b'\'' {
                    quote = Some(b);
                } else if b == b'%' && starts_with(buf, i, close) {
                    return Some(i);
                }
            }
        }
        i += 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = r#"<%@ page contentType="text/html; charset=Windows-31J" %>
<html>
<body>
<% if (user != null) { %>
  <div>こんにちは</div>
<% } %>
</body>
</html>
"#;

    #[test]
    fn masking_keeps_length_and_newlines() {
        let m = mask(SIMPLE);
        assert_eq!(m.text.len(), SIMPLE.len(), "バイト長が変わっている");
        assert_eq!(
            newline_positions(&m.text),
            newline_positions(SIMPLE),
            "改行位置が変わっている"
        );
    }

    #[test]
    fn masking_removes_jsp_markup_from_html() {
        let m = mask(SIMPLE);
        assert!(!m.text.contains("<%"), "JSP タグが残っている");
        assert!(m.text.contains("<div>こんにちは</div>"), "本文が消えている");
    }

    #[test]
    fn finds_all_block_kinds() {
        let src = "<%@ page x %>\n<%-- c --%>\n<%! int n; %>\n<%= n %>\n<% n++; %>";
        let m = mask(src);
        let kinds: Vec<Kind> = m.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                Kind::Directive,
                Kind::Comment,
                Kind::Declaration,
                Kind::Expression,
                Kind::Scriptlet
            ]
        );
    }

    #[test]
    fn body_extraction_matches_source() {
        let m = mask(SIMPLE);
        let bodies: Vec<&str> = m.blocks.iter().map(|b| b.body(SIMPLE).trim()).collect();
        assert_eq!(
            bodies,
            vec![
                r#"page contentType="text/html; charset=Windows-31J""#,
                "if (user != null) {",
                "}"
            ]
        );
    }

    #[test]
    fn percent_gt_inside_java_string_does_not_close() {
        // これが壊れると、以降のブロックが全部ずれる。
        let src = r#"<% String s = "%>"; int x = 1; %>after"#;
        let m = mask(src);
        assert_eq!(m.blocks.len(), 1);
        assert_eq!(
            m.blocks[0].body(src).trim(),
            r#"String s = "%>"; int x = 1;"#
        );
        assert!(m.text.contains("after"));
    }

    #[test]
    fn char_literal_quote_does_not_confuse() {
        let src = r#"<% char c = '"'; String s = "a%>b"; %>tail"#;
        let m = mask(src);
        assert_eq!(m.blocks.len(), 1);
        assert!(m.text.ends_with("tail"));
    }

    #[test]
    fn unclosed_tag_is_skipped_without_losing_alignment() {
        let src = "<div>ok</div>\n<% never closed\n<p>still here</p>";
        let m = mask(src);
        assert_eq!(m.text.len(), src.len());
        assert!(m.text.contains("<p>still here</p>"), "後続が消えている");
    }

    #[test]
    fn multiline_scriptlet_keeps_line_count() {
        let src = "<%\n  int a = 1;\n  int b = 2;\n%>\n<div>x</div>";
        let m = mask(src);
        assert_eq!(newline_positions(&m.text), newline_positions(src));
        assert_eq!(m.text.lines().count(), src.lines().count());
    }

    #[test]
    fn japanese_text_inside_scriptlet_is_masked_safely() {
        // 日本語（マルチバイト）を含む scriptlet。バイト単位で潰すので途中で切れない。
        let src = "<% String 名前 = \"山田\"; %>\n<p>本文</p>";
        let m = mask(src);
        assert_eq!(m.text.len(), src.len());
        assert!(m.text.contains("<p>本文</p>"));
    }

    /// マスクが「位置を変えない」ことを、ランダム挿入で確かめる（R-02 の簡易版）。
    #[test]
    fn random_scriptlet_insertion_never_shifts_positions() {
        let template = "<html>\n<body>\n<div>固定</div>\n</body>\n</html>\n";
        let insert = "<% if (a > 0) { out.print(\"x\"); } %>";
        let mut seed = 12345u64;

        for _ in 0..200 {
            // 決定的な擬似乱数（rand クレートを足さない）。
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let pos = (seed >> 33) as usize % (template.len() + 1);
            let pos = floor_char_boundary(template, pos);

            let mut inserted = String::with_capacity(template.len() + insert.len());
            inserted.push_str(&template[..pos]);
            inserted.push_str(insert);
            inserted.push_str(&template[pos..]);

            let m = mask(&inserted);
            assert_eq!(m.text.len(), inserted.len(), "バイト長が変わった pos={pos}");
            assert_eq!(
                newline_positions(&m.text),
                newline_positions(&inserted),
                "改行位置が変わった pos={pos}"
            );
            // 挿入位置より前は 1 バイトも動いていない。
            assert_eq!(&m.text[..pos], &inserted[..pos], "挿入位置より前が動いた");
            // 挿入位置より後ろは、挿入した長さだけ後ろにずれている。
            assert_eq!(
                &m.text[pos + insert.len()..],
                &inserted[pos + insert.len()..],
                "挿入位置より後ろが動いた"
            );
        }
    }

    #[test]
    fn fragmented_blocks_only_parse_after_synthesis() {
        // これが JSP の本質。`<% if { %>` と `<% } %>` は別ブロックで、
        // 単独では括弧が合わない。合成して初めて 1 つのメソッドになる。
        let src = "<% if (a > 0) { %>\n<div>x</div>\n<% } %>";
        let a = analyze(src);
        assert!(
            a.java.is_clean(),
            "合成しても解析できない: {:?}",
            a.java.first_error
        );
        assert!(a.html.is_clean());
    }

    #[test]
    fn bare_expression_is_wrapped_as_statement() {
        // `<%= expr %>` は式であって文ではない。そのまま並べると Java として不正。
        let src = "<div><%= request.getContextPath() %></div>";
        let a = analyze(src);
        assert!(a.java.is_clean(), "{:?}", a.java.first_error);
    }

    #[test]
    fn declarations_go_to_class_body() {
        let src = "<%! private int n = 0; %>\n<div><%= n %></div>";
        let a = analyze(src);
        assert!(a.members.is_clean(), "宣言がクラス本体で通らない");
        assert!(a.java.is_clean(), "式がメソッド本体で通らない");
    }

    #[test]
    fn map_range_brings_positions_back_to_the_original_file() {
        let src = "<div>\n<%= user.getName() %>\n</div>";
        let a = analyze(src);

        // 合成ソース内で式の本文を探し、元ファイルの位置に戻す。
        let seg = &a.sources.statement_segments[0];
        let mapped = map_range(
            &a.sources.statement_segments,
            seg.synth_start as usize,
            (seg.synth_start + seg.len) as usize,
        )
        .expect("位置が戻せない");

        assert_eq!(
            &src[mapped.0 as usize..mapped.1 as usize],
            " user.getName() ",
            "戻した範囲が元ファイルの式と一致しない"
        );
    }

    #[test]
    fn synthesized_source_keeps_block_order() {
        let src = "<% a(); %><% b(); %><% c(); %>";
        let a = analyze(src);
        let ia = a.sources.statements.find("a()").unwrap();
        let ib = a.sources.statements.find("b()").unwrap();
        let ic = a.sources.statements.find("c()").unwrap();
        assert!(ia < ib && ib < ic, "合成順が崩れている");
    }

    fn newline_positions(s: &str) -> Vec<usize> {
        s.bytes()
            .enumerate()
            .filter(|(_, b)| *b == b'\n')
            .map(|(i, _)| i)
            .collect()
    }

    fn floor_char_boundary(s: &str, mut i: usize) -> usize {
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}
