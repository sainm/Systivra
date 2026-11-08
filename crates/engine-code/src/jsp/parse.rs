//! Tree-sitter による解析と、その品質の測り方（Phase 4 共通基盤）。
//!
//! 「解析できた」を主観で判定しない。**エラーノードの数**で測る。
//! 1 つでもエラーがあれば、その範囲の Fact は `resolution = PARTIAL` に落とす
//! 判断材料になる（`detail/06` §4.3）。

use tree_sitter::{Node, Parser};

/// scriptlet の本文を「メソッド本体」として解析するための前置き。
///
/// JSP の scriptlet は文の並びであってコンパイル単位ではない。そのまま
/// Java grammar に渡すと必ず失敗するので、最小のクラスで包む。
/// **前置きの長さは固定**でなければならない（位置を戻せなくなる）。
pub const JAVA_STMT_PREFIX: &str = "class J{void m(){";
pub const JAVA_STMT_SUFFIX: &str = "}}";

/// `<%! %>` 宣言はクラスメンバ。包み方が違う。
pub const JAVA_MEMBER_PREFIX: &str = "class J{";
pub const JAVA_MEMBER_SUFFIX: &str = "}";

/// 解析結果の粗さ。ノード数を分母にすれば「どれくらい壊れているか」が分かる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseStats {
    pub nodes: usize,
    pub errors: usize,
    pub missing: usize,
    /// 最初のエラー範囲（元テキスト基準）。前置きを足した場合は差し引かない。
    pub first_error: Option<(usize, usize)>,
}

impl ParseStats {
    pub fn is_clean(&self) -> bool {
        self.errors == 0 && self.missing == 0
    }

    /// エラーの割合。ノードが 0 のときは 0。
    pub fn error_ratio(&self) -> f32 {
        if self.nodes == 0 {
            return 0.0;
        }
        (self.errors + self.missing) as f32 / self.nodes as f32
    }
}

fn language_html() -> tree_sitter::Language {
    tree_sitter_html::LANGUAGE.into()
}

fn language_java() -> tree_sitter::Language {
    tree_sitter_java::LANGUAGE.into()
}

/// HTML として解析する。JSP 構文は事前に `jsp::mask` で潰しておくこと。
pub fn parse_html(src: &str) -> ParseStats {
    parse(src, language_html())
}

/// 合成済みの Java ソースを解析する。
///
/// **scriptlet を 1 つずつ解析してはならない。** JSP の scriptlet は断片である：
/// `<% if (a) { %>` と `<% } %>` は別々のブロックで、単独では括弧が合わない。
/// `<%= expr %>` は式であって文ではない。
/// Jasper などの JSP エンジンと同じく、**順に連結して 1 つのメソッドに合成**してから
/// 解析する。位置は `Segment` で元ファイルへ戻す。
pub fn parse_java(src: &str) -> ParseStats {
    parse(src, language_java())
}

fn parse(src: &str, language: tree_sitter::Language) -> ParseStats {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("tree-sitter の言語と ABI が合わない");

    let Some(tree) = parser.parse(src, None) else {
        return ParseStats {
            nodes: 0,
            errors: 1,
            missing: 0,
            first_error: None,
        };
    };

    let root = tree.root_node();
    let mut stats = ParseStats {
        nodes: 0,
        errors: 0,
        missing: 0,
        first_error: None,
    };
    walk(root, &mut stats);
    stats
}

fn walk(node: Node<'_>, stats: &mut ParseStats) {
    stats.nodes += 1;

    if node.is_error() {
        stats.errors += 1;
    }
    if node.is_missing() {
        stats.missing += 1;
    }
    if (node.is_error() || node.is_missing()) && stats.first_error.is_none() {
        stats.first_error = Some((node.start_byte(), node.end_byte()));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, stats);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsp;

    fn stmts(body: &str) -> ParseStats {
        parse_java(&format!("{JAVA_STMT_PREFIX}{body}{JAVA_STMT_SUFFIX}"))
    }

    fn members(body: &str) -> ParseStats {
        parse_java(&format!("{JAVA_MEMBER_PREFIX}{body}{JAVA_MEMBER_SUFFIX}"))
    }

    #[test]
    fn html_without_jsp_parses_clean() {
        let s = "<html><body><div class=\"a\">本文</div></body></html>";
        let st = parse_html(s);
        assert!(st.is_clean(), "{st:?}");
    }

    #[test]
    fn masked_jsp_parses_as_html() {
        let src = r#"<%@ page contentType="text/html; charset=Windows-31J" %>
<html>
<body>
<% if (user != null) { %>
  <div>こんにちは</div>
<% } %>
</body>
</html>
"#;
        let m = jsp::mask(src);
        let st = parse_html(&m.text);
        assert!(st.is_clean(), "マスク後の HTML にエラー: {st:?}");
    }

    #[test]
    fn unmasked_jsp_does_not_parse_cleanly() {
        // マスクが必要であることの証拠。これが clean ならマスクは不要になる。
        let src = "<html><body><% if (x) { %><div>a</div><% } %></body></html>";
        let st = parse_html(src);
        assert!(
            !st.is_clean(),
            "JSP をそのまま HTML に渡して通ってしまった（前提が変わった）"
        );
    }

    #[test]
    fn custom_tag_names_do_not_break_html() {
        // JSTL の <c:forEach> や <jsp:include> はタグ名にコロンを含む。
        let s = "<html><body><c:forEach var=\"u\" items=\"${users}\"><div>x</div></c:forEach></body></html>";
        let st = parse_html(s);
        assert!(st.is_clean(), "カスタムタグで落ちている: {st:?}");
    }

    #[test]
    fn plain_java_statements_parse_clean() {
        let st = stmts("int a = 1; if (a > 0) { a++; }");
        assert!(st.is_clean(), "{st:?}");
    }

    #[test]
    fn java_with_japanese_identifiers_parses() {
        let st = stmts("String 名前 = \"山田\"; 名前 = 名前.trim();");
        assert!(st.is_clean(), "日本語識別子で落ちている: {st:?}");
    }

    #[test]
    fn el_expression_is_not_java() {
        // `${user.name}` は JSP の EL。Java として解析すると失敗するのが正しい。
        let st = stmts("${user.name}");
        assert!(!st.is_clean(), "EL を Java として通してしまっている");
    }

    #[test]
    fn broken_java_is_reported() {
        let st = stmts("if (a > 0 { ");
        assert!(!st.is_clean());
        assert!(st.errors + st.missing > 0);
    }

    #[test]
    fn member_declaration_parses_in_class_body() {
        let st = members("private int count = 0; public String name;");
        assert!(st.is_clean(), "{st:?}");
    }

    #[test]
    fn statement_inside_class_body_is_not_valid_member() {
        // `<%! %>` に文を書くのは誤り。検出できることを確かめる。
        let st = members("int a = 1; a++;");
        assert!(!st.is_clean(), "メンバでない文を通してしまっている");
    }
}
