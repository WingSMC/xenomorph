use serde::{Deserialize, Serialize};

/// A half-open range in the UTF-8 source owned by the enclosing module.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl TextRange {
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start: u32::try_from(start).expect("source exceeds 4 GiB"),
            end: u32::try_from(end).expect("source exceeds 4 GiB"),
        }
    }

    pub fn text(self, source: &str) -> &str {
        source
            .get(self.start as usize..self.end as usize)
            .expect("source range must belong to the guarded module")
    }

    pub fn len(self) -> u32 {
        self.end - self.start
    }
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
    pub fn line(self, source: &str) -> u32 {
        source[..self.start as usize]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count() as u32
    }
    pub fn column(self, source: &str) -> u32 {
        let prefix = &source[..self.start as usize];
        prefix[prefix.rfind('\n').map_or(0, |index| index + 1)..]
            .chars()
            .count() as u32
    }
}

/// Token data contains no source pointers. Resolve it with the enclosing module's source.
pub type TokenData = TextRange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDocument {
    text: String,
    line_starts: Vec<u32>,
}

impl SourceDocument {
    pub fn new(text: String) -> Self {
        assert!(text.len() <= u32::MAX as usize, "source exceeds 4 GiB");
        let mut line_starts = vec![0];
        line_starts.extend(
            text.match_indices('\n')
                .map(|(index, _)| (index + 1) as u32),
        );
        Self { text, line_starts }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn line_starts(&self) -> &[u32] {
        &self.line_starts
    }

    pub fn position(&self, offset: u32, utf16: bool) -> (u32, u32) {
        let line = self
            .line_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        let start = self.line_starts[line] as usize;
        let content = self.line_content(line).unwrap();
        // Both bytes of CRLF map to the end of the line's content.
        let prefix = &self.text[start..(offset as usize).min(start + content.len())];
        let column = if utf16 {
            prefix.encode_utf16().count()
        } else {
            prefix.chars().count()
        };
        (line as u32, column as u32)
    }

    pub fn offset(&self, line: u32, column: u32, utf16: bool) -> Option<u32> {
        let start = *self.line_starts.get(line as usize)? as usize;
        let line = self.line_content(line as usize)?;
        let mut units = 0;
        for (index, character) in line.char_indices() {
            if units == column {
                return Some((start + index) as u32);
            }
            units += if utf16 {
                character.len_utf16() as u32
            } else {
                1
            };
            if units > column {
                return None;
            }
        }
        (units == column).then_some((start + line.len()) as u32)
    }

    fn line_content(&self, line: usize) -> Option<&str> {
        let start = *self.line_starts.get(line)? as usize;
        let end = self
            .line_starts
            .get(line + 1)
            .map_or(self.text.len(), |offset| *offset as usize);
        Some(self.text[start..end].trim_end_matches(['\r', '\n']))
    }
}

impl std::fmt::Display for TextRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bytes {}..{}", self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_positions_roundtrip_across_crlf_and_multiline_ranges() {
        let document = SourceDocument::new("a\u{e9}\u{1f600}\r\nsecond\n".into());
        assert_eq!(document.line_starts(), &[0, 9, 16]);
        assert_eq!(document.position(7, false), (0, 3));
        assert_eq!(document.position(7, true), (0, 4));
        assert_eq!(document.offset(0, 3, true), None);
        assert_eq!(document.offset(0, 4, true), Some(7));
        assert_eq!(document.offset(0, 5, true), None);
        assert_eq!(document.offset(3, 0, true), None);
        for offset in [0, 1, 3, 7, 9, 10, 15, 16] {
            for utf16 in [false, true] {
                let (line, column) = document.position(offset, utf16);
                assert_eq!(document.offset(line, column, utf16), Some(offset));
            }
        }
        assert_eq!(
            TextRange::new(1, 15).text(document.text()),
            "\u{e9}\u{1f600}\r\nsecond"
        );
    }

    #[test]
    fn line_ending_positions_stay_within_content_columns() {
        for content in ["", "a\u{e9}\u{1f600}"] {
            for ending in ["\n", "\r\n"] {
                let document = SourceDocument::new(format!("{content}{ending}next"));
                for utf16 in [false, true] {
                    let column = if utf16 {
                        content.encode_utf16().count()
                    } else {
                        content.chars().count()
                    } as u32;
                    for offset in content.len()..content.len() + ending.len() {
                        assert_eq!(document.position(offset as u32, utf16), (0, column));
                    }
                    assert_eq!(
                        document.offset(0, column, utf16),
                        Some(content.len() as u32)
                    );
                    assert_eq!(document.offset(0, column + 1, utf16), None);
                    assert_eq!(
                        document.position((content.len() + ending.len()) as u32, utf16),
                        (1, 0)
                    );
                }
            }
        }
    }
}
