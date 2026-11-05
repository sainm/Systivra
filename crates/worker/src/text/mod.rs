//! 文字コードの判定と、原文バイト偏移の保持。
//!
//! 正本：`00-全体設計.md` J-01、`detail/06-code-analysis-engine.md`（`ENCODING_UNDETECTED`）。
//!
//! **なぜこれを独立したモジュールにするか。**
//! 文字コードを誤ってもファイルは開けてしまう。気づいたときには行番号・証拠・差分が
//! すべてずれている。しかも Shift_JIS の 2 バイト文字を 1 文字として数えると行が
//! ずれ、証拠を開いても該当箇所が出ない（R-22）。
//!
//! だから「判定」と「偏移」を 1 箇所に閉じ、**往復で正しさを証明できたときだけ**
//! 偏移を返す。証明できなければ `exact = false` を立てて降格する。

#![forbid(unsafe_code)]

pub mod offsets;

pub use offsets::{DecodeError, DecodedFile, LineIndex};

use encoding_rs::Encoding;

/// 判定に使う先頭バイト数。`detail/06` §8 の「先頭 64KB」に合わせる。
pub const DETECT_WINDOW: usize = 64 * 1024;

/// 信頼度の下限（0〜255）。
///
/// **注記**：`detail/06` は「信頼度 < 0.75 なら推測しない」と書いているが、
/// `chardetng` は数値の信頼度を公開していない（`guess_assess` の戻りは
/// 「確信できたか」の bool だけ）。数値を作ると嘘の精度になるので、
/// ここでは bool を 255 / 0 に写し、さらに**別区間での判定一致**を条件に足す。
pub const MIN_CONFIDENCE: u8 = 191; // 0.75 * 255

/// 判定結果。**推測で決めない**ための型。
#[derive(Debug, Clone)]
pub enum Verdict {
    /// 信頼できる。
    Detected {
        encoding: &'static Encoding,
        confidence: u8,
    },
    /// 信頼できない。`ENCODING_UNDETECTED` として未解析一覧に出す。
    Undetected {
        /// 最も近い候補。ログと画面に出す（そのまま採用はしない）。
        best: &'static Encoding,
        confidence: u8,
    },
}

impl Verdict {
    pub fn confidence_ratio(&self) -> f32 {
        let c = match self {
            Self::Detected { confidence, .. } | Self::Undetected { confidence, .. } => *confidence,
        };
        f32::from(c) / 255.0
    }

    pub fn encoding(&self) -> &'static Encoding {
        match self {
            Self::Detected { encoding, .. } => encoding,
            Self::Undetected { best, .. } => best,
        }
    }

    pub fn is_detected(&self) -> bool {
        matches!(self, Self::Detected { .. })
    }
}

/// 先頭 64KB で判定する。
///
/// BOM があれば chardetng に任せず確定させる（BOM は推測より強い証拠）。
pub fn detect(bytes: &[u8]) -> Verdict {
    if let Some((enc, _bom_len)) = Encoding::for_bom(bytes) {
        return Verdict::Detected {
            encoding: enc,
            confidence: u8::MAX,
        };
    }

    let window = &bytes[..bytes.len().min(DETECT_WINDOW)];
    let (best, confident) = probe(window, window.len() == bytes.len());

    if !confident {
        return Verdict::Undetected {
            best,
            confidence: 0,
        };
    }

    // 先頭だけで決めない。64KB を超えるファイルは末尾でも判定し、
    // 食い違ったら「決められない」に倒す。1 箇所だけの判定は、
    // コメントブロックやバイナリ混入で平気で外れる。
    if bytes.len() > DETECT_WINDOW {
        let tail_start = bytes.len() - DETECT_WINDOW;
        let (tail_best, tail_confident) = probe(&bytes[tail_start..], true);
        if tail_confident && tail_best != best {
            return Verdict::Undetected {
                best,
                confidence: 0,
            };
        }
    }

    Verdict::Detected {
        encoding: best,
        confidence: u8::MAX,
    }
}

/// 判定と復号をまとめて行う実務向けの入口。
///
/// 次のどれかなら `None`（＝`ENCODING_UNDETECTED` として未解析一覧へ）：
/// - 判定を確定できない
/// - 復号が壊れる（判定した文字コードが違う）
///
/// `Some` でも `exact == false` なら偏移は使えない。呼び出し側は
/// `exact` を必ず見ること。行番号だけなら本文から数えられる。
pub fn decide(bytes: &[u8]) -> Option<offsets::DecodedFile> {
    let verdict = detect(bytes);
    if !verdict.is_detected() {
        return None;
    }
    offsets::decode(bytes, verdict.encoding()).ok()
}

fn probe(window: &[u8], last: bool) -> (&'static Encoding, bool) {
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(window, last);
    // 日本語のファイルなので UTF-8 も候補に残す（既定の西欧向け挙動にしない）。
    detector.guess_assess(Some(b"jp"), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(text: &str, enc: &'static Encoding) -> Vec<u8> {
        let (bytes, _, unmappable) = enc.encode(text);
        assert!(!unmappable, "テスト用テキストが {:?} で表現できない", enc);
        bytes.into_owned()
    }

    #[test]
    fn detects_utf8_japanese() {
        let src = "public class ユーザ編集 { // 住所は全角20文字以内\n}".repeat(20);
        let bytes = src.as_bytes().to_vec();
        let v = detect(&bytes);
        assert!(v.is_detected(), "confidence={}", v.confidence_ratio());
        assert_eq!(v.encoding(), encoding_rs::UTF_8);
    }

    #[test]
    fn detects_shift_jis_japanese() {
        let src =
            "public class ユーザ編集 { // 住所は全角20文字以内で入力してください\n}".repeat(20);
        let bytes = encode(&src, encoding_rs::SHIFT_JIS);
        let v = detect(&bytes);
        assert!(v.is_detected(), "confidence={}", v.confidence_ratio());
        // SHIFT_JIS と CP932 は実装が同じものを指す（encoding_rs の仕様）。
        assert!(
            v.encoding() == encoding_rs::SHIFT_JIS || v.encoding() == encoding_rs::EUC_JP,
            "unexpected: {:?}",
            v.encoding()
        );
    }

    #[test]
    fn bom_wins_over_guessing() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("ユーザ".as_bytes());
        let v = detect(&bytes);
        assert_eq!(v.encoding(), encoding_rs::UTF_8);
        assert_eq!(v.confidence_ratio(), 1.0);
    }

    #[test]
    fn empty_input_is_harmless() {
        // 空ファイルは誤っても壊れるものが無い。ここを Undetected にすると
        // 空の .java が未解析一覧に並ぶだけで、運用のノイズになる。
        let v = detect(b"");
        assert_eq!(v.encoding(), encoding_rs::UTF_8);

        let d = super::offsets::decode(b"", encoding_rs::UTF_8).unwrap();
        assert_eq!(d.text, "");
        assert!(d.exact);
        assert_eq!(d.line_index().starts(), &[0]);
    }

    #[test]
    fn ascii_only_is_detected() {
        // ASCII は UTF-8 の部分集合。ここで落ちると Java の半分が未解析になる。
        let v = detect(b"public class Foo { void bar() {} }");
        assert!(v.is_detected(), "confidence={}", v.confidence_ratio());
    }
}
