//! A1 形式のセル参照。
//!
//! 内部では 0 始まりの (row, col) で持つ。表示や locator では A1 に戻す。

/// 0 始まりのセル位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    pub fn new(row: u32, col: u32) -> Self {
        Self { row, col }
    }

    /// `A1` 形式へ。locator（`Sheet!A1`）に使う。
    pub fn to_a1(self) -> String {
        let mut col = self.col + 1;
        let mut letters = String::new();
        while col > 0 {
            let rem = ((col - 1) % 26) as u8;
            letters.insert(0, (b'A' + rem) as char);
            col = (col - 1) / 26;
        }
        format!("{letters}{}", self.row + 1)
    }
}

impl std::fmt::Display for CellRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_a1())
    }
}

/// `A1` / `$A$1` を読む。壊れていれば `None`。
pub fn parse_cell(s: &str) -> Option<CellRef> {
    let s = s.trim();
    let mut col = 0u32;
    let mut row = 0u32;
    let mut seen_digit = false;

    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' if !seen_digit => {
                col = col.checked_mul(26)? + (ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
            }
            '0'..='9' => {
                seen_digit = true;
                row = row.checked_mul(10)? + (ch as u32 - '0' as u32);
            }
            '$' => {}
            _ => return None,
        }
    }

    if col == 0 || !seen_digit || row == 0 {
        return None;
    }
    Some(CellRef::new(row - 1, col - 1))
}

/// `A1:C3` を両端に分解する。
pub fn parse_range(s: &str) -> Option<(CellRef, CellRef)> {
    let (a, b) = s.split_once(':')?;
    Some((parse_cell(a)?, parse_cell(b)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_single_cells() {
        for (row, col, text) in [
            (0u32, 0u32, "A1"),
            (0, 25, "Z1"),
            (0, 26, "AA1"),
            (9, 27, "AB10"),
            (99, 701, "ZZ100"),
            (0, 702, "AAA1"),
        ] {
            let c = CellRef::new(row, col);
            assert_eq!(c.to_a1(), text);
            assert_eq!(parse_cell(text), Some(c));
        }
    }

    #[test]
    fn accepts_absolute_references() {
        assert_eq!(parse_cell("$B$2"), Some(CellRef::new(1, 1)));
        assert_eq!(parse_cell("B2"), Some(CellRef::new(1, 1)));
    }

    #[test]
    fn rejects_garbage() {
        for s in ["", "1", "A", "A0", ":A1", "A1B2", "あ1"] {
            assert_eq!(parse_cell(s), None, "{s:?} を通してしまった");
        }
    }

    #[test]
    fn parses_ranges() {
        assert_eq!(
            parse_range("A1:C3"),
            Some((CellRef::new(0, 0), CellRef::new(2, 2)))
        );
    }
}
