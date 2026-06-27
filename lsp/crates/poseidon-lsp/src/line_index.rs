//! Byte-offset ↔ LSP `Position` conversion (UTF-16, the default LSP encoding).

use tower_lsp::lsp_types::Position;

pub struct LineIndex {
    /// Byte offset of the start of each line.
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    /// Convert a byte offset to a `Position` (line, UTF-16 character).
    pub fn position(&self, text: &str, offset: usize) -> Position {
        let offset = offset.min(text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(l) => l,
            Err(l) => l - 1,
        };
        let line_start = self.line_starts[line];
        let col16 = text[line_start..offset].encode_utf16().count() as u32;
        Position { line: line as u32, character: col16 }
    }

    /// Convert a `Position` back to a byte offset.
    pub fn offset(&self, text: &str, pos: Position) -> usize {
        let line = pos.line as usize;
        if line >= self.line_starts.len() {
            return text.len();
        }
        let line_start = self.line_starts[line];
        let line_end = self
            .line_starts
            .get(line + 1)
            .map(|&e| e)
            .unwrap_or(text.len());
        let mut utf16 = 0u32;
        for (byte_idx, ch) in text[line_start..line_end].char_indices() {
            if utf16 >= pos.character {
                return line_start + byte_idx;
            }
            utf16 += ch.len_utf16() as u32;
        }
        line_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_uses_lines_and_utf16_columns() {
        let text = "ab\ncde\nf";
        let li = LineIndex::new(text);
        // byte 0 = (0,0); byte 3 (start of line 2) = (1,0); byte 7 ('f') = (2,0)
        assert_eq!(li.position(text, 0), Position { line: 0, character: 0 });
        assert_eq!(li.position(text, 3), Position { line: 1, character: 0 });
        assert_eq!(li.position(text, 5), Position { line: 1, character: 2 });
        assert_eq!(li.position(text, 7), Position { line: 2, character: 0 });
    }

    #[test]
    fn offset_round_trips_position() {
        let text = "hint \"x\";\n_y = 1;";
        let li = LineIndex::new(text);
        for off in 0..text.len() {
            let p = li.position(text, off);
            assert_eq!(li.offset(text, p), off, "round-trip failed at byte {off}");
        }
    }

    #[test]
    fn utf16_columns_for_astral_chars() {
        // an emoji is 1 char but 2 UTF-16 code units (LSP counts UTF-16)
        let text = "a😀b";
        let li = LineIndex::new(text);
        let b = text.find('b').unwrap();
        assert_eq!(li.position(text, b), Position { line: 0, character: 3 }); // a(1) + 😀(2)
        assert_eq!(li.offset(text, Position { line: 0, character: 3 }), b);
    }
}
