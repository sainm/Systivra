//! 原文バイト偏移の構築（Phase 4 共通基盤）。
//!
//! `detail/06` の要求：`byte_start / byte_end` は**元ファイルのバイト偏移**であり、
//! UTF-8 変換後の偏移ではない。`line_offsets` も同じ基準。
//!
//! 実装方針：**往復で証明する。**
//! 復号したテキストを元のエンコーディングで符号化し直し、元のバイト列と
//! 完全に一致したときだけ偏移を返す。一致しなければ「偏移は出せない」と
//! 明示する（`exact = false`）。もっともらしい数字を返すより、返さないほうがよい。
//! 数字は証拠の同一性判定に使われるので、間違っていると静かに壊れる。

use encoding_rs::Encoding;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    /// 復号に失敗（不正なバイト列）。判定したエンコーディングが違う可能性が高い。
    #[error("復号に失敗（had_errors）。判定した文字コードが誤っている可能性がある: {0:?}")]
    Lossy(&'static str),

    /// 対応していない（状態を持つエンコーディングなど、偏移を保証できない）。
    #[error("この文字コードの偏移は未対応: {0:?}")]
    Unsupported(&'static str),
}

/// 復号結果。
#[derive(Debug, Clone)]
pub struct DecodedFile {
    pub text: String,
    pub encoding: &'static Encoding,
    /// 元バイト列を完全に再現できたか。`false` なら偏移は空。
    pub exact: bool,
    /// BOM のバイト数（元ファイル先頭）。偏移は BOM を含む絶対位置。
    pub bom_len: usize,
    /// 各行の開始バイト位置（元ファイル基準、BOM 込み）。`exact` のときだけ入る。
    lines: LineIndex,
    /// 各文字の開始バイト位置（元ファイル基準）。長さは `chars + 1`。
    /// 最後の要素はテキスト末尾の次バイト（＝ファイル長）。
    char_starts: Vec<u32>,
}

impl DecodedFile {
    /// 「この行の、この文字位置」を元ファイルのバイト範囲に直す。
    ///
    /// `quote` を原文で切り出すときに使う。範囲が取れなければ `None`。
    pub fn byte_range_of(&self, char_start: usize, char_len: usize) -> Option<(u32, u32)> {
        if !self.exact {
            return None;
        }
        let start = *self.char_starts.get(char_start)?;
        let end = *self.char_starts.get(char_start + char_len)?;
        Some((start, end))
    }

    /// UTF-8 byte range (as reported by Tree-sitter) to original-file bytes.
    pub fn byte_range_of_utf8(&self, utf8_start: usize, utf8_end: usize) -> Option<(u32, u32)> {
        if !self.exact {
            return None;
        }
        let char_start = self.text.get(..utf8_start)?.chars().count();
        let char_end = self.text.get(..utf8_end)?.chars().count();
        self.byte_range_of(char_start, char_end - char_start)
    }

    pub fn line_column_of_utf8(&self, utf8_offset: usize) -> Option<(u32, u32)> {
        if !self.exact {
            return None;
        }
        let byte = *self
            .char_starts
            .get(self.text.get(..utf8_offset)?.chars().count())?;
        let line = self.lines.line_of(byte);
        let line_start = self
            .lines
            .starts
            .get(line.saturating_sub(1) as usize)
            .copied()?;
        Some((line, byte.saturating_sub(line_start) + 1))
    }

    pub fn raw_byte_of_utf8(&self, utf8_offset: usize) -> Option<u32> {
        if !self.exact {
            return None;
        }
        self.char_starts
            .get(self.text.get(..utf8_offset)?.chars().count())
            .copied()
    }

    pub fn is_exact(&self) -> bool {
        self.exact
    }

    pub fn line_index(&self) -> &LineIndex {
        &self.lines
    }

    /// 文字インデックス → 行番号（1 始まり）。偏移が無ければ `None`。
    pub fn line_of_char(&self, char_start: usize) -> Option<u32> {
        if !self.exact {
            return None;
        }
        let byte = *self.char_starts.get(char_start)?;
        Some(self.lines.line_of(byte))
    }

    /// 元ファイルのバイト範囲を、復号済みテキストから切り出す。
    ///
    /// 偏移が証明できていない場合は `None`（呼び出し側は `quote` を作らない）。
    pub fn quote_at(&self, char_start: usize, char_len: usize) -> Option<&str> {
        let start = self.text.char_indices().nth(char_start)?.0;
        let end = self
            .text
            .char_indices()
            .nth(char_start + char_len)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.get(start..end)
    }
}

/// 行頭のバイト位置。二分探索で行番号を出す。
#[derive(Debug, Clone, Default)]
pub struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    pub fn starts(&self) -> &[u32] {
        &self.starts
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    /// バイト位置が何行目か（1 始まり）。範囲外は最終行。
    pub fn line_of(&self, byte: u32) -> u32 {
        match self.starts.binary_search(&byte) {
            Ok(i) => (i + 1) as u32,
            Err(0) => 1,
            Err(i) => i as u32,
        }
    }
}

/// 偏移を保証できるエンコーディングか。
///
/// 状態を持つもの（ISO-2022-JP など）は、文字ごとのバイト長が文脈に依存するため
/// 単純な対応表を作れない。ここでは扱わない。
fn supports_exact_offsets(enc: &'static Encoding) -> bool {
    use encoding_rs::{EUC_JP, SHIFT_JIS, UTF_16BE, UTF_16LE, UTF_8};
    enc == UTF_8 || enc == SHIFT_JIS || enc == EUC_JP || enc == UTF_16BE || enc == UTF_16LE
}

/// 復号して、偏移が証明できれば一緒に返す。
pub fn decode(bytes: &[u8], enc: &'static Encoding) -> Result<DecodedFile, DecodeError> {
    if !supports_exact_offsets(enc) {
        return Err(DecodeError::Unsupported(enc.name()));
    }

    let bom_len = Encoding::for_bom(bytes).map(|(_, n)| n).unwrap_or(0);
    let body = &bytes[bom_len..];

    let (text, had_errors) = enc.decode_without_bom_handling(body);
    if had_errors {
        return Err(DecodeError::Lossy(enc.name()));
    }
    let text = text.into_owned();

    // 復号済みの文字列を、元のバイト列と並べて歩く。
    // 1 文字ずつ「元ファイルで何バイト使っているか」を確定させるので、
    // 重複写像（CP932 の 髙 など）があっても偏移が崩れない。
    let index = walk_bytes(&text, body, enc, bom_len);
    let exact = index.is_some();
    let (lines, char_starts) = index.unwrap_or_default();

    Ok(DecodedFile {
        text,
        encoding: enc,
        exact,
        bom_len,
        lines,
        char_starts,
    })
}

/// 元バイト列を先頭から歩き、文字ごとの開始位置を確定する。
///
/// 途中で「この文字は元ファイルで何バイトか」が決められなければ `None`。
/// **推測で埋めない。** 1 文字でも決められないなら、そのファイルの偏移は
/// 一切出さない（证据が別の場所を指すより、出さないほうがよい）。
fn walk_bytes(
    text: &str,
    body: &[u8],
    enc: &'static Encoding,
    bom_len: usize,
) -> Option<(LineIndex, Vec<u32>)> {
    let mut char_starts = Vec::with_capacity(text.chars().count() + 1);
    let mut line_starts = vec![bom_len as u32];
    let mut pos = 0usize;

    char_starts.push(bom_len as u32);

    for ch in text.chars() {
        let len = char_len_at(body, pos, ch, enc)?;
        pos += len;
        char_starts.push((bom_len + pos) as u32);
        if ch == '\n' {
            line_starts.push((bom_len + pos) as u32);
        }
    }

    // 最後まで歩いてバイト数が合わなければ、対応がずれている。
    if pos != body.len() {
        return None;
    }

    Some((
        LineIndex {
            starts: line_starts,
        },
        char_starts,
    ))
}

/// 1 文字が元バイト列で何バイト占めるかを決める。
///
/// 速い道：正規の符号化結果と実際のバイト列が一致すれば、それが答え。
/// 遅い道：一致しない場合（CP932 の重複写像など）、実際のバイト列を
/// 復号して同じ文字になる長さを探す。
fn char_len_at(body: &[u8], pos: usize, ch: char, enc: &'static Encoding) -> Option<usize> {
    let mut buf = [0u8; 4];
    let ch_str = ch.encode_utf8(&mut buf);
    let (canonical, _, unmappable) = enc.encode(ch_str);

    if !unmappable && !canonical.is_empty() {
        let n = canonical.len();
        if body.get(pos..pos + n) == Some(canonical.as_ref()) {
            return Some(n);
        }
    }

    // 重複写像・外字。実際のバイト列を復号して確かめる。
    // 1 バイトから順に試すのは、リードバイトの表を自前で持たないため。
    for n in 1..=4usize {
        let slice = body.get(pos..pos + n)?;
        let (decoded, had_errors) = enc.decode_without_bom_handling(slice);
        if !had_errors && decoded.chars().count() == 1 && decoded.starts_with(ch) {
            return Some(n);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::{EUC_JP, SHIFT_JIS, UTF_8};

    const SRC: &str =
        "public class ユーザ編集 {\n  // 住所は全角20文字以内\n  void update() {}\n}\n";

    fn encoded(text: &str, enc: &'static Encoding) -> Vec<u8> {
        let (b, _, unmappable) = enc.encode(text);
        assert!(!unmappable);
        b.into_owned()
    }

    #[test]
    fn shift_jis_offsets_are_original_bytes_not_utf8_bytes() {
        let bytes = encoded(SRC, SHIFT_JIS);
        let d = decode(&bytes, SHIFT_JIS).unwrap();

        assert!(d.exact);
        assert_eq!(d.text, SRC);

        // 2 行目の開始は、UTF-8 のバイト数ではなく元ファイルのバイト位置。
        let utf8_len_of_line1 = SRC.lines().next().unwrap().len() + 1; // + '\n'
        let sjis_line2 = d.line_index().starts()[1];
        assert_ne!(
            sjis_line2 as usize, utf8_len_of_line1,
            "Shift_JIS の行頭が UTF-8 のバイト数になっている"
        );

        // 原文を切り出すと、ちゃんと元のバイト列に一致する。
        let (s, e) = d.byte_range_of(0, 5).unwrap();
        assert_eq!(&bytes[s as usize..e as usize], b"publi");
    }

    #[test]
    fn utf8_offsets_still_work() {
        let bytes = SRC.as_bytes().to_vec();
        let d = decode(&bytes, UTF_8).unwrap();
        assert!(d.exact);
        assert_eq!(d.line_index().starts()[0], 0);
        let (s, e) = d.byte_range_of(20, 6).unwrap();
        let ch: String = SRC.chars().skip(20).take(6).collect();
        assert_eq!(&bytes[s as usize..e as usize], ch.as_bytes());
    }

    #[test]
    fn every_char_offset_maps_back_to_its_bytes() {
        for enc in [SHIFT_JIS, EUC_JP, UTF_8] {
            let bytes = encoded(SRC, enc);
            let d = decode(&bytes, enc).unwrap();
            assert!(d.exact, "{:?}", enc);

            for (i, ch) in SRC.chars().enumerate() {
                let (s, e) = d.byte_range_of(i, 1).unwrap();
                let slice = &bytes[s as usize..e as usize];
                let (decoded, had_err) = enc.decode_without_bom_handling(slice);
                assert!(!had_err);
                assert_eq!(decoded, ch.to_string(), "{:?} の {i} 文字目", enc);
            }
        }
    }

    #[test]
    fn line_numbers_match_text_lines() {
        let bytes = encoded(SRC, SHIFT_JIS);
        let d = decode(&bytes, SHIFT_JIS).unwrap();

        for (i, line) in SRC.lines().enumerate() {
            let char_start: usize = SRC.lines().take(i).map(|l| l.chars().count() + 1).sum();
            assert_eq!(
                d.line_of_char(char_start),
                Some(i as u32 + 1),
                "{i} 行目の先頭"
            );
            let _ = line;
        }
    }

    #[test]
    fn lossy_decode_is_an_error_not_a_guess() {
        // UTF-8 として不正なバイト列を UTF-8 で復号する。
        let bad = vec![0x82, 0xA0, 0xFF, 0xFF];
        assert!(matches!(decode(&bad, UTF_8), Err(DecodeError::Lossy(_))));
    }

    #[test]
    fn stateful_encoding_is_refused() {
        assert!(matches!(
            decode(b"abc", encoding_rs::ISO_2022_JP),
            Err(DecodeError::Unsupported(_))
        ));
    }

    #[test]
    fn bom_is_counted_in_offsets() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(SRC.as_bytes());
        let d = decode(&bytes, UTF_8).unwrap();

        assert_eq!(d.bom_len, 3);
        assert!(d.exact);
        // 先頭文字の位置は BOM の後ろ。
        assert_eq!(d.byte_range_of(0, 1).unwrap().0, 3);
        assert_eq!(d.line_index().starts()[0], 3);
    }

    #[test]
    fn duplicate_cp932_mappings_are_handled_not_rejected() {
        // CP932 の IBM 拡張（0xED40〜0xEEFC）には、同じ Unicode に写像される
        // 重複バイト列がある。素朴に「復号 → 再符号化」で全体を比べると
        // 別のバイト列に戻り、偏移を諦めてしまう。
        //
        // 実際、髙 のような外字は日本の業務システムに普通に入っているので、
        // これは「めったに起きない例外」ではない。逐語走査なら元のバイト数を
        // そのまま採れるので、この場合は exact = true になる。
        let mut found = None;
        'outer: for hi in 0xED..=0xEEu8 {
            for lo in 0x40..=0xFCu8 {
                let pair = [hi, lo];
                let (text, had_errors) = SHIFT_JIS.decode_without_bom_handling(&pair);
                if had_errors || text.chars().count() != 1 {
                    continue;
                }
                let (back, _, unmappable) = SHIFT_JIS.encode(&text);
                if !unmappable && back.as_ref() != pair {
                    found = Some((pair, text.into_owned()));
                    break 'outer;
                }
            }
        }

        let (pair, expected_text) = found.expect("CP932 の重複写像が見つからない（前提が崩れた）");

        let d = decode(&pair, SHIFT_JIS).unwrap();
        assert_eq!(d.text, expected_text, "復号結果は同じ文字");
        assert!(d.exact, "重複写像でも偏移を出せること");
        assert_eq!(d.byte_range_of(0, 1), Some((0, 2)));
    }

    #[test]
    fn common_gaiji_keeps_offsets() {
        // 髙 は日本の人名で頻出。ここで偏移が落ちると実案件で使えない。
        let src = "String 名 = \"髙\";\nString 名2 = \"﨑\";\n";
        let (bytes, _, unmappable) = SHIFT_JIS.encode(src);
        assert!(!unmappable, "前提：髙 と 﨑 は CP932 で表現できる");

        let d = decode(&bytes, SHIFT_JIS).unwrap();
        assert!(
            d.exact,
            "外字を含むと偏移が出せない、という状態になっている"
        );

        // 2 行目の先頭が正しいバイト位置を指しているか、実際に切り出して確かめる。
        let line2_start = d.line_index().starts()[1] as usize;
        let (back, had) = SHIFT_JIS.decode_without_bom_handling(&bytes[line2_start..]);
        assert!(!had);
        assert!(back.starts_with("String 名2"));
    }

    #[test]
    fn tree_sitter_utf8_offsets_map_to_original_bytes() {
        let bytes = encoded(SRC, SHIFT_JIS);
        let d = decode(&bytes, SHIFT_JIS).unwrap();
        let utf8_start = SRC.find("ユーザ").unwrap();
        let (start, end) = d
            .byte_range_of_utf8(utf8_start, utf8_start + "ユーザ".len())
            .unwrap();
        let (decoded, had_errors) =
            SHIFT_JIS.decode_without_bom_handling(&bytes[start as usize..end as usize]);
        assert!(!had_errors);
        assert_eq!(decoded, "ユーザ");
        assert_eq!(d.line_column_of_utf8(utf8_start).unwrap().0, 1);
    }
}
